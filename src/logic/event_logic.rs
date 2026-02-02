// ============================================================================
// src/logic/event_logic.rs - Business Logic Layer
// ============================================================================
use crate::context::EventContext;
use crate::errors::AppError;
use crate::logic::UserCollectionLogic;
use crate::models::dto::EventResponse;
use crate::models::event_models::NomEvent;
//use crate::repositories::EventRepository;
use crate::logic::user_collection_logic;
use crate::models::user::Claims;
use serde_json::json;
use std::sync::Arc;
pub struct EventLogic {
    repository: EventContext,
    user_collection_logic: Arc<UserCollectionLogic>, //userCollectionLogic: UserCollectionLogic,
}

impl EventLogic {
    pub fn new(repository: EventContext, user_collection_logic: Arc<UserCollectionLogic>) -> Self {
        Self {
            repository,
            user_collection_logic,
        }
    }

    //impl EventLogic {
    //pub fn new(repository: EventContext, userCollectionLogic: UserCollectionLogic) -> Self {
    //Self {
    //repository,
    //userCollectionLogic,
    //}
    //}

    pub async fn get_all_events(&self) -> Result<Vec<EventResponse>, AppError> {
        let rows = self.repository.find_all().await?;

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        //println!("{:#?}", (events.clone()));

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
        ////get user favorites
        //let user_favorite_id_list = self.userCollectionLogic.get_user_favorited_events().await?;
        //
        ////get user saved
        //let user_saved_id_list = self.userCollectionLogic.get_user_saved_events().await?;

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
        //println!("About to make row request");
        let rows = self.repository.find_nearby(lat, lon, radius_miles).await?;
        //println!("number of rows found: {}", rows.iter().count());
        //println!("{:#?}", (rows.clone()));

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        //println!("events: {}", events.iter().map(|res| res.to_string())
        //.collect::<Vec<String>>()
        //.join(", "));

        Ok(events)
    }

    pub async fn get_by_id_list(&self, input: Vec<i64>) -> Result<Vec<EventResponse>, AppError> {
        let rows = self.repository.get_by_id_list(input).await?;

        let events: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();
        Ok(events)
    }

    pub async fn create_event(&self, event: NomEvent) -> Result<i64, AppError> {
        // Business logic: validate event data
        self.validate_event(&event)?;

        //get the user id out of the model
        let user_id = &event
            .user_id
            .as_ref()
            .ok_or_else(|| AppError::ValidationError("user_id is required".to_string()))?;

        let id = self.repository.create(&event).await?;

        //send this data to the usercollection
        self.user_collection_logic
            .event_ownership(id, user_id)
            .await?;

        Ok(id)
    }

    pub async fn update_event(
        &self,
        id: i64,
        event: NomEvent,
        claims: Claims,
    ) -> Result<(), AppError> {
        // Business logic: validate event data
        self.validate_event(&event)?;

        // Check if user is admin or superadmin (bypass ownership check)
        let is_admin = claims.role == "admin" || claims.role == "super_admin";
        if !is_admin {
            // Check if the user is the owner (correct Id is listed in their usercollection)
            let collection = self.user_collection_logic.get(&claims.sub).await?;

            // Check if this id is part of the user's created microevents
            let is_owner = collection.created_events.contains(&id);

            if !is_owner {
                return Err(AppError::Unauthorized(
                    "You do not have permission to update this microevent".to_string(),
                ));
            }
        }
        let updated = self.repository.update(id, &event).await?;

        if !updated {
            return Err(AppError::NotFound("Event not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete_event(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        // Check if user is admin or superadmin (bypass ownership check)
        let is_admin = claims.role == "admin" || claims.role == "super_admin";
        if !is_admin {
            // Check if the user is the owner (correct Id is listed in their usercollection)
            let collection = self.user_collection_logic.get(&claims.sub).await?;

            // Check if this id is part of the user's created microevents
            let is_owner = collection.created_events.contains(&id);

            if !is_owner {
                return Err(AppError::Unauthorized(
                    "You do not have permission to update this microevent".to_string(),
                ));
            }
        }

        let deleted = self.repository.delete(id).await?;
        if !deleted {
            return Err(AppError::NotFound("Event not found".to_string()));
        } else {
            //send this data to the usercollection
            self.user_collection_logic
                .remove_event_ownership(id, &claims.sub)
                .await?;
        }

        Ok(())
    }

    ////adding the favorite and saved sections
    //pub async fn save_toggle(&self, id: i64, user_id: String) -> Result<(), AppError> {
    //let updated = self.repository.update(id, &event).await?;
    //
    //if !updated {
    //return Err(AppError::NotFound("Event not found".to_string()));
    //}
    //
    //Ok(())
    //}
    //
    //pub async fn favorite_toggle(&self, id: i64, user_id: String) -> Result<(), AppError> {
    //let updated = self.repository.update(id, &event).await?;
    //
    //if !updated {
    //return Err(AppError::NotFound("Event not found".to_string()));
    //}
    //
    //Ok(())
    //}

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

        Ok(())
    }
}
