// ============================================================================
// src/services/event_service.rs - Business Logic Layer
// ============================================================================
use crate::errors::AppError;
use crate::models::dto::EventResponse;
use crate::models::event_models::{EventType, NomEvent};
use crate::repositories::EventRepository;
use serde_json::json;

pub struct EventService {
    repository: EventRepository,
}

impl EventService {
    pub fn new(repository: EventRepository) -> Self {
        Self { repository }
    }

    pub async fn get_all_events(&self) -> Result<Vec<EventResponse>, AppError> {
        let rows = self.repository.find_all().await?;

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        Ok(events)
    }

    pub async fn get_event_by_id(&self, id: i64) -> Result<EventResponse, AppError> {
        let row = self.repository.find_by_id(id).await?;
        let event = EventResponse::from_row(row)?;
        Ok(event)
    }

    pub async fn get_events_by_type(
        &self,
        event_type_id: i64,
    ) -> Result<Vec<EventResponse>, AppError> {
        let rows = self.repository.find_by_type(event_type_id).await?;

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        Ok(events)
    }

    pub async fn get_nearby_events(
        &self,
        lat: f64,
        lon: f64,
        radius_miles: f64,
    ) -> Result<Vec<EventResponse>, AppError> {
        if radius_miles <= 0.0 || radius_miles > 500.0 {
            return Err(AppError::ValidationError(
                "Radius must be between 0 and 500 miles".to_string(),
            ));
        }
        println!("About to make row request");
        let rows = self.repository.find_nearby(lat, lon, radius_miles).await?;
        println!("number of rows found: {}", rows.iter().count());

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        //println!("events: {}", events.iter().map(|res| res.to_string())
        //.collect::<Vec<String>>()
        //.join(", "));


        Ok(events)
    }
    //pub async fn get_all_events(&self) -> Result<Vec<NomEvent>, AppError> {
    //let rows = self.repository.find_all().await?;
    //
    //let events: Vec<NomEvent> = rows
    //.into_iter()
    //.filter_map(|row| serde_json::from_str(&row.event_data).ok())
    //.collect();
    //
    //Ok(events)
    //}

    //pub async fn get_event_by_id(&self, id: i64) -> Result<NomEvent, AppError> {
    //let row = self.repository.find_by_id(id).await?;
    //let event: NomEvent = serde_json::from_str(&row.event_data)?;
    //Ok(event)
    //}
    //
    //pub async fn get_events_by_type(&self, event_type: i64) -> Result<Vec<NomEvent>, AppError> {
    //let rows = self.repository.find_by_type(event_type).await?;
    //
    //let events: Vec<NomEvent> = rows
    //.into_iter()
    //.filter_map(|row| serde_json::from_str(&row.event_data).ok())
    //.collect();
    //
    //Ok(events)
    //}
    //
    //pub async fn get_nearby_events(
    //&self,
    //lat: f64,
    //lon: f64,
    //radius_miles: f64,
    //) -> Result<Vec<NomEvent>, AppError> {
    //// Business logic: validate radius
    //if radius_miles <= 0.0 || radius_miles > 500.0 {
    //return Err(AppError::ValidationError(
    //"Radius must be between 0 and 500 miles".to_string(),
    //));
    //}
    //
    //let rows = self.repository.find_nearby(lat, lon, radius_miles).await?;
    //
    //let events: Vec<NomEvent> = rows
    //.into_iter()
    //.filter_map(|row| serde_json::from_str(&row.event_data).ok())
    //.collect();
    //
    //Ok(events)
    //}

    pub async fn create_event(&self, event: NomEvent) -> Result<i64, AppError> {
        // Business logic: validate event data
        self.validate_event(&event)?;

        let id = self.repository.create(&event).await?;
        Ok(id)
    }

    pub async fn update_event(&self, id: i64, event: NomEvent) -> Result<(), AppError> {
        // Business logic: validate event data
        self.validate_event(&event)?;

        let updated = self.repository.update(id, &event).await?;

        if !updated {
            return Err(AppError::NotFound("Event not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete_event(&self, id: i64) -> Result<(), AppError> {
        let deleted = self.repository.delete(id).await?;

        if !deleted {
            return Err(AppError::NotFound("Event not found".to_string()));
        }

        Ok(())
    }

    // Private business logic methods
    fn validate_event(&self, event: &NomEvent) -> Result<(), AppError> {
        if event.name.trim().is_empty() {
            return Err(AppError::ValidationError(
                "Event name cannot be empty".to_string(),
            ));
        }

        if event.description.trim().is_empty() {
            return Err(AppError::ValidationError(
                "Event description cannot be empty".to_string(),
            ));
        }

        // Validate coordinates
        if event.location_info.latitude < -90.0 || event.location_info.latitude > 90.0 {
            return Err(AppError::ValidationError("Invalid latitude".to_string()));
        }

        if event.location_info.longitude < -180.0 || event.location_info.longitude > 180.0 {
            return Err(AppError::ValidationError("Invalid longitude".to_string()));
        }

        // Validate dates
        if let (Some(start), Some(end)) = (event.date_info.start_date, event.date_info.end_date) {
            if end < start {
                return Err(AppError::ValidationError(
                    "End date cannot be before start date".to_string(),
                ));
            }
        }
        //if (event.date_info.start_date != Option::None) {
        //if let Some(end_date) = event.date_info.end_date {
        //if end_date < event.date_info.start_date.expect("REASON") {
        //return Err(AppError::ValidationError(
        //"End date cannot be before start date".to_string(),
        //));
        //}
        //}
        //}

        Ok(())
    }
}
