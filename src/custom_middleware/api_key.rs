// src/custom_middleware/api_key.rs
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use sha2::{Sha256, Digest};

pub async fn validate_api_key(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
//println!("########################################################################################################");
    //println!("🔐 API Key Middleware - Request received");
    //println!("📋 Headers: {:?}", headers);
 //println!("-------------------------------------------------------------------------------------------------------");
    let api_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    //println!("Key: {:?}", api_key);
    //println!("-------------------------------------------------------------------------------------------------------");
    let expected_hash = std::env::var("API_KEY_HASH")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
     //println!("expected_hash: {}", expected_hash);
    //println!("-------------------------------------------------------------------------------------------------------");
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let provided_hash = hex::encode(hasher.finalize());
    //println!("provided_hash: {}", provided_hash);
    //println!("-------------------------------------------------------------------------------------------------------");
    if provided_hash != expected_hash {
        return Err(StatusCode::UNAUTHORIZED);
    }
    
    Ok(next.run(request).await)
}