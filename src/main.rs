use axum::{
    routing::get,
    Router,
};

#[tokio::main]
async fn main() {
    // Router
    let app = Router::new()
    .route("/", get(rust_nomad_events_api));
    
    //listening globally on port 3000    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

