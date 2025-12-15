// ============================================================================
// src/handlers/event_handlers.rs - HTTP Handlers (Controllers)
// ============================================================================
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

use crate::errors::AppError;
use crate::models::dto::EventQueryParams;
use crate::models::event_models::NomEvent;
use crate::services::EventService;

pub async fn get_all_events(
    State(service): State<Arc<EventService>>,
) -> Result<impl IntoResponse, AppError> {
    let events = service.get_all_events().await?;
    Ok(Json(events))
}

pub async fn get_event(
    Path(id): Path<i64>,
    State(service): State<Arc<EventService>>,
) -> Result<impl IntoResponse, AppError> {
    let event = service.get_event_by_id(id).await?;
    Ok(Json(event))
}

pub async fn search_events(
    Query(params): Query<EventQueryParams>,
    State(service): State<Arc<EventService>>,
) -> Result<impl IntoResponse, AppError> {
    println!("lat: {}",params.latitude.unwrap_or(0.0).to_string());
    println!("lon: {}",params.longitude.unwrap_or(0.0).to_string());
    println!("rad: {}",params.radius_miles.unwrap_or(0.0).to_string());

    // If lat/lon/radius provided, search by location
    if let (Some(lat), Some(lon), Some(radius)) =
        (params.latitude, params.longitude, params.radius_miles)
    {
        let events = service.get_nearby_events(lat, lon, radius).await?;
        return Ok(Json(events));
    }

    // If event_type provided, search by type
    if let Some(event_type) = params.event_type {
        //let type_id: i64 = event_type.id();
        //let events = service.get_events_by_type(type_id).await?;
        let events = service.get_events_by_type(event_type).await?;
        return Ok(Json(events));
    }

    // Otherwise return all
    let events = service.get_all_events().await?;

    println!("number of events found: {}", events.iter().count());

    Ok(Json(events))
}

pub async fn create_event(
    State(service): State<Arc<EventService>>,
    Json(event): Json<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    let id = service.create_event(event).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Event created successfully",
            "id": id
        })),
    ))
}

pub async fn update_event(
    Path(id): Path<i64>,
    State(service): State<Arc<EventService>>,
    Json(event): Json<NomEvent>,
) -> Result<impl IntoResponse, AppError> {
    service.update_event(id, event).await?;

    Ok(Json(json!({
        "message": "Event updated successfully"
    })))
}

pub async fn delete_event(
    Path(id): Path<i64>,
    State(service): State<Arc<EventService>>,
) -> Result<impl IntoResponse, AppError> {
    service.delete_event(id).await?;

    Ok(Json(json!({
        "message": "Event deleted successfully"
    })))
}
