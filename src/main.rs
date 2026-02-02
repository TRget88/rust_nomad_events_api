// src/main.rs - Application Entry Point
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::Method;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{
    Json,
    Router,
    middleware, // Import axum's middleware module
    routing::{delete, get, post, put},
};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

mod context;
mod custom_middleware; // Rename to avoid conflict with axum::middleware
mod errors;
mod logic;
mod models;
mod routes; // This imports and registers the trait impl

use context::CampingProfileContext;
use context::EventContext;
use context::EventTypeContext;
use context::MicroeventContext;
use context::UserCollectionContext;
use context::UserContext;
use custom_middleware::*;
use logic::CampingProfileLogic;
use logic::EventLogic;
use logic::EventTypeLogic;
use logic::MicroeventLogic;
use logic::UserCollectionLogic;
use logic::UserLogic;
use routes::events::*;
use routes::microevents::*;
use routes::*;

// Test data modules
mod camping_profiles;
mod event_types;
mod seed_data;

use camping_profiles::create_standard_camping_profiles;
use event_types::create_standard_event_types;
use seed_data::seed_all;

use crate::custom_middleware::auth_middleware;

#[derive(Clone)]
pub struct AppState {
    pub event_logic: Arc<EventLogic>,
    pub microevent_logic: Arc<MicroeventLogic>,
    pub event_type_logic: Arc<EventTypeLogic>,
    pub camping_profile_logic: Arc<CampingProfileLogic>,
    pub oauth_states: Arc<Mutex<HashMap<String, std::time::Instant>>>,
    pub user_logic: Arc<UserLogic>,
    pub user_collection_logic: Arc<UserCollectionLogic>,
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
        println!("📁 Created events.db file");
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

    // Seed database with test data (uncomment to run once)
    seed_all(&db).await.expect("Failed to seed database");

    // Build layers: Repository -> Service
    let eventcontext = EventContext::new(db.clone());
    //let eventlogic = Arc::new(EventLogic::new(eventcontext, usercollectionlogic.clone()));
    let microeventcontext = MicroeventContext::new(db.clone());
    //let microeventlogic = Arc::new(MicroeventLogic::new(
    //microeventcontext,
    //usercollectionlogic.clone(),
    //));
    let eventtypecontext = EventTypeContext::new(db.clone());
    let eventtypelogic = Arc::new(EventTypeLogic::new(eventtypecontext));
    let campingprofilecontext = CampingProfileContext::new(db.clone());
    let campingprofilelogic = Arc::new(CampingProfileLogic::new(campingprofilecontext));
    let oauthstates = Arc::new(Mutex::new(HashMap::new()));
    let usercontext = UserContext::new(db.clone());
    let userlogic = Arc::new(UserLogic::new(usercontext));
    let usercollectioncontext = UserCollectionContext::new(db.clone());
    let usercollectionlogic = Arc::new(UserCollectionLogic::new(
        usercollectioncontext,
        eventcontext,
        microeventcontext,
    ));
    // 3. Now create EventLogic and MicroeventLogic with usercollectionlogic
    let eventcontext2 = EventContext::new(db.clone());
    let eventlogic = Arc::new(EventLogic::new(eventcontext2, usercollectionlogic.clone()));
    let microeventcontext2 = MicroeventContext::new(db.clone());
    let microeventlogic = Arc::new(MicroeventLogic::new(
        microeventcontext2,
        usercollectionlogic.clone(),
    ));

    let app_state = Arc::new(AppState {
        event_logic: eventlogic,
        microevent_logic: microeventlogic,
        camping_profile_logic: campingprofilelogic,
        event_type_logic: eventtypelogic,
        oauth_states: oauthstates,
        user_logic: userlogic,
        user_collection_logic: usercollectionlogic,
    });

    // Configure CORS - very permissive for development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(false); // Set to true if you need cookies

    // 1. Public routes (no authentication)
    let public_routes = Router::new()
        .route("/", get(|| async { "Festival Events API" }))
        .route("/health", get(health_check))
        .route("/event/search", get(routes::events::search))
        .route(
            "/auth/google/signup",
            post(routes::auth::verify_google_create),
        )
        .route(
            "/auth/google/login",
            post(routes::auth::verify_google_login),
        )
        //.route("/auth/facebook/callback", get(routes::auth::exchange_facebook_code))
        .layer(middleware::from_fn(
            custom_middleware::rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::from_fn(
            custom_middleware::api_key::validate_api_key,
        ));

    // 2. API Key protected routes (auth endpoints)
    //let api_key_routes = Router::new()
    //.route("/auth/google", post(routes::auth::google_auth))
    //.route("/auth/facebook", post(routes::auth::facebook_auth))
    //.layer(middleware::from_fn(
    //custom_middleware::rate_limit::rate_limit_middleware,
    //));
    //.layer(middleware::from_fn(custom_middleware::api_key::validate_api_key));

    // 3. JWT protected routes (user endpoints)
    let jwt_routes = Router::new()
        .route("/profile", get(routes::profile::get_profile))
        .route("/profile", post(routes::profile::update_profile))
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
        .route(
            "/microevent/{id}/save",
            get(routes::usercollection::microevent_save_toggle),
        )
        .route(
            "/microevent/{id}/favorite",
            get(routes::usercollection::microevent_favorite_toggle),
        )
        .route(
            "/event/{id}/save",
            get(routes::usercollection::event_save_toggle),
        )
        .route(
            "/event/{id}/favorite",
            get(routes::usercollection::event_favorite_toggle),
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
        .route("/eventtype", get(routes::event_type::get_all))
        .route("/eventtype/{id}", get(routes::event_type::get))
        .route("/campingprofile", get(routes::camping_profiles::get_all))
        .route(
            "/self",
            get(routes::user::get_self).post(routes::user::update_self),
        )
        .route("/campingprofile/{id}", get(routes::camping_profiles::get))
        //.layer(middleware::from_fn(custom_middleware::jwt::validate_jwt));
        //check authentication (new way)
        .route_layer(middleware::from_fn_with_state(
            db.clone(),
            custom_middleware::auth_middleware::auth_middleware,
        ));

    let admin_routes = Router::new()
        .route("/microevent", get(routes::microevents::get_all))
        .route("/user", get(routes::user::get_all))
        .route(
            "/user/{id}",
            post(routes::user::update).get(routes::user::get),
        )
        .route("/event", get(routes::events::get_all))
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
        //check authorization
        .route_layer(middleware::from_fn(
            custom_middleware::auth_middleware::require_super_admin,
        ))
        //check authentication
        .route_layer(middleware::from_fn_with_state(
            db.clone(),
            custom_middleware::auth_middleware::auth_middleware,
        ));

    // Combine all routes
    let app = Router::new()
        //.merge(auth_routes)
        .merge(public_routes)
        //.merge(api_key_routes)
        .merge(jwt_routes)
        .merge(admin_routes)
        //.layer(custom_middleware::cors::configure_cors())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);
    //.with_state(eventlogic);

    let listeningPort = env::var("PORT").expect("PORT must be set");
    let listenerAddress = format!("0.0.0.0:{}", &listeningPort.to_string());
    println!("{}", listenerAddress.to_string());
    let listener = tokio::net::TcpListener::bind(listenerAddress)
        .await
        .unwrap();

    println!("🚀 Server running on http://localhost:{}", listeningPort);

    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy"
        })),
    )
}
