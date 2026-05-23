// ============================================================================
// src/errors/app_error.rs
// ============================================================================
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    DatabaseError(String),
    ValidationError(String),
    SerializationError(String),
    Unauthorized(String),
    Forbidden(String),
    BadRequest(String),
    InternalError(String),
    Conflict(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // For variants that should NEVER expose their detail to the client
        // (DatabaseError, InternalError, SerializationError), log the original
        // message and return a generic body. Other variants carry user-safe
        // messages and pass through.
        let (status, error_message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::DatabaseError(msg) => {
                tracing::error!("DatabaseError: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::ValidationError(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::SerializationError(msg) => {
                tracing::error!("SerializationError: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::InternalError(msg) => {
                tracing::error!("InternalError: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("Resource not found".to_string()),
            sqlx::Error::Database(db_err) => {
                // SQLite extended error codes:
                //   2067 / 1555 = UNIQUE / PRIMARY KEY constraint
                //    787       = FOREIGN KEY constraint
                //   1299       = NOT NULL constraint
                //    275       = CHECK constraint
                // Map known constraint violations to 4xx so the client gets
                // an actionable status; everything else falls through to a
                // logged-but-sanitized 500 (see IntoResponse above).
                match db_err.code().as_deref() {
                    Some("2067") | Some("1555") => {
                        AppError::Conflict("Resource already exists".to_string())
                    }
                    Some("787") => {
                        AppError::BadRequest("Referenced resource does not exist".to_string())
                    }
                    Some("1299") => AppError::BadRequest("Required field is missing".to_string()),
                    Some("275") => AppError::BadRequest("Value violates a constraint".to_string()),
                    _ => AppError::DatabaseError(format!("Database error: {}", db_err)),
                }
            }
            _ => AppError::DatabaseError(format!("Database error: {}", err)),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::SerializationError(format!("Serialization error: {}", err))
    }
}

impl From<Box<dyn std::error::Error>> for AppError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        AppError::InternalError(err.to_string())
    }
}

impl From<std::env::VarError> for AppError {
    fn from(err: std::env::VarError) -> Self {
        AppError::InternalError(format!("Environment variable error: {}", err))
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        AppError::Unauthorized(format!("JWT error: {}", err))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::InternalError(format!("HTTP request error: {}", err))
    }
}

impl From<std::time::SystemTimeError> for AppError {
    fn from(err: std::time::SystemTimeError) -> Self {
        AppError::InternalError(format!("Time error: {}", err))
    }
}
