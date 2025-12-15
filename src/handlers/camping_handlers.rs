// ============================================================================
// API Handlers: src/handlers/camping_handlers.rs
// ============================================================================
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use serde_json::json;

use crate::services::CampingService;
use crate::models::event_models::CampingProfile;
use crate::errors::AppError;

// GET /camping-profiles - List all camping templates
pub async fn get_camping_profiles(
    State(service): State<Arc<CampingService>>,
) -> Result<impl IntoResponse, AppError> {
    let profiles = service.get_all_profiles().await?;
    Ok(Json(profiles))
}

// GET /camping-profiles/{id} - Get specific camping template
pub async fn get_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<CampingService>>,
) -> Result<impl IntoResponse, AppError> {
    let profile = service.get_profile_by_id(id).await?;
    Ok(Json(profile))
}

// POST /camping-profiles - Create new camping template
pub async fn create_camping_profile(
    State(service): State<Arc<CampingService>>,
    Json(profile): Json<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    let id = service.create_profile(profile).await?;
    
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Camping profile created successfully",
            "id": id
        }))
    ))
}

// PUT /camping-profiles/{id} - Update camping template
pub async fn update_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<CampingService>>,
    Json(profile): Json<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    service.update_profile(id, profile).await?;
    
    Ok(Json(json!({
        "message": "Camping profile updated successfully"
    })))
}

// DELETE /camping-profiles/{id} - Delete camping template
pub async fn delete_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<CampingService>>,
) -> Result<impl IntoResponse, AppError> {
    service.delete_profile(id).await?;
    
    Ok(Json(json!({
        "message": "Camping profile deleted successfully"
    })))
}

// GET /camping-profiles/{id}/apply - Get camping info from template (for auto-fill)
pub async fn apply_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<CampingService>>,
) -> Result<impl IntoResponse, AppError> {
    let profile = service.get_profile_by_id(id).await?;
    let camping_info = profile.to_camping_info();
    
    Ok(Json(camping_info))
}