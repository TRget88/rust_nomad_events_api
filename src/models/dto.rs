// ============================================================================
// src/models/dto.rs - Data Transfer Objects
// ============================================================================
use serde::{Deserialize, Serialize};
//use crate::models::event_models::CampingInfo;
use crate::models::database_models::EventRow;
use crate::models::{event_models::*, microevents_models::Microevent};
use chrono::{DateTime, Utc};

/// Sort order for `/event/search` results. The value travels through the
/// API as the lowercase strings `"name"`, `"date"`, `"distance"` — kept as
/// constants on this enum so callers don't pass raw strings into the
/// context layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventSortOrder {
    #[default]
    Name,
    Date,
    Distance,
}

// `CreateEventDto` / `UpdateEventDto` / `EventResponseDto` are early
// sketches of slimmed request/response shapes for the event admin
// routes. The live admin endpoints currently accept full `NomEvent`
// payloads (see `routes/events.rs`); these DTOs stay parked until the
// admin UI calls for a smaller surface. `#[allow(dead_code)]` keeps
// them documented without producing warnings.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEventDto {
    pub name: String,
    pub description: String,
    pub event_type: Option<i64>,
    pub website: Option<String>,
    // Add other required fields
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateEventDto {
    pub name: Option<String>,
    pub description: Option<String>,
    pub event_type: Option<String>,
    pub website: Option<String>,
    // Add other fields that can be updated
}

#[allow(dead_code)]
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
    /// Multi-select type filter. Comma-separated event_type ids, e.g.
    /// `?event_type_ids=1,2,3`. Lets the frontend send "Festivals + Concerts"
    /// in a single request. When both `event_type` (single) and
    /// `event_type_ids` (multi) are present, the union is used. Backend
    /// validates and dedupes; clients send raw user input.
    pub event_type_ids: Option<String>,
    pub camping_allowed: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub radius_miles: Option<f64>,
    /// Inclusive lower bound on the event's date range, `YYYY-MM-DD`.
    /// Matches events whose end_date (or start_date, if end is null) is
    /// on or after this day — i.e. festivals running through this window.
    pub date_from: Option<String>,
    /// Inclusive upper bound on the event's start_date, `YYYY-MM-DD`.
    /// Combined with `date_from` this gives interval overlap: an event
    /// running Fri–Sun matches a search for the Saturday it spans.
    pub date_to: Option<String>,
    /// Case-insensitive substring match against `events.name` and
    /// `events.description`. User input is escaped before composing the
    /// `LIKE` pattern so `%`/`_`/`\` are treated literally.
    pub name_contains: Option<String>,
    /// Ordering for the result set. One of `"name"` (default),
    /// `"date"` (soonest start_date first; null dates last), or
    /// `"distance"` (nearest to the search lat/lon first). Anything else
    /// is rejected with 400 in `validate_sort_param`.
    pub sort: Option<String>,
    /// Page size cap. Default 200, max 500. Anything outside `[1, 500]`
    /// is rejected with 400 in `validate_pagination`. Bounds the server's
    /// response size — without this, a wide-radius search at scale would
    /// return arbitrary amounts of data per request.
    pub limit: Option<i64>,
    /// Zero-indexed row offset for paging. Default 0. Combined with
    /// `limit` to implement `LIMIT ? OFFSET ?` pagination. v1 is offset-
    /// based for simplicity; cursor-based pagination (more robust under
    /// concurrent inserts) is a future upgrade.
    pub offset: Option<i64>,
}

// Parked: a future create endpoint planned to accept a camping-profile id
// + an optional CampingInfo override. The current create path accepts the
// full event payload — see `routes::events::create_event` — so this DTO
// is not yet wired.
#[allow(dead_code)]
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

// Parked: a slim camping-profile list shape. Today `routes::camping_profiles::get_all`
// returns the full `CampingProfile` rows; this DTO is staged for an "/admin"-side
// list view where description and id are all the UI needs.
#[allow(dead_code)]
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
    /// Mirrors `NomEvent.recurring` — surfaces the "happens more than
    /// once" flag to API consumers without forcing the frontend to
    /// parse `event_data` itself.
    #[serde(default)]
    pub recurring: bool,
    /// Mirrors `NomEvent.recurring_annual` — yearly cadence
    /// specifically.
    #[serde(default)]
    pub recurring_annual: bool,
    /// Mirrors `NomEvent.date_verified` — true when a human has
    /// confirmed the current-instance dates are correct. The frontend
    /// surfaces an "unverified date" badge when this is false.
    #[serde(default)]
    pub date_verified: bool,
    /// Mirrors `NomEvent.archive` — true when an admin has archived
    /// the event (or the auto-archive sweep retired a past one-time
    /// event). Listing endpoints filter these out at the DB layer,
    /// so a client receiving an archived event got it via direct
    /// lookup (find_by_id / get_by_id_list) — typically from a saved
    /// link. The frontend uses this to render an admin toggle and a
    /// "this event has been archived" notice.
    #[serde(default)]
    pub archive: bool,
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
            recurring: event.recurring,
            recurring_annual: event.recurring_annual,
            date_verified: event.date_verified,
            archive: event.archive,
            //is_favorite,
            //is_saved,
        })
    }

    // Alternative constructor when user info is not available (e.g., unauthenticated requests)
    //pub fn from_row_no_user(row: EventRow) -> Result<Self, serde_json::Error> {
    //Self::from_row(row, &[], &[])
    //}
}

// Parked: pre-DTO sketch of the "incoming event with nested EventType"
// shape. The current create/update routes use `NomEvent` directly.
#[allow(dead_code)]
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

// Parked: prepared shape for a `MicroeventResponse` that mirrors the
// favorite/saved fields on `EventResponse`. Today `routes::microevents`
// returns `Microevent` directly; this DTO + `from_row` come back online
// when the favorites surface extends to microevents.
#[allow(dead_code)]
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

// Helper to convert Microevent to MicroeventResponse — parked alongside
// the parent DTO until callers exist.
#[allow(dead_code)]
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

// Parked: a slimmer create/update shape paired with the staged
// `MicroeventResponse`. Currently the routes accept `Microevent` directly.
#[allow(dead_code)]
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
/// Helper to convert EventRow to EventResponse
//impl MicroeventResponse {
//pub fn from_row(row: crate::context::microevent_context::MicroeventRow) -> Result<Self, serde_json::Error> {
//
/// Helper function to parse datetime
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
/// Shared query DTO for paginated list endpoints (admin user list, admin
/// audit log, etc.). Endpoints that *also* have other query filters
/// (`/event/search`) carry their own struct with the same `limit`/`offset`
/// shape — the duplication is intentional so each endpoint's query type
/// documents its full surface in one place.
///
/// Defaults and validation live in `util::validate_pagination`.
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

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
    /// Events explicitly added to the user's calendar/schedule via the
    /// "Add to My Schedule" button on event detail. Distinct from
    /// `saved_events` (which is a bookmark/library list) — a user can
    /// save without scheduling and vice versa. Optional on the wire so
    /// the frontend can omit it on legacy reads/writes; backend
    /// defaults to `'[]'` per the migration.
    #[serde(default)]
    pub scheduled_events: Option<Vec<i64>>,
    /// Microevents explicitly added to the user's schedule via the
    /// schedule toggle on the microevent action buttons. Sister of
    /// `scheduled_events` for microevents — a user can save/favorite a
    /// microevent without committing it to their calendar and vice
    /// versa. Optional on the wire so the frontend can omit it on
    /// legacy reads/writes; backend defaults to `'[]'` per migration
    /// 00006.
    #[serde(default)]
    pub scheduled_microevents: Option<Vec<i64>>,
}
