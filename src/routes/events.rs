use crate::AppState;
use crate::errors::AppError;
use crate::logic::EventLogic;
use crate::models::dto::EventQueryParams;
use crate::models::event_models::NomEvent;
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

pub async fn get_all(State(service): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let events = service.event_logic.get_all_events().await?;
    Ok(Json(events))
}

pub async fn get(
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let event = service.event_logic.get_event_by_id(id).await?;
    Ok(Json(event))
}

pub async fn search(
    Query(params): Query<EventQueryParams>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    println!("lat: {}", params.latitude.unwrap_or(0.0).to_string());
    println!("lon: {}", params.longitude.unwrap_or(0.0).to_string());
    println!("rad: {}", params.radius_miles.unwrap_or(0.0).to_string());

    // If lat/lon/radius provided, search by location
    if let (Some(lat), Some(lon), Some(radius)) =
        (params.latitude, params.longitude, params.radius_miles)
    {
        let events = service
            .event_logic
            .get_nearby_events(lat, lon, radius)
            .await?;
        return Ok(Json(events));
    }

    // If event_type provided, search by type
    if let Some(event_type) = params.event_type {
        //let type_id: i64 = event_type.id();
        //let events = service.event_logic.get_events_by_type(type_id).await?;
        let events = service.event_logic.get_events_by_type(event_type).await?;
        return Ok(Json(events));
    }

    // Otherwise return all
    let events = service.event_logic.get_all_events().await?;

    //println!("number of events found: {}", events.iter().count());

    Ok(Json(events))
}

pub async fn create(
    Extension(claims): Extension<Claims>,
    State(service): State<Arc<AppState>>,
    Json(mut event): Json<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    let user_id = &claims.sub;
    // Set user_id on the event
    event.user_id = Some(user_id.clone());
    let id = service.event_logic.create_event(event).await?;

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
    Json(event): Json<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    //pull claims data from request
    //let user_id = &claims.sub;
    service.event_logic.update_event(id, event, claims).await?;

    Ok(Json(json!({
        "message": "Event updated successfully"
    })))
}

pub async fn delete(
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    State(service): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    service.event_logic.delete_event(id, claims).await?;

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
////get the user id
//let user_id = &claims.sub;
//
//service.event_logic.save_toggle(id, user_id).await?;
//
//Ok(Json(json!({
//"message": "Event save toggled!"
//})))
//}
//
//pub async fn favorite_toggle(
//Extension(claims): Extension<Claims>,
//Path(id): Path<i64>,
//State(service): State<Arc<AppState>>,
//) -> Result<impl IntoResponse, AppError> {
////get the user id
//let user_id = &claims.sub;
//
//service.event_logic.favorite_toggle(id, user_id).await?;
//
//Ok(Json(json!({
//"message": "Event favorite toggled!"
//})))
//}
