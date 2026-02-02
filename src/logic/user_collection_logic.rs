// ============================================================================
// src/logic/user_collection_logic.rs - Business Logic Layer
// ============================================================================
use crate::context::EventContext;
use crate::context::MicroeventContext;
use crate::context::UserCollectionContext;
use crate::errors::AppError;
//use crate::repositories::EventRepository;
use crate::models::database_models::UserEventDataRow;
use crate::models::dto::EventResponse;
use crate::models::dto::UserCollection;
use crate::models::microevents_models::Microevent;
use crate::models::user::Claims;
pub struct UserCollectionLogic {
    repository: UserCollectionContext,
    events_context: EventContext,
    microevents_context: MicroeventContext,
}

impl UserCollectionLogic {
    pub fn new(
        repository: UserCollectionContext,
        events_context: EventContext,
        microevents_context: MicroeventContext,
    ) -> Self {
        Self {
            repository,
            events_context,
            microevents_context,
        }
    }

    pub async fn get(&self, user_id: &String) -> Result<UserEventDataRow, AppError> {
        //get list of favorites

        //if new key is not part of the list add it, if it is part of the list remove it

        //resave the list
        let output = self.repository.get(user_id.to_string()).await?;

        Ok(output)
    }

    pub async fn update(&self, input: UserCollection) -> Result<UserEventDataRow, AppError> {
        // Convert UserCollection to UserEventDataRow
        let row = UserEventDataRow {
            id: input
                .id
                .ok_or_else(|| AppError::ValidationError("id is required".to_string()))?,
            user_id: input
                .user_id
                .ok_or_else(|| AppError::ValidationError("user_id is required".to_string()))?,
            favorite_events: input.favorite_events.unwrap_or_default(),
            favorite_microevents: input.favorite_microevents.unwrap_or_default(),
            saved_events: input.saved_events.unwrap_or_default(),
            saved_microevents: input.saved_microevents.unwrap_or_default(),
            created_events: input.created_events.unwrap_or_default(),
            created_microevents: input.created_microevents.unwrap_or_default(),
        };

        self.repository.update(&row).await?;
        Ok(row)

        // Save to repository
        //let output = self.repository.update(&row).await?;

        //Ok(output)
    }

    pub async fn update_without_ownership(
        &self,
        input: UserCollection,
    ) -> Result<UserEventDataRow, AppError> {
        let ogrow = self
            .repository
            .get_by_id(
                input
                    .id
                    .ok_or_else(|| AppError::ValidationError("id is required".to_string()))?,
            )
            .await?;

        //check if the user ids match, if not. reject.
        if &ogrow.user_id
            != input
                .user_id
                .as_ref()
                .ok_or_else(|| AppError::ValidationError("user_id is required".to_string()))?
        {
            return Err(AppError::ValidationError(
                "user ids do not match ownership".to_string(),
            ));
        }

        // Convert UserCollection to UserEventDataRow
        let row = UserEventDataRow {
            id: input
                .id
                .ok_or_else(|| AppError::ValidationError("id is required".to_string()))?,
            user_id: input
                .user_id
                .ok_or_else(|| AppError::ValidationError("user_id is required".to_string()))?,
            favorite_events: input.favorite_events.unwrap_or_default(),
            favorite_microevents: input.favorite_microevents.unwrap_or_default(),
            saved_events: input.saved_events.unwrap_or_default(),
            saved_microevents: input.saved_microevents.unwrap_or_default(),
            created_events: ogrow.created_events,
            created_microevents: ogrow.created_microevents,
        };

        self.repository.update(&row).await?;
        Ok(row)

        // Save to repository
        //let output = self.repository.update(&row).await?;

        //Ok(output)
    }

    ///favorite toggle
    pub async fn event_favorite_toggle(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;

        if let Some(pos) = data.favorite_events.iter().position(|x| *x == id) {
            data.favorite_events.remove(pos);
        } else {
            data.favorite_events.push(id);
        }

        self.repository.update(&data).await?;

        // Already Vec<i64>, just return it
        Ok(data.favorite_events)
    }

    pub async fn microevent_favorite_toggle(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;

        if let Some(pos) = data.favorite_microevents.iter().position(|x| x == &id) {
            data.favorite_microevents.remove(pos);
        } else {
            data.favorite_microevents.push(id);
        }

        self.repository.update(&data).await?;

        Ok(data.favorite_microevents)
    }

    ///save toggle
    pub async fn event_save_toggle(&self, id: i64, user_id: &String) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;

        if let Some(pos) = data.saved_events.iter().position(|x| x == &id) {
            data.saved_events.remove(pos);
        } else {
            data.saved_events.push(id);
        }

        self.repository.update(&data).await?;

        Ok(data.saved_events)
    }

    pub async fn microevent_save_toggle(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;

        if let Some(pos) = data.saved_microevents.iter().position(|x| x == &id) {
            data.saved_microevents.remove(pos);
        } else {
            data.saved_microevents.push(id);
        }

        self.repository.update(&data).await?;

        Ok(data.saved_microevents)
    }

    ///Add ownership of events and microevents
    pub async fn event_ownership(&self, id: i64, user_id: &String) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;
        data.created_events.push(id);
        self.repository.update(&data).await?;
        Ok(data.created_events)
    }

    pub async fn microevent_ownership(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;
        data.created_microevents.push(id);
        self.repository.update(&data).await?;
        Ok(data.created_microevents)
    }

    pub async fn remove_event_ownership(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;

        if let Some(pos) = data.created_events.iter().position(|x| x == &id) {
            data.created_events.remove(pos);
        }
        self.repository.update(&data).await?;
        Ok(data.created_events)
    }

    pub async fn remove_microevent_ownership(
        &self,
        id: i64,
        user_id: &String,
    ) -> Result<Vec<i64>, AppError> {
        let mut data = self.repository.get(user_id.to_string()).await?;
        if let Some(pos) = data.created_microevents.iter().position(|x| x == &id) {
            data.created_microevents.remove(pos);
        }
        self.repository.update(&data).await?;
        Ok(data.created_microevents)
    }

    //gets

    pub async fn get_created_events(
        &self,
        user_id: &String,
    ) -> Result<Vec<EventResponse>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;

        //gather events
        let rows = self
            .events_context
            .get_by_id_list(preoutput.created_events)
            .await?;

        let output: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        //let output = self
        //.events_logic
        //.get_by_id_list(preoutput.created_microevents)
        //.await?;
        Ok(output)
    }

    pub async fn get_created_microevents(
        &self,
        user_id: &String,
    ) -> Result<Vec<Microevent>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;

        //gather microevents
        let output = self
            .microevents_context
            .get_by_id_list(preoutput.created_microevents)
            .await?;

        //let output = self
        //.microevents_logic
        //.get_by_id_list(preoutput.created_microevents)
        //.await?;

        Ok(output)
    }

    pub async fn get_favorite_events(
        &self,
        user_id: &String,
    ) -> Result<Vec<EventResponse>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;
        //gather events
        let rows = self
            .events_context
            .get_by_id_list(preoutput.favorite_events)
            .await?;

        let output: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();
        Ok(output)
    }

    pub async fn get_favorite_microevents(
        &self,
        user_id: &String,
    ) -> Result<Vec<Microevent>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;
        //gather microevents
        let output = self
            .microevents_context
            .get_by_id_list(preoutput.favorite_microevents)
            .await?;

        Ok(output)
    }

    pub async fn get_saved_events(&self, user_id: &String) -> Result<Vec<EventResponse>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;
        //gather events
        let rows = self
            .events_context
            .get_by_id_list(preoutput.saved_events)
            .await?;

        let output: Vec<EventResponse> = rows
            .into_iter()
            .filter_map(|row| EventResponse::from_row(row).ok())
            .collect();

        Ok(output)
    }

    pub async fn get_saved_microevents(
        &self,
        user_id: &String,
    ) -> Result<Vec<Microevent>, AppError> {
        let preoutput = self.repository.get(user_id.to_string()).await?;
        //gather microevents
        let output = self
            .microevents_context
            .get_by_id_list(preoutput.saved_microevents)
            .await?;

        Ok(output)
    }
}
