// ============================================================================
// API Handlers: src/routes/camping_handlers.rs
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
use crate::models::event_models::CampingProfile;
use crate::models::user::Claims;

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

// POST /campingprofile - Create new camping template. Audited.
pub async fn create(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    ApiJson(profile): ApiJson<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    let profile_name = profile.profile_name.clone();
    let id = service
        .camping_profile_logic
        .create_profile(profile)
        .await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::CAMPING_PROFILE_CREATE.to_string(),
            target_type: target_types::CAMPING_PROFILE.to_string(),
            target_id: id.to_string(),
            metadata: json!({ "profile_name": profile_name }),
        })
        .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Camping profile created successfully",
            "id": id
        })),
    ))
}

// PUT /campingprofile/{id} - Update camping template. Audited.
pub async fn update(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    ApiJson(profile): ApiJson<CampingProfile>,
) -> Result<impl IntoResponse, AppError> {
    let profile_name = profile.profile_name.clone();
    service
        .camping_profile_logic
        .update_profile(id, profile)
        .await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::CAMPING_PROFILE_UPDATE.to_string(),
            target_type: target_types::CAMPING_PROFILE.to_string(),
            target_id: id.to_string(),
            // `new_values` only — capturing the pre-state is a queued
            // follow-up that applies to every audited update.
            metadata: json!({ "new_values": { "profile_name": profile_name } }),
        })
        .await;

    Ok(Json(json!({
        "message": "Camping profile updated successfully"
    })))
}

// DELETE /campingprofile/{id} - Delete camping template. Audited.
pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.camping_profile_logic.delete_profile(id).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::CAMPING_PROFILE_DELETE.to_string(),
            target_type: target_types::CAMPING_PROFILE.to_string(),
            target_id: id.to_string(),
            metadata: json!({}),
        })
        .await;

    Ok(Json(json!({
        "message": "Camping profile deleted successfully"
    })))
}

// GET /camping-profiles/{id}/apply - Get camping info from template (for auto-fill).
// Not yet wired into the router — the event-wizard frontend fetches the
// full profile via `GET /camping-profiles/{id}` and derives the
// `CampingInfo` client-side. Kept for the future server-side projection
// (e.g. when the templates grow more shape divergence from `CampingInfo`).
#[allow(dead_code)]
pub async fn apply_camping_profile(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let profile = service.camping_profile_logic.get_profile_by_id(id).await?;
    let camping_info = profile.to_camping_info();

    Ok(Json(camping_info))
}
