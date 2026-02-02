// ============================================================================
// API Handlers: src/routes/camping_handlers.rs
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
use crate::logic::CampingProfileLogic;
use crate::models::event_models::CampingProfile;

// GET /camping-profiles - List all camping templates
pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let profiles = service.camping_profile_logic.get_all_profiles().await?;
    Ok(Json(profiles))
}

// GET /camping-profiles/{id} - Get specific camping template
pub async fn get(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let profile = service.camping_profile_logic.get_profile_by_id(id).await?;
    Ok(Json(profile))
}

// POST /camping-profiles - Create new camping template
pub async fn create(
    State(service): State<Arc<AppState>>,
    Json(profile): Json<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    let id = service
        .camping_profile_logic
        .create_profile(profile)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Camping profile created successfully",
            "id": id
        })),
    ))
}

// PUT /camping-profiles/{id} - Update camping template
pub async fn update(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    Json(profile): Json<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    service
        .camping_profile_logic
        .update_profile(id, profile)
        .await?;

    Ok(Json(json!({
        "message": "Camping profile updated successfully"
    })))
}

// DELETE /camping-profiles/{id} - Delete camping template
pub async fn delete(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.camping_profile_logic.delete_profile(id).await?;

    Ok(Json(json!({
        "message": "Camping profile deleted successfully"
    })))
}

// GET /camping-profiles/{id}/apply - Get camping info from template (for auto-fill)
pub async fn apply_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let profile = service.camping_profile_logic.get_profile_by_id(id).await?;
    let camping_info = profile.to_camping_info();

    Ok(Json(camping_info))
}
