use crate::AppState;
use crate::errors::AppError;
use crate::models::user::Claims;
use crate::models::user::{AuthResponse, GoogleLoginRequest};

use axum::{
    Json,
    extract::{Path, Request, State},
    response::IntoResponse,
};
use std::sync::Arc;
use uuid::Uuid;
pub async fn verify_google_login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GoogleLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    println!("verify_google_login reached");

    let response = state.user_logic.verify_google_login(&payload).await?;

    Ok(Json(response))
}

pub async fn verify_google_create(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GoogleLoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    println!("verify_google_create reached");

    let response = state
        .user_logic
        .verify_google_account_creation(&payload)
        .await?;

    Ok(Json(response))
}
// ============================================================================
// Self Routes - User manages their own account
// ============================================================================

//pub async fn get_self(
//State(service): State<Arc<AppState>>,
//req: axum::extract::Request,
//) -> Result<impl IntoResponse, AppError> {
//let claims = req
//.extensions()
//.get::<Claims>()
//.cloned()
//.ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;
//
//let user_id = Uuid::parse_str(&claims.sub)
//.map_err(|_| AppError::BadRequest("Invalid user ID".to_string()))?;
//
////let user_id = Uuid::parse_str(&claims.sub)?;
//let user = service.user_logic.get(user_id).await?;
//Ok(Json(user))
//}

//
//// Verify Google's ID token
//let google_claims = UserLogic::verify_google_id_token(&payload.credential)
//.await
//.map_err(|e| {
//eprintln!("Failed to verify Google token: {:?}", e);
//StatusCode::UNAUTHORIZED
//})?;
//
//// Ensure user exists and get their role
//let role = context::ensure_user_exists(&pool, &google_claims.sub)
//.await
//.map_err(|e| {
//eprintln!("Database error: {:?}", e);
//StatusCode::INTERNAL_SERVER_ERROR
//})?;
//
//// Create your own JWT
//let jwt = create_jwt_for_user(&google_claims.sub, &google_claims.email)
//.map_err(|e| {
//eprintln!("Failed to create JWT: {:?}", e);
//StatusCode::INTERNAL_SERVER_ERROR
//})?;
//
//
//Ok(Json(AuthResponse {
//token: jwt,
//user: UserInfo {
//id: google_claims.sub,
//email: google_claims.email,
//name: google_claims.name,
//picture: google_claims.picture,
////role: role
//}
//}))

//}

//pub async fn exchange_facebook_code(
//Json(payload): Json<CodeExchangeRequest>,
//) -> Result<Json<TokenResponse>, StatusCode> {
//let jwt = exchange_google_code_for_jwt(payload.code)
//.await
//.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
//
//Ok(Json(TokenResponse { token: jwt }))
//}
