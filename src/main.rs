// src/main.rs - Application Entry Point
use std::sync::Arc;
use std::fs::File;
use std::net::SocketAddr;
use std::env;

use axum::http::StatusCode;
use axum::http::Method;
use axum::response::IntoResponse;
use axum::{
    middleware,  // Import axum's middleware module
    Json, Router,
    routing::{delete, get, post, put},
};
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;

mod errors;
mod handlers;
mod models;
mod repositories;
mod services;
mod custom_middleware;  // Rename to avoid conflict with axum::middleware
mod routes;

use repositories::EventRepository;
use handlers::event_handlers::*;
use repositories::CampingRepository;
use repositories::EventTypeRepository;
use services::EventService;

// Test data modules
mod event_types;
mod camping_profiles;
mod seed_data;

use event_types::create_standard_event_types;
use camping_profiles::create_standard_camping_profiles;
use seed_data::seed_all;

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
    let repository = EventRepository::new(db.clone());
    let service = Arc::new(EventService::new(repository));

      // Configure CORS - very permissive for development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(false);  // Set to true if you need cookies


    // 1. Public routes (no authentication)
    let public_routes = Router::new()
        .route("/", get(|| async { "Festival Events API" }))
        .route("/health", get(health_check))
        .route("/events/search", get(search_events))//;
        .layer(middleware::from_fn(custom_middleware::api_key::validate_api_key));
    
    // 2. API Key protected routes (auth endpoints)
    let api_key_routes = Router::new()
        .route("/auth/google", post(routes::auth::google_auth))
        .route("/auth/facebook", post(routes::auth::facebook_auth))
        .layer(middleware::from_fn(custom_middleware::rate_limit::rate_limit_middleware));
        //.layer(middleware::from_fn(custom_middleware::api_key::validate_api_key));
    
    // 3. JWT protected routes (user endpoints)
    let jwt_routes = Router::new()
        .route("/profile", get(routes::profile::get_profile))
        .route("/profile", post(routes::profile::update_profile))
        .route("/events", post(create_event))
        .route("/events/{id}", get(get_event).put(update_event).delete(delete_event))        
        .layer(middleware::from_fn(custom_middleware::jwt::validate_jwt));
    
    // Combine all routes
    let app = Router::new()
        .merge(public_routes)
        .merge(api_key_routes)
        .merge(jwt_routes)
        //.layer(custom_middleware::cors::configure_cors())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(service);


    let listeningPort = env::var("PORT").expect("PORT must be set");
    let listenerAddress = format!("0.0.0.0:{}",&listeningPort.to_string());
    println!("{}",listenerAddress.to_string());
    let listener = tokio::net::TcpListener::bind(listenerAddress)
        .await
        .unwrap();


    //// Build router
    //let app = Router::new()
        //// Public routes
        //.route("/", get(|| async { "Festival Events API" }))
        //.route("/health", get(health_check))
        //.route("/events/search", get(search_events))
//        
        //// Auth routes (require API key)
        //.route("/auth/google", post(routes::auth::google_auth))
        //.route("/auth/facebook", post(routes::auth::facebook_auth))
        //.layer(middleware::from_fn(custom_middleware::api_key::validate_api_key))
        //.layer(middleware::from_fn(custom_middleware::rate_limit::rate_limit_middleware))
//        
        //// User authenticated routes (require JWT token)
        //.route("/profile", get(routes::profile::get_profile))
        //.route("/profile", post(routes::profile::update_profile))
        //.route("/events/{id}", get(get_event).put(update_event).delete(delete_event))
        ////.route("/events/:id", get(get_event).put(update_event).delete(delete_event))
        //.layer(middleware::from_fn(custom_middleware::jwt::validate_jwt))
//        
        //// Add CORS and tracing
        ////.layer(custom_middleware::cors::configure_cors())
        //.layer(TraceLayer::new_for_http())
        //.with_state(service);  // Add state if your handlers need it
//
    //let listeningPort = env::var("PORT").expect("PORT must be set");
    //let listenerAddress = format!("0.0.0.0:{}",&listeningPort.to_string());
    //println!("{}",listenerAddress.to_string());
    //let listener = tokio::net::TcpListener::bind(listenerAddress)
        //.await
        //.unwrap();

    println!("🚀 Server running on http://localhost:{}",listeningPort);

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