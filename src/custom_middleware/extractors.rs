// src/extractors.rs or src/middleware/extractors.rs
//async_trait is for older vsersions of axum.
use crate::errors::AppError;
use crate::models::user::Claims;
use axum::{extract::FromRequestParts, http::request::Parts};
//use axum::{async_trait, extract::FromRequestParts, http::request::Parts};

//#[async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Claims>()
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))
    }
}
