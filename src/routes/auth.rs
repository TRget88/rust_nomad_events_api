use crate::AppState;
use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::user::Claims;
use crate::models::user::{AuthResponse, FacebookLoginRequest, GoogleLoginRequest};

use axum::Extension;
use axum::http::StatusCode;
use axum::{Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Body shape for `POST /auth/refresh`. The client sends the plaintext
/// refresh token it received at login (or from the previous refresh).
/// Rejected with 400 if missing/empty.
#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Response shape for `POST /auth/refresh`. The new access token
/// (short-lived JWT) plus a *new* refresh token (the presented one
/// has been revoked by rotation). Note the field naming mirrors
/// `AuthResponse` deliberately so the frontend can use the same
/// `setSession({ token, refresh_token })` helper for login and
/// refresh paths.
#[derive(Serialize)]
pub struct RefreshResponse {
    pub token: String,
    pub refresh_token: String,
}

// ============================================================================
// Google OAuth
// ============================================================================

pub async fn verify_google_login(
    State(state): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<GoogleLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Breadcrumb left from earlier debugging. Kept at `debug` so it's
    // filterable via `RUST_LOG=debug` rather than printed unconditionally.
    tracing::debug!("verify_google_login reached");

    let response = state.user_logic.verify_google_login(&payload).await?;

    issue_refresh_and_wrap(&state, response).await
}

pub async fn verify_google_create(
    State(state): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<GoogleLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    tracing::debug!("verify_google_create reached");

    let response = state
        .user_logic
        .verify_google_account_creation(&payload)
        .await?;

    issue_refresh_and_wrap(&state, response).await
}

// ============================================================================
// Facebook OAuth
// ============================================================================
//
// Same login/signup split as Google: `/auth/facebook/login` rejects unknown
// users, `/auth/facebook/signup` rejects existing users. Both verify the
// access token against Facebook's Graph API and issue our own HS256 JWT on
// success. Wired into `public_routes` in `main.rs` alongside the Google
// routes.

pub async fn verify_facebook_login(
    State(state): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<FacebookLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    tracing::debug!("verify_facebook_login reached");

    let response = state.user_logic.verify_facebook_login(&payload).await?;

    issue_refresh_and_wrap(&state, response).await
}

pub async fn verify_facebook_create(
    State(state): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<FacebookLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    tracing::debug!("verify_facebook_create reached");

    let response = state
        .user_logic
        .verify_facebook_account_creation(&payload)
        .await?;

    issue_refresh_and_wrap(&state, response).await
}

/// Helper used by all four login/signup endpoints: take the
/// `AuthResponse` UserLogic just built, issue a paired refresh token,
/// and graft it onto the response before returning to the client.
///
/// Kept here at the route layer (rather than in `UserLogic`) so the
/// auth module is the only place that knows about
/// `RefreshTokenLogic`. UserLogic stays a pure JWT issuer.
async fn issue_refresh_and_wrap(
    state: &AppState,
    mut response: AuthResponse,
) -> Result<Json<AuthResponse>, AppError> {
    let issued = state
        .refresh_token_logic
        .issue_for_user(&response.user.id)
        .await?;
    response.refresh_token = Some(issued.plaintext);
    Ok(Json(response))
}

// ============================================================================
// Refresh token rotation
// ============================================================================

/// `POST /auth/refresh` — exchange a presented refresh token for a
/// fresh access JWT + a fresh refresh token. Validates and rotates in
/// `RefreshTokenLogic::rotate`:
///
///   - Unknown token → 401.
///   - Already-revoked token → reuse detection: revoke the whole
///     family + 401. The user has to log in again from every device.
///   - Expired token → 401, the consumed row is also revoked.
///   - Valid token → revoke it, mint a new one, mint an access JWT
///     for the same user.
///
/// Lives in the **public** route group (no JWT middleware): the access
/// token has already expired by the time the client calls here, so
/// requiring a valid access token would defeat the purpose. The route
/// is still rate-limited and API-key-gated like the rest of the public
/// auth routes.
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    if payload.refresh_token.trim().is_empty() {
        return Err(AppError::BadRequest(
            "refresh_token is required".to_string(),
        ));
    }

    let issued = state
        .refresh_token_logic
        .rotate(&payload.refresh_token)
        .await?;
    let access_token = state
        .user_logic
        .mint_access_token_for_user(&issued.user_id)
        .await?;

    Ok(Json(RefreshResponse {
        token: access_token,
        refresh_token: issued.plaintext,
    }))
}

// ============================================================================
// Logout
// ============================================================================

/// Revoke the caller's JWT by writing its `jti` to the `jwt_revocations`
/// table. Subsequent requests bearing the same token are rejected 401 by the
/// auth middleware. Idempotent: a second logout for the same already-revoked
/// token returns 204 without error.
///
/// This route lives behind the auth middleware (see `main.rs::jwt_routes`),
/// so by the time we reach the handler the claims have already been verified
/// and a `Claims` extension is present.
pub async fn logout(
    Extension(claims): Extension<Claims>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    state.jwt_revocation_logic.revoke_claims(&claims).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    //! Route-layer tests for `/auth/refresh`. The login/signup routes
    //! aren't covered here because they require a live Google/Facebook
    //! callback — those are exercised at the `UserLogic` layer with
    //! recorded fixtures elsewhere. The refresh route has no external
    //! dependency, so the route-layer + logic test pair pins the
    //! complete behavior of the endpoint.
    //!
    //! End-to-end shape mirrors `routes::event_type::tests`: a
    //! `setup_pool` + `make_app_state` + `oneshot`-driven Router.

    use super::*;
    use crate::context::{
        AuditLogContext, CampingProfileContext, EventContext, EventTypeContext,
        JwtRevocationContext, MicroeventContext, RefreshTokenContext, UserCollectionContext,
        UserContext,
    };
    use crate::logic::{
        AuditLogLogic, CampingProfileLogic, EventLogic, EventTypeLogic, JwtRevocationLogic,
        MicroeventLogic, RefreshTokenLogic, UserCollectionLogic, UserLogic,
    };
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

    async fn setup_pool() -> sqlx::SqlitePool {
        // Same as the other route-test setups. We seed one users row
        // because `mint_access_token_for_user` reads it back; without
        // a user the rotate path succeeds but minting the access JWT
        // fails with NotFound.
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

        // Seed one real user. `created_at`/`updated_at` need to be valid
        // RFC3339-ish for sqlx's DateTime decode; SQLite's `datetime`
        // returns "YYYY-MM-DD HH:MM:SS" which sqlx parses fine.
        sqlx::query(
            "INSERT INTO users (id, oauth_id, oauth_provider, user_name, \
                                email, email_verified, locked_out, role, \
                                created_at, updated_at) \
             VALUES ('user-1', 'oauth-1', 'google', 'tester', \
                     'tester@example.com', 1, 0, 'user', \
                     datetime('now'), datetime('now'))",
        )
        .execute(&pool)
        .await
        .expect("seed user");
        pool
    }

    /// Same shape as `routes::event_type::tests::make_app_state`.
    /// Refresh route uses `user_logic` (to mint the access token) and
    /// `refresh_token_logic` (to rotate); both must be wired.
    fn make_app_state(pool: sqlx::SqlitePool) -> Arc<crate::AppState> {
        let event_context = Arc::new(EventContext::new(pool.clone()));
        let microevent_context = Arc::new(MicroeventContext::new(pool.clone()));
        let event_type_context = EventTypeContext::new(pool.clone());
        let event_type_logic = Arc::new(EventTypeLogic::new(event_type_context));
        let camping_profile_context = CampingProfileContext::new(pool.clone());
        let camping_profile_logic = Arc::new(CampingProfileLogic::new(camping_profile_context));
        let user_context = UserContext::new(pool.clone());
        let user_logic = Arc::new(UserLogic::new(user_context));
        let user_collection_context = UserCollectionContext::new(pool.clone());
        let user_collection_logic = Arc::new(UserCollectionLogic::new(
            user_collection_context,
            event_context.clone(),
            microevent_context.clone(),
        ));
        let event_logic = Arc::new(EventLogic::new(
            event_context.clone(),
            user_collection_logic.clone(),
        ));
        let microevent_logic = Arc::new(MicroeventLogic::new(
            microevent_context.clone(),
            user_collection_logic.clone(),
        ));
        let jwt_revocation_context = JwtRevocationContext::new(pool.clone());
        let jwt_revocation_logic = Arc::new(JwtRevocationLogic::new(jwt_revocation_context));
        let audit_log_context = AuditLogContext::new(pool.clone());
        let audit_log_logic = Arc::new(AuditLogLogic::new(audit_log_context));
        let refresh_token_context = Arc::new(RefreshTokenContext::new(pool));
        let refresh_token_logic = Arc::new(RefreshTokenLogic::new(refresh_token_context));

        Arc::new(crate::AppState {
            event_logic,
            microevent_logic,
            event_type_logic,
            camping_profile_logic,
            user_logic,
            user_collection_logic,
            jwt_revocation_logic,
            audit_log_logic,
            refresh_token_logic,
        })
    }

    fn build_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_app_state(pool);
        Router::new()
            .route("/auth/refresh", post(super::refresh))
            .with_state(state)
    }

    async fn read_body_value(response: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&body).expect("parse JSON")
    }

    fn json_body(value: serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    /// JWT minting reads `JWT_SECRET` from the env. Set it to the same
    /// value `custom_middleware::auth_middleware::tests` uses — a
    /// different value here would clobber the auth-middleware tests'
    /// signing key under parallel test execution and surface as random
    /// 401s in those tests. The auth-middleware module caches the
    /// signing key in a `static`, so the FIRST setter wins; using the
    /// same string everywhere makes that race a no-op.
    fn init_env() {
        // SAFETY: setting a stable env var to a stable value. The
        // racey part is `set_var` itself; the value is identical
        // across every test that sets it, so a concurrent re-set is
        // idempotent.
        unsafe {
            std::env::set_var("JWT_SECRET", "test-secret-do-not-use-in-prod");
        }
    }

    #[tokio::test]
    async fn refresh_rejects_empty_token() {
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(serde_json::json!({ "refresh_token": "" })))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn refresh_rejects_unknown_token() {
        // Plausibly-shaped but never-issued token. Logic layer
        // surfaces an Unauthorized; the route handler preserves it.
        init_env();
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(serde_json::json!({
                        "refresh_token": "deadbeef".repeat(8)
                    })))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_happy_path_returns_new_token_pair() {
        // Full end-to-end: issue a refresh token directly via the logic
        // layer (skipping the OAuth flow), then POST /auth/refresh and
        // confirm we get back a fresh access JWT and a fresh refresh
        // token that differs from the presented one.
        init_env();
        let pool = setup_pool().await;
        let state = make_app_state(pool);

        let issued = state
            .refresh_token_logic
            .issue_for_user("user-1")
            .await
            .expect("issue");
        let original_plaintext = issued.plaintext.clone();

        let app = Router::new()
            .route("/auth/refresh", post(super::refresh))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(serde_json::json!({
                        "refresh_token": original_plaintext
                    })))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_value(response).await;

        let access = body["token"].as_str().expect("token in response");
        let new_refresh = body["refresh_token"]
            .as_str()
            .expect("refresh_token in response");

        assert!(!access.is_empty(), "access token present");
        // The new refresh token differs from the presented one — rotation
        // actually happened. (A regression that returned the same
        // plaintext back would silently break reuse detection.)
        assert_ne!(new_refresh, original_plaintext);
        // 32-byte hex = 64 chars.
        assert_eq!(new_refresh.len(), 64);
    }

    #[tokio::test]
    async fn refresh_twice_with_same_token_triggers_reuse_detection() {
        // The classic reuse-detection scenario, exercised end-to-end
        // through the HTTP route rather than the logic layer alone:
        //   1. Issue refresh token A.
        //   2. POST /auth/refresh with A → 200, new pair (B, refresh_B).
        //   3. POST /auth/refresh with A again → 401 (reuse detected).
        //   4. POST /auth/refresh with refresh_B → 401 (family revoked).
        init_env();
        let pool = setup_pool().await;
        let state = make_app_state(pool);

        let issued = state
            .refresh_token_logic
            .issue_for_user("user-1")
            .await
            .unwrap();
        let token_a = issued.plaintext.clone();

        // Step 2: first refresh succeeds.
        let app = Router::new()
            .route("/auth/refresh", post(super::refresh))
            .with_state(state.clone());
        let r1 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(
                        serde_json::json!({ "refresh_token": token_a.clone() }),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let r1_body = read_body_value(r1).await;
        let refresh_b = r1_body["refresh_token"].as_str().unwrap().to_string();

        // Step 3: replay of A — must fail (reuse detected).
        let app = Router::new()
            .route("/auth/refresh", post(super::refresh))
            .with_state(state.clone());
        let r2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(serde_json::json!({ "refresh_token": token_a })))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);

        // Step 4: B should also be dead now (its family was revoked
        // when reuse fired on A's row).
        let app = Router::new()
            .route("/auth/refresh", post(super::refresh))
            .with_state(state);
        let r3 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header("content-type", "application/json")
                    .body(json_body(serde_json::json!({ "refresh_token": refresh_b })))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            r3.status(),
            StatusCode::UNAUTHORIZED,
            "the entire family must be dead after reuse detection"
        );
    }
}
