use std::sync::Arc;

use crate::AppState;
use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::audit::{AuditLogQuery, AuditRecord, actions, target_types};
use crate::models::dto::PaginationQuery;
use crate::models::user::Claims;
use crate::models::user::*;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use uuid::Uuid;

// Audit logging is a method on `AuditLogLogic` — see
// `audit_log_logic.rs::record_best_effort`. Call it directly as
// `service.audit_log_logic.record_best_effort(record).await` from any
// handler that needs it. Previously this file held a local helper; it
// got hoisted to the logic struct so the other admin route modules
// (event_type, camping_profiles, events) can share it without duplicating
// the "log on failure but don't propagate" pattern.

// ============================================================================
// Self Routes — operations on the authenticated user's own account
// ============================================================================

pub async fn get_self(
    State(service): State<Arc<AppState>>,
    req: Request, // Request must be last
) -> Result<impl IntoResponse, AppError> {
    let claims = req
        .extensions()
        .get::<Claims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::BadRequest("Invalid user ID".to_string()))?;

    let user = service.user_logic.get(user_id).await?;
    Ok(Json(user))
}
pub async fn update_self(
    Extension(claims): Extension<Claims>, // Extract from extensions
    State(service): State<Arc<AppState>>,
    ApiJson(update): ApiJson<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    service
        .user_logic
        .update_profile(
            &claims.sub,
            update.user_name.as_deref(),
            update.email.as_deref(),
            update.timezone.as_deref(),
            update.language.as_deref(),
        )
        .await?;

    Ok(Json(json!({
        "message": "Profile updated successfully"
    })))
}

/// DELETE /self — soft-delete the authenticated user. Closes the GDPR/CCPA
/// "right to erasure" loop: `user_logic.delete_user` sets `deleted_at` so
/// (1) the auth middleware's `WHERE deleted_at IS NULL` clause rejects
/// every subsequent request bearing the same JWT, (2) public queries don't
/// surface the soft-deleted row, and (3) cached counts on `users` aren't
/// touched (the row still exists for foreign-key integrity).
///
/// We deliberately don't revoke the JWT explicitly — the soft-delete makes
/// the next `auth_middleware` lookup return None, which already produces a
/// 401. Adding a jwt_revocations write would be belt-and-suspenders but
/// requires the user to be alive in the DB (FK to users.id), so the
/// natural order is: soft-delete first, the next request 401s.
pub async fn delete_self(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.user_logic.delete_user(&claims.sub).await?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "message": "Account deleted. Subsequent requests with this token will be rejected."
        })),
    ))
}

/// GET /self/data-export — bundle every piece of user-owned data into one
/// JSON response for GDPR/CCPA portability compliance. Caller is the
/// authenticated user; the export only ever contains data tied to their
/// own user_id (no cross-user leakage). The response is intended to be
/// downloaded and saved by the client; the frontend's account page wraps
/// this in a "Download my data" button.
///
/// Orchestrates across UserLogic (profile) + UserCollectionLogic (events,
/// microevents, collection arrays). This is the route layer's appropriate
/// job — composing multiple logics for a cross-resource operation —
/// per the project's layering rule.
pub async fn data_export_self(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = &claims.sub;
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|_| AppError::BadRequest("Invalid user ID in token".to_string()))?;

    // Run the aggregating reads. These are independent; serializing them
    // is fine for now — the cost is dominated by JSON encoding, not the
    // queries. Parallelizing via tokio::try_join! is a follow-up if needed.
    let user_row = service.user_logic.get_self(user_uuid).await?;
    let collection = service.user_collection_logic.get(user_id).await?;
    let created_events = service
        .user_collection_logic
        .get_created_events(user_id)
        .await?;
    let created_microevents = service
        .user_collection_logic
        .get_created_microevents(user_id)
        .await?;
    let favorite_events = service
        .user_collection_logic
        .get_favorite_events(user_id)
        .await?;
    let favorite_microevents = service
        .user_collection_logic
        .get_favorite_microevents(user_id)
        .await?;
    let saved_events = service
        .user_collection_logic
        .get_saved_events(user_id)
        .await?;
    let saved_microevents = service
        .user_collection_logic
        .get_saved_microevents(user_id)
        .await?;

    let export = UserDataExport {
        exported_at: chrono::Utc::now(),
        user: UserInfo {
            id: user_row.id.clone(),
            email: user_row.email.clone().unwrap_or_default(),
            // `user_name` in our schema IS the Google display name set at
            // signup, so we surface it on both fields for compatibility
            // with the AuthResponse-shape consumers expect.
            name: Some(user_row.user_name.clone()),
            user_name: Some(user_row.user_name),
            picture_url: user_row.profile_picture_url,
            role: user_row.role,
            provider: user_row.oauth_provider,
            provider_id: user_row.oauth_id,
            created_at: user_row.created_at.to_rfc3339(),
            updated_at: user_row.updated_at.to_rfc3339(),
        },
        collection,
        created_events,
        created_microevents,
        favorite_events,
        favorite_microevents,
        saved_events,
        saved_microevents,
    };

    Ok((StatusCode::OK, Json(export)))
}

// ============================================================================
// Admin Routes — manage all users
// ============================================================================

/// GET /api/admin/users - Get all users (Admin only)
pub async fn get_all(
    Query(params): Query<PaginationQuery>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    tracing::debug!("reaching get_all user route");
    let users = service
        .user_logic
        .get_all(params.limit, params.offset)
        .await?;
    Ok(Json(users))
}

/// GET /api/admin/users/:id - Get user by ID (Admin only)
pub async fn get(
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid UUID".into()))?;

    let user = service.user_logic.get(user_id).await?;
    Ok(Json(user))
}

/// PUT /api/admin/users/:id - Update user (Admin only)
pub async fn update(
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
    ApiJson(update): ApiJson<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    service
        .user_logic
        .update_profile(
            &id,
            update.user_name.as_deref(),
            update.email.as_deref(),
            update.timezone.as_deref(),
            update.language.as_deref(),
        )
        .await?;

    Ok(Json(json!({
        "message": "User updated successfully"
    })))
}

/// DELETE /api/admin/users/:id - Soft-delete user (Admin only).
/// Returns 404 if the user doesn't exist or was already deleted.
pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.user_logic.delete_user(&id).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::USER_DELETE.to_string(),
            target_type: target_types::USER.to_string(),
            target_id: id.clone(),
            metadata: json!({}),
        })
        .await;

    Ok(Json(json!({
        "message": "User deleted successfully"
    })))
}

/// PUT /admin/users/:id/role — promote or demote a user.
/// Body: `{"role": "user" | "admin" | "super_admin"}`. Rejects demoting the
/// last SuperAdmin (enforced by `UserContext::update_user_role`).
pub async fn update_role(
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<UpdateRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let role = match payload.role.as_str() {
        "user" => UserRole::User,
        "admin" => UserRole::Admin,
        "super_admin" => UserRole::SuperAdmin,
        other => {
            return Err(AppError::BadRequest(format!(
                "Invalid role: {other:?}. Expected user, admin, or super_admin."
            )));
        }
    };

    service.user_logic.update_role(&id, role).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::USER_UPDATE_ROLE.to_string(),
            target_type: target_types::USER.to_string(),
            target_id: id.clone(),
            metadata: json!({ "new_role": payload.role }),
        })
        .await;

    Ok(Json(json!({ "message": "Role updated successfully" })))
}

/// POST /admin/users/:id/lock — lock a user out.
/// Body: `{"reason": "...", "until": "<ISO8601>"?}`. Permanent if `until` omitted.
pub async fn lock(
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
    ApiJson(payload): ApiJson<LockUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    service
        .user_logic
        .lockout_user(&id, &payload.reason, payload.until)
        .await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::USER_LOCK.to_string(),
            target_type: target_types::USER.to_string(),
            target_id: id.clone(),
            metadata: json!({
                "reason": payload.reason,
                "until": payload.until.map(|d| d.to_rfc3339()),
            }),
        })
        .await;

    Ok(Json(json!({ "message": "User locked" })))
}

/// POST /admin/users/:id/unlock — clear an existing lockout.
pub async fn unlock(
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.user_logic.unlock_user(&id).await?;

    service
        .audit_log_logic
        .record_best_effort(AuditRecord {
            actor_user_id: claims.sub,
            action: actions::USER_UNLOCK.to_string(),
            target_type: target_types::USER.to_string(),
            target_id: id.clone(),
            metadata: json!({}),
        })
        .await;

    Ok(Json(json!({ "message": "User unlocked" })))
}

/// GET /admin/audit-log?limit=N&offset=M — list recent admin audit
/// entries, newest first. Limit/offset semantics match the shared
/// `util::validate_pagination` contract (default 200, max 500). Pagination
/// here lets a SuperAdmin walk deep into history without growing the
/// per-request response indefinitely.
pub async fn list_audit_log(
    Query(params): Query<AuditLogQuery>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let entries = service
        .audit_log_logic
        .list_recent(params.limit, params.offset)
        .await?;
    Ok(Json(entries))
}
