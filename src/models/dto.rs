// ============================================================================
// src/models/dto.rs - Data Transfer Objects
// ============================================================================
use serde::{Deserialize, Serialize};
//use crate::models::event_models::CampingInfo;
use crate::models::event_models::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEventDto {
    pub name: String,
    pub description: String,
    pub event_type: Option<i64>,
    pub website: Option<String>,
    // Add other required fields
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateEventDto {
    pub name: Option<String>,
    pub description: Option<String>,
    pub event_type: Option<String>,
    pub website: Option<String>,
    // Add other fields that can be updated
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventResponseDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub event_type: Option<i64>,
    pub website: Option<String>,
    // Simplified response
}

#[derive(Debug, Deserialize)]
pub struct EventQueryParams {
    pub event_type: Option<i64>,
    pub camping_allowed: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub radius_miles: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEventRequest {
    pub name: String,
    pub description: String,
    pub event_type: Option<i64>,
    pub website: Option<String>,
    // ... other fields
    pub camping_profile_id_to_apply: Option<i64>, // Optional: pre-fill from template
    pub camping_info: Option<CampingInfo>, // User can customize after applying template
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CampingProfileListResponse {
    pub id: i64,
    pub profile_name: String,
    pub description: Option<String>,
}

// This is what we return from the API - includes full EventType object
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventResponse {
    pub name: String,
    pub description: String,
    pub event_type: EventType,  // Full object, not just ID
    pub website: Option<String>,
    pub date_info: EventDate,
    pub location_info: Location,
    pub amenities: Option<Amenities>,
    pub camping_info: Option<CampingInfo>,
}

// Helper to convert EventRow to EventResponse
impl EventResponse {
    pub fn from_row(row: crate::repositories::event_repository::EventRow) -> Result<Self, serde_json::Error> {
        let mut event: NomEvent = serde_json::from_str(&row.event_data)?;
        
        Ok(EventResponse {
            name: event.name,
            description: event.description,
            event_type: EventType {
                id: Some(row.event_type_id),
                name: row.event_type_name,
                description: row.event_type_description,
                map_indicator: row.event_type_map_indicator,
                category: row.event_type_category,
            },
            website: event.website,
            date_info: event.date_info,
            location_info: event.location_info,
            amenities: event.amenities,
            camping_info: event.camping_info,
        })
    }
}