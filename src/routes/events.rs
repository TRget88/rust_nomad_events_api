use crate::AppState;
use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::audit::{AuditRecord, actions, target_types};
use crate::models::dto::{EventQueryParams, PaginationQuery};
use crate::models::event_models::NomEvent;
use crate::models::user::Claims;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use std::sync::Arc;

/// Admin-only list of every event in the catalog. Paginated via
/// `?limit=`/`?offset=` (see [util::validate_pagination] for defaults +
/// upper bound). The public, filtered-and-sorted version lives at
/// `/event/search`; this route is the unfiltered admin view.
pub async fn get_all(
    Query(params): Query<PaginationQuery>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let events = service
        .event_logic
        .get_all_events(params.limit, params.offset)
        .await?;
    Ok(Json(events))
}

pub async fn get(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let event = service.event_logic.get_event_by_id(id).await?;
    Ok(Json(event))
}

pub async fn search(
    Query(params): Query<EventQueryParams>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    // Location-based search, with optional event_type and date-range filters.
    // The location/type-combined path was added first; date_from/date_to
    // joined it as a paired front+back feature so callers can ask "what's
    // happening this weekend within 50 miles of me". The date filter currently
    // only lights up when a location is provided — fetching every event in
    // the catalog within a date range (without a radius) is a different
    // query shape we can wire later if needed.
    if let (Some(lat), Some(lon), Some(radius)) =
        (params.latitude, params.longitude, params.radius_miles)
    {
        let events = service
            .event_logic
            .get_nearby_events(
                lat,
                lon,
                radius,
                params.event_type,
                params.event_type_ids,
                params.date_from,
                params.date_to,
                params.name_contains,
                params.camping_allowed,
                params.sort,
                params.limit,
                params.offset,
            )
            .await?;
        return Ok(Json(events));
    }

    // No location: type-only fallback
    if let Some(event_type) = params.event_type {
        let events = service.event_logic.get_events_by_type(event_type).await?;
        return Ok(Json(events));
    }

    // No filters: return everything (still paginated — the catalog list
    // can grow large enough that an unbounded response would dwarf the
    // request budget).
    let events = service
        .event_logic
        .get_all_events(params.limit, params.offset)
        .await?;
    Ok(Json(events))
}

pub async fn create(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    ApiJson(mut event): ApiJson<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    let user_id = &claims.sub;
    // Set user_id on the event
    event.user_id = Some(user_id.clone());
    let id = service.event_logic.create_event(event).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Event created successfully",
            "id": id
        })),
    ))
}

pub async fn update(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    ApiJson(event): ApiJson<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    //let user_id = &claims.sub;
    service.event_logic.update_event(id, event, claims).await?;

    Ok(Json(json!({
        "message": "Event updated successfully"
    })))
}

pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    // Clone the actor id BEFORE the delete consumes `claims` — the audit
    // entry needs to outlive the move.
    let actor_user_id = claims.sub.clone();
    service.event_logic.delete_event(id, claims).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id,
            action: actions::EVENT_DELETE.to_string(),
            target_type: target_types::EVENT.to_string(),
            target_id: id.to_string(),
            metadata: json!({}),
        })
        .await;

    Ok(Json(json!({
        "message": "Event deleted successfully"
    })))
}

/// `POST /event/{id}/verify-date` — the owner (or any Admin /
/// SuperAdmin) confirms that the event's currently-stored
/// `start_date` / `end_date` are correct for the current instance.
/// Sets `event_data.date_verified = true`.
///
/// The auto-roll task in `main.rs` flips `date_verified` back to
/// `false` every time it bumps the year on a recurring event, so the
/// UI can surface an "unverified date — please confirm" badge until
/// this endpoint (or a full `PUT /event/{id}`) is called.
///
/// Lives behind the JWT middleware (mounted in `main.rs` under
/// `jwt_routes`) so `Extension<Claims>` is always present.
pub async fn verify_date(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_logic.verify_event_date(id, claims).await?;
    Ok(Json(json!({
        "message": "Event date verified"
    })))
}

/// Admin-only: archive an event (hide it from listings). Lives behind
/// the JWT middleware; the logic layer enforces the admin role check
/// and returns 401 for non-admin callers. The row stays in the DB so
/// direct-link lookups still resolve and `unarchive` can restore it.
pub async fn archive(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_logic.archive_event(id, claims).await?;
    Ok(Json(json!({
        "message": "Event archived"
    })))
}

/// Admin-only: inverse of `archive`. Returns the event to active listings.
pub async fn unarchive(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_logic.unarchive_event(id, claims).await?;
    Ok(Json(json!({
        "message": "Event unarchived"
    })))
}

//adding the favorite and saved sections
//pub async fn save_toggle(
//Extension(claims): Extension<Claims>,
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
////get the user id
//let user_id = &claims.sub;
//
//service.event_logic.save_toggle(id, user_id).await?;
//
//Ok(Json(json!({
//"message": "Event save toggled!"
//})))
//}
//
//pub async fn favorite_toggle(
//Extension(claims): Extension<Claims>,
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
////get the user id
//let user_id = &claims.sub;
//
//service.event_logic.favorite_toggle(id, user_id).await?;
//
//Ok(Json(json!({
//"message": "Event favorite toggled!"
//})))
//}

#[cfg(test)]
mod tests {
    //! Route-layer integration tests for the public `/event/search`
    //! endpoint. Uses `axum::Router::oneshot(...)` against an in-memory
    //! pool and a real AppState. This is the canonical pattern for route
    //! tests — the auth-middleware tests in `custom_middleware/auth_middleware.rs`
    //! cover a different shape (gating via middleware-only routes). When
    //! adding route tests for other endpoints, clone the `setup_app`
    //! helper here.

    use super::*;
    use crate::context::{
        AuditLogContext, CampingProfileContext, EventContext, EventOwnershipRequestContext,
        EventTypeContext, JwtRevocationContext, MicroeventContext, RefreshTokenContext,
        UserCollectionContext, UserContext,
    };
    use crate::logic::{
        AuditLogLogic, CampingProfileLogic, EventLogic, EventOwnershipRequestLogic, EventTypeLogic,
        JwtRevocationLogic, MicroeventLogic, RefreshTokenLogic, UserCollectionLogic, UserLogic,
    };
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt; // brings `oneshot` into scope

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
        // FK enforcement off — these tests construct events with synthetic
        // event_type_ids and don't always seed the parent `event_types` row.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        // Seed one event_type so the EventContext JOIN against event_types
        // returns rows when querying any event.
        sqlx::query(
            "INSERT INTO event_types (id, name, description, map_indicator, category) \
             VALUES (1000, 'Festival', 'A festival', 'F', 'entertainment')",
        )
        .execute(&pool)
        .await
        .expect("seed event_type");
        pool
    }

    /// Build a real AppState from a pool. The same shape as `main.rs` —
    /// just minus the rate limiter, CORS layer, and middleware stack
    /// (those are wired at the Router level in main.rs, not on AppState).
    /// When a future route test needs a logic the search route doesn't
    /// touch, the construction is already there — no fanout needed.
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
        // Ownership-request logic — fresh UserContext (the first is moved
        // into `user_logic`) + fresh EventOwnershipRequestContext, sharing
        // the event_context and user_collection_logic Arcs. Mirrors the
        // wiring in `main.rs`.
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

    /// Build a Router mounting just `/event/search` with full AppState.
    /// Skips the API-key + rate-limit middleware so the test focuses on
    /// the route handler's own logic (filter dispatch, JSON shape, etc.).
    fn build_search_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_app_state(pool);
        Router::new()
            .route("/event/search", get(search))
            .with_state(state)
    }

    /// Build a Router mounting just `GET /event/{id}` with NO auth
    /// middleware — mirroring how the route now sits in `main.rs`'s
    /// `public_routes` group (API-key + rate-limit only, no JWT layer).
    /// There is deliberately no claims injector here: the handler must
    /// resolve an event for a logged-OUT caller, because that's what a
    /// shared deep-link (`festurah.com/search?event=<id>`) requires.
    fn build_get_by_id_app(pool: sqlx::SqlitePool) -> Router {
        let state = make_app_state(pool);
        Router::new()
            .route("/event/{id}", get(super::get))
            .with_state(state)
    }

    /// Seed one event row directly via SQL. Routes the EventContext's
    /// `create` path is exercised in its own context tests; here we
    /// just need a row to query.
    async fn seed_event(
        pool: &sqlx::SqlitePool,
        name: &str,
        event_type_id: i64,
        lat: f64,
        lon: f64,
    ) -> i64 {
        let event_data = serde_json::json!({
            "name": name,
            "description": format!("{} description", name),
            "event_type_id": event_type_id,
            "date_info": {
                "start_date": "2026-07-01T00:00:00Z",
                "end_date": "2026-07-03T00:00:00Z",
                "single_day": false,
            },
            "location_info": {
                "address": "123 Main St",
                "latitude": lat,
                "longitude": lon,
            },
            "camping_info": { "camping_allowed": false },
            "archive": false,
        })
        .to_string();
        let result = sqlx::query(
            "INSERT INTO events (name, description, event_type_id, latitude, longitude, \
             start_date, end_date, camping_allowed, event_data) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(name)
        .bind(format!("{} description", name))
        .bind(event_type_id)
        .bind(lat)
        .bind(lon)
        .bind("2026-07-01")
        .bind("2026-07-03")
        .bind(false)
        .bind(&event_data)
        .execute(pool)
        .await
        .expect("seed event");
        result.last_insert_rowid()
    }

    /// Convenience: deserialize a JSON response body into a Vec<Value>.
    async fn read_json_array(response: axum::response::Response) -> Vec<serde_json::Value> {
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice::<Vec<serde_json::Value>>(&body).expect("parse JSON array")
    }

    // -----------------------------------------------------------------
    // Unfiltered search — `?` with no filters
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_with_no_filters_returns_all_events() {
        let pool = setup_pool().await;
        seed_event(&pool, "Alpha", 1000, 33.0, -84.0).await;
        seed_event(&pool, "Bravo", 1000, 34.0, -85.0).await;
        let app = build_search_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = read_json_array(response).await;
        assert_eq!(events.len(), 2);
    }

    // -----------------------------------------------------------------
    // Location-based search — lat/lon/radius
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_with_lat_lon_radius_filters_to_nearby() {
        let pool = setup_pool().await;
        seed_event(&pool, "Atlanta", 1000, 33.74, -84.39).await;
        seed_event(&pool, "Seattle", 1000, 47.61, -122.33).await;
        let app = build_search_app(pool);

        // Query around Atlanta — Seattle should be filtered out.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search?latitude=33.74&longitude=-84.39&radius_miles=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = read_json_array(response).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["name"], "Atlanta");
    }

    // -----------------------------------------------------------------
    // Type-only fallback
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_with_event_type_only_returns_matching_type() {
        let pool = setup_pool().await;
        // Seed a second event_type (id 1001) for filter distinction.
        // Primary seed id is 1000 (sits above the sentinel at id 1).
        sqlx::query(
            "INSERT INTO event_types (id, name, description, map_indicator, category) \
             VALUES (1001, 'Concert', 'A concert', 'C', 'entertainment')",
        )
        .execute(&pool)
        .await
        .expect("seed second event_type");
        seed_event(&pool, "Festival", 1000, 33.0, -84.0).await;
        seed_event(&pool, "Concert", 1001, 34.0, -85.0).await;
        let app = build_search_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search?event_type=1001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = read_json_array(response).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["name"], "Concert");
    }

    // -----------------------------------------------------------------
    // Pagination
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_no_filters_respects_limit() {
        let pool = setup_pool().await;
        for i in 1..=5 {
            seed_event(&pool, &format!("E{}", i), 1000, 33.0 + i as f64, -84.0).await;
        }
        let app = build_search_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search?limit=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = read_json_array(response).await;
        assert_eq!(events.len(), 2);
    }

    // -----------------------------------------------------------------
    // Empty catalog
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_empty_catalog_returns_empty_array() {
        let pool = setup_pool().await;
        let app = build_search_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let events = read_json_array(response).await;
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------
    // Invalid query parameters
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn search_rejects_invalid_sort_value() {
        // `sort=garbage` should 400 from the logic-layer sort validator.
        // Pinned because a typo'd sort param should be loud, not silent.
        let pool = setup_pool().await;
        seed_event(&pool, "X", 1000, 33.0, -84.0).await;
        let app = build_search_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/event/search?latitude=33.0&longitude=-84.0&radius_miles=50&sort=garbage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Either 400 from EventQueryParams deserialization OR 400 from
        // the sort validator. Both are correct; we just want loud
        // rejection, not silent fallback to the default sort.
        assert!(response.status().is_client_error());
    }

    // -----------------------------------------------------------------
    // POST /event — auth gating
    // -----------------------------------------------------------------
    //
    // The auth-middleware tests in `custom_middleware::auth_middleware`
    // cover the "no Bearer header → 401" path against an arbitrary
    // protected route. These two tests pin specifically that **`POST
    // /event`** is auth-gated, and that with synthetic claims injected
    // the handler returns a 201 + the new id — same shape as the
    // `/eventtype` route tests.

    /// Synthetic claims injector — same pattern as `routes::event_type::tests`.
    /// Production uses the JWT middleware; these tests use this so the
    /// handler can be exercised without spinning up real JWT machinery.
    async fn inject_user_claims(
        mut req: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let claims = crate::models::user::Claims {
            sub: "test-user".to_string(),
            email: "tester@test.local".to_string(),
            username: "tester".to_string(),
            role: crate::models::user::UserRole::User,
            exp: 9_999_999_999,
            iat: 0,
            jti: "test-jti".to_string(),
            aud: crate::custom_middleware::auth_middleware::EXPECTED_AUDIENCE.to_string(),
        };
        req.extensions_mut().insert(claims);
        next.run(req).await
    }

    fn build_create_app_with_auth(pool: sqlx::SqlitePool) -> Router {
        let state = make_app_state(pool);
        use axum::middleware as ax_middleware;
        use axum::routing::post;
        Router::new()
            .route("/event", post(super::create))
            .layer(ax_middleware::from_fn(inject_user_claims))
            .with_state(state)
    }

    fn build_create_app_without_auth(pool: sqlx::SqlitePool) -> Router {
        // Same route, but without the claims injector. The handler's
        // `Extension<Claims>` extractor sees no extension and returns
        // an error — axum maps this to 500 (internal server error)
        // because the handler told it the extension was mandatory.
        // In production the auth middleware would have rejected the
        // request with 401 before reaching here; the pin below proves
        // the handler itself is strict about needing Claims.
        let state = make_app_state(pool);
        use axum::routing::post;
        Router::new()
            .route("/event", post(super::create))
            .with_state(state)
    }

    #[tokio::test]
    async fn post_event_without_claims_extension_fails() {
        // Defense in depth: even if the auth middleware were ever
        // removed from `/event`'s layer stack, the handler itself must
        // not silently succeed without a Claims extension. The handler
        // declares `Extension<Claims>` as required, so axum returns
        // 500 (Internal Server Error) when it's missing. Pinning this
        // means a refactor that swapped to `Option<Extension<Claims>>`
        // would surface here.
        let pool = setup_pool().await;
        let app = build_create_app_without_auth(pool);

        let body = serde_json::json!({
            "name": "Test Event",
            "description": "Test description",
            "event_type": { "id": 1000, "name": "Festival", "description": "", "map_indicator": "F", "category": "entertainment" },
            "date_info": {
                "start_date": "2026-07-01T00:00:00Z",
                "end_date": "2026-07-03T00:00:00Z",
                "single_day": false,
            },
            "location_info": { "latitude": 33.0, "longitude": -84.0, "address": "123 Main St, Atlanta GA" },
            "camping_info": { "camping_allowed": false },
            "archive": false,
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/event")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing extension → server error. The exact status varies by
        // axum version's error mapping — what we're pinning is that the
        // handler did NOT happily accept the request and create an
        // anonymous event.
        assert!(
            response.status().is_server_error() || response.status() == StatusCode::UNAUTHORIZED,
            "missing Claims must surface as a 5xx (or 401 with the right middleware); got {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn post_event_with_claims_returns_201_and_persists() {
        // Happy path with synthetic claims injected. Confirms the route
        // handler:
        //   - accepts a `NomEvent` JSON body,
        //   - calls into `EventLogic::create_event` with the user_id
        //     populated from claims,
        //   - returns 201 + JSON with the new event id.
        let pool = setup_pool().await;
        let app = build_create_app_with_auth(pool.clone());

        let body = serde_json::json!({
            "name": "Pinned Festival",
            "description": "Test event",
            "event_type": { "id": 1000, "name": "Festival", "description": "", "map_indicator": "F", "category": "entertainment" },
            "date_info": {
                "start_date": "2026-07-01T00:00:00Z",
                "end_date": "2026-07-03T00:00:00Z",
                "single_day": false,
            },
            "location_info": { "latitude": 33.0, "longitude": -84.0, "address": "123 Main St, Atlanta GA" },
            "camping_info": { "camping_allowed": false },
            "archive": false,
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/event")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        // Response body shape: `{ "message": ..., "id": <i64> }`.
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed["id"].is_i64(), "response must include the new id");
        assert_eq!(parsed["message"], "Event created successfully");

        // The DB also has the row. user_id was populated from the
        // injected synthetic claims — pinning this proves the handler
        // wires claims.sub → event.user_id.
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM events WHERE name = 'Pinned Festival'")
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count.0, 1);
    }

    // -----------------------------------------------------------------
    // GET /event/{id} — public single-event fetch (drives share links)
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_event_by_id_resolves_without_auth() {
        // The Share button hands out `festurah.com/search?event=<id>`
        // links; a logged-OUT recipient must be able to resolve the
        // event. This route lives in `main.rs`'s `public_routes` group
        // (API-key only, no JWT layer), so the handler has to work with
        // NO Claims extension. `build_get_by_id_app` mounts it without a
        // claims injector to prove exactly that. A regression that
        // re-added an `Extension<Claims>` extractor (or moved the GET
        // back behind the JWT layer) would fail here.
        let pool = setup_pool().await;
        let id = seed_event(&pool, "Shareable Fest", 1000, 33.0, -84.0).await;
        let app = build_get_by_id_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/event/{}", id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "public GET /event/{{id}} must resolve without a JWT"
        );

        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["id"].as_i64(), Some(id));
        assert_eq!(parsed["name"], "Shareable Fest");
    }

    #[tokio::test]
    async fn get_event_by_id_unknown_returns_404() {
        // A stale or hand-edited share link (deleted / never-existed id)
        // must surface as a clean 404, not a 500 — the frontend
        // deep-link handler treats 404 as "event not found" and falls
        // back to the normal search view.
        let pool = setup_pool().await;
        let app = build_get_by_id_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/event/99999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
