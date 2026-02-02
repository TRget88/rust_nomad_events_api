// ============================================================================
// src/models/dto.rs - Data Transfer Objects
// ============================================================================
use serde::{Deserialize, Serialize};
//use crate::models::event_models::CampingInfo;
use crate::models::database_models::EventRow;
use crate::models::{event_models::*, microevents_models::Microevent};
use chrono::{DateTime, Utc};

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
    pub camping_info: Option<CampingInfo>,        // User can customize after applying template
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
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub event_type: EventType, // Full object, not just ID
    pub website: Option<String>,
    pub date_info: EventDate,
    pub location_info: Location,
    pub amenities: Option<Amenities>,
    pub camping_info: Option<CampingInfo>,
    //pub is_favorite: bool,
    //pub is_saved: bool,
}

// Helper to convert EventRow to EventResponse -moving list of saved and favorites to local storage
impl EventResponse {
    pub fn from_row(
        row: EventRow,
        //user_favorites: &[i64],
        //user_saved: &[i64],
    ) -> Result<Self, serde_json::Error> {
        let event: NomEvent = serde_json::from_str(&row.event_data)?;

        // Check if this event's ID is in the user's favorites list
        //let is_favorite: bool = user_favorites.contains(&row.id);

        // Check if this event's ID is in the user's saved list
        //let is_saved: bool = user_saved.contains(&row.id);

        Ok(EventResponse {
            id: Some(row.id),
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
            //is_favorite,
            //is_saved,
        })
    }

    // Alternative constructor when user info is not available (e.g., unauthenticated requests)
    //pub fn from_row_no_user(row: EventRow) -> Result<Self, serde_json::Error> {
    //Self::from_row(row, &[], &[])
    //}
}

pub struct EventRequest {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub event_type: EventType, // Full object, not just ID
    pub website: Option<String>,
    pub date_info: EventDate,
    pub location_info: Location,
    pub amenities: Option<Amenities>,
    pub camping_info: Option<CampingInfo>,
}

// This is what we return from the API - includes full EventType object
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MicroeventResponse {
    pub id: Option<i64>,
    pub event_id: Option<i64>,
    pub user_id: Option<String>,
    pub name: String,
    pub archive: bool,
    pub description: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    //pub is_favorite: bool,
    //pub is_saved: bool,
}

// Helper to convert Microevent to MicroeventResponse
impl MicroeventResponse {
    pub fn from_row(
        row: Microevent,
        //user_favorites: &[i64],
        //user_saved: &[i64],
    ) -> Result<Self, serde_json::Error> {
        // Check if this event's ID is in the user's favorites list
        //let is_favorite: bool = user_favorites.contains(&row.id);

        // Check if this event's ID is in the user's saved list
        //let is_saved: bool = user_saved.contains(&row.id);

        //don't know why but ai decided to make things a lot more complicated here.
        //// Check if this microevent's ID is in the user's favorites list
        //let is_favorite: bool = match row.id {
        //Some(id) => user_favorites.contains(&id),
        //None => false,
        //};
        //
        //// Check if this microevent's ID is in the user's saved list
        //let is_saved: bool = match row.id {
        //Some(id) => user_saved.contains(&id),
        //None => false,
        //};

        Ok(MicroeventResponse {
            id: Some(row.id),
            event_id: Some(row.event_id),
            user_id: Some(row.user_id),
            name: row.name,
            archive: row.archive,
            description: row.description,
            start_time: row.start_time,
            end_time: row.end_time,
            created_at: row.created_at,
            updated_at: row.updated_at,
            //is_favorite,
            //is_saved,
        })
    }

    // Alternative constructor when user info is not available (e.g., unauthenticated requests)
    //pub fn from_row_no_user(row: Microevent) -> Result<Self, serde_json::Error> {
    //Self::from_row(row, &[], &[])
    //}
}

pub struct MicroeventRequest {
    pub id: Option<i64>,
    pub event_id: Option<i64>,
    pub user_id: Option<String>,
    pub name: String,
    pub archive: bool,
    pub description: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    //pub is_favorite: bool,
    //pub is_saved: bool,
}

//Dont think this is ever used.
// This is what we return from the API - includes full EventType object
//#[derive(Debug, Serialize, Deserialize, Clone)]
//pub struct MicroeventResponse {
//pub name: String,
//pub description: Option<String>,
//pub user_id: i64,
//pub event_id: i64,
//pub start_time: Option<DateTime<Utc>>,
//pub end_time: Option<DateTime<Utc>>,
//pub created_at: Option<DateTime<Utc>>,
//pub updated_at: Option<DateTime<Utc>>
//}
//
//// Helper to convert EventRow to EventResponse
//impl MicroeventResponse {
//pub fn from_row(row: crate::context::microevent_context::MicroeventRow) -> Result<Self, serde_json::Error> {
//
//// Helper function to parse datetime
//let parse_datetime = |s: Option<String>| -> Option<DateTime<Utc>> {
//s.and_then(|date_str| {
//DateTime::parse_from_rfc3339(&date_str)
//.ok()
//.map(|dt| dt.with_timezone(&Utc))
//})
//};
//
//Ok(MicroeventResponse {
//name: row.name,
//description: Some(row.description),
//user_id: row.user_id,
//event_id: row.event_id,
//start_time: parse_datetime(row.start_time),
//end_time: parse_datetime(row.end_time),
//created_at: parse_datetime(row.created_at),
//updated_at: parse_datetime(row.updated_at),
//})
//}
//}
#[derive(Debug, Serialize, Deserialize)]
pub struct UserCollection {
    pub id: Option<i64>,
    pub user_id: Option<String>,
    pub favorite_events: Option<Vec<i64>>,
    pub favorite_microevents: Option<Vec<i64>>,
    pub saved_events: Option<Vec<i64>>,
    pub saved_microevents: Option<Vec<i64>>,
    pub created_events: Option<Vec<i64>>,
    pub created_microevents: Option<Vec<i64>>,
}
