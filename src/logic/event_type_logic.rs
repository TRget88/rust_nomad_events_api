// ============================================================================
// src/logic/event_service.rs - Business Logic Layer
// ============================================================================
use crate::context::EventTypeContext;
use crate::errors::AppError;
use crate::models::event_models::{EventType, NomEvent};
//use crate::repositories::EventRepository;
use serde_json::json;

pub struct EventTypeLogic {
    repository: EventTypeContext,
}

impl EventTypeLogic {
    pub fn new(repository: EventTypeContext) -> Self {
        Self { repository }
    }

    pub async fn get_all(&self) -> Result<Vec<EventType>, AppError> {
        let rows = self.repository.find_all().await?;

        let events: Vec<EventType> = rows
            .into_iter()
            //.filter_map(|row| EventType::from_row(row).ok())
            .collect();

        Ok(events)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<EventType, AppError> {
        let row = self.repository.find_by_id(id).await?;
        //let event = EventType::from_row(row)?;
        //Ok(event)
        Ok(row)
    }

    pub async fn create(&self, event: EventType) -> Result<i64, AppError> {
        let id = self.repository.create(&event).await?;
        Ok(id)
    }

    pub async fn update(&self, id: i64, event: EventType) -> Result<(), AppError> {
        let updated = self.repository.update(id, &event).await?;

        if !updated {
            return Err(AppError::NotFound("Event not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<(), AppError> {
        let deleted = self.repository.delete(id).await?;

        if !deleted {
            return Err(AppError::NotFound("Event not found".to_string()));
        }

        Ok(())
    }
}
