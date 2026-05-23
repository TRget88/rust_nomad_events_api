// src/middleware/auth.rs
use crate::errors::AppError;
use crate::logic::JwtRevocationLogic;
use crate::models::user::{Claims, UserRole};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{DecodingKey, Validation, decode};
use sqlx::SqlitePool;
use std::sync::{Arc, OnceLock};

/// Audience expected on every well-formed token. Stamped by
/// `create_jwt_for_user` and checked by `auth_middleware`. Defends against
/// shared-secret confusion if Festurah ever adds a second service that
/// shares `JWT_SECRET` — a token minted for the API can't be silently
/// accepted by the other service and vice versa.
///
/// Pinned as a constant rather than read from an env var so the check
/// can't be silently disabled by a missing config — a typo there would
/// turn every token into a "wrong audience" rejection, which would be
/// obvious in QA, but the constant approach is "decision in code." Move
/// to env later if we ever support multiple deployments with different
/// audience names.
pub const EXPECTED_AUDIENCE: &str = "festurah-api";

/// State the auth middleware needs. Previously the middleware took just a
/// `SqlitePool`, which meant the revocation check had to inline a duplicate
/// of `JwtRevocationContext::is_revoked` — the middleware couldn't reach
/// the logic layer. Bundling both here lets the middleware go through the
/// logic layer cleanly, with one canonical query.
///
/// `Clone` is required by axum's `from_fn_with_state` — both fields are
/// cheap to clone (`SqlitePool` is an `Arc` internally, `Arc<...>` is the
/// usual reference clone).
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub pool: SqlitePool,
    pub jwt_revocation_logic: Arc<JwtRevocationLogic>,
}

// JWT signing key, derived once from JWT_SECRET on first use. Avoids the
// `env::var(...)` lookup on every request. Tests set JWT_SECRET before any
// middleware runs, so the first call inside a test still picks up the right
// value. Caveat: once initialized, it sticks for the lifetime of the process —
// tests that need to swap the secret would need a different pattern.
static JWT_DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

fn get_jwt_decoding_key() -> Result<&'static DecodingKey, AppError> {
    if let Some(key) = JWT_DECODING_KEY.get() {
        return Ok(key);
    }
    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::InternalError("JWT_SECRET not configured".to_string()))?;
    // .set() may race with another thread; only one wins but both observe
    // the same key via the subsequent .get(). The race is harmless.
    let _ = JWT_DECODING_KEY.set(DecodingKey::from_secret(secret.as_bytes()));
    Ok(JWT_DECODING_KEY
        .get()
        .expect("JWT decoding key was just initialized"))
}

// ============================================================================
// Auth Middleware - Verifies JWT, confirms user exists, handles lockout state
// ============================================================================
pub async fn auth_middleware(
    State(state): State<AuthMiddlewareState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_token(&req)?;

    let decoding_key = get_jwt_decoding_key()?;
    // Disable jsonwebtoken's built-in audience validation so we can run
    // our own three-case check (correct / legacy-empty / wrong). The
    // built-in defaults to `validate_aud = true`, which rejects any token
    // with an aud field when no audience is registered on the validator
    // — that would block the new aud-stamped tokens before reaching us.
    let mut validation = Validation::default();
    validation.validate_aud = false;
    let token_data = decode::<Claims>(&token, decoding_key, &validation).map_err(|e| {
        tracing::warn!("JWT verification failed: {:?}", e);
        AppError::Unauthorized("Invalid token".to_string())
    })?;

    let mut claims = token_data.claims;

    // Revocation check: if the user explicitly logged out (or an admin force-
    // logged-out a session), the JWT is still cryptographically valid until
    // `exp` but its jti is on the revocation list. Short-circuit before any
    // other DB work — a revoked token shouldn't even count toward auto-clear
    // of lockouts or role re-reads.
    //
    // jti is now required on every token. The transitional carve-out that
    // accepted jti="" (legacy tokens from before the jti rollout) has been
    // dropped — one full JWT TTL (24h) has passed since the rollout, so any
    // token without a jti is either malformed or a forgery. Reject as 401.
    if claims.jti.is_empty() {
        tracing::warn!(
            user_id = %claims.sub,
            "Rejecting token without jti — pre-rollout sessions have expired."
        );
        return Err(AppError::Unauthorized(
            "Token is missing required claim".to_string(),
        ));
    }
    if state.jwt_revocation_logic.is_revoked(&claims.jti).await? {
        tracing::info!(
            user_id = %claims.sub,
            jti = %claims.jti,
            "Rejecting revoked token"
        );
        return Err(AppError::Unauthorized("Token has been revoked".to_string()));
    }

    // Verify the user still exists, isn't soft-deleted, and isn't locked out;
    // *and* re-read the current role from the DB rather than trusting the
    // role that was baked into the JWT 0–24h ago. Without this re-check, a
    // demotion (super_admin → user, or admin → user) takes up to the JWT TTL
    // to actually downgrade the user's effective permissions. Same query, one
    // extra column — the cost is negligible.
    //
    // Returns None for deleted / missing users — reject those rather than
    // silently authenticating (was a bug pre-fix).
    let user_state: Option<(bool, Option<DateTime<Utc>>, UserRole)> = sqlx::query_as(
        "SELECT locked_out, lockout_until, role \
         FROM users \
         WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(&claims.sub)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let Some((locked_out, lockout_until, db_role)) = user_state else {
        return Err(AppError::Unauthorized("User not found".to_string()));
    };

    if locked_out {
        match lockout_until {
            // Temporary lockout that has now expired — auto-clear and allow.
            // Without this, locked_out=1 stays in the DB indefinitely after the
            // deadline passes, even though the user is effectively unlocked.
            Some(until) if until <= Utc::now() => {
                sqlx::query(
                    "UPDATE users \
                     SET locked_out = 0, lockout_reason = NULL, lockout_until = NULL, updated_at = ?1 \
                     WHERE id = ?2",
                )
                .bind(Utc::now())
                .bind(&claims.sub)
                .execute(&state.pool)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            }
            // Active lockout (either still-in-effect temp or permanent NULL).
            _ => return Err(AppError::Forbidden("Account is locked".to_string())),
        }
    }

    // Overwrite the in-JWT role with the DB role before any downstream
    // handler reads `claims.role`. When they disagree, log it — that's
    // useful drift telemetry (e.g. a stolen old token mid-demotion) without
    // being noisy: drift is expected immediately after a role change and
    // until the user re-authenticates.
    if claims.role != db_role {
        tracing::warn!(
            user_id = %claims.sub,
            jwt_role = %claims.role,
            db_role = %db_role,
            "JWT role differs from DB role; using DB role"
        );
        claims.role = db_role;
    }

    // Audience check. Two cases now:
    //   - aud == EXPECTED_AUDIENCE → accept.
    //   - anything else (including empty) → reject. The token was minted
    //     for a different service that happens to share our JWT_SECRET,
    //     which is exactly the confusion this claim defends against.
    //
    // The empty-aud carve-out for pre-rollout tokens has been dropped —
    // one full JWT TTL (24h) past the rollout, any token without an aud
    // is either malformed, forged, or minted for a sibling service that
    // shouldn't be trusted by this API.
    if claims.aud != EXPECTED_AUDIENCE {
        tracing::warn!(
            user_id = %claims.sub,
            aud = %claims.aud,
            expected = EXPECTED_AUDIENCE,
            "Rejecting token with wrong audience"
        );
        return Err(AppError::Unauthorized(
            "Token audience does not match this service".to_string(),
        ));
    }

    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

// ============================================================================
// Role-Based Middleware - Requires specific role
// ============================================================================
// Generic role gate. The live routes use the role-specific `require_admin`
// / `require_super_admin` middlewares directly; `require_role` is kept as
// the building block for any future role tier (e.g. moderator) without
// duplicating the extension-and-permission-check boilerplate.
#[allow(dead_code)]
pub async fn require_role(
    required_role: UserRole,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?
        .clone();

    let user_role = claims.role;

    if !has_permission(&user_role, &required_role) {
        return Err(AppError::Forbidden(format!(
            "Requires {:?} role or higher",
            required_role
        )));
    }

    Ok(next.run(req).await)
}

// ============================================================================
// Admin Middleware - Requires Admin or SuperAdmin
// ============================================================================
pub async fn require_admin(req: Request, next: Next) -> Result<Response, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    match claims.role {
        UserRole::Admin | UserRole::SuperAdmin => Ok(next.run(req).await),
        UserRole::User => Err(AppError::Forbidden("Admin access required".to_string())),
    }
}

// ============================================================================
// SuperAdmin Middleware - Requires SuperAdmin only
// ============================================================================
pub async fn require_super_admin(req: Request, next: Next) -> Result<Response, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    match claims.role {
        UserRole::SuperAdmin => Ok(next.run(req).await),
        _ => Err(AppError::Forbidden(
            "SuperAdmin access required".to_string(),
        )),
    }
}

// ============================================================================
// Resource Owner Middleware - User can only access their own resources
// ============================================================================
// Each domain (events, microevents, user_collection) carries its own
// `require_owner_or_admin` against the relevant ownership column today,
// since the column varies. This helper is the staged generic — when an
// abstract "owns resource" trait lands, route handlers can fall back to
// this without re-implementing the claims-extraction dance.
#[allow(dead_code)]
pub async fn require_owner_or_admin(resource_user_id: &str, req: &Request) -> Result<(), AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    if claims.sub == resource_user_id
        || matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin)
    {
        Ok(())
    } else {
        Err(AppError::Forbidden("Access denied".to_string()))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract JWT token from Authorization header
fn extract_token(req: &Request) -> Result<String, AppError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".to_string()))?
        .to_str()
        .map_err(|_| AppError::Unauthorized("Invalid Authorization header".to_string()))?;

    if !auth_header.starts_with("Bearer ") {
        return Err(AppError::Unauthorized(
            "Authorization header must start with 'Bearer '".to_string(),
        ));
    }

    Ok(auth_header[7..].to_string())
}

/// Check if user role has permission for required role.
///
/// Used only by `require_role` (above), which is itself unused today.
/// Kept here as the canonical role-comparison table — when a future
/// route needs "Admin or SuperAdmin can do X, plain User cannot," this
/// is the table it compares against. The `#[allow(dead_code)]` mirrors
/// the gate on `require_role`.
#[allow(dead_code)]
fn has_permission(user_role: &UserRole, required_role: &UserRole) -> bool {
    match (user_role, required_role) {
        (UserRole::SuperAdmin, _) => true,
        (UserRole::Admin, UserRole::Admin) => true,
        (UserRole::Admin, UserRole::User) => true,
        (UserRole::Admin, UserRole::SuperAdmin) => false,
        (UserRole::User, UserRole::User) => true,
        (UserRole::User, _) => false,
    }
}

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Extension, Router,
        body::{Body, to_bytes},
        http::Request,
        http::StatusCode,
        middleware,
        routing::get,
    };
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::util::ServiceExt;

    const TEST_JWT_SECRET: &str = "test-secret-do-not-use-in-prod";

    fn init_env() {
        // SAFETY: `set_var` is unsafe in Rust 2024 because it can race with
        // concurrent reads. Tests set this once before building the app and
        // no other threads touch it.
        unsafe {
            std::env::set_var("JWT_SECRET", TEST_JWT_SECRET);
        }
    }

    async fn setup_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");

        pool
    }

    /// Insert a user row with explicit role. Returns the inserted user id.
    async fn insert_user(pool: &sqlx::SqlitePool, id: &str, role: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, oauth_id, oauth_provider, user_name, role, created_at, updated_at) \
             VALUES (?1, ?2, 'google', 'tester', ?3, ?4, ?4)",
        )
        .bind(id)
        .bind(format!("oauth-{}", id))
        .bind(role)
        .bind(&now)
        .execute(pool)
        .await
        .expect("insert test user");
    }

    /// Build a fresh token with a random jti and the production audience.
    /// Tests that need a different jti or audience call the underlying
    /// builder directly.
    fn make_token(user_id: &str, jwt_role: &str) -> String {
        make_token_full(
            user_id,
            jwt_role,
            &uuid::Uuid::new_v4().to_string(),
            EXPECTED_AUDIENCE,
        )
    }

    /// Build a token with an explicit jti — used by revocation tests so the
    /// test can revoke the *same* jti it stamped on the token.
    fn make_token_with_jti(user_id: &str, jwt_role: &str, jti: &str) -> String {
        make_token_full(user_id, jwt_role, jti, EXPECTED_AUDIENCE)
    }

    /// Lowest-level token builder. `aud=""` simulates a legacy token from
    /// before the aud rollout; `aud="other-service"` simulates a token
    /// minted for a different service. The audience-rejection tests pass
    /// these values directly.
    fn make_token_full(user_id: &str, jwt_role: &str, jti: &str, aud: &str) -> String {
        #[derive(Serialize)]
        struct FakeClaims<'a> {
            sub: &'a str,
            email: &'a str,
            username: &'a str,
            role: &'a str,
            exp: usize,
            iat: usize,
            jti: &'a str,
            aud: &'a str,
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        encode(
            &Header::default(),
            &FakeClaims {
                sub: user_id,
                email: "test@example.com",
                username: "tester",
                role: jwt_role,
                exp: now + 3600,
                iat: now,
                jti,
                aud,
            },
            &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
        )
        .expect("encode test token")
    }

    /// Build the AuthMiddlewareState the production middleware expects.
    /// Wraps the in-memory pool with a real `JwtRevocationLogic` so the
    /// revocation-check tests exercise the actual logic-layer code path,
    /// not a stub.
    fn make_state(pool: sqlx::SqlitePool) -> AuthMiddlewareState {
        use crate::context::JwtRevocationContext;
        let context = JwtRevocationContext::new(pool.clone());
        let logic = Arc::new(JwtRevocationLogic::new(context));
        AuthMiddlewareState {
            pool,
            jwt_revocation_logic: logic,
        }
    }

    fn build_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_state(pool);
        Router::new()
            .route("/protected", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    /// Probe app exposes whatever role landed in the Claims extension after
    /// the middleware ran. Used by the role-swap test to verify the DB role
    /// (not the JWT role) drives downstream authz. Returns the role's
    /// canonical wire-format string (`"user"` / `"admin"` / `"super_admin"`)
    /// via the `Display` impl — same shape the original String-typed Claims
    /// field returned, so the existing assertions still match.
    fn build_role_probe_app(pool: sqlx::SqlitePool) -> Router {
        async fn read_role(Extension(claims): Extension<Claims>) -> String {
            claims.role.to_string()
        }
        let state = make_state(pool);
        Router::new()
            .route("/role", get(read_role))
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    #[tokio::test]
    async fn rejects_missing_authorization_header() {
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_non_bearer_authorization() {
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Basic abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_garbage_bearer_token() {
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Bearer this.is.not.a.jwt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_valid_jwt_for_unknown_user() {
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let token = make_token("00000000-0000-0000-0000-000000000000", "user");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Valid signature but the user doesn't exist in the DB. Pre-fix this
        // returned 200 (the bug). Post-fix it should return 401.
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stale_jwt_role_gets_overwritten_by_db_role() {
        // A user was demoted from super_admin to user but is still presenting
        // the JWT that was issued when they were super_admin. Downstream
        // handlers must see role="user" (the DB value), not role="super_admin"
        // (the JWT value). This is the core staleness fix.
        init_env();
        let pool = setup_pool().await;
        let user_id = "11111111-1111-1111-1111-111111111111";
        insert_user(&pool, user_id, "user").await;
        let app = build_role_probe_app(pool);

        let token = make_token(user_id, "super_admin");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/role")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body_str, "user", "DB role must override JWT role");
    }

    #[tokio::test]
    async fn jwt_role_passes_through_when_db_matches() {
        // The happy path: JWT and DB agree, so no overwrite warning fires and
        // the role is the same on both sides. Exercises the equality branch
        // so we don't silently regress to always-overwriting (which would
        // hide other bugs).
        init_env();
        let pool = setup_pool().await;
        let user_id = "22222222-2222-2222-2222-222222222222";
        insert_user(&pool, user_id, "admin").await;
        let app = build_role_probe_app(pool);

        let token = make_token(user_id, "admin");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/role")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body_str, "admin");
    }

    #[tokio::test]
    async fn rejects_revoked_token() {
        // After /auth/logout writes the jti to jwt_revocations, any further
        // request bearing the same token must 401 — even though the JWT
        // signature is still valid and the `exp` hasn't passed.
        init_env();
        let pool = setup_pool().await;
        let user_id = "44444444-4444-4444-4444-444444444444";
        insert_user(&pool, user_id, "user").await;

        let jti = "test-revocation-jti";
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        sqlx::query("INSERT INTO jwt_revocations (jti, user_id, expires_at) VALUES (?1, ?2, ?3)")
            .bind(jti)
            .bind(user_id)
            .bind(now + 3600)
            .execute(&pool)
            .await
            .expect("insert revocation");

        let app = build_app(pool);
        let token = make_token_with_jti(user_id, "user", jti);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_non_revoked_token_with_jti() {
        // The happy path for a jti-stamped token: present in the JWT,
        // absent from the revocation list → request goes through.
        init_env();
        let pool = setup_pool().await;
        let user_id = "55555555-5555-5555-5555-555555555555";
        insert_user(&pool, user_id, "user").await;
        let app = build_app(pool);

        let token = make_token(user_id, "user");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn token_without_jti_is_rejected() {
        // The legacy-jti carve-out was dropped one JWT TTL after rollout —
        // any token without a jti now is either malformed, a forgery, or
        // an ancient session that should have been logged out. Reject 401.
        init_env();
        let pool = setup_pool().await;
        let user_id = "66666666-6666-6666-6666-666666666666";
        insert_user(&pool, user_id, "user").await;
        let app = build_app(pool);

        // Empty jti — emulates a pre-rollout token (or a malformed one).
        let token = make_token_with_jti(user_id, "user", "");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn promotion_in_db_takes_effect_without_relogin() {
        // The mirror case: a user was promoted from `user` to `admin` after
        // the JWT was issued. They should get admin permissions on the next
        // request without having to log out and back in.
        init_env();
        let pool = setup_pool().await;
        let user_id = "33333333-3333-3333-3333-333333333333";
        insert_user(&pool, user_id, "admin").await;
        let app = build_role_probe_app(pool);

        // JWT says "user" (stale, pre-promotion); DB says "admin".
        let token = make_token(user_id, "user");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/role")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body_str, "admin", "Promotion must apply without re-login");
    }

    /// Build a router that requires the Admin tier (Admin OR SuperAdmin).
    /// The stack mirrors how `admin_view_routes` is composed in main.rs:
    /// auth_middleware first, then require_admin, then the handler.
    fn build_admin_gated_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_state(pool);
        Router::new()
            .route("/admin-only", get(|| async { "ok" }))
            .route_layer(middleware::from_fn(require_admin))
            .route_layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    #[tokio::test]
    async fn require_admin_accepts_admin_role() {
        init_env();
        let pool = setup_pool().await;
        let user_id = "77777777-7777-7777-7777-777777777777";
        insert_user(&pool, user_id, "admin").await;
        let app = build_admin_gated_app(pool);

        let token = make_token(user_id, "admin");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin-only")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn require_admin_accepts_super_admin_role() {
        // SuperAdmin must be able to do everything Admin can — otherwise
        // splitting routes by privilege tier would lock SuperAdmins out
        // of the view/lock routes, which is the opposite of the intent.
        init_env();
        let pool = setup_pool().await;
        let user_id = "88888888-8888-8888-8888-888888888888";
        insert_user(&pool, user_id, "super_admin").await;
        let app = build_admin_gated_app(pool);

        let token = make_token(user_id, "super_admin");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin-only")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_token_with_expected_audience() {
        // The happy path for new tokens: aud is stamped and matches
        // EXPECTED_AUDIENCE. Pinned so a typo-fix on the constant breaks
        // exactly one test instead of silently breaking every signin.
        init_env();
        let pool = setup_pool().await;
        let user_id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        insert_user(&pool, user_id, "user").await;
        let app = build_app(pool);

        let token = make_token_full(
            user_id,
            "user",
            &uuid::Uuid::new_v4().to_string(),
            EXPECTED_AUDIENCE,
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_token_without_aud_claim() {
        // The legacy-aud carve-out was dropped one JWT TTL after rollout
        // — any token decoding with `aud=""` is treated the same as a
        // wrong-audience token: 401. This closes the "shared JWT_SECRET
        // between sibling services" attack surface.
        init_env();
        let pool = setup_pool().await;
        let user_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        insert_user(&pool, user_id, "user").await;
        let app = build_app(pool);

        let token = make_token_full(
            user_id,
            "user",
            &uuid::Uuid::new_v4().to_string(),
            "", // empty aud — was legacy-accepted, now rejected
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_token_with_wrong_audience() {
        // The defensive case: a token minted for a different service that
        // happens to share our JWT_SECRET. Without the aud check, our
        // middleware would happily accept it; with the check, we reject 401.
        init_env();
        let pool = setup_pool().await;
        let user_id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        insert_user(&pool, user_id, "user").await;
        let app = build_app(pool);

        let token = make_token_full(
            user_id,
            "user",
            &uuid::Uuid::new_v4().to_string(),
            "festurah-admin-tool", // different service entirely
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn require_admin_rejects_plain_user_role() {
        // The whole point of splitting: regular users still can't reach
        // admin endpoints. If this regresses, the privilege model is broken.
        init_env();
        let pool = setup_pool().await;
        let user_id = "99999999-9999-9999-9999-999999999999";
        insert_user(&pool, user_id, "user").await;
        let app = build_admin_gated_app(pool);

        let token = make_token(user_id, "user");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin-only")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
