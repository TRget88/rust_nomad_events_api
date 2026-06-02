// ============================================================================
// src/routes/event_ownership.rs - HTTP handlers for the ownership workflow
// ============================================================================
//
// The "claim / request ownership of an existing event" feature. Five
// endpoints, all JWT-gated (mounted under `jwt_routes` in `main.rs`):
//
//   POST /event/{id}/ownership-request   request()        file a request
//   GET  /ownership-requests/mine        list_mine()      my outgoing list
//   GET  /ownership-requests/incoming    list_incoming()  my review queue
//   POST /ownership-request/{id}/approve approve()        owner/admin approve
//   POST /ownership-request/{id}/reject  reject()         owner/admin reject
//
// All the business rules — existence checks, RBAC, the verified-domain
// auto-approval, the ownership transfer — live in
// `EventOwnershipRequestLogic`. This layer is thin: extract, call the
// logic, and (mirroring `events::delete`) record best-effort audit rows
// from the rich outcome types the logic returns. Audit lives here, not in
// the logic, because the route is where the actor's `Claims` and the HTTP
// boundary are — same split the rest of the codebase uses.

use crate::AppState;
use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::audit::{AuditRecord, actions, target_types};
use crate::models::ownership::{
    ApprovalOutcome, OwnershipRequestInput, RejectionOutcome, RequestOutcome,
};
use crate::models::user::Claims;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use std::sync::Arc;

/// `POST /event/{id}/ownership-request` — file a request to own an event
/// the caller didn't create. The note is the only client-supplied field
/// and the whole body is optional: a bare bodyless POST is a valid "no
/// note" request (`Option<ApiJson<_>>` yields `None`).
///
/// Returns 201 in both outcomes — a request resource was created either
/// way. The body's `auto_approved` flag (and `request.status` /
/// `request.resolution_method`) tell the client whether ownership already
/// moved (verified-domain match) or the request is parked for review.
pub async fn request(
    Extension(claims): Extension<Claims>,
    Path(event_id): Path<i64>,
    State(service): State<Arc<AppState>>,
    body: Option<ApiJson<OwnershipRequestInput>>,
) -> Result<impl IntoResponse, AppError> {
    let note = body.and_then(|ApiJson(input)| input.note);
    let actor_user_id = claims.sub.clone();

    let outcome = service
        .event_ownership_request_logic
        .request_ownership(event_id, &claims, note)
        .await?;

    match outcome {
        RequestOutcome::Pending(request) => {
            service
                .audit_log_logic
                .record_best_effort(AuditRecord {
                    actor_user_id,
                    action: actions::EVENT_OWNERSHIP_REQUEST.to_string(),
                    target_type: target_types::EVENT_OWNERSHIP_REQUEST.to_string(),
                    target_id: request.id.to_string(),
                    metadata: json!({
                        "event_id": event_id,
                        "auto_approved": false,
                    }),
                })
                .await;

            Ok((
                StatusCode::CREATED,
                Json(json!({
                    "message": "Ownership request submitted; awaiting owner or admin review",
                    "auto_approved": false,
                    "request": request,
                })),
            ))
        }
        RequestOutcome::AutoApproved {
            request,
            previous_owner,
            matched_domain,
            superseded_count,
        } => {
            let request_id = request.id;
            let new_owner = request.requester_user_id.clone();

            // Two rows: the request, then the no-human approval. Recording
            // both keeps the audit trail symmetric with the manual path
            // (request + approve) so the queue reads consistently.
            service
                .audit_log_logic
                .record_best_effort(AuditRecord {
                    actor_user_id: actor_user_id.clone(),
                    action: actions::EVENT_OWNERSHIP_REQUEST.to_string(),
                    target_type: target_types::EVENT_OWNERSHIP_REQUEST.to_string(),
                    target_id: request_id.to_string(),
                    metadata: json!({
                        "event_id": event_id,
                        "auto_approved": true,
                    }),
                })
                .await;
            service
                .audit_log_logic
                .record_best_effort(AuditRecord {
                    actor_user_id,
                    action: actions::EVENT_OWNERSHIP_AUTO_APPROVE.to_string(),
                    target_type: target_types::EVENT_OWNERSHIP_REQUEST.to_string(),
                    target_id: request_id.to_string(),
                    metadata: json!({
                        "event_id": event_id,
                        "matched_domain": matched_domain.clone(),
                        "previous_owner": previous_owner,
                        "new_owner": new_owner,
                        "superseded_count": superseded_count,
                    }),
                })
                .await;

            Ok((
                StatusCode::CREATED,
                Json(json!({
                    "message": "Ownership auto-approved: your verified email domain matched the event website",
                    "auto_approved": true,
                    "matched_domain": matched_domain,
                    "request": request,
                })),
            ))
        }
    }
}

/// `GET /ownership-requests/mine` — every request the caller has filed,
/// newest first. Read-only; no audit.
pub async fn list_mine(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let requests = service
        .event_ownership_request_logic
        .list_my_requests(&claims)
        .await?;
    Ok(Json(requests))
}

/// `GET /ownership-requests/incoming` — the review queue. A regular user
/// sees pending requests for events they own; an admin sees the global
/// pending queue (the only path that surfaces requests for unowned seed
/// events). Read-only; no audit.
pub async fn list_incoming(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let requests = service
        .event_ownership_request_logic
        .list_incoming_requests(&claims)
        .await?;
    Ok(Json(requests))
}

/// `POST /ownership-request/{id}/approve` — the event's current owner or
/// any admin approves; ownership transfers to the requester. Bodyless.
pub async fn approve(
    Extension(claims): Extension<Claims>,
    Path(request_id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let actor_user_id = claims.sub.clone();
    let ApprovalOutcome {
        request,
        method,
        previous_owner,
        new_owner,
        superseded_count,
    } = service
        .event_ownership_request_logic
        .approve_request(request_id, &claims)
        .await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id,
            action: actions::EVENT_OWNERSHIP_APPROVE.to_string(),
            target_type: target_types::EVENT_OWNERSHIP_REQUEST.to_string(),
            target_id: request_id.to_string(),
            metadata: json!({
                "event_id": request.event_id,
                "method": method,
                "previous_owner": previous_owner,
                "new_owner": new_owner,
                "superseded_count": superseded_count,
            }),
        })
        .await;

    Ok(Json(json!({
        "message": "Ownership request approved",
        "request": request,
    })))
}

/// `POST /ownership-request/{id}/reject` — owner or admin declines. No
/// ownership moves. Bodyless.
pub async fn reject(
    Extension(claims): Extension<Claims>,
    Path(request_id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let actor_user_id = claims.sub.clone();
    let RejectionOutcome { request, method } = service
        .event_ownership_request_logic
        .reject_request(request_id, &claims)
        .await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id,
            action: actions::EVENT_OWNERSHIP_REJECT.to_string(),
            target_type: target_types::EVENT_OWNERSHIP_REQUEST.to_string(),
            target_id: request_id.to_string(),
            metadata: json!({
                "event_id": request.event_id,
                "method": method,
            }),
        })
        .await;

    Ok(Json(json!({
        "message": "Ownership request rejected",
        "request": request,
    })))
}

#[cfg(test)]
mod tests {
    //! Route-layer integration tests for the ownership workflow. These
    //! exercise the full HTTP path via `Router::oneshot(...)` over an
    //! in-memory pool + real `AppState`, with synthetic `Claims` injected
    //! by a `from_fn_with_state` middleware (production uses the JWT
    //! layer). The logic-layer branches (auto-approval, RBAC, transfer)
    //! are pinned exhaustively in `logic::event_ownership_request_logic`;
    //! the job HERE is to pin the things only the route layer owns:
    //!   - the HTTP status + JSON envelope of each outcome,
    //!   - that the right audit rows get written (and only those), and
    //!   - that a missing-`Claims` request can't silently succeed.

    use crate::context::{
        AuditLogContext, CampingProfileContext, EventContext, EventOwnershipRequestContext,
        EventTypeContext, JwtRevocationContext, MicroeventContext, RefreshTokenContext,
        UserCollectionContext, UserContext,
    };
    use crate::logic::{
        AuditLogLogic, CampingProfileLogic, EventLogic, EventOwnershipRequestLogic, EventTypeLogic,
        JwtRevocationLogic, MicroeventLogic, RefreshTokenLogic, UserCollectionLogic, UserLogic,
    };
    use crate::models::audit::actions;
    use crate::models::user::{Claims, UserRole};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, post};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt; // brings `oneshot` into scope

    // --- fixtures -----------------------------------------------------

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
        // Migrations already seed the `event_types` id=1 sentinel
        // ("Uncategorized"), which is what `seed_event` references, so no
        // extra event_type seeding is needed here.
        pool
    }

    /// Build a real AppState from a pool — same construction as `main.rs`
    /// and the `routes::events` test helper, including the ownership logic.
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
        let event_ownership_request_context = EventOwnershipRequestContext::new(pool.clone());
        let ownership_user_context = UserContext::new(pool.clone());
        let event_ownership_request_logic = Arc::new(EventOwnershipRequestLogic::new(
            event_ownership_request_context,
            event_context.clone(),
            ownership_user_context,
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
            event_ownership_request_logic,
        })
    }

    /// Synthetic-claims injector keyed by the value handed to
    /// `from_fn_with_state` — lets each test act as a different user/role
    /// against the same pool. Production injects `Claims` via the JWT
    /// middleware; this stands in for it.
    async fn inject_claims(
        axum::extract::State(claims): axum::extract::State<Claims>,
        mut req: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        req.extensions_mut().insert(claims);
        next.run(req).await
    }

    /// Router mounting all five ownership routes with `claims` injected.
    fn build_app(pool: sqlx::SqlitePool, claims: Claims) -> Router {
        let state = make_app_state(pool);
        Router::new()
            .route("/event/{id}/ownership-request", post(super::request))
            .route("/ownership-requests/mine", get(super::list_mine))
            .route("/ownership-requests/incoming", get(super::list_incoming))
            .route("/ownership-request/{id}/approve", post(super::approve))
            .route("/ownership-request/{id}/reject", post(super::reject))
            .layer(axum::middleware::from_fn_with_state(claims, inject_claims))
            .with_state(state)
    }

    fn claims(sub: &str, email: &str, role: UserRole) -> Claims {
        Claims {
            sub: sub.to_string(),
            email: email.to_string(),
            username: "tester".to_string(),
            role,
            exp: 9_999_999_999,
            iat: 0,
            jti: "test-jti".to_string(),
            aud: crate::custom_middleware::auth_middleware::EXPECTED_AUDIENCE.to_string(),
        }
    }

    async fn seed_user(
        pool: &sqlx::SqlitePool,
        id: &str,
        email: Option<&str>,
        verified: bool,
        role: &str,
    ) {
        sqlx::query(
            "INSERT INTO users \
             (id, oauth_id, oauth_provider, user_name, email, email_verified, role, created_at, updated_at) \
             VALUES (?1, ?2, 'google', ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
        )
        .bind(id)
        .bind(format!("oauth-{id}"))
        .bind(format!("name-{id}"))
        .bind(email)
        .bind(verified)
        .bind(role)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_event(pool: &sqlx::SqlitePool, name: &str, website: Option<&str>) -> i64 {
        let event_data = serde_json::json!({ "name": name, "archive": false }).to_string();
        let res = sqlx::query(
            "INSERT INTO events (name, description, website, event_type_id, event_data) \
             VALUES (?1, ?2, ?3, 1, ?4)",
        )
        .bind(name)
        .bind(format!("{name} description"))
        .bind(website)
        .bind(event_data)
        .execute(pool)
        .await
        .expect("seed event");
        res.last_insert_rowid()
    }

    /// Seed initial ownership through the real `UserCollectionLogic` so the
    /// `created_events` JSON encoding matches what the code reads back.
    async fn seed_ownership(pool: &sqlx::SqlitePool, event_id: i64, user_id: &str) {
        let state = make_app_state(pool.clone());
        state
            .user_collection_logic
            .event_ownership(event_id, &user_id.to_string())
            .await
            .expect("seed ownership");
    }

    async fn current_owner(pool: &sqlx::SqlitePool, event_id: i64) -> Option<String> {
        let state = make_app_state(pool.clone());
        state
            .user_collection_logic
            .find_event_owner(event_id)
            .await
            .expect("find owner")
    }

    async fn count_audit(pool: &sqlx::SqlitePool, action: &str) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM admin_audit_log WHERE action = ?1")
            .bind(action)
            .fetch_one(pool)
            .await
            .expect("count audit")
    }

    /// Parse the `metadata` JSON of the most recent audit row for `action`.
    /// Used to assert the route threaded `superseded_count` into the trail.
    async fn latest_audit_metadata(pool: &sqlx::SqlitePool, action: &str) -> serde_json::Value {
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT metadata FROM admin_audit_log WHERE action = ?1 ORDER BY id DESC LIMIT 1",
        )
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("fetch latest audit metadata");
        serde_json::from_str(&raw).expect("parse audit metadata json")
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("parse json")
    }

    /// File a pending request as `requester` (whose email must NOT match the
    /// event website, so it parks pending) and return the new request id.
    async fn file_pending_request(
        pool: &sqlx::SqlitePool,
        requester: Claims,
        event_id: i64,
    ) -> i64 {
        let app = build_app(pool.clone(), requester);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/event/{event_id}/ownership-request"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let parsed = body_json(response).await;
        assert_eq!(
            parsed["auto_approved"],
            serde_json::json!(false),
            "fixture must park pending"
        );
        parsed["request"]["id"].as_i64().expect("request id")
    }

    // --- tests --------------------------------------------------------

    #[tokio::test]
    async fn request_without_claims_extension_fails() {
        // Defense in depth: if the JWT layer were ever dropped from these
        // routes, the handler's required `Extension<Claims>` must still
        // refuse to create an anonymous request rather than succeed.
        let pool = setup_pool().await;
        let event = seed_event(&pool, "Orphan Fest", None).await;
        let state = make_app_state(pool.clone());
        let app = Router::new()
            .route("/event/{id}/ownership-request", post(super::request))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/event/{event}/ownership-request"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status().is_server_error() || response.status() == StatusCode::UNAUTHORIZED,
            "missing Claims must surface as 5xx (or 401); got {}",
            response.status()
        );
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_REQUEST).await,
            0
        );
    }

    #[tokio::test]
    async fn request_creates_pending_and_writes_one_audit_row() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "user-b", Some("b@somewhere.else"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;

        let app = build_app(
            pool.clone(),
            claims("user-b", "b@somewhere.else", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/event/{event}/ownership-request"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let parsed = body_json(response).await;
        assert_eq!(parsed["auto_approved"], serde_json::json!(false));
        assert_eq!(parsed["request"]["status"], "pending");
        assert_eq!(parsed["request"]["requester_user_id"], "user-b");

        // Exactly one request row; no auto-approve row on the pending path.
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_REQUEST).await,
            1
        );
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_AUTO_APPROVE).await,
            0
        );
        // Ownership unchanged while the request waits.
        assert_eq!(
            current_owner(&pool, event).await.as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn request_auto_approves_on_verified_domain_match() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "band-b", Some("booking@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;

        let app = build_app(
            pool.clone(),
            claims("band-b", "booking@coolfest.com", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/event/{event}/ownership-request"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let parsed = body_json(response).await;
        assert_eq!(parsed["auto_approved"], serde_json::json!(true));
        assert_eq!(parsed["matched_domain"], "coolfest.com");
        assert_eq!(parsed["request"]["status"], "approved");
        assert_eq!(parsed["request"]["resolution_method"], "domain_auto");

        // Both audit rows: the request AND the no-human auto-approve.
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_REQUEST).await,
            1
        );
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_AUTO_APPROVE).await,
            1
        );
        // Ownership moved A -> B inline.
        assert_eq!(current_owner(&pool, event).await.as_deref(), Some("band-b"));
    }

    #[tokio::test]
    async fn approve_by_non_owner_non_admin_is_forbidden() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "user-b", Some("b@somewhere.else"), true, "user").await;
        seed_user(&pool, "stranger-c", Some("c@nowhere.test"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;
        let request_id = file_pending_request(
            &pool,
            claims("user-b", "b@somewhere.else", UserRole::User),
            event,
        )
        .await;

        // C is neither the owner nor an admin.
        let app = build_app(
            pool.clone(),
            claims("stranger-c", "c@nowhere.test", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/ownership-request/{request_id}/approve"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        // The rejected path writes no approve row and moves no ownership.
        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_APPROVE).await,
            0
        );
        assert_eq!(
            current_owner(&pool, event).await.as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn approve_by_owner_transfers_and_writes_audit() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "user-b", Some("b@somewhere.else"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;
        let request_id = file_pending_request(
            &pool,
            claims("user-b", "b@somewhere.else", UserRole::User),
            event,
        )
        .await;

        let app = build_app(
            pool.clone(),
            claims("owner-a", "a@old.example", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/ownership-request/{request_id}/approve"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let parsed = body_json(response).await;
        assert_eq!(parsed["request"]["status"], "approved");
        assert_eq!(parsed["request"]["resolution_method"], "owner_approval");
        assert_eq!(parsed["request"]["resolved_by_user_id"], "owner-a");

        assert_eq!(
            count_audit(&pool, actions::EVENT_OWNERSHIP_APPROVE).await,
            1
        );
        // Ownership now sits with the requester.
        assert_eq!(current_owner(&pool, event).await.as_deref(), Some("user-b"));
    }

    #[tokio::test]
    async fn approve_supersedes_sibling_and_records_count_in_audit() {
        // Two different users hold open claims on one event (the partial
        // unique index only blocks duplicate (event, requester) pairs, so
        // distinct requesters coexist). Approving one must retire the other
        // as `superseded` AND surface the retired count in the approve
        // audit metadata — the route-owned half of the F1 mitigation.
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "band-b", Some("b@somewhere.else"), true, "user").await;
        seed_user(&pool, "crew-c", Some("c@nowhere.test"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;

        let req_b = file_pending_request(
            &pool,
            claims("band-b", "b@somewhere.else", UserRole::User),
            event,
        )
        .await;
        let req_c = file_pending_request(
            &pool,
            claims("crew-c", "c@nowhere.test", UserRole::User),
            event,
        )
        .await;

        // Owner approves B's request.
        let app = build_app(
            pool.clone(),
            claims("owner-a", "a@old.example", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/ownership-request/{req_b}/approve"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        // Ownership moved to B, and only B.
        assert_eq!(current_owner(&pool, event).await.as_deref(), Some("band-b"));

        // The approve audit row carries the retired-sibling count.
        let metadata = latest_audit_metadata(&pool, actions::EVENT_OWNERSHIP_APPROVE).await;
        assert_eq!(metadata["superseded_count"], serde_json::json!(1));
        assert_eq!(metadata["new_owner"], "band-b");

        // C's still-pending sibling is now retired as `superseded`, not a
        // human rejection — the distinguishing resolution_method.
        let (status, method): (String, Option<String>) = sqlx::query_as(
            "SELECT status, resolution_method FROM event_ownership_requests WHERE id = ?1",
        )
        .bind(req_c)
        .fetch_one(&pool)
        .await
        .expect("read sibling request");
        assert_eq!(status, "rejected");
        assert_eq!(method.as_deref(), Some("superseded"));

        // And C can no longer be approved — its claim is closed.
        let app = build_app(
            pool.clone(),
            claims("owner-a", "a@old.example", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/ownership-request/{req_c}/approve"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn reject_by_admin_writes_audit_and_keeps_owner() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "user-b", Some("b@somewhere.else"), true, "user").await;
        seed_user(&pool, "admin-z", Some("z@admin.test"), true, "admin").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;
        let request_id = file_pending_request(
            &pool,
            claims("user-b", "b@somewhere.else", UserRole::User),
            event,
        )
        .await;

        // Admin fallback can resolve any request in the queue.
        let app = build_app(
            pool.clone(),
            claims("admin-z", "z@admin.test", UserRole::Admin),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/ownership-request/{request_id}/reject"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let parsed = body_json(response).await;
        assert_eq!(parsed["request"]["status"], "rejected");
        assert_eq!(parsed["request"]["resolution_method"], "admin_rejection");

        assert_eq!(count_audit(&pool, actions::EVENT_OWNERSHIP_REJECT).await, 1);
        // A rejection moves no ownership.
        assert_eq!(
            current_owner(&pool, event).await.as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn list_mine_returns_only_the_callers_requests() {
        let pool = setup_pool().await;
        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "user-b", Some("b@somewhere.else"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        seed_ownership(&pool, event, "owner-a").await;
        let request_id = file_pending_request(
            &pool,
            claims("user-b", "b@somewhere.else", UserRole::User),
            event,
        )
        .await;

        let app = build_app(
            pool.clone(),
            claims("user-b", "b@somewhere.else", UserRole::User),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/ownership-requests/mine")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let parsed = body_json(response).await;
        let arr = parsed.as_array().expect("array response");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"].as_i64(), Some(request_id));
        assert_eq!(arr[0]["requester_user_id"], "user-b");
    }
}
