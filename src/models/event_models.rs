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
