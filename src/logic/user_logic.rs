use crate::context::UserContext;
use crate::errors::app_error::AppError;
use crate::models::database_models::UserRow;
use crate::models::user::{
    AuthResponse, Claims, FacebookLoginRequest, FacebookUser, GoogleIdToken, GoogleLoginRequest,
    UserInfo, UserRole,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ============================================================================
// Google JWKS cache
// ============================================================================
//
// Google's signing keys rotate roughly once per day and the response carries
// `Cache-Control: max-age=21600` (~6h). Re-fetching them on every signin
// meant every signin was bottlenecked on a fresh HTTPS round-trip to
// googleapis.com, and any blip in Google's availability killed signin
// entirely.
//
// We honor the response's `Cache-Control: max-age` when present, capped at
// `JWKS_MAX_TTL` so a misbehaving upstream can't pin us to a stale JWKS
// for days. If the header is missing or unparseable we fall back to
// `JWKS_FALLBACK_TTL` (1h) — conservative enough that a key rotation is
// picked up within an hour.
//
// Edge case: Google rotates keys mid-TTL. The incoming token's `kid` won't
// match anything in our cached JWKS. The verifier handles this by forcing
// one refresh and retrying the `kid` lookup. If it still doesn't match,
// the token really is signed by a key Google doesn't expose — reject.

/// Access-token TTL in seconds (15 min). RFC 9700 §2.2.2 recommends
/// 5–30 minutes for OAuth-style access tokens paired with refresh-token
/// rotation; 15 min is a sweet spot — short enough that a stolen token's
/// blast radius is small, long enough that the frontend doesn't have to
/// rotate every few requests.
///
/// Paired with `RefreshTokenLogic`'s 30-day refresh tokens. Bumping
/// either value should be done together: a very short access token
/// against a long refresh token amplifies rotation churn; a long access
/// token against a short refresh token defeats the rotation benefit.
pub(crate) const ACCESS_TOKEN_TTL_SECS: u64 = 15 * 60;

const JWKS_FALLBACK_TTL: Duration = Duration::from_secs(3600);
/// Upper bound on the TTL we'll honor from a Cache-Control header. Even
/// if Google ever publishes `max-age=2592000` (30 days), we'd refuse to
/// cache that long — keys could rotate well within the window and a
/// kid-miss force-refresh would handle the gap, but stale rotated-out
/// keys for a month feels like a bad failure mode.
const JWKS_MAX_TTL: Duration = Duration::from_secs(24 * 60 * 60); // 1 day

struct JwksCacheEntry {
    jwks: Arc<serde_json::Value>,
    fetched_at: Instant,
    /// TTL computed from the response's `Cache-Control: max-age` at fetch
    /// time, or `JWKS_FALLBACK_TTL` when the header was missing/unparseable.
    /// Stored per-entry rather than as a module constant so each fetch can
    /// honor whatever Google currently advertises.
    ttl: Duration,
}

/// Type alias so the cache parameter on `get_google_jwks_inner` is one
/// short name everywhere. Production uses the module-level static via
/// `jwks_cache()`; tests construct local instances so they don't share
/// state across parallel test runs.
type JwksCache = RwLock<Option<JwksCacheEntry>>;

static JWKS_CACHE: OnceLock<JwksCache> = OnceLock::new();

fn jwks_cache() -> &'static JwksCache {
    JWKS_CACHE.get_or_init(|| RwLock::new(None))
}

/// Network boundary for the JWKS fetch. Pulling the HTTP call behind a
/// trait lets `get_google_jwks_inner`'s cache-hit / cache-miss /
/// force-refresh paths run in tests against a hand-rolled fake without
/// spinning up a real HTTP server. Production uses
/// `ReqwestJwksFetcher::google()`.
///
/// The fetch returns the parsed JWKS body and the raw Cache-Control
/// header value (when present), so the cache layer can apply the
/// per-response TTL.
#[async_trait::async_trait]
pub(crate) trait JwksFetcher: Send + Sync {
    async fn fetch(&self) -> Result<(serde_json::Value, Option<String>), AppError>;
}

/// Production JWKS fetcher: real HTTPS GET against Google's endpoint.
pub(crate) struct ReqwestJwksFetcher {
    url: String,
}

impl ReqwestJwksFetcher {
    pub(crate) fn google() -> Self {
        Self {
            url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl JwksFetcher for ReqwestJwksFetcher {
    async fn fetch(&self) -> Result<(serde_json::Value, Option<String>), AppError> {
        let client = reqwest::Client::new();
        let resp = client.get(&self.url).send().await.map_err(|e| {
            tracing::error!("Failed to fetch Google JWKS: {:?}", e);
            AppError::InternalError("Failed to verify Google token upstream".to_string())
        })?;

        // Read Cache-Control before `.json()` consumes the response.
        let cache_control = resp
            .headers()
            .get("cache-control")
            .and_then(|h| h.to_str().ok())
            .map(String::from);

        let jwks: serde_json::Value = resp.json().await.map_err(|e| {
            tracing::error!("Failed to parse Google JWKS: {:?}", e);
            AppError::InternalError("Failed to verify Google token upstream".to_string())
        })?;

        Ok((jwks, cache_control))
    }
}

/// Returns true if `entry` is present and not yet TTL-expired. Pure function
/// of the inputs so it's testable without a clock injection. Each entry
/// carries its own TTL (computed from Google's `Cache-Control: max-age`
/// at fetch time), so the freshness check has to read it off the entry
/// rather than from a module constant.
fn is_cache_fresh(entry: &Option<JwksCacheEntry>, now: Instant) -> bool {
    entry
        .as_ref()
        .map(|e| now.duration_since(e.fetched_at) < e.ttl)
        .unwrap_or(false)
}

/// Parse a `Cache-Control` header value and extract the `max-age` directive
/// as a `Duration`. Returns `None` if `max-age` is absent, isn't a valid
/// non-negative integer, or if any other parse trouble appears.
///
/// Caps the result at `JWKS_MAX_TTL` so a misbehaving upstream can't pin
/// us to a multi-day stale JWKS even if it advertised `max-age=99999999`.
///
/// Pure function — testable against literal header strings without a
/// network round-trip.
pub(crate) fn parse_cache_control_max_age(header: &str) -> Option<Duration> {
    // Cache-Control values are comma-separated directives. Split, trim,
    // and find one that starts with `max-age=`.
    for raw_directive in header.split(',') {
        let directive = raw_directive.trim();
        // Directive names are case-insensitive per RFC 9111. Compare
        // lowercased prefix; values after `=` are caller-supplied numbers
        // so case doesn't matter there.
        if let Some(value) = directive
            .strip_prefix("max-age=")
            .or_else(|| directive.strip_prefix("Max-Age="))
            .or_else(|| {
                // One more pass for arbitrary casing without allocating —
                // the two common forms above cover ~all real responses.
                let lower = directive.to_ascii_lowercase();
                if lower.starts_with("max-age=") {
                    // Re-slice the original (preserving the value's case)
                    // by length of the prefix.
                    Some(&directive["max-age=".len()..])
                } else {
                    None
                }
            })
        {
            let seconds: u64 = value.trim().parse().ok()?;
            let raw = Duration::from_secs(seconds);
            return Some(raw.min(JWKS_MAX_TTL));
        }
    }
    None
}

/// Look up an RSA `(n, e)` modulus/exponent pair for the given `kid` inside
/// a JWKS document. Returns None if `keys` is missing/malformed or no entry
/// matches the kid. Pure function — easy to test against fixture JWKS.
fn extract_rsa_components(jwks: &serde_json::Value, kid: &str) -> Option<(String, String)> {
    let key = jwks["keys"].as_array()?.iter().find(|k| k["kid"] == kid)?;
    Some((
        key["n"].as_str()?.to_string(),
        key["e"].as_str()?.to_string(),
    ))
}

/// Production entry-point. Fetches against Google's real JWKS endpoint
/// and shares one process-wide cache. Tests should call
/// `get_google_jwks_inner` with their own fetcher + cache instead.
async fn get_google_jwks(force_refresh: bool) -> Result<Arc<serde_json::Value>, AppError> {
    get_google_jwks_inner(jwks_cache(), &ReqwestJwksFetcher::google(), force_refresh).await
}

/// Cache+fetch coordination, separated from the global cache and the
/// reqwest client so tests can drive each path (cache hit / cache miss /
/// force-refresh / stale-entry / Cache-Control-honored TTL) against
/// hand-rolled fakes without HTTP or shared global state. The `cache`
/// and `fetcher` parameters carry no defaults — the production wrapper
/// above supplies both.
async fn get_google_jwks_inner(
    cache: &JwksCache,
    fetcher: &dyn JwksFetcher,
    force_refresh: bool,
) -> Result<Arc<serde_json::Value>, AppError> {
    if !force_refresh {
        let guard = cache
            .read()
            .map_err(|_| AppError::InternalError("JWKS cache lock poisoned".to_string()))?;
        if is_cache_fresh(&guard, Instant::now()) {
            // Safe to unwrap: `is_cache_fresh` already confirmed `Some(_)`.
            return Ok(guard.as_ref().unwrap().jwks.clone());
        }
    }

    let (jwks, cache_control) = fetcher.fetch().await?;

    // Honor the response's `Cache-Control: max-age` when present; fall
    // back to the conservative 1h default when missing/unparseable. The
    // parser caps at `JWKS_MAX_TTL` so a misbehaving upstream can't
    // extend us indefinitely.
    let ttl = cache_control
        .as_deref()
        .and_then(parse_cache_control_max_age)
        .unwrap_or(JWKS_FALLBACK_TTL);

    let arc_jwks = Arc::new(jwks);
    {
        let mut guard = cache
            .write()
            .map_err(|_| AppError::InternalError("JWKS cache lock poisoned".to_string()))?;
        *guard = Some(JwksCacheEntry {
            jwks: arc_jwks.clone(),
            fetched_at: Instant::now(),
            ttl,
        });
    }
    Ok(arc_jwks)
}

pub struct UserLogic {
    context: UserContext,
}

impl UserLogic {
    pub fn new(context: UserContext) -> Self {
        Self { context }
    }

    pub async fn verify_google_login(
        &self,
        payload: &GoogleLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        let google_claims =
            Self::verify_google_id_token(&payload.credential, payload.nonce.as_deref()).await?;

        // Reject unverified-email tokens. Google normally requires email
        // verification before issuing accounts; tokens with this flag false
        // shouldn't be trusted to identify the underlying email address.
        if !google_claims.email_verified {
            return Err(AppError::Unauthorized(
                "Google account email is not verified".to_string(),
            ));
        }

        let exists = self
            .context
            .user_exists(&google_claims.sub, "google")
            .await?;

        if !exists {
            return Err(AppError::Conflict(
                "An account with these credentials does not exist. Please create an account instead."
                    .to_string(),
            ));
        }

        let user_data = self
            .context
            .find_by_oauth(&google_claims.sub, "google")
            .await?;

        // Create your own JWT
        let jwt = self.create_jwt_for_user(
            &user_data.id,
            &google_claims.email,
            &user_data.role,
            &user_data.user_name,
        )?;

        Ok(AuthResponse {
            token: jwt,
            // UserLogic builds the access JWT only. The paired refresh
            // token is issued at the route layer (see `routes::auth`)
            // so this module stays decoupled from RefreshTokenLogic.
            refresh_token: None,
            user: UserInfo {
                id: user_data.id,
                email: google_claims.email,
                name: google_claims.name,
                user_name: Some(user_data.user_name),
                picture_url: google_claims.picture,
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: google_claims.sub,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    pub async fn verify_google_account_creation(
        &self,
        payload: &GoogleLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        let google_claims =
            Self::verify_google_id_token(&payload.credential, payload.nonce.as_deref()).await?;

        if !google_claims.email_verified {
            return Err(AppError::Unauthorized(
                "Google account email is not verified".to_string(),
            ));
        }

        let exists = self
            .context
            .user_exists(&google_claims.sub, "google")
            .await?;

        // if the user does not exist, create the user
        if exists {
            return Err(AppError::Conflict(
                "An account with these credentials already exists. Please use login instead."
                    .to_string(),
            ));
        }

        let user_data = self
            .context
            .create_user(
                &google_claims.sub,
                "google",
                google_claims.name.as_deref().unwrap_or("Unknown"),
                Some(google_claims.email.clone()),
                google_claims.picture.clone(),
                google_claims.email_verified,
            )
            .await?;

        // Create your own JWT
        let jwt = self.create_jwt_for_user(
            &user_data.id,
            &google_claims.email,
            &user_data.role,
            &user_data.user_name,
        )?;

        Ok(AuthResponse {
            token: jwt,
            // UserLogic builds the access JWT only. The paired refresh
            // token is issued at the route layer (see `routes::auth`)
            // so this module stays decoupled from RefreshTokenLogic.
            refresh_token: None,
            user: UserInfo {
                id: user_data.id,
                email: google_claims.email,
                name: google_claims.name,
                user_name: Some(user_data.user_name),
                picture_url: google_claims.picture,
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: google_claims.sub,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    async fn verify_google_id_token(
        id_token: &str,
        expected_nonce: Option<&str>,
    ) -> Result<GoogleIdToken, AppError> {
        let google_client_id = std::env::var("GOOGLE_CLIENT_ID")
            .map_err(|_| AppError::InternalError("GOOGLE_CLIENT_ID not configured".to_string()))?;

        // Token decoding errors are user-facing auth failures.
        let header = decode_header(id_token).map_err(|e| {
            tracing::warn!("Google token header decode failed: {:?}", e);
            AppError::Unauthorized("Invalid Google token".to_string())
        })?;
        let kid = header
            .kid
            .ok_or_else(|| AppError::Unauthorized("Token has no key id".to_string()))?;

        // Try the cached JWKS first. If the kid isn't present, the cache may
        // be stale across a Google key rotation — force-refresh once and
        // retry. If it's still missing, the token is signed by a key Google
        // doesn't currently advertise — reject as Unauthorized rather than
        // looping forever.
        let jwks = get_google_jwks(false).await?;
        let (n, e) = match extract_rsa_components(&jwks, &kid) {
            Some(pair) => pair,
            None => {
                tracing::info!(kid = %kid, "kid not found in cached JWKS; force-refreshing");
                let refreshed = get_google_jwks(true).await?;
                extract_rsa_components(&refreshed, &kid)
                    .ok_or_else(|| AppError::Unauthorized("Signing key not found".to_string()))?
            }
        };

        let decoding_key = DecodingKey::from_rsa_components(&n, &e).map_err(|e| {
            tracing::error!("Failed to construct decoding key: {:?}", e);
            AppError::InternalError("Failed to construct verification key".to_string())
        })?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&google_client_id]);
        // Google issues tokens with either form of the `iss` claim — both
        // are documented as valid. Reject if it matches neither.
        validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);

        let token_data =
            decode::<GoogleIdToken>(id_token, &decoding_key, &validation).map_err(|e| {
                tracing::warn!("Google token verification failed: {:?}", e);
                AppError::Unauthorized("Invalid Google token".to_string())
            })?;

        verify_nonce(&token_data.claims.nonce, expected_nonce)?;

        Ok(token_data.claims)
    }

    // ========================================================================
    // Facebook OAuth
    // ========================================================================
    //
    // Facebook tokens are opaque (not JWTs) — they can only be verified by
    // round-tripping through Graph API. We use the user access token to call
    // `/me`; if Facebook returns a valid user payload, the token is valid.
    //
    // Unlike Google's ID-token flow, Facebook doesn't sign a verifiable
    // assertion — the security model relies on Facebook's own backend
    // rejecting stolen/invalidated tokens. A hardening upgrade (queued in
    // the backend ROADMAP under "Auth & product surface") is to additionally
    // call `/debug_token` with our app credentials to verify the token
    // belongs to *our* Facebook app, defending against tokens minted for
    // some other app being replayed against ours. Not implemented yet — for
    // a low-security app the `/me` round-trip is sufficient.

    pub async fn verify_facebook_login(
        &self,
        payload: &FacebookLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        let fb_user = Self::verify_facebook_access_token(&payload.access_token).await?;

        // Facebook doesn't expose an `email_verified` signal on /me. We
        // require *some* email to be present (a Graph response without one
        // means the user signed up by phone or revoked email permission) —
        // the absence is treated the same as Google's unverified-email
        // rejection: a viable account needs a contact address.
        let email = fb_user.email.clone().ok_or_else(|| {
            AppError::Unauthorized(
                "Facebook account did not return an email address. \
                 Grant the email permission and try again, or use Google sign-in."
                    .to_string(),
            )
        })?;

        let exists = self.context.user_exists(&fb_user.id, "facebook").await?;
        if !exists {
            return Err(AppError::Conflict(
                "An account with these credentials does not exist. \
                 Please create an account instead."
                    .to_string(),
            ));
        }

        let user_data = self.context.find_by_oauth(&fb_user.id, "facebook").await?;

        let jwt =
            self.create_jwt_for_user(&user_data.id, &email, &user_data.role, &user_data.user_name)?;

        Ok(AuthResponse {
            token: jwt,
            // UserLogic builds the access JWT only. The paired refresh
            // token is issued at the route layer (see `routes::auth`)
            // so this module stays decoupled from RefreshTokenLogic.
            refresh_token: None,
            user: UserInfo {
                id: user_data.id,
                email,
                name: fb_user.name,
                user_name: Some(user_data.user_name),
                picture_url: fb_user.picture.map(|p| p.data.url),
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: fb_user.id,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    pub async fn verify_facebook_account_creation(
        &self,
        payload: &FacebookLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        let fb_user = Self::verify_facebook_access_token(&payload.access_token).await?;

        let email = fb_user.email.clone().ok_or_else(|| {
            AppError::Unauthorized(
                "Facebook account did not return an email address. \
                 Grant the email permission and try again, or use Google sign-in."
                    .to_string(),
            )
        })?;

        let exists = self.context.user_exists(&fb_user.id, "facebook").await?;
        if exists {
            return Err(AppError::Conflict(
                "An account with these credentials already exists. \
                 Please use login instead."
                    .to_string(),
            ));
        }

        let picture_url = fb_user.picture.as_ref().map(|p| p.data.url.clone());

        // Facebook doesn't distinguish "email_verified" — see the comment
        // above on /me. Pass `true` because the user has authenticated
        // against Facebook itself (which gated on email control during
        // signup); the value reflects "Facebook attests to this email" not
        // "we re-verified it ourselves".
        let user_data = self
            .context
            .create_user(
                &fb_user.id,
                "facebook",
                fb_user.name.as_deref().unwrap_or("Unknown"),
                Some(email.clone()),
                picture_url.clone(),
                true,
            )
            .await?;

        let jwt =
            self.create_jwt_for_user(&user_data.id, &email, &user_data.role, &user_data.user_name)?;

        Ok(AuthResponse {
            token: jwt,
            // UserLogic builds the access JWT only. The paired refresh
            // token is issued at the route layer (see `routes::auth`)
            // so this module stays decoupled from RefreshTokenLogic.
            refresh_token: None,
            user: UserInfo {
                id: user_data.id,
                email,
                name: fb_user.name,
                user_name: Some(user_data.user_name),
                picture_url,
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: fb_user.id,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    /// Exchange a Facebook user access token for the underlying account
    /// details via Graph API. The request goes to `/me?fields=id,name,email,picture`;
    /// success means Facebook accepted the token. Failures (network error,
    /// Facebook returning a JSON error envelope, malformed response) all
    /// surface as `Unauthorized` so we don't leak why the token failed.
    ///
    /// **App-ownership check**: when both `FACEBOOK_APP_ID` and
    /// `FACEBOOK_APP_SECRET` are set, we also call `/debug_token` to
    /// verify the access token was minted *for our Facebook App*, not
    /// some other app that shares the same Graph API. Without that
    /// check, an attacker holding a token issued for `app A` could
    /// replay it against our `/auth/facebook/login` and impersonate
    /// the Facebook user (Graph's `/me` would happily accept any
    /// valid Facebook user-token). Dev / test envs without the secret
    /// pair skip the check with a warn log — the env-gating lets local
    /// developers keep working without the App credentials.
    ///
    /// Pinned to Graph API v19.0 — bumping the version is a one-line change.
    /// Facebook deprecates major versions roughly every two years; the
    /// `v19.0` choice matches what the frontend's `facebook-oauth-react`
    /// negotiates against as of this writing.
    async fn verify_facebook_access_token(access_token: &str) -> Result<FacebookUser, AppError> {
        if access_token.trim().is_empty() {
            return Err(AppError::Unauthorized(
                "Facebook access token is empty".to_string(),
            ));
        }

        // App-ownership check (if configured). We do this BEFORE /me so a
        // wrong-app token gets rejected without hitting our user-lookup
        // path. Skipped silently in dev when credentials are absent.
        check_facebook_app_ownership(access_token).await?;

        let client = reqwest::Client::new();
        let resp = client
            .get("https://graph.facebook.com/v19.0/me")
            .query(&[
                ("access_token", access_token),
                ("fields", "id,name,email,picture"),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Failed to call Facebook Graph API: {:?}", e);
                AppError::InternalError("Failed to verify Facebook token upstream".to_string())
            })?;

        if !resp.status().is_success() {
            // Facebook returns a structured `{"error": {...}}` envelope on
            // invalid/expired tokens, with a 400. Log the body for ops
            // visibility but surface a generic Unauthorized to the client
            // — don't leak Facebook's error codes to API consumers.
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body = %body,
                "Facebook /me rejected the access token",
            );
            return Err(AppError::Unauthorized("Invalid Facebook token".to_string()));
        }

        let fb_user: FacebookUser = resp.json().await.map_err(|e| {
            tracing::warn!("Failed to parse Facebook /me response: {:?}", e);
            AppError::Unauthorized("Invalid Facebook token".to_string())
        })?;

        // `id` should always be present on a 200 response, but defend.
        if fb_user.id.trim().is_empty() {
            return Err(AppError::Unauthorized(
                "Facebook /me returned no user id".to_string(),
            ));
        }

        Ok(fb_user)
    }

    /// Mint a fresh access JWT for a known user id. Used by the
    /// `/auth/refresh` route after rotation succeeds: we already know
    /// the user from the refresh-token row, but the access JWT needs
    /// the email / role / username pulled from the current user row so
    /// a role change applied between login and refresh actually shows
    /// up in the next token.
    ///
    /// Errors:
    /// - `Forbidden` if the user is locked out (active permanent or
    ///   temporary lockout) — same shape as `authenticate_user`, so a
    ///   future "lock this account" admin action takes effect on the
    ///   next refresh.
    /// - `NotFound` if the user no longer exists (soft-deleted between
    ///   login and refresh).
    pub async fn mint_access_token_for_user(&self, user_id: &str) -> Result<String, AppError> {
        let user = self.context.find_by_id(user_id).await?;

        if user.locked_out {
            // Mirror the authenticate_user lockout-with-TTL check so a
            // temporary lockout that has expired auto-unlocks on the
            // next refresh — matches login-time behavior.
            if let Some(until) = user.lockout_until {
                if until > Utc::now() {
                    return Err(AppError::Forbidden(format!(
                        "Account locked until {}",
                        until
                    )));
                }
                // Falls through to mint — login's auto-unlock applies
                // there; mirror it here would couple the modules. The
                // user's next *login* will clean the flag up.
            } else {
                return Err(AppError::Forbidden(
                    user.lockout_reason
                        .unwrap_or_else(|| "Account locked".to_string()),
                ));
            }
        }

        // `email` is Option on the row (signup without email is allowed
        // for some OAuth providers); fall back to the empty string so
        // the JWT still encodes — the email claim is informational, not
        // part of the auth check, and downstream code already tolerates
        // empty values there.
        let email = user.email.as_deref().unwrap_or("");
        self.create_jwt_for_user(&user.id, email, &user.role, &user.user_name)
    }

    fn create_jwt_for_user(
        &self,
        user_id: &str,
        email: &str,
        role: &str,
        user_name: &str,
    ) -> Result<String, AppError> {
        let jwt_secret = std::env::var("JWT_SECRET")
            .map_err(|_| AppError::InternalError("JWT_SECRET not configured".to_string()))?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AppError::InternalError(format!("System clock error: {}", e)))?
            .as_secs();
        // Access-token TTL: 15 min, the lower end of RFC 9700 §2.2.2's
        // 5–30 min recommended window. A stolen access token can no
        // longer authenticate for a full day; the user-facing UX is
        // covered by the refresh-token rotation flow — frontend
        // (`api_request_factory.tsx`) transparently rotates on 401.
        //
        // Pre-rotation clients (logged in before this constant
        // changed) keep their existing 24h tokens until they expire
        // naturally; subsequent logins issue the 15-min flavor with a
        // paired refresh token.
        let expiration = now_secs + ACCESS_TOKEN_TTL_SECS;
        let issued = now_secs;

        // `jti` is a fresh UUID per issued token. The auth middleware uses it
        // to look up the revocation list — without it, a logout could only
        // invalidate the whole token family (every JWT for the user), not
        // this specific token. UUID v4 is collision-safe at our issuance rate.
        //
        // `aud` identifies which service the token was issued for. Pinned
        // as a module-level constant in the middleware (`EXPECTED_AUDIENCE`)
        // so any future audience-validation change happens in one place.
        let claims = Claims {
            sub: user_id.to_string(),
            email: email.to_string(),
            // `role` arrives as a `&str` from the DB row's `role` column.
            // `from_db_string` defensively falls back to `User` on
            // anything unexpected — failing closed (least privilege) is
            // the right behavior if the column ever holds corrupted data.
            role: UserRole::from_db_string(role),
            exp: expiration as usize,
            username: user_name.to_string(),
            iat: issued as usize,
            jti: Uuid::new_v4().to_string(),
            aud: crate::custom_middleware::auth_middleware::EXPECTED_AUDIENCE.to_string(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(jwt_secret.as_bytes()),
        )
        .map_err(|e| {
            tracing::error!("JWT encode failed: {:?}", e);
            AppError::InternalError("Failed to issue authentication token".to_string())
        })
    }

    pub async fn get_all(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<UserRow>, AppError> {
        let (l, o) = crate::util::validate_pagination(limit, offset)?;
        let rows = self.context.get_all(l, o).await?;

        let events: Vec<UserRow> = rows.into_iter().collect();

        Ok(events)
    }

    pub async fn get_self(&self, id: Uuid) -> Result<UserRow, AppError> {
        self.context.find_by_id(&id.to_string()).await
    }

    pub async fn get(&self, id: Uuid) -> Result<UserRow, AppError> {
        self.context.find_by_id(&id.to_string()).await
    }
    // ========================================================================
    // User Management (Update)
    // ========================================================================

    pub async fn update_profile(
        &self,
        user_id: &str,
        user_name: Option<&str>,
        email: Option<&str>,
        timezone: Option<&str>,
        language: Option<&str>,
    ) -> Result<(), AppError> {
        // Validate email if provided
        if let Some(email) = email {
            validate_email_format(email)?;
        }

        let updated = self
            .context
            .update(user_id, user_name, email, timezone, language)
            .await?;

        if !updated {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // For the update_self handler that passes UserRow
    pub async fn update(&self, user_id: Uuid, user_data: UserRow) -> Result<(), AppError> {
        // Extract the fields you want to allow updating
        let updated = self
            .context
            .update(
                &user_id.to_string(),
                Some(&user_data.user_name),
                user_data.email.as_deref(),
                user_data.timezone.as_deref(),
                user_data.language.as_deref(),
            )
            .await?;

        if !updated {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // ========================================================================
    // User Management (Delete)
    // ========================================================================

    pub async fn delete_user(&self, user_id: &str) -> Result<(), AppError> {
        let deleted = self.context.delete(user_id).await?;

        if !deleted {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // ========================================================================
    // Security & Moderation
    // ========================================================================

    pub async fn lockout_user(
        &self,
        user_id: &str,
        reason: &str,
        until: Option<DateTime<Utc>>,
    ) -> Result<(), AppError> {
        if reason.trim().is_empty() {
            return Err(AppError::BadRequest("Lockout reason required".to_string()));
        }
        // A lockout deadline already in the past means the auth middleware
        // will treat the user as effectively unlocked (and auto-clear the
        // flag), making the call a silent no-op. Reject loudly instead.
        if let Some(until) = until
            && until <= Utc::now()
        {
            return Err(AppError::BadRequest(
                "lockout_until must be in the future".to_string(),
            ));
        }

        let locked = self.context.lockout_user(user_id, reason, until).await?;

        if !locked {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    /// Promote/demote a user. Backed by `UserContext::update_user_role`,
    /// which has the "can't demote the last SuperAdmin" guard.
    pub async fn update_role(&self, user_id: &str, role: UserRole) -> Result<(), AppError> {
        self.context.update_user_role(user_id, role).await
    }

    pub async fn unlock_user(&self, user_id: &str) -> Result<(), AppError> {
        let unlocked = self.context.unlock_user(user_id).await?;

        if !unlocked {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    pub async fn is_locked_out(&self, user_id: &str) -> Result<bool, AppError> {
        self.context.is_locked_out(user_id).await
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    pub async fn count_total_users(&self) -> Result<i64, AppError> {
        self.context.count_total().await
    }

    pub async fn count_users_by_role(&self, role: &str) -> Result<i64, AppError> {
        self.context.count_by_role(role).await
    }

    // ========================================================================
    // Validation Helpers
    // ========================================================================
}

/// Shape of `debug_token`'s `data` envelope. Facebook returns:
///   `{"data": {"app_id": "...", "is_valid": true, "user_id": "...", ...}}`
/// on success, or a top-level `{"error": {...}}` on failure. We only
/// care about `app_id` and `is_valid` for the ownership check.
#[derive(serde::Deserialize)]
struct FacebookDebugTokenData {
    app_id: Option<String>,
    is_valid: Option<bool>,
}

#[derive(serde::Deserialize)]
struct FacebookDebugTokenResponse {
    data: Option<FacebookDebugTokenData>,
}

/// Verify the access token was minted for *our* Facebook App.
///
/// Calls Graph API `/debug_token` with our app credentials
/// (`<app_id>|<app_secret>`) as the access_token. Facebook responds
/// with the input token's metadata — we assert `is_valid==true` AND
/// `app_id==FACEBOOK_APP_ID`. Skipped (with a warn log) when the env
/// pair is unset so dev / test envs without App credentials still work.
///
/// Returning `Ok(())` means either the check passed or it was skipped.
/// Returning `Err(Unauthorized)` means the check ran AND the token is
/// not minted for our app.
async fn check_facebook_app_ownership(access_token: &str) -> Result<(), AppError> {
    let app_id = match std::env::var("FACEBOOK_APP_ID") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            tracing::warn!(
                "FACEBOOK_APP_ID not configured; skipping debug_token app-ownership check. \
                 Set FACEBOOK_APP_ID + FACEBOOK_APP_SECRET in production."
            );
            return Ok(());
        }
    };
    let app_secret = match std::env::var("FACEBOOK_APP_SECRET") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            tracing::warn!(
                "FACEBOOK_APP_SECRET not configured; skipping debug_token app-ownership check. \
                 Without it, a token minted for a different Facebook App could be replayed."
            );
            return Ok(());
        }
    };

    let app_access_token = format!("{}|{}", app_id, app_secret);

    let client = reqwest::Client::new();
    let resp = client
        .get("https://graph.facebook.com/v19.0/debug_token")
        .query(&[
            ("input_token", access_token),
            ("access_token", app_access_token.as_str()),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to call Facebook debug_token: {:?}", e);
            AppError::InternalError("Failed to verify Facebook token upstream".to_string())
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %body,
            "Facebook debug_token rejected the request",
        );
        return Err(AppError::Unauthorized("Invalid Facebook token".to_string()));
    }

    let envelope: FacebookDebugTokenResponse = resp.json().await.map_err(|e| {
        tracing::warn!("Failed to parse Facebook debug_token response: {:?}", e);
        AppError::Unauthorized("Invalid Facebook token".to_string())
    })?;

    let data = envelope.data.ok_or_else(|| {
        tracing::warn!("Facebook debug_token returned no `data` block");
        AppError::Unauthorized("Invalid Facebook token".to_string())
    })?;

    if !data.is_valid.unwrap_or(false) {
        tracing::warn!("Facebook debug_token reports is_valid=false");
        return Err(AppError::Unauthorized("Invalid Facebook token".to_string()));
    }

    match data.app_id.as_deref() {
        Some(returned_app_id) if returned_app_id == app_id => Ok(()),
        Some(other) => {
            tracing::warn!(
                token_app_id = %other,
                expected_app_id = %app_id,
                "Facebook token was minted for a different App — rejecting",
            );
            Err(AppError::Unauthorized(
                "Facebook token was not issued for this app".to_string(),
            ))
        }
        None => {
            tracing::warn!("Facebook debug_token returned no app_id");
            Err(AppError::Unauthorized("Invalid Facebook token".to_string()))
        }
    }
}

/// Verify the OIDC `nonce` round-trip per OIDC Core §3.1.2.1.
///
/// The acceptance matrix balances strict security with rollout
/// tolerance:
///
///   | request nonce | token nonce | result                |
///   |---------------|-------------|-----------------------|
///   | Some(a)       | Some(a)     | accept                |
///   | Some(a)       | Some(b)     | reject — replay shape |
///   | Some(_)       | None        | reject — client lied  |
///   | None          | Some(_)     | reject — client lied  |
///   | None          | None        | accept — legacy flow  |
///
/// The legacy-accept path (neither side) exists so a frontend that
/// hasn't rolled out the nonce wiring yet still authenticates. Drop
/// this carve-out once the frontend is fully migrated (same pattern
/// as the jti/aud carve-outs that were dropped after their TTL window).
pub(crate) fn verify_nonce(
    token_nonce: &Option<String>,
    expected_nonce: Option<&str>,
) -> Result<(), AppError> {
    match (token_nonce.as_deref(), expected_nonce) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(a), Some(b)) => {
            tracing::warn!(
                token_nonce = %a,
                expected = %b,
                "OIDC nonce mismatch — rejecting token",
            );
            Err(AppError::Unauthorized(
                "Token nonce does not match the request".to_string(),
            ))
        }
        (Some(_), None) => {
            tracing::warn!("Token carries a nonce but request omitted it — rejecting",);
            Err(AppError::Unauthorized(
                "Request is missing the OIDC nonce that the token carries".to_string(),
            ))
        }
        (None, Some(_)) => {
            tracing::warn!("Request sent a nonce but token has none — rejecting",);
            Err(AppError::Unauthorized(
                "Token is missing the OIDC nonce the request claimed".to_string(),
            ))
        }
        (None, None) => Ok(()),
    }
}

/// Validate that `email` looks like a usable email address. Hand-rolled
/// (no `regex` dep for one use case) to catch the common malformations the
/// old `contains('@') && contains('.')` check let through:
///   - leading/trailing whitespace
///   - empty local part: `@example.com`
///   - empty domain part: `user@`
///   - missing TLD: `user@domain`
///   - leading/trailing dots in either half: `.user@x.com`, `user.@x.com`
///   - consecutive dots: `user..name@x.com`
///   - bad chars: spaces, control bytes
///
/// Intentionally NOT full RFC 5322 — that grammar is famously hard to get
/// right and most real-world addresses fit the strict-but-narrower shape
/// below. UTF-8 emails (RFC 6531) are rejected; if internationalized
/// addresses become a requirement, add a punycode-allowing variant.
///
/// Length bounds per RFC 5321: whole address ≤ 254 octets, local part
/// ≤ 64 octets. The 254 is the upper bound for SMTP transport (RFC 5321
/// §4.5.3.1.3); some sources cite 320, but 254 is the safer real-world cap.
pub(crate) fn validate_email_format(email: &str) -> Result<(), AppError> {
    // Whitespace anywhere is a bug — even a stray trailing space is
    // almost always a paste-error and never a real address.
    if email != email.trim() {
        return Err(AppError::BadRequest(
            "Email must not have leading or trailing whitespace".to_string(),
        ));
    }
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required".to_string()));
    }
    if email.len() > 254 {
        return Err(AppError::BadRequest(
            "Email is longer than 254 characters".to_string(),
        ));
    }
    // Must contain exactly one '@' — `a@b@c.com` is malformed under our
    // (strict) rules, even though some lax parsers tolerate quoted forms.
    let at_count = email.bytes().filter(|&b| b == b'@').count();
    if at_count != 1 {
        return Err(AppError::BadRequest(
            "Email must contain exactly one '@'".to_string(),
        ));
    }
    let Some((local, domain)) = email.split_once('@') else {
        // Unreachable given the at_count check above, but matches the
        // type — keep as a defensive return rather than .unwrap().
        return Err(AppError::BadRequest("Invalid email format".to_string()));
    };

    validate_email_local_part(local)?;
    validate_email_domain_part(domain)?;
    Ok(())
}

fn validate_email_local_part(local: &str) -> Result<(), AppError> {
    if local.is_empty() {
        return Err(AppError::BadRequest(
            "Email local part (before @) must not be empty".to_string(),
        ));
    }
    if local.len() > 64 {
        return Err(AppError::BadRequest(
            "Email local part exceeds 64 characters".to_string(),
        ));
    }
    if local.starts_with('.') || local.ends_with('.') {
        return Err(AppError::BadRequest(
            "Email local part must not start or end with a dot".to_string(),
        ));
    }
    if local.contains("..") {
        return Err(AppError::BadRequest(
            "Email local part must not contain consecutive dots".to_string(),
        ));
    }
    for ch in local.chars() {
        // Allowed: alphanumeric ASCII + `.`, `_`, `%`, `+`, `-`. This is
        // strict — we explicitly omit other special chars (`!`, `#`, `&`,
        // `'`, `*`, `/`, `=`, `?`, `^`, ``, `{`, `|`, `}`, `~`) that the
        // RFC technically permits but are vanishingly rare in real use
        // and frequently come from bad data.
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-');
        if !ok {
            return Err(AppError::BadRequest(format!(
                "Email local part contains invalid character: {:?}",
                ch
            )));
        }
    }
    Ok(())
}

fn validate_email_domain_part(domain: &str) -> Result<(), AppError> {
    if domain.is_empty() {
        return Err(AppError::BadRequest(
            "Email domain (after @) must not be empty".to_string(),
        ));
    }
    if domain.len() > 253 {
        return Err(AppError::BadRequest(
            "Email domain exceeds 253 characters".to_string(),
        ));
    }
    if domain.starts_with('.') || domain.ends_with('.') {
        return Err(AppError::BadRequest(
            "Email domain must not start or end with a dot".to_string(),
        ));
    }
    if domain.contains("..") {
        return Err(AppError::BadRequest(
            "Email domain must not contain consecutive dots".to_string(),
        ));
    }
    // Must have at least one dot separating a TLD.
    let Some((_, tld)) = domain.rsplit_once('.') else {
        return Err(AppError::BadRequest(
            "Email domain must contain a top-level domain (e.g. .com)".to_string(),
        ));
    };
    if tld.len() < 2 {
        return Err(AppError::BadRequest(
            "Email TLD must be at least 2 characters".to_string(),
        ));
    }
    if !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        // The actual TLD list contains only letters (`com`, `co`, `museum`).
        // Numeric TLDs don't exist; reject as a strong defense against
        // typos like `user@example.c0m`.
        return Err(AppError::BadRequest(
            "Email TLD must contain only letters".to_string(),
        ));
    }
    for label in domain.split('.') {
        if label.is_empty() {
            // Caught by the `..` check above, but defensive.
            return Err(AppError::BadRequest(
                "Email domain has empty label between dots".to_string(),
            ));
        }
        if label.len() > 63 {
            return Err(AppError::BadRequest(
                "Email domain label exceeds 63 characters".to_string(),
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(AppError::BadRequest(
                "Email domain label must not start or end with a hyphen".to_string(),
            ));
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(AppError::BadRequest(format!(
                "Email domain label contains invalid character in {:?}",
                label
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // verify_nonce — OIDC nonce round-trip enforcement
    // ========================================================================
    //
    // Pin each cell of the four-case acceptance matrix documented on
    // `verify_nonce`. A regression that flips one case (e.g. accidentally
    // accepting a missing-request-nonce when the token has one) is a real
    // security regression; these tests catch it.

    #[test]
    fn verify_nonce_accepts_matching_pair() {
        assert!(
            verify_nonce(&Some("n1".to_string()), Some("n1")).is_ok(),
            "matching nonces must accept",
        );
    }

    #[test]
    fn verify_nonce_rejects_mismatched_pair() {
        let err = verify_nonce(&Some("token-side".to_string()), Some("request-side")).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn verify_nonce_rejects_token_nonce_without_request_nonce() {
        // The frontend sent the credential but forgot to echo the nonce —
        // could be a downgrade attempt or a stale client. Reject either way.
        let err = verify_nonce(&Some("n1".to_string()), None).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn verify_nonce_rejects_request_nonce_without_token_nonce() {
        // The request claims a nonce but Google didn't issue the token with
        // one — the client is lying about which nonce it sent to Google.
        let err = verify_nonce(&None, Some("n1")).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn verify_nonce_accepts_legacy_neither_side() {
        // Backward-compat path for the rollout window. After the frontend
        // is fully migrated to always sending a nonce, this acceptance
        // should be tightened (drop the carve-out the same way the jti/aud
        // carve-outs were dropped after their TTL window).
        assert!(verify_nonce(&None, None).is_ok());
    }

    fn sample_jwks() -> serde_json::Value {
        // Two keys, the shape Google's JWKS endpoint returns. Real `n` values
        // are ~340 base64 chars; the test ones are placeholders since we only
        // exercise the kid lookup path.
        serde_json::json!({
            "keys": [
                {
                    "kid": "abc123",
                    "kty": "RSA",
                    "alg": "RS256",
                    "use": "sig",
                    "n": "modulus-of-abc",
                    "e": "AQAB"
                },
                {
                    "kid": "def456",
                    "kty": "RSA",
                    "alg": "RS256",
                    "use": "sig",
                    "n": "modulus-of-def",
                    "e": "AQAB"
                }
            ]
        })
    }

    #[test]
    fn extract_rsa_components_finds_matching_kid() {
        let jwks = sample_jwks();
        let (n, e) = extract_rsa_components(&jwks, "def456").expect("kid is present");
        assert_eq!(n, "modulus-of-def");
        assert_eq!(e, "AQAB");
    }

    // ========================================================================
    // get_google_jwks_inner — cache+fetcher integration
    // ========================================================================
    //
    // These cover the cache state-machine end-to-end. Each test owns its
    // own `JwksCache` and `FakeJwksFetcher` so parallel test execution
    // never sees shared state — the previous implementation used the
    // module-level static cache, which had to be carefully reset
    // between tests; the refactored `get_google_jwks_inner` takes both
    // as parameters so the tests are hermetic by construction.

    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    struct FakeJwksFetcher {
        body: serde_json::Value,
        cache_control: Option<String>,
        calls: AtomicUsize,
    }

    impl FakeJwksFetcher {
        fn new(body: serde_json::Value, cache_control: Option<String>) -> Self {
            Self {
                body,
                cache_control,
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(AtomicOrdering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl JwksFetcher for FakeJwksFetcher {
        async fn fetch(&self) -> Result<(serde_json::Value, Option<String>), AppError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok((self.body.clone(), self.cache_control.clone()))
        }
    }

    struct ErrorFetcher;

    #[async_trait::async_trait]
    impl JwksFetcher for ErrorFetcher {
        async fn fetch(&self) -> Result<(serde_json::Value, Option<String>), AppError> {
            Err(AppError::InternalError(
                "synthetic upstream failure".to_string(),
            ))
        }
    }

    fn fresh_cache() -> JwksCache {
        RwLock::new(None)
    }

    #[tokio::test]
    async fn jwks_cache_miss_fetches_and_populates() {
        let cache = fresh_cache();
        let fetcher = FakeJwksFetcher::new(sample_jwks(), None);

        let jwks = get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("fetch");
        assert_eq!(fetcher.call_count(), 1, "cold cache should fetch once");
        // Returned doc matches the fetcher.
        assert_eq!(*jwks, sample_jwks());
        // Cache populated for the next call.
        let guard = cache.read().unwrap();
        assert!(guard.is_some(), "cache should hold the entry");
    }

    #[tokio::test]
    async fn jwks_cache_hit_returns_cached_without_fetching() {
        let cache = fresh_cache();
        let fetcher = FakeJwksFetcher::new(sample_jwks(), None);

        // Prime the cache.
        let first = get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("prime");
        assert_eq!(fetcher.call_count(), 1);

        // Second call within TTL: must NOT fetch again.
        let second = get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("hit");
        assert_eq!(
            fetcher.call_count(),
            1,
            "cache hit must not trigger a second fetch",
        );
        assert!(Arc::ptr_eq(&first, &second), "same Arc — same cache entry");
    }

    #[tokio::test]
    async fn jwks_force_refresh_bypasses_cache() {
        let cache = fresh_cache();
        let fetcher = FakeJwksFetcher::new(sample_jwks(), None);

        // Prime.
        get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("prime");
        assert_eq!(fetcher.call_count(), 1);

        // Force refresh: fetches even though the cache is fresh.
        get_google_jwks_inner(&cache, &fetcher, true)
            .await
            .expect("refresh");
        assert_eq!(fetcher.call_count(), 2, "force_refresh=true must re-fetch",);
    }

    #[tokio::test]
    async fn jwks_stale_cache_triggers_refetch() {
        let cache = fresh_cache();
        // Pre-populate the cache with an entry whose TTL has already
        // elapsed. The fetcher should be called.
        {
            let mut guard = cache.write().unwrap();
            let stale_fetched_at = Instant::now()
                .checked_sub(JWKS_FALLBACK_TTL + Duration::from_secs(60))
                .expect("clock arithmetic");
            *guard = Some(JwksCacheEntry {
                jwks: Arc::new(serde_json::json!({"keys": []})),
                fetched_at: stale_fetched_at,
                ttl: JWKS_FALLBACK_TTL,
            });
        }

        let fetcher = FakeJwksFetcher::new(sample_jwks(), None);
        let refreshed = get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("refresh after stale");

        assert_eq!(fetcher.call_count(), 1, "stale cache must refetch");
        assert_eq!(
            *refreshed,
            sample_jwks(),
            "returned doc should be the fresh one, not the stale Arc",
        );
    }

    #[tokio::test]
    async fn jwks_honors_cache_control_max_age_for_ttl() {
        // The fetcher returns `Cache-Control: public, max-age=300, ...`
        // — the cache entry's TTL must reflect 300s, not the 1h fallback.
        let cache = fresh_cache();
        let fetcher = FakeJwksFetcher::new(
            sample_jwks(),
            Some("public, max-age=300, must-revalidate".to_string()),
        );

        get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("fetch");

        let guard = cache.read().unwrap();
        let entry = guard.as_ref().expect("cache populated");
        assert_eq!(
            entry.ttl,
            Duration::from_secs(300),
            "Cache-Control max-age should drive the entry TTL",
        );
    }

    #[tokio::test]
    async fn jwks_falls_back_to_default_ttl_when_no_cache_control() {
        // No Cache-Control header → fallback to the conservative 1h.
        let cache = fresh_cache();
        let fetcher = FakeJwksFetcher::new(sample_jwks(), None);

        get_google_jwks_inner(&cache, &fetcher, false)
            .await
            .expect("fetch");

        let guard = cache.read().unwrap();
        let entry = guard.as_ref().expect("cache populated");
        assert_eq!(
            entry.ttl, JWKS_FALLBACK_TTL,
            "missing Cache-Control → fallback TTL",
        );
    }

    #[tokio::test]
    async fn jwks_fetcher_error_does_not_poison_cache() {
        // Fetch fails on a cold cache. The cache should stay empty, and
        // a subsequent successful fetch should populate it.
        let cache = fresh_cache();
        let err_fetcher = ErrorFetcher;
        let result = get_google_jwks_inner(&cache, &err_fetcher, false).await;
        assert!(result.is_err(), "fetcher error propagates");
        {
            let guard = cache.read().unwrap();
            assert!(guard.is_none(), "failed fetch must not populate the cache",);
        }

        // Now a real fetcher: should fetch + populate.
        let ok_fetcher = FakeJwksFetcher::new(sample_jwks(), None);
        get_google_jwks_inner(&cache, &ok_fetcher, false)
            .await
            .expect("recovery fetch");
        let guard = cache.read().unwrap();
        assert!(guard.is_some(), "post-recovery cache populated");
    }

    #[test]
    fn extract_rsa_components_returns_none_for_unknown_kid() {
        let jwks = sample_jwks();
        assert!(extract_rsa_components(&jwks, "key-google-rotated-out").is_none());
    }

    #[test]
    fn extract_rsa_components_handles_malformed_response() {
        // Missing `keys` array entirely.
        let jwks = serde_json::json!({});
        assert!(extract_rsa_components(&jwks, "abc123").is_none());

        // `keys` is the wrong shape.
        let jwks = serde_json::json!({ "keys": "should-be-array" });
        assert!(extract_rsa_components(&jwks, "abc123").is_none());

        // Matching kid but missing `n` field — return None rather than panic.
        let jwks = serde_json::json!({
            "keys": [{ "kid": "abc123", "e": "AQAB" }]
        });
        assert!(extract_rsa_components(&jwks, "abc123").is_none());
    }

    #[test]
    fn is_cache_fresh_returns_false_for_empty() {
        // No entry → cache miss → must refetch.
        assert!(!is_cache_fresh(&None, Instant::now()));
    }

    #[test]
    fn is_cache_fresh_returns_true_for_recent_entry() {
        let fetched_at = Instant::now();
        let entry = Some(JwksCacheEntry {
            jwks: Arc::new(serde_json::json!({})),
            fetched_at,
            ttl: JWKS_FALLBACK_TTL,
        });
        // Same instant as fetched_at → elapsed = 0 < TTL → fresh.
        assert!(is_cache_fresh(&entry, fetched_at));
    }

    #[test]
    fn is_cache_fresh_returns_false_past_ttl() {
        let fetched_at = Instant::now();
        let entry = Some(JwksCacheEntry {
            jwks: Arc::new(serde_json::json!({})),
            fetched_at,
            ttl: JWKS_FALLBACK_TTL,
        });
        // Simulate "now" being TTL + a buffer after fetched_at.
        // Use checked_add because Instant arithmetic can saturate.
        let later = fetched_at
            .checked_add(JWKS_FALLBACK_TTL + Duration::from_secs(1))
            .expect("clock arithmetic");
        assert!(!is_cache_fresh(&entry, later));
    }

    #[test]
    fn is_cache_fresh_uses_per_entry_ttl_not_a_global_constant() {
        // Critical pin: if someone refactors this to read a module
        // constant again, entries that came in with a long max-age
        // would expire prematurely. Construct an entry with a 10-second
        // TTL and verify it's fresh just under that and stale just over.
        let fetched_at = Instant::now();
        let entry = Some(JwksCacheEntry {
            jwks: Arc::new(serde_json::json!({})),
            fetched_at,
            ttl: Duration::from_secs(10),
        });
        let nine_secs_later = fetched_at
            .checked_add(Duration::from_secs(9))
            .expect("clock arithmetic");
        assert!(is_cache_fresh(&entry, nine_secs_later));
        let eleven_secs_later = fetched_at
            .checked_add(Duration::from_secs(11))
            .expect("clock arithmetic");
        assert!(!is_cache_fresh(&entry, eleven_secs_later));
    }

    // ========================================================================
    // parse_cache_control_max_age
    // ========================================================================

    #[test]
    fn parse_cache_control_extracts_simple_max_age() {
        assert_eq!(
            parse_cache_control_max_age("max-age=600"),
            Some(Duration::from_secs(600)),
        );
    }

    #[test]
    fn parse_cache_control_extracts_from_real_google_header_shape() {
        // The shape Google actually returns — multiple directives, commas,
        // spaces. max-age is somewhere in the middle.
        assert_eq!(
            parse_cache_control_max_age("public, max-age=21600, must-revalidate, no-transform"),
            Some(Duration::from_secs(21600)),
        );
    }

    #[test]
    fn parse_cache_control_caps_at_max_ttl() {
        // 30 days >> JWKS_MAX_TTL (1 day). Result clamps.
        let thirty_days = 30 * 24 * 60 * 60;
        let parsed =
            parse_cache_control_max_age(&format!("max-age={}", thirty_days)).expect("should parse");
        assert_eq!(parsed, JWKS_MAX_TTL);
    }

    #[test]
    fn parse_cache_control_returns_none_when_max_age_absent() {
        assert_eq!(parse_cache_control_max_age("public, must-revalidate"), None,);
        assert_eq!(parse_cache_control_max_age("no-cache"), None);
        assert_eq!(parse_cache_control_max_age(""), None);
    }

    #[test]
    fn parse_cache_control_returns_none_for_unparseable_max_age() {
        // Spec says max-age is a non-negative decimal integer. Reject
        // anything else rather than guessing — fall back to JWKS_FALLBACK_TTL.
        assert_eq!(parse_cache_control_max_age("max-age=abc"), None);
        assert_eq!(parse_cache_control_max_age("max-age="), None);
        // Negative values are unparseable into u64 — also rejected.
        assert_eq!(parse_cache_control_max_age("max-age=-1"), None);
    }

    #[test]
    fn parse_cache_control_is_case_insensitive_for_directive_name() {
        // RFC 9111 §5.2: directive names are case-insensitive. Real
        // upstream responses sometimes capitalize.
        assert_eq!(
            parse_cache_control_max_age("Max-Age=600"),
            Some(Duration::from_secs(600)),
        );
        assert_eq!(
            parse_cache_control_max_age("MAX-AGE=600"),
            Some(Duration::from_secs(600)),
        );
    }

    #[test]
    fn parse_cache_control_handles_max_age_zero() {
        // `max-age=0` means "don't cache". We honor it literally —
        // freshness will fail immediately, forcing every signin to refetch.
        // Acceptable because Google wouldn't send 0 in normal operation.
        assert_eq!(
            parse_cache_control_max_age("max-age=0"),
            Some(Duration::from_secs(0)),
        );
    }

    // ========================================================================
    // validate_email_format
    // ========================================================================
    //
    // Test cases pin both the addresses the OLD validator wrongly accepted
    // (regression: `.@.`, `@.com`, `a.@b`) and the addresses real users
    // need to send (single-char locals, subdomains, +tag aliases). The
    // common-malformations table at the end is the "would the OLD impl
    // have caught this?" pin.

    #[test]
    fn email_validator_accepts_canonical_addresses() {
        for ok in [
            "a@b.co", // single-char local + minimum TLD
            "user@example.com",
            "user.name@example.com",
            "user+tag@example.com",      // +tag aliases (Gmail-style)
            "user-name@sub.example.com", // subdomain
            "USER@EXAMPLE.COM",          // case-insensitive
            "u1@x.org",
            "user%percent@example.co.uk", // multi-level TLD
        ] {
            assert!(
                validate_email_format(ok).is_ok(),
                "Should accept {:?}, got {:?}",
                ok,
                validate_email_format(ok)
            );
        }
    }

    #[test]
    fn email_validator_rejects_addresses_the_old_check_let_through() {
        // These all matched `email.contains('@') && email.contains('.')`
        // but are obviously broken. This test pins the regression behavior.
        for bad in [
            ".@.",       // empty local + empty domain
            "@.com",     // empty local
            "a@",        // empty domain
            "a@.com",    // domain starts with dot
            "a@b.",      // domain ends with dot
            ".a@b.co",   // local starts with dot
            "a.@b.co",   // local ends with dot
            "a..b@c.co", // consecutive dots in local
            "a@b..co",   // consecutive dots in domain
            "a@b",       // no TLD
            "a@b.c",     // TLD too short (1 char)
            "a@b.c0m",   // numeric TLD (typo of .com)
        ] {
            assert!(
                validate_email_format(bad).is_err(),
                "Should reject {:?}",
                bad,
            );
        }
    }

    #[test]
    fn email_validator_rejects_whitespace() {
        assert!(validate_email_format(" a@b.co").is_err());
        assert!(validate_email_format("a@b.co ").is_err());
        assert!(validate_email_format("a @b.co").is_err());
        assert!(validate_email_format("a@b .co").is_err());
        assert!(validate_email_format("a@b.c o").is_err());
        // Tabs and newlines too.
        assert!(validate_email_format("a@b.co\t").is_err());
        assert!(validate_email_format("a@b.co\n").is_err());
    }

    #[test]
    fn email_validator_rejects_multiple_at_signs() {
        // Quoted-local-part forms with multiple @ are technically RFC 5322
        // valid but extremely rare; we reject them as part of the
        // strict-but-narrower contract.
        assert!(validate_email_format("a@b@c.co").is_err());
        assert!(validate_email_format("a@@b.co").is_err());
    }

    #[test]
    fn email_validator_rejects_oversized_inputs() {
        // Whole-address cap is 254. Local cap is 64.
        let too_long = format!("{}@example.com", "a".repeat(255));
        assert!(validate_email_format(&too_long).is_err());

        let local_too_long = format!("{}@example.com", "a".repeat(65));
        assert!(validate_email_format(&local_too_long).is_err());

        // Domain label cap is 63.
        let domain_label_too_long = format!("user@{}.com", "a".repeat(64));
        assert!(validate_email_format(&domain_label_too_long).is_err());
    }

    #[test]
    fn email_validator_rejects_empty() {
        assert!(validate_email_format("").is_err());
    }

    #[test]
    fn email_validator_rejects_hyphen_at_label_boundary() {
        // Domain labels can have hyphens internally but not at edges.
        assert!(validate_email_format("a@-example.com").is_err());
        assert!(validate_email_format("a@example-.com").is_err());
        // Internal hyphens are fine.
        assert!(validate_email_format("a@my-example.com").is_ok());
    }

    #[test]
    fn email_validator_rejects_disallowed_special_chars_in_local() {
        // Strict-but-narrower: we reject the rarely-used RFC 5322 specials.
        for bad in [
            "a!@b.co", "a#@b.co", "a&@b.co", "a'@b.co", "a/@b.co", "a=@b.co", "a?@b.co", "a^@b.co",
            "a{@b.co", "a}@b.co",
        ] {
            assert!(
                validate_email_format(bad).is_err(),
                "Should reject special char in local: {:?}",
                bad,
            );
        }
    }

    #[test]
    fn email_validator_accepts_punycode_idn_domains() {
        // International domain names encoded as ACE/punycode look like
        // ASCII to our validator. The actual UTF-8 form (e.g. `用户@例え.jp`)
        // is rejected; clients must encode to punycode first.
        assert!(validate_email_format("user@xn--r8jz45g.jp").is_ok());
    }

    // ========================================================================
    // Access-token TTL — RFC 9700 §2.2.2
    // ========================================================================
    //
    // The TTL value itself is the security knob: a regression that bumped
    // it back to 24h would silently defeat the rotation flow's blast-radius
    // bound. Pin the constant + that the issued JWT's exp - iat actually
    // matches it. Decodes the token with the test JWT_SECRET so the test
    // sees exactly what a client would.

    #[test]
    fn access_token_ttl_is_fifteen_minutes() {
        assert_eq!(
            ACCESS_TOKEN_TTL_SECS,
            15 * 60,
            "RFC 9700 §2.2.2 recommends 5-30 min; we sit at 15."
        );
    }

    #[tokio::test]
    async fn issued_jwt_carries_15min_ttl() {
        // End-to-end: spin up a UserLogic over an in-memory pool, seed
        // one user, mint an access token via the public entry point,
        // decode the JWT, and assert exp - iat matches the TTL constant.
        // A regression that hard-coded `+ 3600 * 24` again would surface
        // here as `86400 != 900`.
        use sqlx::sqlite::SqlitePoolOptions;

        // SAFETY: same env var dance the rest of the user_logic tests
        // use; the value is stable across every test that sets it.
        unsafe {
            std::env::set_var("JWT_SECRET", "test-secret-do-not-use-in-prod");
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("FK off");
        sqlx::query(
            "INSERT INTO users (id, oauth_id, oauth_provider, user_name, \
                                email, email_verified, locked_out, role, \
                                created_at, updated_at) \
             VALUES ('ttl-user', 'oauth-1', 'google', 'tester', \
                     'tester@example.com', 1, 0, 'user', \
                     datetime('now'), datetime('now'))",
        )
        .execute(&pool)
        .await
        .expect("seed user");

        let logic = UserLogic::new(UserContext::new(pool));
        let token = logic
            .mint_access_token_for_user("ttl-user")
            .await
            .expect("mint");

        // Decode without validating exp so a test that runs slightly
        // past the issuance instant doesn't trip token-already-expired.
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        // The aud claim is checked against `EXPECTED_AUDIENCE` in the
        // middleware; we don't need that gate here either.
        validation.validate_aud = false;
        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(b"test-secret-do-not-use-in-prod"),
            &validation,
        )
        .expect("decode");

        let ttl = decoded.claims.exp as u64 - decoded.claims.iat as u64;
        assert_eq!(
            ttl, ACCESS_TOKEN_TTL_SECS,
            "exp - iat must match the configured TTL constant"
        );
    }
}
