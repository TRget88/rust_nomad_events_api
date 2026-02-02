use crate::errors::AppError;
use async_trait::async_trait;
use axum::{extract::FromRequestParts, http::request::Parts};
//use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "snake_case")]
pub enum UserRole {
    User,
    Admin,
    SuperAdmin,
}

//-- In your users table, the user_role column will contain:
//'user'         -- for UserRole::User
//'admin'        -- for UserRole::Admin
//'super_admin'  -- for UserRole::SuperAdmin

#[derive(Debug, Deserialize)]
pub struct GoogleIdToken {
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub sub: String, // Google user ID
    pub email_verified: bool,
}

// Your JWT claims
//#[derive(Debug, Serialize, Deserialize, Clone)]
//pub struct Claims {
//pub sub: String,
//pub email: String,
////pub role: UserRole,
//pub role: String,
//pub exp: usize,
//}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,      // "subject" - the user ID (who the token is about)
    pub email: String,    // Custom claim - user's email
    pub username: String, // Custom claim - user's username
    pub role: String,     // Custom claim - user's role
    pub exp: usize,       // "expiration" - when token expires (Unix timestamp)
    pub iat: usize,       // "issued at" - when token was created (Unix timestamp)
}

#[derive(Deserialize)]
pub struct GoogleLoginRequest {
    pub credential: String, // The ID token from Google
}

//this needs to match what is on the otherside, shown below
//export interface User {
//id: string;                    // UUID from backend
//email: string;
//name: string;
//role: string;
//picture_url?: string;
//provider: 'google' | 'facebook';
//provider_id: string;           // Google's "sub" or Facebook's "id"
//created_at: string;            // ISO date string
//updated_at: string;            // ISO date string
//}
//
// For responses that include the token
//export interface AuthResponse {
//token: string;
//user: User;  // ✅ Use the same User type
//}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub user_name: Option<String>,
    pub picture_url: Option<String>,
    pub role: String,
    pub provider: String,
    pub provider_id: String, // Google's "sub" or Facebook's "id"
    pub created_at: String,  // ISO date string
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub user_name: Option<String>,
    pub email: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
}
impl UserRole {
    pub fn can_manage_users(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::SuperAdmin)
    }

    pub fn can_manage_admins(&self) -> bool {
        matches!(self, UserRole::SuperAdmin)
    }

    pub fn can_delete_any_content(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::SuperAdmin)
    }
}
