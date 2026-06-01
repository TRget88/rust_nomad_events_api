// src/custom_middleware/api_key.rs
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub async fn validate_api_key(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let api_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let expected_hash =
        std::env::var("API_KEY_HASH").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let provided_hash = hex::encode(hasher.finalize());

    // Constant-time compare. SHA-256 hex strings are always 64 chars so the
    // lengths match; even if they didn't, `subtle::ConstantTimeEq` returns
    // Choice(0) on length mismatch without revealing where via timing.
    if !bool::from(provided_hash.as_bytes().ct_eq(expected_hash.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(request).await)
}
