///This is a simple api that can be used to pull event information
///
use axum::{
    body::Body,
    Json, Router,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    middleware::{self, Next},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
//use tower_http::cors::CorsLayer;
use chrono::NaiveDate;
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer,
};


mod models;
use models::event_models::*;

///router
async fn set_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route(
            "/events", 
            get(get_all_events)
        //.post(create_event)
        )
        .route(
            "/events/{id}",
            get(get_event)//.put(update_event).delete(delete_event),
        )
        .route("/health", get(health_check))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(middleware::from_fn(logging_middleware))
        .with_state(state)
}

// Application state to hold our events
#[derive(Clone)]
struct AppState {
    events: Arc<Mutex<Vec<NomEvent>>>,
}

#[tokio::main]
async fn main() {
    // Initialize state
    let state = AppState {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    // Build our application with routes - Moved
    //let app = Router::new()
        //.route("/", get(root))
        //.route("/events", get(get_all_events).post(create_event))
        //.route(
            //"/events/{id}",
            //get(get_event).put(update_event).delete(delete_event),
        //)
        //.route("/health", get(health_check))
        //.layer(CorsLayer::permissive())
        //.with_state(state);

    let app = set_router(state).await;

    // Run the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("🚀 Server running on http://localhost:3000");

    axum::serve(listener, app).await.unwrap();
}

// Root endpoint
async fn root() -> &'static str {
    "Festival Events API - Use /events for event operations"
}

// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy",
            "service": "festival-events-api"
        })),
    )
}

/// GET /events - Get all events
async fn get_all_events(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.events.lock().unwrap();
    let events_copy: Vec<NomEvent> = events.iter().cloned().collect();
    (StatusCode::OK, Json(events_copy))
}

/// GET /events/:id - Get a specific event
async fn get_event(Path(id): Path<usize>, State(state): State<AppState>) -> impl IntoResponse {
    let events = state.events.lock().unwrap();

    match events.get(id) {
        Some(event) => (StatusCode::OK, Json(event)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Event not found"
            })),
        )
            .into_response(),
    }
}

/// POST /events - Create a new event
//async fn create_event(
    //State(state): State<AppState>,
    //Json(payload): Json<NomEvent>,
//) -> impl IntoResponse {
    //let mut events = state.events.lock().unwrap();
    //events.push(payload);
//
    //(
        //StatusCode::CREATED,
        //Json(serde_json::json!({
            //"message": "Event created successfully",
            //"id": events.len() - 1
        //})),
    //)
//}

/// PUT /events/:id - Update an event
//async fn update_event(
    //Path(id): Path<usize>,
    //State(state): State<AppState>,
    //Json(payload): Json<NomEvent>,
//) -> impl IntoResponse {
    //let mut events = state.events.lock().unwrap();
//
    //if id < events.len() {
        //events[id] = payload;
        //(
            //StatusCode::OK,
            //Json(serde_json::json!({
                //"message": "Event updated successfully"
            //})),
        //)
    //} else {
        //(
            //StatusCode::NOT_FOUND,
            //Json(serde_json::json!({
                //"error": "Event not found"
            //})),
        //)
    //}
//}

/// DELETE /events/:id - Delete an event
//async fn delete_event(Path(id): Path<usize>, State(state): State<AppState>) -> impl IntoResponse {
    //let mut events = state.events.lock().unwrap();
//
    //if id < events.len() {
        //events.remove(id);
        //(
            //StatusCode::OK,
            //Json(serde_json::json!({
                //"message": "Event deleted successfully"
            //})),
        //)
    //} else {
        //(
            //StatusCode::NOT_FOUND,
            //Json(serde_json::json!({
                //"error": "Event not found"
            //})),
        //)
    //}
//}

// src/models/mod.rs
//pub mod event_models;
//pub mod event_modules;


///Middleware
/// Logging
async fn logging_middleware(
    req: Request<Body>,
    next: Next,
) -> Response {
    println!("Request: {} {}", req.method(), req.uri());
    let response = next.run(req).await;
    println!("Response: {}", response.status());
    response
}
/// Rout specific - Not needed yet. Going to just use Gets now and will post in a different way.
fn stop_throwing_error(){}