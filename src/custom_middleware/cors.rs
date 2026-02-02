// src/custom_middleware/cors.rs
//use tower_http::cors::{CorsLayer, Any};
use axum::http::Method;

//pub fn configure_cors() -> CorsLayer {
//CorsLayer::new()
//.allow_origin(
//std::env::var("FRONTEND_URL")
//.unwrap_or_else(|_| "http://localhost:5173".to_string())
//.parse::<axum::http::HeaderValue>()
//.unwrap()
//)
//.allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
//.allow_headers(Any)
//.allow_credentials(true)
//}
