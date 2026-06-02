// src/routes/analytics.rs
//
// Admin-facing analytics summary. Read-only aggregate queries against
// the existing tables — no separate analytics store. Returns the
// counts the dashboard surfaces (total + last-30-days for users,
// events, microevents) plus a few cross-cutting numbers.
//
// Routed under /admin/analytics/summary, gated through the existing
// `require_admin` middleware in main.rs's admin_view_routes block.
// Queries are SQLite-flavor SQL — `datetime('now', '-30 days')` is
// the SQLite idiom for "30 days ago" against TEXT-stored timestamps.

use crate::AppState;
use crate::errors::AppError;
use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct AnalyticsSummary {
    pub total_users: i64,
    pub signups_last_30d: i64,
    pub active_users_30d: i64,
    pub total_events: i64,
    pub events_added_last_30d: i64,
    pub total_microevents: i64,
    pub microevents_added_last_30d: i64,
    pub total_event_types: i64,
    /// Sum of items across all users' personal schedules. Indicates
    /// how many "I'm going to this" commitments the platform holds.
    pub total_scheduled_items: i64,
    /// Sum of items across all users' saved-event libraries.
    pub total_saved_items: i64,
}

/// GET /admin/analytics/summary — admin-gated by main.rs's
/// `admin_view_routes` `route_layer`.
///
/// Opens a fresh single-connection pool from `DATABASE_URL` per call.
/// Acceptable because: this endpoint is admin-only (low call volume)
/// and the queries are pure aggregates that complete in milliseconds.
/// If call volume ever justifies it, thread the main pool through
/// AppState and read from it instead — that's the only refactor.
pub async fn summary(State(_service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://events.db".to_string());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .map_err(|e| AppError::DatabaseError(format!("analytics pool: {}", e)))?;

    // Helper closure pattern would be cleaner but trait bounds get
    // unwieldy. Each query is short; we just inline the await chain.
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let signups_last_30d: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE datetime(created_at) >= datetime('now', '-30 days')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let active_users_30d: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE last_login_at IS NOT NULL \
         AND datetime(last_login_at) >= datetime('now', '-30 days')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let events_added_last_30d: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE datetime(created_at) >= datetime('now', '-30 days')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let total_microevents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM microevents")
        .fetch_one(&pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let microevents_added_last_30d: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM microevents WHERE datetime(created_at) >= datetime('now', '-30 days')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let total_event_types: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_types")
        .fetch_one(&pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let total_scheduled_items: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(json_array_length(scheduled_events)), 0) FROM user_event_data",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let total_saved_items: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(json_array_length(saved_events)), 0) FROM user_event_data",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let summary = AnalyticsSummary {
        total_users,
        signups_last_30d,
        active_users_30d,
        total_events,
        events_added_last_30d,
        total_microevents,
        microevents_added_last_30d,
        total_event_types,
        total_scheduled_items,
        total_saved_items,
    };
    Ok(Json(summary))
}
