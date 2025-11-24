///Nomadic Event, This is the base class/model/struct for the events
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NomEvent {
    pub name: String,
    pub description: String,
    pub event_type: EventType,    
    pub website: Option<String>,
    pub date_info: EventDate,
    pub location_info: Location,
    pub amenities: Amenities,
    pub camping_info: Option<CampingInfo>,
}
///Self explanitory
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventDate {
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub single_day: bool,
    pub early_arrival_available: bool,
    pub early_arrival_date: Option<String>,
    pub late_departure_available: bool,
}

///Is this a Ren Faire, a music festival, car show, or something new?
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventType {
    pub name: String,
    pub description: String,
    pub map_indicator: String,
}


///using the address or the long and lat to get an address so we can tell poeple what events are nearby
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Location {
    pub address: String,
    pub longitude: f64,
    pub latitude: f64,
    pub venue_name: Option<String>,
    pub parking_info: Option<String>,
}

///Rather comprehensive list of things to consider when camping
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CampingInfo {
    pub camping_allowed: bool,
    pub walking_distance: bool,
    pub tent_camping: bool,
    pub rv_camping: RvCampingOptions,
    pub vehicle_camping: VehicleCampingOptions,
    pub campsite_reservations_required: bool,
    pub primitive_camping: bool,
    pub developed_campsites: bool,
    pub max_stay_nights: Option<u32>,
    pub pet_friendly: bool,
    pub quiet_hours: Option<String>,
    pub fires_allowed : bool,    
    pub generator_options: Option<GeneratorOptions>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RvCampingOptions {
    pub allowed: bool,
    pub class_a_allowed: bool,
    pub class_b_allowed: bool,
    pub class_c_allowed: bool,
    pub travel_trailers_allowed: bool,
    pub fifth_wheel_allowed: bool,
    pub max_length_feet: Option<u32>,
    pub max_width_feet: Option<u32>,
    pub hookups_available: Option<Hookups>,
    pub dump_station: bool,    
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VehicleCampingOptions {
    pub van_camping: bool,
    pub car_camping: bool,
    pub truck_camping: bool,
    pub rooftop_tent_allowed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Hookups {
    pub electric: bool,
    pub water: bool,
    pub sewer: bool,
    pub amp_service: Option<String>, // "30/50 amp"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Amenities {
    pub bathrooms: bool,
    pub showers: bool,
    pub potable_water: bool,
    pub wifi: bool,
    pub cell_service_quality: Option<String>, // "good", "spotty", "none"
    pub firewood_available: bool,
    pub ice_available: bool,
    pub trash_service: bool,
    pub recycling: bool,
    pub laundry: bool,
}

///I don't think I am going to include pricing here. That should be something on the event sites
//pub struct CostInfo {
//    pub ticket_price: Option<f64>,
//    pub camping_fee: Option<f64>,
//    pub rv_fee: Option<f64>,
//    pub early_arrival_fee: Option<f64>,
//    pub vehicle_pass_fee: Option<f64>,
//    pub currency: String, // "USD", "CAD", etc.
//}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneratorOptions {
    pub generators_allowed: bool,
    pub quiet_hours: Option<GeneratorQuietHours>,
    pub max_decibel_limit: Option<u32>,
    pub inverter_generators_only: bool,
    pub propane_generators_allowed: bool,
    pub gasoline_generators_allowed: bool,
    pub diesel_generators_allowed: bool,
    pub designated_generator_areas: bool,
    pub distance_from_neighbors_feet: Option<u32>,
    pub fuel_storage_restrictions: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneratorQuietHours {
    pub all_day_restriction: bool, // Some events ban them entirely
    pub start_time: Option<String>, // "22:00" or "10:00 PM"
    pub end_time: Option<String>,   // "08:00" or "8:00 AM"
    pub days_of_week: Option<Vec<String>>, // ["Friday", "Saturday"] if different per day
}