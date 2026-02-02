use std::sync::Arc;

use crate::AppState;
use crate::errors::AppError;
use crate::logic::MicroeventLogic;
use crate::models::dto::EventQueryParams;
use crate::models::microevents_models::Microevent;
use crate::models::user::Claims;
use axum::Extension;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use uuid::Uuid;

pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let events = service.microevent_logic.get_all().await?;
    Ok(Json(events))
}

pub async fn get_by_event(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let events = service.microevent_logic.get_by_event(id).await?;
    Ok(Json(events))
}

pub async fn get(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let event = service.microevent_logic.get(id).await?;
    Ok(Json(event))
}

pub async fn create(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    Json(mut event): Json<Microevent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    let user_id = &claims.sub;
    // Set user_id on the microevent (remove '&' and add 'mut' above)
    event.user_id = user_id.clone(); // or user_id.to_string() if sub is not String

    let id = service.microevent_logic.create(event).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Event created successfully",
            "id": id
        })),
    ))
}

pub async fn create_by_event(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    Json(mut event): Json<Microevent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    let user_id = &claims.sub;
    // Set user_id on the microevent (remove '&' and add 'mut' above)
    event.user_id = user_id.clone(); // or user_id.to_string() if sub is not String

    let id = service.microevent_logic.create(event).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Event created successfully",
            "id": id
        })),
    ))
}

pub async fn update(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
    Json(event): Json<Microevent>,
) -> Result<impl IntoResponse, AppError> {
    service.microevent_logic.update(id, event, claims).await?;

    Ok(Json(json!({
        "message": "Event updated successfully"
    })))
}

pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    //check to see if the event belongs to the user or if the user is an admin
    service.microevent_logic.delete(id, claims).await?;

    Ok(Json(json!({
        "message": "Event deleted successfully"
    })))
}

//adding the favorite and saved sections
//pub async fn save_toggle(
//Extension(claims): Extension<Claims>,
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//let user_id = &claims.sub;
//
//service.microevent_logic.save_toggle(id, user_id).await?;
//
//Ok(Json(json!({
//"message": "Microevent save toggled!"
//})))
//}
//
//pub async fn favorite_toggle(
//Extension(claims): Extension<Claims>,
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
//let user_id = &claims.sub;
//
//service
//.microevent_logic
//.favorite_toggle(id, user_id)
//.await?;
//
//Ok(Json(json!({
//"message": "Microevent favorite toggled!"
//})))
//}
