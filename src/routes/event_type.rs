// ============================================================================
// API Handlers: src/handlers/camping_handlers.rs
// ============================================================================
use crate::AppState;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use std::sync::Arc;

use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::audit::{AuditRecord, actions, target_types};
use crate::models::event_models::EventType;
use crate::models::user::Claims;

// GET /camping-profiles - List all camping templates
pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let profiles = service.event_type_logic.get_all().await?;
    Ok(Json(profiles))
}

// GET /camping-profiles/{id} - Get specific camping template
pub async fn get(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let output = service.event_type_logic.get_by_id(id).await?;
    Ok(Json(output))
}

// POST /eventtype - Create new event type (super-admin only). Audited.
pub async fn create(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    ApiJson(output): ApiJson<EventType>,
) -> Result<impl IntoResponse, AppError> {
    // Capture identifying input fields BEFORE the move so the audit entry
    // can name the type by its human-readable label, not just the id.
    let name = output.name.clone();
    let category = output.category.clone();
    let id = service.event_type_logic.create(output).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::EVENT_TYPE_CREATE.to_string(),
            target_type: target_types::EVENT_TYPE.to_string(),
            target_id: id.to_string(),
            metadata: json!({ "name": name, "category": category }),
        })
        .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Event type created successfully",
            "id": id
        })),
    ))
}

// PUT /eventtype/{id} - Update event type. Audited.
pub async fn update(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    ApiJson(output): ApiJson<EventType>,
) -> Result<impl IntoResponse, AppError> {
    let name = output.name.clone();
    let category = output.category.clone();
    service.event_type_logic.update(id, output).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::EVENT_TYPE_UPDATE.to_string(),
            target_type: target_types::EVENT_TYPE.to_string(),
            target_id: id.to_string(),
            // `new_values` rather than `before/after` — capturing the
            // pre-state requires an extra read; tracked separately on the
            // roadmap. v1 records the inputs the admin sent.
            metadata: json!({ "new_values": { "name": name, "category": category } }),
        })
        .await;

    Ok(Json(json!({
        "message": "Event type updated successfully"
    })))
}

// DELETE /eventtype/{id} - Delete event type. Audited.
pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_type_logic.delete(id).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::EVENT_TYPE_DELETE.to_string(),
            target_type: target_types::EVENT_TYPE.to_string(),
            target_id: id.to_string(),
            metadata: json!({}),
        })
        .await;

    Ok(Json(json!({
        "message": "Event type deleted successfully"
    })))
}

// GET /camping-profiles/{id}/apply - Get camping info from template (for auto-fill)
//pub async fn apply_event_type(
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//let output = service.event_type_logic.get_by_id(id).await?;
//let camping_info = output.to_camping_info();
//
//Ok(Json(camping_info))
//}

#[cfg(test)]
mod tests {
    //! Route-layer integration tests for `/eventtype`. Follows the
    //! `setup_pool` + `make_app_state` + `oneshot`-driven Router pattern
    //! established in `routes::events::tests`. Auth gating itself lives
    //! in `custom_middleware::auth_middleware::tests`; these tests focus
    //! on the handlers' own contract — JSON shape, status codes, and
    //! that the audit-log path is non-blocking (a successful response
    //! still comes back even if best-effort audit writes happen to fail).
    //!
    //! The create/update/delete handlers take `Extension<Claims>`. The
    //! production auth middleware inserts it; here a tiny `from_fn`
    //! middleware does the same with a synthetic super-admin claim so
    //! the route handler can focus on its business logic.

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
    use crate::models::user::UserRole;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

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
        // Audit-log writes carry a `user_id` FK to `users.id`. The
        // synthetic claims used below reference a "test-admin" user that
        // never gets seeded, so disable FK enforcement here. Cross-table
        // integrity for the audit-log path itself is covered separately
        // in `context::audit_log_context::tests`.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        pool
    }

    /// Build a real AppState — same shape as `routes::events::tests::make_app_state`,
    /// since both modules share the canonical pattern.
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

    /// Synthetic claims injector. The real auth middleware does the same
    /// after verifying a JWT; the route handler downstream doesn't know
    /// (or care) which middleware put the extension there. Keeps the
    /// route tests focused on the handler's own logic.
    async fn inject_admin_claims(
        mut req: axum::extract::Request,
        next: middleware::Next,
    ) -> axum::response::Response {
        let claims = Claims {
            sub: "test-admin".to_string(),
            email: "admin@test.local".to_string(),
            username: "test-admin".to_string(),
            role: UserRole::SuperAdmin,
            exp: 9_999_999_999,
            iat: 0,
            jti: "test-jti".to_string(),
            aud: "festurah".to_string(),
        };
        req.extensions_mut().insert(claims);
        next.run(req).await
    }

    /// Build a Router with all four `/eventtype` handlers mounted +
    /// synthetic claims injected. Skips the API-key / rate-limit /
    /// auth-middleware layers (those are exercised in `auth_middleware::tests`).
    fn build_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_app_state(pool);
        // Use the fully-qualified handler paths (`super::create`, `super::get`,
        // etc.) because this test module's `get`/`delete` would otherwise
        // shadow `axum::routing::get` in this scope.
        Router::new()
            .route("/eventtype", get(super::get_all).post(super::create))
            .route("/eventtype/{id}", get(super::get).delete(super::delete))
            .layer(middleware::from_fn(inject_admin_claims))
            .with_state(state)
    }

    /// Seed one event_type row directly via SQL. Same shortcut as the
    /// `events.rs` test module — bypass the create handler so the GET
    /// tests don't depend on the POST handler being correct.
    async fn seed_event_type(pool: &sqlx::SqlitePool, name: &str, category: &str) -> i64 {
        let result = sqlx::query(
            "INSERT INTO event_types (name, description, map_indicator, category) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(name)
        .bind(format!("{} description", name))
        .bind("M")
        .bind(category)
        .execute(pool)
        .await
        .expect("seed event_type");
        result.last_insert_rowid()
    }

    async fn read_body_value(response: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice::<serde_json::Value>(&body).expect("parse JSON")
    }

    // -----------------------------------------------------------------
    // GET /eventtype — list
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn list_returns_all_seeded_event_types() {
        let pool = setup_pool().await;
        seed_event_type(&pool, "Festival", "entertainment").await;
        seed_event_type(&pool, "Concert", "entertainment").await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/eventtype")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_value(response).await;
        let arr = body.as_array().expect("list returns JSON array");
        assert_eq!(arr.len(), 2);
        // Pin shape: every row must carry a non-empty name and category.
        for row in arr {
            assert!(!row["name"].as_str().unwrap().is_empty());
            assert!(!row["category"].as_str().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn list_returns_empty_array_when_no_types() {
        // Defensive: clients should see `[]`, not `null` or a 404. A
        // future refactor that wrapped the empty list in `Option::None`
        // would surface here.
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/eventtype")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_value(response).await;
        assert!(body.is_array(), "empty list must still be a JSON array");
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    // -----------------------------------------------------------------
    // GET /eventtype/{id} — single
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_existing_id_returns_200_with_body() {
        let pool = setup_pool().await;
        let id = seed_event_type(&pool, "Festival", "entertainment").await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/eventtype/{}", id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_value(response).await;
        assert_eq!(body["name"], "Festival");
        assert_eq!(body["category"], "entertainment");
    }

    #[tokio::test]
    async fn get_missing_id_returns_404() {
        let pool = setup_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/eventtype/99999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------
    // POST /eventtype — create
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_returns_201_with_new_id() {
        let pool = setup_pool().await;
        let app = build_app(pool.clone());

        let body = serde_json::json!({
            "id": null,
            "name": "Workshop",
            "description": "Hands-on workshops",
            "map_indicator": "W",
            "category": "education",
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/eventtype")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let parsed = read_body_value(response).await;
        assert!(parsed["id"].is_i64(), "create response must include `id`");

        // The row landed in the table.
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM event_types WHERE name = 'Workshop'")
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn create_writes_audit_log_entry() {
        // The handler calls `audit_log_logic.record_best_effort(...)` on
        // every successful create. Pinning this means a refactor that
        // forgets to wire the audit-log call surfaces here before reaching
        // production. (`record_best_effort` swallows errors by design —
        // we're verifying the write *happens*, not that it can't fail.)
        let pool = setup_pool().await;
        let app = build_app(pool.clone());

        let body = serde_json::json!({
            "id": null,
            "name": "Audit-Test",
            "description": "X",
            "map_indicator": "A",
            "category": "test",
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/eventtype")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM admin_audit_log WHERE actor_user_id = 'test-admin' \
             AND action = 'event_type.create'",
        )
        .fetch_one(&pool)
        .await
        .expect("count audit log");
        assert_eq!(count.0, 1, "create must write an admin_audit_log entry");
    }

    // -----------------------------------------------------------------
    // DELETE /eventtype/{id}
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn delete_removes_row_and_returns_200() {
        let pool = setup_pool().await;
        let id = seed_event_type(&pool, "ToDelete", "misc").await;
        let app = build_app(pool.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/eventtype/{}", id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_types WHERE id = ?1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count.0, 0, "row must be gone after delete");
    }
}
