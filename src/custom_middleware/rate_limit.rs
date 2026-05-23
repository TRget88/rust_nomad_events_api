// src/custom_middleware/rate_limit.rs
use crate::models::user::Claims;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    pub async fn check_rate_limit(&self, key: &str) -> bool {
        let mut requests = self.requests.lock().await;
        let now = Instant::now();

        let entries = requests.entry(key.to_string()).or_insert_with(Vec::new);
        entries.retain(|&timestamp| now.duration_since(timestamp) < self.window);

        if entries.len() >= self.max_requests {
            return false;
        }

        entries.push(now);
        true
    }
}

/// Per-request rate-limit check.
///
/// Keys on the peer's remote IP when available (axum populates
/// `ConnectInfo<SocketAddr>` into request extensions via
/// `into_make_service_with_connect_info` in main.rs). Falls back to the
/// `X-API-Key` header for synthetic test requests, then "unknown" as a final
/// catch-all so unkeyed callers share one bucket.
///
/// The limiter itself is owned by `main` and threaded through
/// `from_fn_with_state` — constructing one per request was the original bug.
pub async fn rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_id = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .or_else(|| {
            request
                .headers()
                .get("X-API-Key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    if !rate_limiter.check_rate_limit(&client_id).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}

/// Per-user rate limit. Pulls the authenticated `Claims` out of request
/// extensions (populated by `auth_middleware`) and buckets requests by user
/// UUID rather than IP — so a logged-in scraper can't rotate IPs to bypass.
///
/// Keys are prefixed `user:<uuid>` so that, if some future caller ever
/// shares a `RateLimiter` instance with the IP-keyed middleware, the
/// namespaces can't collide.
///
/// This middleware MUST be installed downstream of `auth_middleware` —
/// otherwise no Claims are in extensions and we fail closed with 500.
/// In execution-order terms: auth runs first (outermost), this runs after.
pub async fn user_rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(claims) = request.extensions().get::<Claims>() else {
        // Defensive: auth_middleware should always run before this layer.
        // If it doesn't, fail closed — refusing to serve is safer than
        // skipping the rate-limit check.
        tracing::error!(
            "user_rate_limit_middleware reached without Claims in extensions; \
             check middleware order in main.rs"
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    let key = format!("user:{}", claims.sub);

    if !rate_limiter.check_rate_limit(&key).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Extension, Router, body::Body, extract::Request as ExtractRequest, http::Request,
        routing::get,
    };
    use tower::util::ServiceExt;

    fn make_claims(sub: &str) -> Claims {
        Claims {
            sub: sub.to_string(),
            email: "t@example.com".to_string(),
            username: "tester".to_string(),
            role: crate::models::user::UserRole::User,
            exp: 0,
            iat: 0,
            jti: "test-jti".to_string(),
            // Audience doesn't matter for these tests (the rate-limit
            // middleware runs after auth_middleware in production, so
            // these tests stub auth and inject Claims directly). Use the
            // production value so the struct stays valid for future
            // code paths that might inspect it.
            aud: crate::custom_middleware::auth_middleware::EXPECTED_AUDIENCE.to_string(),
        }
    }

    /// Build a router with `user_rate_limit_middleware` and a stub auth that
    /// just injects the provided Claims into request extensions. Lets us
    /// exercise the rate-limit middleware in isolation without standing up
    /// the full auth stack.
    fn build_user_rl_app(rl: Arc<RateLimiter>, claims: Claims) -> Router {
        async fn stub_auth(
            Extension(claims_holder): Extension<Arc<Claims>>,
            mut req: ExtractRequest,
            next: Next,
        ) -> Response {
            req.extensions_mut().insert((*claims_holder).clone());
            next.run(req).await
        }
        Router::new()
            .route("/protected", get(|| async { "ok" }))
            .route_layer(axum::middleware::from_fn_with_state(
                rl,
                user_rate_limit_middleware,
            ))
            .route_layer(axum::middleware::from_fn(stub_auth))
            .layer(Extension(Arc::new(claims)))
    }

    #[tokio::test]
    async fn allows_requests_under_limit() {
        let rl = Arc::new(RateLimiter::new(3, 60));
        let app = build_user_rl_app(rl, make_claims("user-aaa"));

        for i in 0..3 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/protected")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "request {} should pass under the 3/60s budget",
                i + 1
            );
        }
    }

    #[tokio::test]
    async fn rejects_once_user_exceeds_limit() {
        let rl = Arc::new(RateLimiter::new(2, 60));
        let app = build_user_rl_app(rl, make_claims("user-bbb"));

        // First two ride the budget.
        for _ in 0..2 {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/protected")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // Third must hit the limit.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn separate_users_have_separate_buckets() {
        let rl = Arc::new(RateLimiter::new(1, 60));

        // Build two parallel apps with the same RateLimiter — same instance,
        // different Claims. This is exactly the production setup (one
        // RateLimiter shared across users).
        let app_a = build_user_rl_app(rl.clone(), make_claims("user-a"));
        let app_b = build_user_rl_app(rl.clone(), make_claims("user-b"));

        // user-a consumes their entire budget.
        let resp = app_a
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app_a
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // user-b's bucket is independent — they still get their first request.
        let resp = app_b
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "user-b's bucket must be independent of user-a's"
        );
    }

    #[tokio::test]
    async fn fails_closed_when_claims_are_missing() {
        // No upstream auth layer, so Claims never lands in extensions.
        // The middleware must refuse the request rather than skip the check.
        let rl = Arc::new(RateLimiter::new(100, 60));
        let app = Router::new()
            .route("/protected", get(|| async { "ok" }))
            .route_layer(axum::middleware::from_fn_with_state(
                rl,
                user_rate_limit_middleware,
            ));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
