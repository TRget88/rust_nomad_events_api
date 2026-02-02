use crate::models::event_models::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Microevent {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub event_id: i64, // Foreign key to events table
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub archive: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventWithMicroevents {
    #[serde(flatten)]
    #[serde(default)]
    pub event: NomEvent,
    #[serde(default)]
    pub microevents: Vec<Microevent>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMicroeventRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub start_time: DateTime<Utc>,
    #[serde(default)]
    pub end_time: DateTime<Utc>,
}
