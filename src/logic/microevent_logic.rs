// ============================================================================
// src/logic/microevent_logic.rs - Business Logic Layer
// ============================================================================
use crate::context::MicroeventContext;
use crate::errors::AppError;
use crate::logic::UserCollectionLogic;
use crate::models::microevents_models::Microevent;
use crate::models::user::{Claims, UserRole};
use std::sync::Arc;
pub struct MicroeventLogic {
    // `Arc<MicroeventContext>` mirrors the `EventContext` Arc pattern in
    // `EventLogic` — `UserCollectionLogic` also references the same context
    // instance for its microevent-lookup paths, so both share one
    // construction in `main.rs`.
    context: Arc<MicroeventContext>,
    user_collection_logic: Arc<UserCollectionLogic>,
}

impl MicroeventLogic {
    pub fn new(
        context: Arc<MicroeventContext>,
        user_collection_logic: Arc<UserCollectionLogic>,
    ) -> Self {
        Self {
            context,
            user_collection_logic,
        }
    }

    pub async fn get_all(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Microevent>, AppError> {
        let (l, o) = crate::util::validate_pagination(limit, offset)?;
        let rows = self.context.find_all(l, o).await?;

        Ok(rows.into_iter().collect())
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
        tracing::debug!(event_id = id, "About to fetch microevents by event");
        let rows = self.context.find_by_event(id).await?;
        tracing::debug!(count = rows.len(), "Microevents fetched");

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

        // Ownership gate: admins/superadmins bypass; everyone else must
        // own the row. Backed by the `microevents.user_id` column —
        // querying the column directly is uniform with how every other
        // table checks ownership and removes the JSON-blob round-trip
        // through `user_collection`.
        self.require_owner_or_admin(id, &claims).await?;

        let updated = self.context.update(id, &event).await?;

        if !updated {
            return Err(AppError::NotFound("Microevent not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        self.require_owner_or_admin(id, &claims).await?;

        let deleted = self.context.delete(id).await?;

        if !deleted {
            return Err(AppError::NotFound("Event not found".to_string()));
        } else {
            // Keep the user_collection JSON denormalization in lock-step
            // — the frontend reads it for the profile "Created" tab even
            // though we no longer use it for ownership decisions. Best-
            // effort: a write failure here doesn't undo the delete.
            self.user_collection_logic
                .remove_microevent_ownership(id, &claims.sub)
                .await?;
        }

        Ok(())
    }

    /// Reject the request unless `claims.sub` owns microevent `id` or holds
    /// an Admin/SuperAdmin role. Ownership is determined by `microevents.user_id`
    /// — single-column compare against the JWT subject. If the microevent
    /// doesn't exist, surface `NotFound` rather than `Unauthorized` so the
    /// 404 path is the same whether the caller is a logged-in stranger or
    /// the rightful owner of a deleted row.
    async fn require_owner_or_admin(&self, id: i64, claims: &Claims) -> Result<(), AppError> {
        if matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin) {
            return Ok(());
        }

        let row = self.context.find_by_id(id).await?;
        if row.user_id != claims.sub {
            return Err(AppError::Unauthorized(
                "You do not have permission to modify this microevent".to_string(),
            ));
        }
        Ok(())
    }

    // Private business logic methods
    fn validate_event(&self, event: &Microevent) -> Result<(), AppError> {
        validate_microevent(event)
    }
}

/// Free-function validator, separated from `MicroeventLogic` so it can be
/// unit-tested without instantiating a context. Mirrors the shape of
/// `event_logic::validate_event` — same field-length bounds, same
/// reject-on-ordering rule for start/end.
pub(crate) fn validate_microevent(event: &Microevent) -> Result<(), AppError> {
    // Field bounds — match the API-level event validator so a microevent
    // can't push a megabyte payload through the bind protocol when the
    // parent event can't. The number choice is the same as `validate_event`
    // (200 / 5000) so a future change is one-line in both places.
    const MAX_NAME_LEN: usize = 200;
    const MAX_DESC_LEN: usize = 5000;

    if event.name.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Microevent name cannot be empty".to_string(),
        ));
    }
    if event.name.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Microevent name must be {} characters or fewer",
            MAX_NAME_LEN
        )));
    }

    if let Some(desc) = event.description.as_ref()
        && desc.len() > MAX_DESC_LEN
    {
        return Err(AppError::ValidationError(format!(
            "Microevent description must be {} characters or fewer",
            MAX_DESC_LEN
        )));
    }

    // start_time / end_time are both Optional. Only enforce ordering when
    // both are provided — a microevent with only a start ("show begins at
    // 9pm, runs as long as the crowd stays") is a legitimate shape.
    if let (Some(start), Some(end)) = (event.start_time, event.end_time)
        && end < start
    {
        return Err(AppError::ValidationError(
            "End time cannot be before start time".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_microevent(name: &str) -> Microevent {
        Microevent {
            id: 0,
            event_id: 1,
            user_id: "owner".to_string(),
            name: name.to_string(),
            archive: false,
            description: None,
            start_time: None,
            end_time: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn rejects_empty_name() {
        let err = validate_microevent(&make_microevent("")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_whitespace_only_name() {
        let err = validate_microevent(&make_microevent("   ")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_oversize_name() {
        let big = "x".repeat(500);
        let err = validate_microevent(&make_microevent(&big)).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_oversize_description() {
        let mut event = make_microevent("Jousting");
        event.description = Some("d".repeat(6000));
        let err = validate_microevent(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_end_before_start() {
        let mut event = make_microevent("Jousting");
        event.start_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 18, 0, 0).unwrap());
        event.end_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 17, 0, 0).unwrap());
        let err = validate_microevent(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn accepts_start_only() {
        // Open-ended microevent — only a start time, no end. Realistic
        // shape for "show begins at X" listings.
        let mut event = make_microevent("Open Mic");
        event.start_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 21, 0, 0).unwrap());
        event.end_time = None;
        assert!(validate_microevent(&event).is_ok());
    }

    #[test]
    fn accepts_end_only() {
        // Documents the lack-of-ordering check when only end is provided.
        // Unusual but not invalid — could mean "ends by Y, start TBA".
        let mut event = make_microevent("Open Mic");
        event.start_time = None;
        event.end_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 23, 0, 0).unwrap());
        assert!(validate_microevent(&event).is_ok());
    }

    #[test]
    fn accepts_equal_start_end() {
        // A zero-duration microevent (a single moment) is allowed — `end < start`
        // is rejected, not `end <= start`. Pin the boundary.
        let mut event = make_microevent("Pyrotechnic Cue");
        let t = Utc.with_ymd_and_hms(2026, 6, 15, 22, 0, 0).unwrap();
        event.start_time = Some(t);
        event.end_time = Some(t);
        assert!(validate_microevent(&event).is_ok());
    }

    #[test]
    fn accepts_valid_microevent() {
        let mut event = make_microevent("Jousting");
        event.description = Some("Knights tilt at 2pm and 5pm.".to_string());
        event.start_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 14, 0, 0).unwrap());
        event.end_time = Some(Utc.with_ymd_and_hms(2026, 6, 15, 15, 30, 0).unwrap());
        assert!(validate_microevent(&event).is_ok());
    }
}
