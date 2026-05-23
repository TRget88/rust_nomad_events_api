// src/main.rs - Application Entry Point
use std::env;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::IntoResponse;
use axum::{
    Json, Router, middleware,
    routing::{delete, get, post, put},
};
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

mod context;
mod custom_middleware; // Rename to avoid conflict with axum::middleware
mod errors;
mod extractors;
mod logic;
mod models;
mod routes; // This imports and registers the trait impl
mod util;

use context::AuditLogContext;
use context::CampingProfileContext;
use context::EventContext;
use context::EventTypeContext;
use context::JwtRevocationContext;
use context::MicroeventContext;
use context::RefreshTokenContext;
use context::UserCollectionContext;
use context::UserContext;
use custom_middleware::rate_limit::RateLimiter;
use logic::AuditLogLogic;
use logic::CampingProfileLogic;
use logic::EventLogic;
use logic::EventTypeLogic;
use logic::JwtRevocationLogic;
use logic::MicroeventLogic;
use logic::RefreshTokenLogic;
use logic::UserCollectionLogic;
use logic::UserLogic;

// Test data modules
mod camping_profiles;
mod event_types;
mod seed_data;

use camping_profiles::create_standard_camping_profiles;
use event_types::create_standard_event_types;
use seed_data::seed_all;

use crate::custom_middleware::auth_middleware::AuthMiddlewareState;

#[derive(Clone)]
pub struct AppState {
    pub event_logic: Arc<EventLogic>,
    pub microevent_logic: Arc<MicroeventLogic>,
    pub event_type_logic: Arc<EventTypeLogic>,
    pub camping_profile_logic: Arc<CampingProfileLogic>,
    pub user_logic: Arc<UserLogic>,
    pub user_collection_logic: Arc<UserCollectionLogic>,
    pub jwt_revocation_logic: Arc<JwtRevocationLogic>,
    pub audit_log_logic: Arc<AuditLogLogic>,
    pub refresh_token_logic: Arc<RefreshTokenLogic>,
}

#[tokio::main]
async fn main() {
    // Load environment variables
    dotenv::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create database file if it doesn't exist
    if !std::path::Path::new("events.db").exists() {
        File::create("events.db").expect("Failed to create database file");
        tracing::info!("📁 Created events.db file");
    }

    // Database setup
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite://events.db?mode=rwc")
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("Failed to run migrations");

    // Seed database with test data only when explicitly requested. `seed_all`
    // is internally idempotent, but gating avoids the extra startup query on
    // every boot and makes seeding behavior explicit.
    if env::var("SEED").is_ok() {
        seed_all(&db).await.expect("Failed to seed database");
    }

    // Build layers: Context (DB queries) -> Logic (business rules) -> Routes.
    //
    // Contexts that have multiple consumers are wrapped in `Arc` so they
    // construct once and clone cheaply (refcount bump, no DB pool re-wiring).
    // Pre-refactor `EventContext` and `MicroeventContext` were each built
    // twice — once for `UserCollectionLogic`, once for the per-resource
    // logics — because the type isn't `Clone`. With `Arc<…>` both consumers
    // share one allocation.
    let eventcontext = Arc::new(EventContext::new(db.clone()));
    let microeventcontext = Arc::new(MicroeventContext::new(db.clone()));

    let eventtypecontext = EventTypeContext::new(db.clone());
    let eventtypelogic = Arc::new(EventTypeLogic::new(eventtypecontext));
    let campingprofilecontext = CampingProfileContext::new(db.clone());
    let campingprofilelogic = Arc::new(CampingProfileLogic::new(campingprofilecontext));
    let usercontext = UserContext::new(db.clone());
    let userlogic = Arc::new(UserLogic::new(usercontext));
    let usercollectioncontext = UserCollectionContext::new(db.clone());
    let usercollectionlogic = Arc::new(UserCollectionLogic::new(
        usercollectioncontext,
        eventcontext.clone(),
        microeventcontext.clone(),
    ));
    // EventLogic / MicroeventLogic share the same context Arcs that
    // UserCollectionLogic already holds.
    let eventlogic = Arc::new(EventLogic::new(
        eventcontext.clone(),
        usercollectionlogic.clone(),
    ));
    let microeventlogic = Arc::new(MicroeventLogic::new(
        microeventcontext.clone(),
        usercollectionlogic.clone(),
    ));

    let jwtrevocationcontext = JwtRevocationContext::new(db.clone());
    let jwtrevocationlogic = Arc::new(JwtRevocationLogic::new(jwtrevocationcontext));

    let auditlogcontext = AuditLogContext::new(db.clone());
    let auditloglogic = Arc::new(AuditLogLogic::new(auditlogcontext));

    // Refresh-token machinery. Context is Arc-wrapped because the logic
    // takes ownership of an Arc (lets a future admin endpoint share the
    // same context for direct queries without re-wiring the pool).
    let refreshtokencontext = Arc::new(RefreshTokenContext::new(db.clone()));
    let refreshtokenlogic = Arc::new(RefreshTokenLogic::new(refreshtokencontext));

    // State for the auth middleware. Bundles the DB pool (used for the
    // user-status lookup) with the JwtRevocationLogic (used for the
    // revocation check). Pre-refactor the middleware took just the pool
    // and inlined a duplicate revocation query; now it routes through
    // the logic layer per the project's `routes → logic → context` rule.
    let auth_middleware_state = AuthMiddlewareState {
        pool: db.clone(),
        jwt_revocation_logic: jwtrevocationlogic.clone(),
    };

    let app_state = Arc::new(AppState {
        event_logic: eventlogic,
        microevent_logic: microeventlogic,
        camping_profile_logic: campingprofilelogic,
        event_type_logic: eventtypelogic,
        user_logic: userlogic,
        user_collection_logic: usercollectionlogic,
        jwt_revocation_logic: jwtrevocationlogic.clone(),
        audit_log_logic: auditloglogic,
        refresh_token_logic: refreshtokenlogic.clone(),
    });

    // Background retention sweep — drops rows past their natural
    // expiry from `jwt_revocations` and `refresh_tokens`. Both tables
    // are bounded by their respective TTLs (24h for revoked access
    // tokens, 30d for refresh tokens), so without this they grow
    // forever even though the rows have lost any security value the
    // moment their `expires_at` passed.
    //
    // Interval is `RETENTION_SWEEP_INTERVAL_SECS` (default 3600s / 1h).
    // Set to `0` to disable — useful for one-shot CLI runs that don't
    // want a long-running background task. The first sweep fires at
    // `interval` (not on boot) so we don't pile retention I/O onto
    // startup; on a fresh deploy there's nothing to delete anyway.
    let sweep_interval_secs = env::var("RETENTION_SWEEP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600);
    if sweep_interval_secs > 0 {
        let jwt_logic_for_sweep = jwtrevocationlogic.clone();
        let refresh_logic_for_sweep = refreshtokenlogic.clone();
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(sweep_interval_secs));
            // Skip the immediate `.tick()` at startup — `interval` fires
            // immediately on the first poll otherwise, which pile-drives
            // retention I/O into the cold-cache boot window.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match jwt_logic_for_sweep.sweep_expired().await {
                    Ok(n) if n > 0 => {
                        tracing::info!(rows = n, "Retention sweep: dropped expired JWT revocations")
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = ?e, "JWT-revocation sweep failed; will retry");
                    }
                }
                match refresh_logic_for_sweep.sweep_expired().await {
                    Ok(n) if n > 0 => {
                        tracing::info!(rows = n, "Retention sweep: dropped expired refresh tokens")
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = ?e, "Refresh-token sweep failed; will retry");
                    }
                }
            }
        });
        tracing::info!(
            interval_secs = sweep_interval_secs,
            "Retention sweep task spawned"
        );
    } else {
        tracing::info!("RETENTION_SWEEP_INTERVAL_SECS=0 — retention sweep disabled");
    }

    // Single shared per-IP rate limiter — pre-fix this was constructed per-
    // request inside the middleware, which meant every request saw an empty
    // bucket. Gates the public auth routes (signup/login) and unauthenticated
    // search endpoints.
    let rate_limiter = Arc::new(RateLimiter::new(100, 60));

    // Per-user rate limiter for authenticated routes. Same 100/60s budget as
    // the per-IP bucket, but keyed on the JWT's `sub` claim — so an
    // authenticated scraper that rotates IPs still hits a single bucket.
    // Sized intentionally generous; trims if real usage stays well under it.
    let user_rate_limiter = Arc::new(RateLimiter::new(100, 60));

    // CORS: lock to known origins when CORS_ORIGINS is set (comma-separated list
    // of full origins, e.g. "https://festurah.com,https://www.festurah.com").
    // Falls back to allow-any for dev, which logs a warning so it's obvious
    // when a deploy slipped through unconfigured.
    let cors = match env::var("CORS_ORIGINS") {
        Ok(value) if !value.trim().is_empty() => {
            let origins: Vec<HeaderValue> = value
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                    HeaderName::from_static("x-api-key"),
                ])
        }
        _ => {
            tracing::warn!(
                "CORS_ORIGINS not set — falling back to allow-any. \
                 Set CORS_ORIGINS=https://yourdomain.com before production."
            );
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    };

    // 1. Public routes (no authentication). /health is registered separately
    // below with its own state (DB pool) since it needs to ping the DB.
    let public_routes = Router::new()
        .route("/", get(|| async { "Festival Events API" }))
        .route("/event/search", get(routes::events::search))
        // Event-type catalog is public catalog data; the search page needs
        // it to populate the type filter, and the search page runs
        // unauthenticated. API-key gating still applies (middleware below).
        .route("/eventtype", get(routes::event_type::get_all))
        .route("/eventtype/{id}", get(routes::event_type::get))
        .route(
            "/auth/google/signup",
            post(routes::auth::verify_google_create),
        )
        .route(
            "/auth/google/login",
            post(routes::auth::verify_google_login),
        )
        .route(
            "/auth/facebook/signup",
            post(routes::auth::verify_facebook_create),
        )
        .route(
            "/auth/facebook/login",
            post(routes::auth::verify_facebook_login),
        )
        // Refresh-token rotation. Public-route group because the access
        // JWT has typically expired by the time the client calls here;
        // the route still rate-limits and API-key-gates via the layers
        // below. Auth comes from the *refresh* token in the body, not
        // a `Bearer ...` header.
        .route("/auth/refresh", post(routes::auth::refresh))
        .layer(middleware::from_fn_with_state(
            rate_limiter.clone(),
            custom_middleware::rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::from_fn(
            custom_middleware::api_key::validate_api_key,
        ));

    // 3. JWT protected routes (user endpoints)
    let jwt_routes = Router::new()
        .route(
            "/event",
            post(routes::events::create), // .get(routes::events::get_all),
        )
        .route(
            "/event/{id}",
            get(routes::events::get).put(routes::events::update),
        )
        .route(
            "/event/{id}/microevent",
            get(routes::microevents::get_by_event), //.post(routes::microevents::create),
        )
        //.route("/event/{id}/microevent/{id}", get(routes::events::get))
        .route("/microevent", post(routes::microevents::create))
        .route(
            "/microevent/{id}",
            get(routes::microevents::get)
                .put(routes::microevents::update)
                .delete(routes::microevents::delete),
        )
        // Toggle endpoints: state-mutating, so POST per HTTP semantics.
        // Were previously GET — that risked browser prefetch / CDN caching
        // silently flipping a user's saves/favorites and put mutating
        // actions in access logs as plain GET URLs. The current frontend
        // syncs via /usercollection/sync, not these endpoints, so the verb
        // change is backend-isolated; any future direct caller MUST POST.
        .route(
            "/microevent/{id}/save",
            post(routes::usercollection::microevent_save_toggle),
        )
        .route(
            "/microevent/{id}/favorite",
            post(routes::usercollection::microevent_favorite_toggle),
        )
        .route(
            "/event/{id}/save",
            post(routes::usercollection::event_save_toggle),
        )
        .route(
            "/event/{id}/favorite",
            post(routes::usercollection::event_favorite_toggle),
        )
        .route("/usercollection", get(routes::usercollection::get))
        .route("/usercollection/sync", post(routes::usercollection::sync))
        .route(
            "/user/created/events",
            get(routes::usercollection::get_created_events),
        )
        .route(
            "/user/created/microevents",
            get(routes::usercollection::get_created_microevents),
        )
        .route(
            "/user/favorites/events",
            get(routes::usercollection::get_favorite_events),
        )
        .route(
            "/user/favorites/microevents",
            get(routes::usercollection::get_favorite_microevents),
        )
        .route(
            "/user/saved/events",
            get(routes::usercollection::get_saved_events),
        )
        .route(
            "/user/saved/microevents",
            get(routes::usercollection::get_saved_microevents),
        )
        // /eventtype{,/:id} now live in public_routes — JWT users still see
        // them via the merged router. Kept off jwt_routes to avoid duplicate
        // registration.
        .route("/campingprofile", get(routes::camping_profiles::get_all))
        .route(
            "/self",
            get(routes::user::get_self)
                .post(routes::user::update_self)
                // GDPR/CCPA right-to-erasure: soft-deletes the current user.
                // After the response, the JWT this caller is bearing will
                // 401 on every subsequent request (auth_middleware's
                // `WHERE deleted_at IS NULL` filters the user out).
                .delete(routes::user::delete_self),
        )
        // GDPR/CCPA right-to-portability: returns the user's full data
        // bundle (profile, collection, events, microevents) as a single
        // JSON response the frontend downloads as a file.
        .route("/self/data-export", get(routes::user::data_export_self))
        .route("/campingprofile/{id}", get(routes::camping_profiles::get))
        // Logout — must be inside jwt_routes so the auth middleware
        // injects the verified Claims (and so a revoked token is rejected
        // before reaching here). Frontend calls this on user logout to
        // close the stolen-token blast-radius window.
        .route("/auth/logout", post(routes::auth::logout))
        //.layer(middleware::from_fn(custom_middleware::jwt::validate_jwt));
        // Layer order matters: in axum, the LAST `.route_layer` call is the
        // outermost (runs first on the way in). We want
        //   auth_middleware → user_rate_limit → handler
        // so auth runs first (populating Claims), then rate limit can read
        // Claims, then the handler. So rate-limit is added first (inner) and
        // auth is added last (outer).
        .route_layer(middleware::from_fn_with_state(
            user_rate_limiter.clone(),
            custom_middleware::rate_limit::user_rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            custom_middleware::auth_middleware::auth_middleware,
        ));

    // Admin routes split by privilege tier. Pre-refactor everything was
    // gated by `require_super_admin`, which made the 3-tier role enum
    // (User | Admin | SuperAdmin) functionally a 2-tier one. Now:
    //
    //   admin_view_routes: Admin or SuperAdmin
    //     - listing users / events / microevents
    //     - viewing a single user
    //     - updating a user profile (non-role fields)
    //     - lock/unlock users
    //     - reading the audit log
    //
    //   admin_super_routes: SuperAdmin only
    //     - role changes (Admins can't promote themselves or others)
    //     - user deletes (irreversible)
    //     - event_type / camping_profile catalog mutations (global state)
    //     - event deletes (irreversible)
    //
    // The audit log records who did what; pairing the Admin tier with the
    // existing audit log gives oversight over the new privilege without
    // additional infrastructure.
    let admin_view_routes = Router::new()
        .route("/microevent", get(routes::microevents::get_all))
        .route("/user", get(routes::user::get_all))
        .route(
            "/user/{id}",
            post(routes::user::update).get(routes::user::get),
        )
        .route("/admin/users/{id}/lock", post(routes::user::lock))
        .route("/admin/users/{id}/unlock", post(routes::user::unlock))
        .route("/admin/audit-log", get(routes::user::list_audit_log))
        .route("/event", get(routes::events::get_all))
        // Layer order (innermost → outermost / last-to-execute → first):
        //   require_admin → user_rate_limit → auth_middleware → handler
        // becomes execution order:
        //   auth → user_rate_limit → require_admin → handler
        .route_layer(middleware::from_fn(
            custom_middleware::auth_middleware::require_admin,
        ))
        .route_layer(middleware::from_fn_with_state(
            user_rate_limiter.clone(),
            custom_middleware::rate_limit::user_rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            custom_middleware::auth_middleware::auth_middleware,
        ));

    let admin_super_routes = Router::new()
        .route("/admin/users/{id}", delete(routes::user::delete))
        .route("/admin/users/{id}/role", put(routes::user::update_role))
        .route("/eventtype", post(routes::event_type::create))
        .route(
            "/eventtype/{id}",
            put(routes::event_type::update).delete(routes::event_type::delete),
        )
        .route("/campingprofile", post(routes::camping_profiles::create))
        .route(
            "/campingprofile/{id}",
            put(routes::camping_profiles::update).delete(routes::camping_profiles::delete),
        )
        .route("/event/{id}", delete(routes::events::delete))
        // Same layer ordering as admin_view_routes — auth → rate_limit → role-check.
        .route_layer(middleware::from_fn(
            custom_middleware::auth_middleware::require_super_admin,
        ))
        .route_layer(middleware::from_fn_with_state(
            user_rate_limiter.clone(),
            custom_middleware::rate_limit::user_rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            custom_middleware::auth_middleware::auth_middleware,
        ));

    // /health needs the DB pool to issue a SELECT 1, but the rest of the
    // public routes only need AppState. Build /health as a separate router
    // with its own state, then merge.
    let health_router = Router::new()
        .route("/health", get(health_check))
        .with_state(db.clone());

    let app = Router::new()
        .merge(public_routes)
        .merge(jwt_routes)
        .merge(admin_view_routes)
        .merge(admin_super_routes)
        .layer(cors)
        // 2 MB request body cap — fully-populated NomEvent payloads with all
        // camping/amenity options can be a few KB; 2 MB is roomy without
        // letting anyone DoS us with megabyte JSON.
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state)
        .merge(health_router);

    // Port defaults to 8080 if unset so a misconfigured deploy still boots
    // and the operator sees the bind address in the log rather than a panic.
    let listening_port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let listener_address = format!("0.0.0.0:{}", listening_port);
    let listener = tokio::net::TcpListener::bind(&listener_address)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind {}: {}", listener_address, e));

    tracing::info!("🚀 Server running on http://{}", listener_address);

    // `into_make_service_with_connect_info` lets handlers / middleware extract
    // the peer's SocketAddr — used by the rate limiter to key on remote IP.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();
}

/// Resolves when the process receives Ctrl-C (or SIGTERM on Unix). Returning
/// from this future tells axum to stop accepting new connections and let
/// in-flight requests finish.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("🛑 Shutdown signal received; draining in-flight requests");
}

/// Liveness + readiness probe. Pings the database so orchestrators detect
/// a broken pool (was previously a static 200 regardless of DB state).
async fn health_check(
    axum::extract::State(pool): axum::extract::State<sqlx::SqlitePool>,
) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "healthy" })),
        ),
        Err(e) => {
            tracing::error!("Health check DB ping failed: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "unhealthy",
                    "reason": "database unreachable"
                })),
            )
        }
    }
}
