use crate::AppState;
use crate::errors::AppError;
use crate::logic::EventLogic;
use crate::models::database_models::UserEventDataRow;
use crate::models::dto::UserCollection;
use crate::models::user::Claims;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
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
    Json(input): Json<UserCollection>,
) -> Result<impl IntoResponse, AppError> {
    //get the user id
    let user_id = &claims.sub;
    //check if user_id and input.user_id are equal

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
