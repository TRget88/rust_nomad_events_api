// ============================================================================
// src/logic/microevent_logic.rs - Business Logic Layer
// ============================================================================
use crate::errors::AppError;
//use crate::models::dto::MicroeventResponse;
use crate::context::MicroeventContext;
use crate::logic::{UserCollectionLogic, user_collection_logic};
use crate::models::microevents_models::Microevent;
use crate::models::user::Claims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;
pub struct MicroeventLogic {
    context: MicroeventContext,
    user_collection_logic: Arc<UserCollectionLogic>,
}

impl MicroeventLogic {
    pub fn new(
        context: MicroeventContext,
        user_collection_logic: Arc<UserCollectionLogic>,
    ) -> Self {
        Self {
            context,
            user_collection_logic,
        }
    }

    pub async fn get_all(&self) -> Result<Vec<Microevent>, AppError> {
        let rows = self.context.find_all().await?;

        let events: Vec<Microevent> = rows
            .into_iter()
            //.filter_map(|row| Microevent::from_row(row).ok())
            .collect();

        Ok(events)
    }

    pub async fn get(&self, id: i64) -> Result<Microevent, AppError> {
        let row = self.context.find_by_id(id).await?;
        //let event = Microevent::from_row(row)?;
        //Ok(event)
        Ok(row)
    }

    pub async fn get_by_id_list(&self, input: Vec<i64>) -> Result<Vec<Microevent>, AppError> {
        let row = self.context.get_by_id_list(input).await?;
        Ok(row)
    }

    pub async fn get_by_event(&self, id: i64) -> Result<Vec<Microevent>, AppError> {
        println!("About to make row request");
        let rows = self.context.find_by_event(id).await?;
        println!("number of rows found: {}", rows.iter().count());

        let events: Vec<Microevent> = rows
            .into_iter()
            //.filter_map(|row| Microevent::from_row(row).ok())
            .collect();

        //println!("events: {}", events.iter().map(|res| res.to_string())
        //.collect::<Vec<String>>()
        //.join(", "));

        Ok(events)
    }

    //the userid should have been added in the last round but it still needs to be added to the user collection
    pub async fn create(&self, event: Microevent) -> Result<i64, AppError> {
        self.validate_event(&event)?;

        //get the user id out of the model
        let user_id = &event.user_id;
        let id = self.context.create(&event).await?;
        self.user_collection_logic
            .microevent_ownership(id, user_id)
            .await?;

        Ok(id)
    }

    pub async fn update(&self, id: i64, event: Microevent, claims: Claims) -> Result<(), AppError> {
        // Business logic: validate event data
        self.validate_event(&event)?;

        // Check if user is admin or superadmin (bypass ownership check)
        let is_admin = claims.role == "admin" || claims.role == "super_admin";

        if !is_admin {
            // Check if the user is the owner (correct Id is listed in their usercollection)
            let collection = self.user_collection_logic.get(&claims.sub).await?;

            // Check if this id is part of the user's created microevents
            let is_owner = collection.created_microevents.contains(&id);

            if !is_owner {
                return Err(AppError::Unauthorized(
                    "You do not have permission to update this microevent".to_string(),
                ));
            }
        }

        let updated = self.context.update(id, &event).await?;

        if !updated {
            return Err(AppError::NotFound("Microevent not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        // Check if user is admin or superadmin (bypass ownership check)
        let is_admin = claims.role == "admin" || claims.role == "super_admin";
        if !is_admin {
            // Check if the user is the owner (correct Id is listed in their usercollection)
            let collection = self.user_collection_logic.get(&claims.sub).await?;

            // Check if this id is part of the user's created microevents
            let is_owner = collection.created_microevents.contains(&id);

            if !is_owner {
                return Err(AppError::Unauthorized(
                    "You do not have permission to update this microevent".to_string(),
                ));
            }
        }

        let deleted = self.context.delete(id).await?;

        if !deleted {
            return Err(AppError::NotFound("Event not found".to_string()));
        } else {
            //send this data to the usercollection
            self.user_collection_logic
                .remove_microevent_ownership(id, &claims.sub)
                .await?;
        }

        Ok(())
    }

    // Private business logic methods
    fn validate_event(&self, event: &Microevent) -> Result<(), AppError> {
        if event.name.trim().is_empty() {
            return Err(AppError::ValidationError(
                "Event name cannot be empty".to_string(),
            ));
        }
        // Validate dates
        //if let (Some(start), Some(end)) = (event.start_time, event.end_time) {
        //if end < start {
        //return Err(AppError::ValidationError(
        //"End date cannot be before start date".to_string(),
        //));
        //}
        //}
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
