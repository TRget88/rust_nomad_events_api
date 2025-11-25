// ============================================================================
// src/models/dto.rs - Data Transfer Objects
// ============================================================================
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEventDto {
    pub name: String,
    pub description: String,
    pub event_type: String,
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
    pub event_type: String,
    pub website: Option<String>,
    // Simplified response
}

#[derive(Debug, Deserialize)]
pub struct EventQueryParams {
    pub event_type: Option<String>,
    pub camping_allowed: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub radius_miles: Option<f64>,
}