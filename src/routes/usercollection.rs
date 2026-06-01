use crate::AppState;
use crate::errors::AppError;
use crate::extractors::ApiJson;
use crate::models::dto::UserCollection;
use crate::models::user::Claims;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use serde_json::json;
use std::sync::Arc;

pub async fn get(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;
    let output = service.user_collection_logic.get(user_id).await?;
    Ok(Json(output))
}
//Claude.ai decided to disreguard what I already had written so in order to moce a little faster and not argue with a machine, I just wrote another method for input.
#[axum::debug_handler]
pub async fn sync(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    ApiJson(input): ApiJson<UserCollection>,
) -> Result<impl IntoResponse, AppError> {
    // `update_without_ownership` intentionally bypasses the per-user owner
    // check — `claims.sub` is not consulted here. Documented in the logic
    // layer; the original `user_id` extraction is left as a marker for the
    // planned tightening (require `claims.sub == input.user_id`).
    let _user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .update_without_ownership(input)
        .await?;
    Ok(Json(output))
}

//adding the favorite and saved sections
pub async fn microevent_save_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    service
        .user_collection_logic
        .microevent_save_toggle(id, user_id)
        .await?;

    Ok(Json(json!({
        "message": "Event save toggled!"
    })))
}

pub async fn microevent_favorite_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    service
        .user_collection_logic
        .microevent_favorite_toggle(id, user_id)
        .await?;

    Ok(Json(json!({
        "message": "Event favorite toggled!"
    })))
}
//adding the favorite and saved sections
pub async fn event_save_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    service
        .user_collection_logic
        .event_save_toggle(id, user_id)
        .await?;

    Ok(Json(json!({
        "message": "Event save toggled!"
    })))
}

/// Toggle an event's presence on the user's personal schedule.
/// Sister of `event_save_toggle` — distinct collection so a user can
/// schedule without saving and vice versa. Wired to
/// `POST /event/{id}/schedule-toggle`.
pub async fn event_schedule_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = &claims.sub;
    service
        .user_collection_logic
        .event_schedule_toggle(id, user_id)
        .await?;
    Ok(Json(json!({
        "message": "Event schedule toggled!"
    })))
}

/// Return hydrated EventResponse rows for every event on the user's
/// schedule. Mirrors `get_saved_events` but reads `scheduled_events`.
/// Wired to `GET /user/scheduled/events`.
pub async fn get_scheduled_events(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = &claims.sub;
    let output = service
        .user_collection_logic
        .get_scheduled_events(user_id)
        .await?;
    Ok(Json(output))
}

/// Toggle a microevent's presence on the user's personal schedule.
/// Sister of `event_schedule_toggle` — same shape, different column.
/// Wired to `POST /microevent/{id}/schedule-toggle`.
pub async fn microevent_schedule_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = &claims.sub;
    service
        .user_collection_logic
        .microevent_schedule_toggle(id, user_id)
        .await?;
    Ok(Json(json!({
        "message": "Microevent schedule toggled!"
    })))
}

/// Return hydrated Microevent rows for every microevent on the user's
/// schedule. Mirrors `get_saved_microevents` but reads
/// `scheduled_microevents`. Wired to `GET /user/scheduled/microevents`.
pub async fn get_scheduled_microevents(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = &claims.sub;
    let output = service
        .user_collection_logic
        .get_scheduled_microevents(user_id)
        .await?;
    Ok(Json(output))
}

pub async fn event_favorite_toggle(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    service
        .user_collection_logic
        .event_favorite_toggle(id, user_id)
        .await?;

    Ok(Json(json!({
        "message": "Event favorite toggled!"
    })))
}

pub async fn get_created_events(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_created_events(user_id)
        .await?;

    Ok(Json(output))
}

pub async fn get_created_microevents(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_created_microevents(user_id)
        .await?;

    Ok(Json(output))
}

pub async fn get_favorite_events(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_favorite_events(user_id)
        .await?;

    Ok(Json(output))
}

pub async fn get_favorite_microevents(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_favorite_microevents(user_id)
        .await?;

    Ok(Json(output))
}

pub async fn get_saved_events(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_saved_events(user_id)
        .await?;

    Ok(Json(output))
}

pub async fn get_saved_microevents(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;

    let output = service
        .user_collection_logic
        .get_saved_microevents(user_id)
        .await?;

    Ok(Json(output))
}
