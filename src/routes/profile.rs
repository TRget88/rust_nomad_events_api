// src/routes/profile.rs
use crate::models::user::Claims;
use axum::{Extension, Json, http::StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ProfileResponse {
    pub email: String,
    pub name: String,
}

pub async fn get_profile(
    Extension(claims): Extension<Claims>,
) -> Result<Json<ProfileResponse>, StatusCode> {
    Ok(Json(ProfileResponse {
        email: claims.email,
        name: "User Name".to_string(),
    }))
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub name: String,
}

pub async fn update_profile(
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Json<ProfileResponse>, StatusCode> {
    Ok(Json(ProfileResponse {
        email: claims.email,
        name: payload.name,
    }))
}
