// src/routes/auth.rs
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct GoogleAuthRequest {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GoogleTokenClaims {
    email: String,
    name: String,
    picture: String,
    sub: String, // Google user ID
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub email: String,
    pub name: String,
}

pub async fn google_auth(
    Json(payload): Json<GoogleAuthRequest>,
) -> Result<Json<AuthResponse>, StatusCode> {
    // TODO: Implement Google token verification
    Ok(Json(AuthResponse {
        token: "jwt_token_here".to_string(),
        user: UserInfo {
            email: "user@example.com".to_string(),
            name: "User Name".to_string(),
        },
    }))
}

pub async fn facebook_auth(
    Json(payload): Json<GoogleAuthRequest>,
) -> Result<Json<AuthResponse>, StatusCode> {
    // TODO: Implement Facebook token verification
    Ok(Json(AuthResponse {
        token: "jwt_token_here".to_string(),
        user: UserInfo {
            email: "user@example.com".to_string(),
            name: "User Name".to_string(),
        },
    }))
}