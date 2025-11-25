// Project Structure:
// src/
// ├── main.rs                 // Application entry point & routing
// ├── handlers/               // HTTP handlers (controllers)
// │   ├── mod.rs
// │   └── event_handlers.rs
// ├── services/               // Business logic layer
// │   ├── mod.rs
// │   └── event_service.rs
// ├── repositories/           // Data access layer
// │   ├── mod.rs
// │   └── event_repository.rs
// ├── models/                 // Domain models
// │   ├── mod.rs
// │   ├── event_models.rs
// │   └── dto.rs             // Data Transfer Objects
// └── errors/                 // Error handling
//     ├── mod.rs
//     └── app_error.rs


// ============================================================================
// src/main.rs - Application Entry Point
// ============================================================================
use std::sync::Arc; 

use axum::{
    routing::{get, post, put, delete},
    Router,
    Json,
};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

mod models;
mod errors;
mod repositories;
mod services;
mod handlers;

use repositories::EventRepository;
use services::EventService;
use handlers::event_handlers::*;

mod test_data;
use test_data::seed_database;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Database setup
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite://events.db")
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("Failed to run migrations");

    // Seed database with test data (uncomment to run once)
    seed_database(&db).await.expect("Failed to seed database");

    // Build layers: Repository -> Service
    let repository = EventRepository::new(db);
    let service = Arc::new(EventService::new(repository));

    // Build router
    let app = Router::new()
        .route("/", get(|| async { "Festival Events API" }))
        .route("/health", get(health_check))
        .route("/events", get(get_all_events).post(create_event))
        .route("/events/search", get(search_events))
        .route("/events/{id}", get(get_event).put(update_event).delete(delete_event))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(service);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    
    println!("🚀 Server running on http://localhost:3000");
    
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({
        "status": "healthy"
    })))
}