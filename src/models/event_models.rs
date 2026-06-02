///Nomadic Event, This is the base class/model/struct for the events
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NomEvent {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub event_type_id: i64,
    pub website: Option<String>,
    #[serde(default)]
    pub date_info: EventDate,
    #[serde(default)]
    pub location_info: Location,
    pub amenities: Option<Amenities>,
    pub camping_info: Option<CampingInfo>,
    #[serde(default)]
    pub archive: bool,
    /// True if the event runs more than once on any cadence (weekly
    /// farmers market, monthly meetup, annual festival, etc.). Broader
    /// than `recurring_annual`. Defaults to false on legacy rows whose
    /// JSON predates this field — pair with the bulk DB backfill that
    /// set `recurring = true` for everything where `recurring_annual`
    /// is also true.
    #[serde(default)]
    pub recurring: bool,
    /// True if the event recurs once per year (most festivals fall
    /// here). Tighter than `recurring`: a weekly market is `recurring`
    /// but not `recurring_annual`. Defaults to false; existing JSON
    /// without the field deserializes cleanly via `#[serde(default)]`.
    #[serde(default)]
    pub recurring_annual: bool,
    /// Has a human confirmed that `date_info.start_date` and
    /// `end_date` are correct for the *current* instance of this
    /// recurring event? Auto-roll (see `EventLogic::roll_past_recurring_events`)
    /// flips this back to `false` every time it bumps the year, so the
    /// UI can surface an "unverified date — please confirm" badge.
    /// User-side `POST /event/{id}/verify-date` and any successful
    /// `PUT /event/{id}` that touches dates flip it to `true`.
    /// Defaults to false on legacy rows that predate this field.
    #[serde(default)]
    pub date_verified: bool,
    /// How often this event recurs, as a `{ unit, count }` cadence
    /// (e.g. `{ year, 1 }` annual, `{ year, 2 }` biennial,
    /// `{ month, 1 }` monthly). This is the source of truth the
    /// auto-roll advances by; when it's `None` the roll falls back to
    /// the legacy `recurring_annual` flag (treated as one year). `None`
    /// on legacy rows that predate the field — see
    /// `EventLogic::effective_interval` for the resolution order.
    #[serde(default)]
    pub recurrence_interval: Option<RecurrenceInterval>,
}

/// The unit half of a `RecurrenceInterval`. Serializes lowercase
/// (`"week"` / `"month"` / `"year"`) so the JSON stored in `event_data`
/// and surfaced to the frontend stays human-readable.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceUnit {
    Week,
    Month,
    #[default]
    Year,
}

/// How often a recurring event repeats, as a `{ unit, count }` pair.
/// A uniform shape (rather than a mixed-variant enum) keeps the stored
/// JSON predictable and trivial for the frontend to render and edit:
/// annual = `{ "unit": "year", "count": 1 }`, biennial =
/// `{ "unit": "year", "count": 2 }`, every-3-weeks =
/// `{ "unit": "week", "count": 3 }`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct RecurrenceInterval {
    pub unit: RecurrenceUnit,
    pub count: u32,
}

impl RecurrenceInterval {
    pub const fn new(unit: RecurrenceUnit, count: u32) -> Self {
        Self { unit, count }
    }

    /// The most common festival cadence — once per calendar year. Used
    /// as the auto-roll fallback for events flagged recurring without an
    /// explicit interval.
    pub const fn annual() -> Self {
        Self {
            unit: RecurrenceUnit::Year,
            count: 1,
        }
    }
}

fn deserialize_optional_date<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(date_str) => {
            // Try parsing as DateTime first
            if let Ok(dt) = DateTime::parse_from_rfc3339(&date_str) {
                return Ok(Some(dt.with_timezone(&Utc)));
            }

            // If that fails, try parsing as just a date (YYYY-MM-DD)
            if let Ok(naive_date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                let dt = Utc.from_utc_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap());
                return Ok(Some(dt));
            }

            Err(serde::de::Error::custom("Invalid date format"))
        }
        None => Ok(None),
    }
}

///Self explanitory
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EventDate {
    #[serde(deserialize_with = "deserialize_optional_date")]
    pub start_date: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "deserialize_optional_date")]
    pub end_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub single_day: bool,
    #[serde(default)]
    pub early_arrival_available: bool,
    pub early_arrival_date: Option<String>,
    #[serde(default)]
    pub late_departure_available: bool,
}

///Is this a Ren Faire, a music festival, car show, or something new?
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventType {
    pub id: Option<i64>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub map_indicator: String,
    #[serde(default)]
    pub category: String,
}

///using the address or the long and lat to get an address so we can tell people what events are nearby
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Location {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub latitude: f64,
    pub venue_name: Option<String>,
    pub parking_info: Option<String>,
}

///Rather comprehensive list of things to consider when camping
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CampingInfo {
    #[serde(default)]
    pub camping_allowed: bool,
    #[serde(default)]
    pub walking_distance: bool,
    #[serde(default)]
    pub tent_camping: bool,
    #[serde(default)]
    pub rv_camping: RvCampingOptions,
    #[serde(default)]
    pub vehicle_camping: VehicleCampingOptions,
    #[serde(default)]
    pub campsite_reservations_required: bool,
    #[serde(default)]
    pub primitive_camping: bool,
    #[serde(default)]
    pub developed_campsites: bool,
    pub max_stay_nights: Option<u32>,
    #[serde(default)]
    pub pet_friendly: bool,
    pub quiet_hours: Option<String>,
    #[serde(default)]
    pub fires_allowed: bool,
    pub generator_options: Option<GeneratorOptions>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Hookups {
    #[serde(default)]
    pub electric: bool,
    #[serde(default)]
    pub water: bool,
    #[serde(default)]
    pub sewer: bool,
    pub amp_service: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RvCampingOptions {
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub class_a_allowed: bool,
    #[serde(default)]
    pub class_b_allowed: bool,
    #[serde(default)]
    pub class_c_allowed: bool,
    #[serde(default)]
    pub travel_trailers_allowed: bool,
    #[serde(default)]
    pub fifth_wheel_allowed: bool,
    pub max_length_feet: Option<u32>,
    pub max_width_feet: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_hookups")]
    pub hookups_available: Option<Hookups>,
    #[serde(default)]
    pub dump_station: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct VehicleCampingOptions {
    #[serde(default)]
    pub van_camping: bool,
    #[serde(default)]
    pub car_camping: bool,
    #[serde(default)]
    pub truck_camping: bool,
    #[serde(default)]
    pub rooftop_tent_allowed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Amenities {
    #[serde(default)]
    pub bathrooms: bool,
    #[serde(default)]
    pub showers: bool,
    #[serde(default)]
    pub potable_water: bool,
    #[serde(default)]
    pub wifi: bool,
    pub cell_service_quality: Option<String>,
    #[serde(default)]
    pub firewood_available: bool,
    #[serde(default)]
    pub ice_available: bool,
    #[serde(default)]
    pub trash_service: bool,
    #[serde(default)]
    pub recycling: bool,
    #[serde(default)]
    pub laundry: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GeneratorOptions {
    #[serde(default)]
    pub generators_allowed: bool,
    pub quiet_hours: Option<GeneratorQuietHours>,
    pub max_decibel_limit: Option<u32>,
    #[serde(default)]
    pub inverter_generators_only: bool,
    #[serde(default)]
    pub propane_generators_allowed: bool,
    #[serde(default)]
    pub gasoline_generators_allowed: bool,
    #[serde(default)]
    pub diesel_generators_allowed: bool,
    #[serde(default)]
    pub designated_generator_areas: bool,
    pub distance_from_neighbors_feet: Option<u32>,
    pub fuel_storage_restrictions: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GeneratorQuietHours {
    #[serde(default)]
    pub all_day_restriction: bool, // Some events ban them entirely
    pub start_time: Option<String>,        // "22:00" or "10:00 PM"
    pub end_time: Option<String>,          // "08:00" or "8:00 AM"
    pub days_of_week: Option<Vec<String>>, // ["Friday", "Saturday"] if different per day
}

///Camping profiles to build out a standardized
/// This will be used to populate the options without specifically being referenced.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CampingProfile {
    pub id: Option<i64>,
    #[serde(default)]
    pub profile_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub camping_allowed: bool,
    #[serde(default)]
    pub walking_distance: bool,
    #[serde(default)]
    pub tent_camping: bool,
    #[serde(default)]
    pub rv_camping: RvCampingOptions,
    #[serde(default)]
    pub vehicle_camping: VehicleCampingOptions,
    #[serde(default)]
    pub campsite_reservations_required: bool,
    #[serde(default)]
    pub primitive_camping: bool,
    #[serde(default)]
    pub developed_campsites: bool,
    pub max_stay_nights: Option<u32>,
    #[serde(default)]
    pub pet_friendly: bool,
    pub quiet_hours: Option<String>,
    #[serde(default)]
    pub fires_allowed: bool,
    #[serde(default, deserialize_with = "deserialize_generator_options")]
    pub generator_options: Option<GeneratorOptions>,
}

// Helper to convert profile to camping info
impl CampingProfile {
    pub fn to_camping_info(&self) -> CampingInfo {
        CampingInfo {
            camping_allowed: self.camping_allowed,
            walking_distance: self.walking_distance,
            tent_camping: self.tent_camping,
            rv_camping: self.rv_camping.clone(),
            vehicle_camping: self.vehicle_camping.clone(),
            campsite_reservations_required: self.campsite_reservations_required,
            primitive_camping: self.primitive_camping,
            developed_campsites: self.developed_campsites,
            max_stay_nights: self.max_stay_nights,
            pet_friendly: self.pet_friendly,
            quiet_hours: self.quiet_hours.clone(),
            fires_allowed: self.fires_allowed,
            generator_options: self.generator_options.clone(),
        }
    }
}

///Fixing issues caused by the enrichment of the data by AI
// Then define the deserializer
fn deserialize_hookups<'de, D>(deserializer: D) -> Result<Option<Hookups>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize as DeserializeTrait;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HookupValue {
        Bool(bool),
        Object(Hookups),
    }

    match Option::<HookupValue>::deserialize(deserializer)? {
        None => Ok(None),
        Some(HookupValue::Bool(true)) => Ok(Some(Hookups {
            electric: true,
            water: true,
            sewer: true,
            amp_service: None,
        })),
        Some(HookupValue::Bool(false)) => Ok(None),
        Some(HookupValue::Object(hookups)) => Ok(Some(hookups)),
    }
}

// After GeneratorOptions and GeneratorQuietHours structs
fn deserialize_generator_options<'de, D>(
    deserializer: D,
) -> Result<Option<GeneratorOptions>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize as DeserializeTrait;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum GeneratorValue {
        Bool(bool),
        Object(GeneratorOptions),
    }

    match Option::<GeneratorValue>::deserialize(deserializer)? {
        None => Ok(None),
        Some(GeneratorValue::Bool(true)) => Ok(Some(GeneratorOptions {
            generators_allowed: true,
            quiet_hours: None,
            max_decibel_limit: None,
            inverter_generators_only: false,
            propane_generators_allowed: true,
            gasoline_generators_allowed: true,
            diesel_generators_allowed: true,
            designated_generator_areas: false,
            distance_from_neighbors_feet: None,
            fuel_storage_restrictions: None,
        })),
        Some(GeneratorValue::Bool(false)) => Ok(Some(GeneratorOptions {
            generators_allowed: false,
            ..Default::default()
        })),
        Some(GeneratorValue::Object(options)) => Ok(Some(options)),
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the recurrence-marker contract on `NomEvent`. The
    //! markers are written into `event_data` JSON in the DB; if the
    //! struct ever loses them they'll be silently dropped on the next
    //! API read/write round-trip. These tests catch that regression.
    //!
    //! Date / location / camping deserialization is covered separately.
    //! Here we only pin the new boolean fields and their `#[serde(default)]`
    //! behavior against legacy JSON.
    use super::*;

    /// Legacy JSON shape that predates the recurrence markers — same
    /// `event_data` text every pre-curation row in the DB carries. The
    /// `#[serde(default)]` on both fields must let this deserialize
    /// cleanly with both bools defaulted to `false`.
    const LEGACY_JSON: &str = r#"{
        "id": 1,
        "user_id": null,
        "name": "Legacy Event",
        "description": "No recurring markers in the JSON",
        "event_type_id": 1,
        "website": null,
        "date_info": {
            "start_date": null,
            "end_date": null,
            "single_day": false,
            "early_arrival_available": false,
            "early_arrival_date": null,
            "late_departure_available": false
        },
        "location_info": {
            "address": "Atlanta, GA",
            "longitude": -84.39,
            "latitude": 33.74,
            "venue_name": null,
            "parking_info": null
        },
        "amenities": null,
        "camping_info": null,
        "archive": false
    }"#;

    #[test]
    fn legacy_json_without_recurrence_markers_defaults_to_false() {
        let event: NomEvent =
            serde_json::from_str(LEGACY_JSON).expect("legacy JSON must deserialize");
        assert!(!event.recurring);
        assert!(!event.recurring_annual);
    }

    #[test]
    fn recurrence_markers_round_trip_through_json() {
        // Serialize an event with both markers on, deserialize it, and
        // confirm both flags survived. Catches a regression where the
        // serde derive forgot to include the field or accidentally
        // renamed the JSON key.
        let event = NomEvent {
            id: None,
            user_id: None,
            name: "Roundtripper".to_string(),
            description: "Tests serde".to_string(),
            event_type_id: 1,
            website: None,
            date_info: EventDate::default(),
            location_info: Location::default(),
            amenities: None,
            camping_info: None,
            archive: false,
            recurring: true,
            recurring_annual: true,
            // Default false so the round-trip pins the default-on-
            // serialize path. Verification-state coverage lives in
            // its own test below.
            date_verified: false,
            // Recurrence-interval coverage lives in its own test below;
            // keep this fixture on the legacy (None) path.
            recurrence_interval: None,
        };

        let raw = serde_json::to_string(&event).expect("serialize");
        // Pin the JSON key names — a future refactor that renamed
        // either field would change these strings, and the DB
        // backfill (which writes raw `recurring` and
        // `recurring_annual` keys via SQLite's `json_set`) would
        // silently stop matching.
        assert!(raw.contains("\"recurring\":true"));
        assert!(raw.contains("\"recurring_annual\":true"));

        let parsed: NomEvent = serde_json::from_str(&raw).expect("deserialize");
        assert!(parsed.recurring);
        assert!(parsed.recurring_annual);
    }

    #[test]
    fn recurring_independent_of_recurring_annual() {
        // A weekly farmers market is `recurring` but not
        // `recurring_annual`. Pin that the two flags decode independently
        // — a regression that collapsed them into one field would
        // surface here.
        let json = r#"{
            "id": 2,
            "name": "Saturday Market",
            "description": "Every Saturday",
            "event_type_id": 1,
            "date_info": { "start_date": null, "end_date": null,
                "single_day": false, "early_arrival_available": false,
                "early_arrival_date": null, "late_departure_available": false },
            "location_info": { "address": "Town Square",
                "longitude": 0.0, "latitude": 0.0,
                "venue_name": null, "parking_info": null },
            "archive": false,
            "recurring": true,
            "recurring_annual": false
        }"#;
        let event: NomEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.recurring);
        assert!(!event.recurring_annual);
    }

    #[test]
    fn recurrence_interval_json_shape() {
        // Pin the on-wire shape: a uniform { unit, count } object with a
        // lowercase unit. The frontend renders/edits this directly and a
        // DB backfill could write it via SQLite `json_set`, so the exact
        // key/value strings are a contract.
        let interval = RecurrenceInterval::new(RecurrenceUnit::Year, 2);
        let raw = serde_json::to_string(&interval).expect("serialize");
        assert_eq!(raw, r#"{"unit":"year","count":2}"#);

        let parsed: RecurrenceInterval =
            serde_json::from_str(r#"{"unit":"month","count":3}"#).expect("deserialize");
        assert_eq!(parsed, RecurrenceInterval::new(RecurrenceUnit::Month, 3));
    }

    #[test]
    fn legacy_json_without_recurrence_interval_decodes_to_none() {
        // The field is `#[serde(default)]`, so the same legacy `event_data`
        // text every pre-curation row carries must decode with
        // `recurrence_interval == None`.
        let event: NomEvent =
            serde_json::from_str(LEGACY_JSON).expect("legacy JSON must deserialize");
        assert!(event.recurrence_interval.is_none());
    }

    #[test]
    fn recurrence_interval_round_trips_on_event() {
        // Full NomEvent path: an event carrying an explicit interval must
        // round-trip through `event_data` JSON with the value intact.
        let json = r#"{
            "id": 3,
            "name": "Biennale",
            "description": "Every two years",
            "event_type_id": 1,
            "date_info": { "start_date": null, "end_date": null,
                "single_day": false, "early_arrival_available": false,
                "early_arrival_date": null, "late_departure_available": false },
            "location_info": { "address": "Venice",
                "longitude": 12.34, "latitude": 45.43,
                "venue_name": null, "parking_info": null },
            "archive": false,
            "recurring": true,
            "recurrence_interval": { "unit": "year", "count": 2 }
        }"#;
        let event: NomEvent = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            event.recurrence_interval,
            Some(RecurrenceInterval::new(RecurrenceUnit::Year, 2))
        );

        let raw = serde_json::to_string(&event).expect("serialize");
        let parsed: NomEvent = serde_json::from_str(&raw).expect("re-deserialize");
        assert_eq!(parsed.recurrence_interval, event.recurrence_interval);
    }
}
