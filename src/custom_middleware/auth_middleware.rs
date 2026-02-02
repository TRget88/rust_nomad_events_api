// src/middleware/auth.rs
use crate::errors::AppError;
use crate::models::user::{Claims, UserRole};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

// ============================================================================
// JWT Claims Structure
// ============================================================================

impl Claims {
    pub fn get_role(&self) -> UserRole {
        match self.role.as_str() {
            "admin" => UserRole::Admin,
            "super_admin" => UserRole::SuperAdmin,
            _ => UserRole::User,
        }
    }
}

// ============================================================================
// Auth Middleware - Verifies JWT and checks lockout status
// ============================================================================
pub async fn auth_middleware(
    State(pool): State<SqlitePool>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    println!("auth_middleware is going to pull token out.");

    // Extract token from Authorization header
    let token = extract_token(&req)?;

    // Verify JWT
    let jwt_secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::InternalError("JWT_SECRET not configured".to_string()))?;

    let token_data = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| {
        eprintln!("JWT verification failed: {:?}", e);
        AppError::Unauthorized("Invalid token".to_string())
    })?;

    let claims = token_data.claims;

    // Check if user is locked out (real-time check)
    let locked_out: bool = sqlx::query_scalar(
        "SELECT locked_out FROM users 
         WHERE id = ?1 
         AND deleted_at IS NULL
         AND (lockout_until IS NULL OR lockout_until > datetime('now'))",
    )
    .bind(&claims.sub)
    .fetch_optional(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .unwrap_or(false);

    if locked_out {
        return Err(AppError::Forbidden("Account is locked".to_string()));
    }

    // Attach claims to request extensions for handlers to use
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

// ============================================================================
// Role-Based Middleware - Requires specific role
// ============================================================================
pub async fn require_role(
    required_role: UserRole,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Get claims from request extensions (set by auth_middleware)
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?
        .clone();

    let user_role = claims.get_role();

    // Check if user has required role
    if !has_permission(&user_role, &required_role) {
        return Err(AppError::Forbidden(format!(
            "Requires {:?} role or higher",
            required_role
        )));
    }

    Ok(next.run(req).await)
}

// ============================================================================
// Admin Middleware - Requires Admin or SuperAdmin
// ============================================================================
pub async fn require_admin(req: Request, next: Next) -> Result<Response, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let role = claims.get_role();

    match role {
        UserRole::Admin | UserRole::SuperAdmin => Ok(next.run(req).await),
        UserRole::User => Err(AppError::Forbidden("Admin access required".to_string())),
    }
}

// ============================================================================
// SuperAdmin Middleware - Requires SuperAdmin only
// ============================================================================
pub async fn require_super_admin(req: Request, next: Next) -> Result<Response, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    println!("checking if user is super admin");

    let role = claims.get_role();

    println!("user is in role {:?}", role.clone());

    match role {
        UserRole::SuperAdmin => Ok(next.run(req).await),
        _ => Err(AppError::Forbidden(
            "SuperAdmin access required".to_string(),
        )),
    }
}

// ============================================================================
// Resource Owner Middleware - User can only access their own resources
// ============================================================================
pub async fn require_owner_or_admin(resource_user_id: &str, req: &Request) -> Result<(), AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let role = claims.get_role();

    // Allow if user owns the resource OR is admin/super_admin
    if claims.sub == resource_user_id || matches!(role, UserRole::Admin | UserRole::SuperAdmin) {
        Ok(())
    } else {
        Err(AppError::Forbidden("Access denied".to_string()))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract JWT token from Authorization header
fn extract_token(req: &Request) -> Result<String, AppError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".to_string()))?
        .to_str()
        .map_err(|_| AppError::Unauthorized("Invalid Authorization header".to_string()))?;

    if !auth_header.starts_with("Bearer ") {
        return Err(AppError::Unauthorized(
            "Authorization header must start with 'Bearer '".to_string(),
        ));
    }

    Ok(auth_header[7..].to_string())
}

/// Check if user role has permission for required role
fn has_permission(user_role: &UserRole, required_role: &UserRole) -> bool {
    match (user_role, required_role) {
        // SuperAdmin can do anything
        (UserRole::SuperAdmin, _) => true,

        // Admin can do admin and user things
        (UserRole::Admin, UserRole::Admin) => true,
        (UserRole::Admin, UserRole::User) => true,
        (UserRole::Admin, UserRole::SuperAdmin) => false,

        // User can only do user things
        (UserRole::User, UserRole::User) => true,
        (UserRole::User, _) => false,
    }
}

// ============================================================================
// Usage Examples in Router
// ============================================================================

/*
use axum::{
    routing::{get, post, delete},
    middleware,
    Router,
};

pub fn create_router(pool: SqlitePool) -> Router {
    Router::new()
        // Public routes (no auth)
        .route("/api/auth/login", post(login_handler))
        .route("/api/auth/signup", post(signup_handler))

        // Protected routes (requires any authenticated user)
        .route("/api/events", get(get_events))
        .route("/api/events/:id", get(get_event))
        .route_layer(middleware::from_fn_with_state(pool.clone(), auth_middleware))

        // User-specific routes (requires ownership or admin)
        .route("/api/users/:id", get(get_user_profile))
        .route("/api/users/:id", put(update_user_profile))
        .route_layer(middleware::from_fn_with_state(pool.clone(), auth_middleware))

        // Admin-only routes
        .route("/api/admin/users", get(list_all_users))
        .route("/api/admin/users/:id/lockout", post(lockout_user))
        .route_layer(middleware::from_fn(require_admin))
        .route_layer(middleware::from_fn_with_state(pool.clone(), auth_middleware))

        // SuperAdmin-only routes
        .route("/api/admin/users/:id/role", put(update_user_role))
        .route("/api/admin/users/:id/delete", delete(hard_delete_user))
        .route_layer(middleware::from_fn(require_super_admin))
        .route_layer(middleware::from_fn_with_state(pool.clone(), auth_middleware))

        .with_state(pool)
}

// Example handler that uses Claims extractor
async fn get_user_profile(
    Path(user_id): Path<String>,
    claims: Claims,  // Automatically extracted from request
    State(pool): State<SqlitePool>,
) -> Result<Json<UserProfile>, AppError> {
    // Check if user can access this profile
    require_owner_or_admin(&user_id, &claims)?;

    // Fetch and return profile
    // ...
    Ok(Json(profile))
}

// Example handler checking role manually
async fn delete_event(
    Path(event_id): Path<String>,
    claims: Claims,
    State(pool): State<SqlitePool>,
) -> Result<StatusCode, AppError> {
    // Get event to check ownership
    let event = get_event_from_db(&event_id, &pool).await?;

    // Allow if user created it OR is admin
    if event.creator_id != claims.sub {
        let role = claims.get_role();
        if !matches!(role, UserRole::Admin | UserRole::SuperAdmin) {
            return Err(AppError::Forbidden("Cannot delete others' events".into()));
        }
    }

    // Delete event
    // ...
    Ok(StatusCode::NO_CONTENT)
}
*/
