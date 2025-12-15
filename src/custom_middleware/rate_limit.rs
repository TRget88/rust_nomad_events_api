// src/middleware/rate_limit.rs
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }
    
    pub async fn check_rate_limit(&self, key: &str) -> bool {
        let mut requests = self.requests.lock().await;
        let now = Instant::now();
        
        // Get or create entry for this key
        let entries = requests.entry(key.to_string()).or_insert_with(Vec::new);
        
        // Remove old entries outside the time window
        entries.retain(|&timestamp| now.duration_since(timestamp) < self.window);
        
        // Check if limit exceeded
        if entries.len() >= self.max_requests {
            return false;
        }
        
        // Add current request
        entries.push(now);
        true
    }
}

pub async fn rate_limit_middleware(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Get client identifier (IP or API key)
    let client_id = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    
    // Get rate limiter from app state (you'll need to add this to your app state)
    // For now, this is a simplified example
    let rate_limiter = RateLimiter::new(100, 60); // 100 requests per minute
    
    if !rate_limiter.check_rate_limit(client_id).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    
    Ok(next.run(request).await)
}
