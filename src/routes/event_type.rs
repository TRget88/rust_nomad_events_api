// ============================================================================
// API Handlers: src/handlers/camping_handlers.rs
// ============================================================================
use crate::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use std::sync::Arc;

use crate::errors::AppError;
use crate::logic::EventTypeLogic;
use crate::models::event_models::EventType;

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

// POST /camping-profiles - Create new camping template
pub async fn create(
    State(service): State<Arc<AppState>>,
    Json(output): Json<EventType>,
) -> Result<impl IntoResponse, AppError> {
    let id = service.event_type_logic.create(output).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Camping output created successfully",
            "id": id
        })),
    ))
}

// PUT /camping-profiles/{id} - Update camping template
pub async fn update(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    Json(output): Json<EventType>,
) -> Result<impl IntoResponse, AppError> {
    service.event_type_logic.update(id, output).await?;

    Ok(Json(json!({
        "message": "Camping output updated successfully"
    })))
}

// DELETE /camping-profiles/{id} - Delete camping template
pub async fn delete(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_type_logic.delete(id).await?;

    Ok(Json(json!({
        "message": "Camping output deleted successfully"
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
