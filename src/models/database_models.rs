//This file will contain only models that are directly used in the database
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::SqlitePool;

//####################################################################
//Event model
//####################################################################
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct EventRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub website: Option<String>,
    pub event_type_id: i64, // Changed: now stores FK to event_types
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub camping_allowed: Option<bool>,
    pub event_data: String, // Still stores full event as JSON

    // Event type fields from JOIN --- This seems very wrong. It seems to be doing more work than nessicary, Need something closer to a VM but this seems like it will store these again?
    pub event_type_name: String,
    pub event_type_description: String,
    pub event_type_map_indicator: String,
    pub event_type_category: String,
}

//####################################################################
//microevent model
//####################################################################
#[derive(sqlx::FromRow)]
pub struct MicroeventRow {
    pub id: i64,
    pub archive: bool,
    pub user_id: String,
    pub event_id: i64,
    pub name: String,
    pub description: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

//####################################################################
//camping profile model
//####################################################################
#[derive(sqlx::FromRow)]
pub struct CampingProfileRow {
    pub id: i64,
    pub profile_name: String,
    pub description: Option<String>,
    pub camping_data: String,
}

//####################################################################
//Event Type model
//####################################################################
#[derive(sqlx::FromRow)]
pub struct EventTypeRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub map_indicator: String,
    pub category: String,
}
//####################################################################
//User model
//####################################################################

#[derive(Serialize, Deserialize, sqlx::FromRow, Debug, Clone)]
pub struct UserRow {
    //Identity
    pub id: String, //This will be a uuid
    pub oauth_id: String,
    pub oauth_provider: String,
    pub user_name: String,

    // Contact (optional but useful)
    pub email: Option<String>, // ADD: For notifications, account recovery
    pub email_verified: bool,  // ADD: Track if email is verified
    pub profile_picture_url: Option<String>, // ADD: From OAuth provider

    //Security & Status
    pub locked_out: bool,
    pub lockout_reason: Option<String>, // ADD: Why are they locked out?
    pub lockout_until: Option<DateTime<Utc>>, // ADD: Temporary bans
    pub role: String,

    // Timestamps
    pub created_at: DateTime<Utc>,            // ADD: When did they join
    pub updated_at: DateTime<Utc>,            // ADD: Last profile update
    pub last_login_at: Option<DateTime<Utc>>, // ADD: Track activity
    pub deleted_at: Option<DateTime<Utc>>,    // ADD: Soft delete support

    // Activity & Stats (optional but nice)
    pub login_count: i32,          // ADD: How many times have they logged in?
    pub events_created_count: i32, // ADD: Cache for performance
    pub microevents_created_count: i32, // ADD: Cache for performance
    pub favorite_events_count: i32, // ADD: Cache for performance
    pub favorite_microevents_count: i32, // ADD: Cache for performance
    pub saved_events_count: i32,   // ADD: Cache for performance
    pub saved_microevents_count: i32, // ADD: Cache for performance

    // Preferences (optional)
    pub timezone: Option<String>, // ADD: For displaying event times
    pub language: Option<String>, // ADD: For i18n
    pub notification_preferences: Option<String>, // ADD: JSON of settings
}

#[derive(Debug, Serialize)]
pub struct UserEventDataRow {
    //#[serde(flatten)]
    pub id: i64,
    pub user_id: String,                //uuid
    pub favorite_events: Vec<i64>,      // Event IDs
    pub favorite_microevents: Vec<i64>, // Microevent IDs
    pub saved_events: Vec<i64>,
    pub saved_microevents: Vec<i64>,
    pub created_events: Vec<i64>,
    pub created_microevents: Vec<i64>,
}

//####################################################################
//Analytics Model
//####################################################################
#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DailyAnalytics {
    pub date: String, // "2025-12-19"

    // User Metrics
    pub new_users: i32,
    pub active_users: i32, // Users who did ANYTHING today
    pub returning_users: i32,
    pub deleted_users: i32,
    pub total_users: i32, // Running total

    // Engagement Metrics
    pub total_logins: i32,
    pub total_sessions: i32,
    pub avg_session_duration_seconds: f64,

    // Content Creation
    pub events_created: i32,
    pub microevents_created: i32,
    pub events_deleted: i32,
    pub microevents_deleted: i32,

    // Interaction Metrics
    pub total_favorites: i32, // New favorites added
    pub total_unfavorites: i32,
    pub total_saves: i32,
    pub total_unsaves: i32,

    // Traffic
    pub page_views: i32,
    pub unique_visitors: i32,
    pub bounce_rate: f64, // Percentage

    // Performance
    pub avg_api_response_time_ms: f64,
    pub error_count: i32,
    pub error_rate: f64, // Percentage

    // Metadata
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct EventStats {
    pub event_id: String,
    pub title: String,
    pub creator_id: String,
    pub creator_name: String,

    pub views: i32,
    pub unique_viewers: i32,
    pub favorites_count: i32,
    pub saves_count: i32,
    pub microevents_count: i32,

    pub created_at: String,
    pub engagement_score: f64, // Custom metric: weighted sum of interactions
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct CreatorStats {
    pub user_id: String,
    pub user_name: String,

    pub events_created: i32,
    pub microevents_created: i32,
    pub total_favorites_received: i32,
    pub total_saves_received: i32,
    pub total_views: i32,

    pub avg_favorites_per_event: f64,
    pub creator_score: f64, // Custom metric
}
