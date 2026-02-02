use std::sync::Arc;

use crate::AppState;
use crate::custom_middleware::auth_middleware::auth_middleware;
use crate::errors::AppError;
use crate::models::database_models::UserRow;
use crate::models::user::Claims;
use crate::models::user::*;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use uuid::Uuid;
//pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
//let events = service.user_logic.get_all().await?;
//Ok(Json(events))
//}
//
//pub async fn get(
//Path(id): Path<String>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//let corrected_id =
//Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid UUID".into()))?;
//
//let event = service.user_logic.get(corrected_id).await?;
//Ok(Json(event))
//}
//
//pub async fn update(
//Path(id): Path<String>,
//State(service): State<Arc<AppState>>,
//Json(event): Json<UserRow>,
//) -> Result<impl IntoResponse, AppError> {
//let corrected_id =
//Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid UUID".into()))?;
//
//service.user_logic.update(corrected_id, event).await?;
//
//Ok(Json(json!({
//"message": "Event updated successfully"
//})))
//}
//
//pub async fn delete(
//Path(id): Path<String>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
////service.user_logic.delete(id).await?;
//
//Ok(Json(json!({
//"message": "Event deleted successfully"
//})))
//}
//
///Self updates
pub async fn get_self(
    State(service): State<Arc<AppState>>,
    req: Request, // Request must be last
) -> Result<impl IntoResponse, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::BadRequest("Invalid user ID".to_string()))?;

    let user = service.user_logic.get(user_id).await?;
    Ok(Json(user))
}
pub async fn update_self(
    Extension(claims): Extension<Claims>, // Extract from extensions
    State(service): State<Arc<AppState>>,
    Json(update): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    service
        .user_logic
        .update_profile(
            &claims.sub,
            update.user_name.as_deref(),
            update.email.as_deref(),
            update.timezone.as_deref(),
            update.language.as_deref(),
        )
        .await?;

    Ok(Json(json!({
        "message": "Profile updated successfully"
    })))
}

//pub async fn get_self(
//claims: Claims,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//let user_id = Uuid::parse_str(&claims.sub)
//.map_err(|_| AppError::BadRequest("Invalid user ID in token".into()))?;
//let events = service.user_logic.get_self(user_id).await?;
//Ok(Json(events))
//}

//pub async fn update_self(
//claims: Claims,
//State(service): State<Arc<AppState>>,
//Json(event): Json<UserRow>,
//) -> Result<impl IntoResponse, AppError> {
////Pull uuid from jwt
//let user_id = Uuid::parse_str(&claims.sub)
//.map_err(|_| AppError::BadRequest("Invalid user ID in token".into()))?;
//
//service.user_logic.update(user_id, event).await?;
//
//Ok(Json(json!({
//"message": "Event updated successfully"
//})))
//}
// ============================================================================
// Admin Routes - Manage all users
// ============================================================================

/// GET /api/admin/users - Get all users (Admin only)
pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    println!("reaching get_all user route");
    let users = service.user_logic.get_all().await?;
    Ok(Json(users))
}

/// GET /api/admin/users/:id - Get user by ID (Admin only)
pub async fn get(
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid UUID".into()))?;

    let user = service.user_logic.get(user_id).await?;
    Ok(Json(user))
}

/// PUT /api/admin/users/:id - Update user (Admin only)
pub async fn update(
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
    Json(update): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    service
        .user_logic
        .update_profile(
            &id,
            update.user_name.as_deref(),
            update.email.as_deref(),
            update.timezone.as_deref(),
            update.language.as_deref(),
        )
        .await?;

    Ok(Json(json!({
        "message": "User updated successfully"
    })))
}

/// DELETE /api/admin/users/:id - Delete user (Admin only)
pub async fn delete(
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.user_logic.delete_user(&id).await?;

    Ok(Json(json!({
        "message": "User deleted successfully"
    })))
}

// PUT /api/users/me - Update current user's profile -- this is a shit implementation and needs to be changed heavily.
//pub async fn update_self(
//claims: Claims, // Extracted by auth middleware
//State(service): State<Arc<AppState>>,
//Json(update): Json<UpdateUserRequest>,
//) -> Result<impl IntoResponse, AppError> {
//// User ID comes from JWT claims, not from request body
//service
//.user_logic
//.update_profile(
//&claims.sub,
//update.user_name.as_deref(),
//update.email.as_deref(),
//update.timezone.as_deref(),
//update.language.as_deref(),
//)
//.await?;
//
//Ok(Json(json!({
//"message": "Profile updated successfully"
//})))
//}

// DELETE /api/users/me - Delete current user's account (soft delete)
//pub async fn delete_self(
//claims: Claims,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//service.user_logic.delete_user(&claims.sub).await?;
//
//Ok(Json(json!({
//"message": "Account deleted successfully"
//})))
//}
