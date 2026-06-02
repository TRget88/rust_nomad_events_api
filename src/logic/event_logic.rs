// ============================================================================
// src/logic/event_logic.rs - Business Logic Layer
// ============================================================================
use crate::context::EventContext;
use crate::errors::AppError;
use crate::logic::UserCollectionLogic;
use crate::models::database_models::EventRow;
use crate::models::dto::{EventResponse, EventSortOrder};
use crate::models::event_models::{NomEvent, RecurrenceInterval, RecurrenceUnit};
use crate::models::user::{Claims, UserRole};
use crate::util::validate_pagination;
use chrono::{DateTime, Duration, Months, Utc};
use std::sync::Arc;

pub struct EventLogic {
    // `Arc<EventContext>` rather than an owned context: the same context
    // instance is also referenced from `UserCollectionLogic` (for the
    // user-event-data lookups it does). Pre-refactor `main.rs` constructed
    // two `EventContext`s because the type wasn't Clone; now both
    // consumers share one and `clone()` is a refcount bump.
    repository: Arc<EventContext>,
    user_collection_logic: Arc<UserCollectionLogic>,
}

impl EventLogic {
    pub fn new(
        repository: Arc<EventContext>,
        user_collection_logic: Arc<UserCollectionLogic>,
    ) -> Self {
        Self {
            repository,
            user_collection_logic,
        }
    }

    pub async fn get_all_events(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<EventResponse>, AppError> {
        let (l, o) = crate::util::validate_pagination(limit, offset)?;
        let rows = self.repository.find_all(l, o).await?;

        let events: Vec<EventResponse> =
            rows.into_iter().filter_map(event_response_or_log).collect();

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

        let events: Vec<EventResponse> =
            rows.into_iter().filter_map(event_response_or_log).collect();

        Ok(events)
    }

    // 13 args — same shape as `EventQueryParams`. A struct-wrap would just
    // deconstruct one layer up. Suppress at the function.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_nearby_events(
        &self,
        lat: f64,
        lon: f64,
        radius_miles: f64,
        event_type_id: Option<i64>,
        event_type_ids: Option<String>,
        date_from: Option<String>,
        date_to: Option<String>,
        name_contains: Option<String>,
        camping_allowed: Option<bool>,
        sort: Option<String>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<EventResponse>, AppError> {
        if radius_miles <= 0.0 || radius_miles > 500.0 {
            return Err(AppError::ValidationError(
                "Radius must be between 0 and 500 miles".to_string(),
            ));
        }
        if !lat.is_finite() || !lon.is_finite() {
            return Err(AppError::ValidationError(
                "Latitude and longitude must be finite".to_string(),
            ));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(AppError::ValidationError(
                "Latitude must be between -90 and 90".to_string(),
            ));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(AppError::ValidationError(
                "Longitude must be between -180 and 180".to_string(),
            ));
        }
        // Near the poles, the bounding-box approach in find_nearby uses
        // cos(lat) in the denominator and blows up. Reject near-polar inputs
        // explicitly rather than returning meaningless results.
        if lat.abs() >= 89.0 {
            return Err(AppError::ValidationError(
                "Near-polar searches are not supported".to_string(),
            ));
        }

        // Validate date params strictly as YYYY-MM-DD before they hit the SQL
        // — lexicographic comparison silently returns wrong results when the
        // format is off (e.g. "06/12/2026" sorts before any ISO date).
        let validated_from = match date_from.as_deref() {
            Some(s) => Some(validate_iso_date(s, "date_from")?),
            None => None,
        };
        let validated_to = match date_to.as_deref() {
            Some(s) => Some(validate_iso_date(s, "date_to")?),
            None => None,
        };
        if let (Some(from), Some(to)) = (validated_from.as_deref(), validated_to.as_deref())
            && from > to
        {
            return Err(AppError::ValidationError(
                "date_from must be on or before date_to".to_string(),
            ));
        }

        // Free-text query: trim, drop if empty after trimming, cap length so
        // the LIKE pattern can't be used to push huge strings through the
        // bind protocol. The cap is loose — 100 chars is well past any
        // legitimate event-name fragment.
        let validated_name = validate_name_contains(name_contains.as_deref())?;

        // Sort param: defaults to Name (the historical behavior). Anything
        // outside the closed set of legal values is rejected with 400
        // rather than silently falling back to the default — that way a
        // typo'd `?sort=naem` is loud, not invisibly wrong.
        let sort_order = validate_sort_param(sort.as_deref())?;

        // Merge the single `event_type` + multi `event_type_ids` params into
        // one deduplicated, length-capped Vec. Single-id callers (`?event_type=5`)
        // and multi-id callers (`?event_type_ids=1,2,3`) both work; if both
        // are present they're unioned.
        let validated_type_ids = validate_event_type_ids(event_type_id, event_type_ids.as_deref())?;

        // Pagination. Defaults: limit=200 (wide enough that current frontend
        // behavior is preserved at realistic event densities), offset=0.
        // Caps at limit=500. Reject explicit out-of-bounds inputs loudly
        // instead of silently clamping — caller passing limit=-1 likely
        // has a bug they want surfaced.
        let (validated_limit, validated_offset) = validate_pagination(limit, offset)?;

        // camping_allowed needs no validation — it's already a bool by
        // the time it arrives (axum's serde-deserialized Option<bool>).
        // We just pass it through. Future "camping_allowed=any" semantics
        // would need a tri-state but the current Option<bool> covers
        // {required-true, required-false, absent=any} cleanly.
        let rows = self
            .repository
            .find_nearby(
                lat,
                lon,
                radius_miles,
                validated_type_ids,
                validated_from,
                validated_to,
                validated_name,
                camping_allowed,
                sort_order,
                validated_limit,
                validated_offset,
            )
            .await?;

        let events: Vec<EventResponse> =
            rows.into_iter().filter_map(event_response_or_log).collect();

        Ok(events)
    }

    pub async fn get_by_id_list(&self, input: Vec<i64>) -> Result<Vec<EventResponse>, AppError> {
        let rows = self.repository.get_by_id_list(input).await?;

        let events: Vec<EventResponse> =
            rows.into_iter().filter_map(event_response_or_log).collect();
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
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
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
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
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

    /// Mark `date_verified = true` on an event. Gated to event owners
    /// and Admin/SuperAdmin roles (same ownership shape as `update_event`
    /// and `delete_event`). Re-reads the row, flips the flag, and writes
    /// it back through the existing `update` path so the JSON column and
    /// the denormalized columns stay in sync. Idempotent — verifying an
    /// already-verified event is a no-op (and skips the write so the
    /// `updated_at` trigger doesn't fire for nothing).
    pub async fn verify_event_date(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        // Same gate as update_event / delete_event: admins bypass the
        // ownership check; everyone else must own the event.
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if !is_admin {
            let collection = self.user_collection_logic.get(&claims.sub).await?;
            if !collection.created_events.contains(&id) {
                return Err(AppError::Unauthorized(
                    "You do not have permission to verify this event's date".to_string(),
                ));
            }
        }

        let row = self.repository.find_by_id(id).await?;
        let mut event: NomEvent = serde_json::from_str(&row.event_data)?;
        if event.date_verified {
            return Ok(());
        }
        event.date_verified = true;
        self.repository.update(id, &event).await?;
        Ok(())
    }

    /// Advance every recurring event whose date has already passed to its
    /// next occurrence. "Recurring" means the row carries an explicit
    /// `recurrence_interval` or the legacy `recurring_annual` flag (see
    /// `EventContext::find_past_recurring`). For each matching row:
    ///
    ///   * `start_date` / `end_date` shift forward by one period of the
    ///     event's cadence — one year for `recurring_annual`, otherwise
    ///     whatever `recurrence_interval` specifies (annual, biennial,
    ///     monthly, ...). Month/day is preserved, with Feb 29 → Feb 28 in
    ///     a common year. See `advance_event_to_next_occurrence`.
    ///   * `date_verified = false` (the shift is approximate — most
    ///     festivals fall on a calendar weekend, not a calendar date,
    ///     so the user has to confirm the new exact day)
    ///
    /// Returns the number of events rolled. Logs at `info!` when any
    /// roll happens so ops can see the activity in the daily logs.
    ///
    /// Wired into the retention-sweep tokio task in `main.rs` so it
    /// runs on the same cadence as the other recurring data tasks. A
    /// future enhancement is to also create a verification reminder
    /// for each rolled event so the user is nudged before it goes
    /// public.
    pub async fn roll_past_recurring_events(&self) -> Result<u64, AppError> {
        let rows = self.repository.find_past_recurring().await?;
        let mut rolled: u64 = 0;
        for row in rows {
            let event: NomEvent = match serde_json::from_str(&row.event_data) {
                Ok(e) => e,
                Err(err) => {
                    // Corrupt JSON — log and skip, same shape as
                    // `event_response_or_log` below. One bad row
                    // shouldn't block the rest of the roll.
                    tracing::warn!(
                        event_id = row.id,
                        error = %err,
                        "Skipping event in roll_past_recurring_events — event_data JSON failed to parse"
                    );
                    continue;
                }
            };
            let advanced = advance_event_to_next_occurrence(event);
            match self.repository.update(row.id, &advanced).await {
                Ok(true) => rolled += 1,
                Ok(false) => {
                    // Race: the row was deleted between find and
                    // update. Acceptable; just log and move on.
                    tracing::warn!(
                        event_id = row.id,
                        "Row disappeared during roll_past_recurring_events"
                    );
                }
                Err(err) => {
                    // One failed update doesn't abort the whole sweep.
                    // Log and continue so the remaining events still
                    // get rolled.
                    tracing::warn!(
                        event_id = row.id,
                        error = ?err,
                        "Failed to roll event year"
                    );
                }
            }
        }
        if rolled > 0 {
            tracing::info!(rows = rolled, "Rolled past recurring events to next year");
        }
        Ok(rolled)
    }

    /// Admin-only: archive an event. Removes it from listing endpoints
    /// (find_all / find_by_type / find_nearby) but keeps the row in the
    /// DB so saved-event links resolve and unarchiving can restore it.
    ///
    /// Owner self-archive is intentionally NOT allowed in this iteration —
    /// the field is reserved for catalog-curation decisions. A future
    /// `hide_from_my_calendar` flag could give users per-account hiding
    /// without affecting global visibility.
    pub async fn archive_event(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if !is_admin {
            return Err(AppError::Unauthorized(
                "Only admins can archive events".to_string(),
            ));
        }
        self.repository.archive(id).await?;
        Ok(())
    }

    /// Admin-only: inverse of `archive_event`.
    pub async fn unarchive_event(&self, id: i64, claims: Claims) -> Result<(), AppError> {
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if !is_admin {
            return Err(AppError::Unauthorized(
                "Only admins can unarchive events".to_string(),
            ));
        }
        self.repository.unarchive(id).await?;
        Ok(())
    }

    /// Sweep one-time events past their end date and archive them. Sister
    /// of `roll_past_recurring_events` (which handles the recurring case);
    /// together they keep stale rows out of the active catalog. Wired into
    /// the retention sweep so it runs hourly by default.
    pub async fn auto_archive_past_non_recurring_events(&self) -> Result<u64, AppError> {
        let n = self.repository.auto_archive_past_non_recurring().await?;
        if n > 0 {
            tracing::info!(rows = n, "Auto-archived past non-recurring events");
        }
        Ok(n)
    }

    // Private business logic methods
    fn validate_event(&self, event: &NomEvent) -> Result<(), AppError> {
        validate_event(event)
    }
}

/// Advance a recurring event's dates to its next occurrence and clear
/// `date_verified`. The generalized successor to the old year-only
/// `bump_event_year`: it shifts both date fields forward by exactly one
/// period of the event's recurrence cadence (see `effective_interval`
/// for how that cadence is resolved), then flips `date_verified` off —
/// the new dates are an estimate the owner should reconfirm, since
/// festivals usually track a weekday ("third weekend of June") rather
/// than a fixed calendar date.
///
/// Pure (no DB I/O, no clock read) so the calendar edge cases stay
/// unit-pinnable. Year/month shifts use chrono's `checked_add_months`,
/// which clamps an out-of-range target day to the last valid day of the
/// month (Jan 31 + 1 month → Feb 28/29; Feb 29 + 1 year → Feb 28 in a
/// common year). Week shifts add `7 * count` days.
///
/// Advances by a single period per call. An event stale by more than one
/// period is re-selected on the next retention sweep and steps forward
/// again, so it converges on a future date without this function having
/// to read the clock.
pub(crate) fn advance_event_to_next_occurrence(mut event: NomEvent) -> NomEvent {
    // The roll only ever runs this on rows the DB flagged recurring, so a
    // missing interval (a legacy row with neither field set) safely
    // defaults to the historical annual behavior.
    let interval = effective_interval(&event).unwrap_or_else(RecurrenceInterval::annual);
    event.date_info.start_date = event
        .date_info
        .start_date
        .map(|d| advance_date(d, interval));
    event.date_info.end_date = event.date_info.end_date.map(|d| advance_date(d, interval));
    event.date_verified = false;
    event
}

/// Resolve the cadence the auto-roll should advance an event by.
/// Precedence:
///   1. an explicit `recurrence_interval` (the new source of truth),
///   2. else `recurring_annual == true` → one calendar year (the legacy
///      bridge, kept until every row carries an explicit interval),
///   3. else `None` — the event isn't flagged recurring on either field.
pub(crate) fn effective_interval(event: &NomEvent) -> Option<RecurrenceInterval> {
    event
        .recurrence_interval
        .or_else(|| event.recurring_annual.then(RecurrenceInterval::annual))
}

/// Shift one timestamp forward by a single `interval` period. See
/// `advance_event_to_next_occurrence` for the day-clamping semantics.
fn advance_date(dt: DateTime<Utc>, interval: RecurrenceInterval) -> DateTime<Utc> {
    match interval.unit {
        // `7 * count` days; `count` is bounded by `validate_event`, so the
        // multiply can't realistically overflow.
        RecurrenceUnit::Week => dt + Duration::days(7 * interval.count as i64),
        RecurrenceUnit::Month => dt
            .checked_add_months(Months::new(interval.count))
            .unwrap_or(dt),
        RecurrenceUnit::Year => dt
            .checked_add_months(Months::new(12u32.saturating_mul(interval.count)))
            .unwrap_or(dt),
    }
}

/// Convert an `EventRow` to `EventResponse`, logging and dropping rows
/// whose `event_data` JSON failed to parse. The row's id is captured before
/// the move so the warning identifies the specific corrupt row — ops can
/// then go look at it directly in SQLite.
///
/// Used by every list endpoint that builds `Vec<EventResponse>`. Pre-fix
/// these all called `.filter_map(|row| EventResponse::from_row(row).ok())`,
/// which silently dropped malformed rows with zero telemetry — operators
/// had no way to know a row had gone bad.
pub(crate) fn event_response_or_log(row: EventRow) -> Option<EventResponse> {
    let row_id = row.id;
    match EventResponse::from_row(row) {
        Ok(response) => Some(response),
        Err(err) => {
            tracing::warn!(
                event_id = row_id,
                error = ?err,
                "Skipping event row with malformed event_data JSON",
            );
            None
        }
    }
}

/// Free-function validator, separated from `EventLogic` so it can be unit-tested
/// without instantiating a context. Called from `EventLogic::validate_event`.
pub(crate) fn validate_event(event: &NomEvent) -> Result<(), AppError> {
    // String length bounds — prevents bypass-via-megabyte-payload and
    // keeps stored data within UX-reasonable sizes.
    const MAX_NAME_LEN: usize = 200;
    const MAX_DESC_LEN: usize = 5000;
    const MAX_WEBSITE_LEN: usize = 2048;
    // Upper bound on a recurrence count — "every 100 years/months/weeks"
    // is already well past anything real, and the cap guards the
    // year-shift multiply in `advance_date` from overflow.
    const MAX_RECURRENCE_COUNT: u32 = 100;

    if event.name.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Event name cannot be empty".to_string(),
        ));
    }
    if event.name.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Event name must be {} characters or fewer",
            MAX_NAME_LEN
        )));
    }

    if event.description.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Event description cannot be empty".to_string(),
        ));
    }
    if event.description.len() > MAX_DESC_LEN {
        return Err(AppError::ValidationError(format!(
            "Event description must be {} characters or fewer",
            MAX_DESC_LEN
        )));
    }

    if let Some(w) = event.website.as_ref()
        && w.len() > MAX_WEBSITE_LEN
    {
        return Err(AppError::ValidationError(
            "Website URL is too long".to_string(),
        ));
    }

    // Recurrence cadence sanity: a present interval must advance the
    // event by at least one period — count 0 would never move a past
    // event forward, so the roll would re-select it every sweep — and
    // is capped so the year-shift multiply in `advance_date` can't
    // overflow.
    if let Some(interval) = event.recurrence_interval
        && !(1..=MAX_RECURRENCE_COUNT).contains(&interval.count)
    {
        return Err(AppError::ValidationError(format!(
            "Recurrence interval count must be between 1 and {}",
            MAX_RECURRENCE_COUNT
        )));
    }

    // String length bound for the location address. Empty is a
    // separate check below — UX-bounded so a paste-error can't push
    // a megabyte of HTML through the bind protocol.
    const MAX_ADDRESS_LEN: usize = 512;
    if event.location_info.address.len() > MAX_ADDRESS_LEN {
        return Err(AppError::ValidationError(format!(
            "Address must be {} characters or fewer",
            MAX_ADDRESS_LEN
        )));
    }

    // Empty `{}` POSTs deserialize with `Location::default()`, leaving
    // `address=""` and `(latitude=0, longitude=0)`. That puts the
    // event at the "null island" off Africa with no way for a user
    // to find it. Reject these as a partially-populated payload
    // rather than persisting unusable rows. This also catches the
    // common AI-hallucination shape that the curator already rejects
    // upstream — defense in depth.
    if event.location_info.address.trim().is_empty() {
        return Err(AppError::ValidationError("Address is required".to_string()));
    }

    // Reject NaN/Inf before doing range checks — they bypass < / > naively.
    if !event.location_info.latitude.is_finite() || !event.location_info.longitude.is_finite() {
        return Err(AppError::ValidationError(
            "Latitude and longitude must be finite".to_string(),
        ));
    }
    if !(-90.0..=90.0).contains(&event.location_info.latitude) {
        return Err(AppError::ValidationError("Invalid latitude".to_string()));
    }
    if !(-180.0..=180.0).contains(&event.location_info.longitude) {
        return Err(AppError::ValidationError("Invalid longitude".to_string()));
    }
    // (0, 0) is "null island" — a frequent default-fallthrough sentinel.
    // Real festivals aren't 600km off the coast of Africa; treat as
    // a partially-populated payload and reject.
    if event.location_info.latitude == 0.0 && event.location_info.longitude == 0.0 {
        return Err(AppError::ValidationError(
            "Coordinates (0, 0) are not a valid festival location".to_string(),
        ));
    }

    if let (Some(start), Some(end)) = (event.date_info.start_date, event.date_info.end_date)
        && end < start
    {
        return Err(AppError::ValidationError(
            "End date cannot be before start date".to_string(),
        ));
    }

    Ok(())
}

/// Accept exactly `YYYY-MM-DD` (10 chars, all-digit / dash positions correct,
/// and a real calendar date). Used to validate `date_from` / `date_to` query
/// params for `/event/search` before they reach the dynamic SQL — anything
/// not in this shape makes lexicographic comparison silently incorrect.
/// Returns the input unchanged so callers can pass through validated strings.
pub(crate) fn validate_iso_date(s: &str, field: &str) -> Result<String, AppError> {
    if chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err() {
        return Err(AppError::ValidationError(format!(
            "{} must be in YYYY-MM-DD format",
            field
        )));
    }
    Ok(s.to_string())
}

// `validate_pagination` + `DEFAULT_PAGINATION_LIMIT` + `MAX_PAGINATION_LIMIT`
// moved to `src/util.rs` so every list endpoint shares one definition. See
// the doc comment there.

/// Hard cap on how many type ids a caller can pass. SQLite's
/// `SQLITE_MAX_VARIABLE_NUMBER` is 999 by default; we're well below that.
/// Cap exists mainly to refuse pathological inputs (`?event_type_ids=` with
/// 10k comma-separated values) rather than enforce a meaningful product
/// limit — real usage will be well under 20.
const MAX_EVENT_TYPE_IDS: usize = 100;

/// Merge `?event_type=N` (single) and `?event_type_ids=A,B,C` (multi) into
/// one deduplicated `Vec<i64>` ordered by first appearance. Both empty →
/// empty Vec (no filter). Either alone or both together → union.
///
/// Validation:
///   - Each comma-separated entry must parse as `i64` (positive integers in
///     practice; negative would be a no-op match but we don't bother).
///   - Trims whitespace between commas so `"1, 2, 3"` is accepted.
///   - Empty entries (trailing commas, double commas) are silently skipped.
///   - List length capped at MAX_EVENT_TYPE_IDS — anything bigger is a 400.
pub(crate) fn validate_event_type_ids(
    single: Option<i64>,
    multi: Option<&str>,
) -> Result<Vec<i64>, AppError> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<i64> = Vec::new();

    if let Some(id) = single
        && seen.insert(id)
    {
        out.push(id);
    }

    if let Some(raw) = multi {
        for piece in raw.split(',') {
            let trimmed = piece.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed: i64 = trimmed.parse().map_err(|_| {
                AppError::ValidationError(format!(
                    "event_type_ids must be a comma-separated list of integers (got {:?})",
                    trimmed
                ))
            })?;
            if seen.insert(parsed) {
                out.push(parsed);
            }
        }
    }

    if out.len() > MAX_EVENT_TYPE_IDS {
        return Err(AppError::ValidationError(format!(
            "event_type_ids: too many entries ({}, max {})",
            out.len(),
            MAX_EVENT_TYPE_IDS
        )));
    }

    Ok(out)
}

/// Parse the `?sort=` query param into an `EventSortOrder`. Absent or
/// empty → `Name` (the historical default). Anything outside the closed
/// set → 400 so typos like `?sort=naem` fail loud.
pub(crate) fn validate_sort_param(input: Option<&str>) -> Result<EventSortOrder, AppError> {
    let Some(raw) = input else {
        return Ok(EventSortOrder::default());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(EventSortOrder::default());
    }
    match trimmed {
        "name" => Ok(EventSortOrder::Name),
        "date" => Ok(EventSortOrder::Date),
        "distance" => Ok(EventSortOrder::Distance),
        other => Err(AppError::ValidationError(format!(
            "sort must be one of: name, date, distance (got {:?})",
            other
        ))),
    }
}

/// Trim, drop empty, and cap length on the `name_contains` query param.
/// Returns:
///   - `Ok(None)` if the input is absent OR empty/whitespace-only after
///     trimming — there's nothing to filter on, so the caller should skip
///     the LIKE clause entirely.
///   - `Ok(Some(trimmed))` if the input is in-bounds.
///   - `Err(ValidationError)` if the input exceeds the cap.
const NAME_CONTAINS_MAX_LEN: usize = 100;

pub(crate) fn validate_name_contains(input: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(raw) = input else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > NAME_CONTAINS_MAX_LEN {
        return Err(AppError::ValidationError(format!(
            "name_contains must be {} characters or fewer",
            NAME_CONTAINS_MAX_LEN
        )));
    }
    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::event_models::{
        EventDate, Location, NomEvent, RecurrenceInterval, RecurrenceUnit,
    };

    fn make_event(name: &str, description: &str) -> NomEvent {
        NomEvent {
            id: None,
            user_id: None,
            name: name.to_string(),
            description: description.to_string(),
            event_type_id: 1,
            website: None,
            date_info: EventDate {
                start_date: None,
                end_date: None,
                single_day: false,
                early_arrival_available: false,
                early_arrival_date: None,
                late_departure_available: false,
            },
            // Use real coordinates (Atlanta-ish) rather than (0, 0) — the
            // validator rejects null-island (0, 0) as a partially-populated
            // payload. Per-test mutations can still set 0.0 explicitly to
            // exercise that rejection path.
            location_info: Location {
                address: "anywhere".into(),
                longitude: -84.39,
                latitude: 33.74,
                venue_name: None,
                parking_info: None,
            },
            amenities: None,
            camping_info: None,
            archive: false,
            // Default the recurrence markers + date_verified off for
            // the test fixture. Tests that need them on flip per-test
            // via field-mutation before invoking the validator.
            recurring: false,
            recurring_annual: false,
            date_verified: false,
            recurrence_interval: None,
        }
    }

    #[test]
    fn rejects_empty_name() {
        let err = validate_event(&make_event("", "desc")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_whitespace_only_name() {
        let err = validate_event(&make_event("   ", "desc")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_oversize_name() {
        let big = "x".repeat(500);
        let err = validate_event(&make_event(&big, "desc")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_empty_description() {
        let err = validate_event(&make_event("name", "")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_non_finite_coordinates() {
        let mut event = make_event("name", "desc");
        event.location_info.latitude = f64::NAN;
        let err = validate_event(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));

        event.location_info.latitude = f64::INFINITY;
        let err = validate_event(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_out_of_range_lat_lon() {
        let mut event = make_event("name", "desc");
        event.location_info.latitude = 91.0;
        assert!(validate_event(&event).is_err());

        event.location_info.latitude = 0.0;
        event.location_info.longitude = 181.0;
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn rejects_null_island_coordinates() {
        // (0, 0) is the partially-populated-payload sentinel: an empty
        // `{}` POST deserializes with Location::default(), landing the
        // event 600km off the coast of Africa. Reject loudly.
        let mut event = make_event("name", "desc");
        event.location_info.latitude = 0.0;
        event.location_info.longitude = 0.0;
        let err = validate_event(&event).unwrap_err();
        match err {
            AppError::ValidationError(msg) => {
                assert!(
                    msg.contains("0, 0") || msg.contains("(0, 0)"),
                    "expected null-island message, got: {msg}",
                );
            }
            _ => panic!("expected ValidationError"),
        }
    }

    #[test]
    fn rejects_empty_address() {
        // Same partially-populated-payload concern as null-island, just
        // from the other direction — a client could fake plausible
        // coords but omit the address. Without a postal address users
        // can't actually find the event.
        let mut event = make_event("name", "desc");
        event.location_info.address = String::new();
        let err = validate_event(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));

        event.location_info.address = "   ".to_string();
        let err = validate_event(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn rejects_oversize_address() {
        let mut event = make_event("name", "desc");
        event.location_info.address = "a".repeat(1000);
        let err = validate_event(&event).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn accepts_valid_event() {
        let event = make_event("Concert", "Live music");
        assert!(validate_event(&event).is_ok());
    }

    #[test]
    fn validate_iso_date_accepts_canonical_form() {
        assert_eq!(
            validate_iso_date("2026-06-12", "date_from").unwrap(),
            "2026-06-12"
        );
    }

    #[test]
    fn validate_iso_date_rejects_us_format() {
        let err = validate_iso_date("06/12/2026", "date_from").unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_iso_date_rejects_partial_dates() {
        assert!(validate_iso_date("2026-06", "date_to").is_err());
        assert!(validate_iso_date("2026", "date_to").is_err());
    }

    #[test]
    fn validate_iso_date_rejects_impossible_calendar_dates() {
        // chrono's NaiveDate parser is calendar-aware — Feb 30 doesn't exist.
        assert!(validate_iso_date("2026-02-30", "date_from").is_err());
        assert!(validate_iso_date("2026-13-01", "date_from").is_err());
    }

    #[test]
    fn validate_iso_date_rejects_datetime_strings() {
        // Only the date — datetimes belong to event records, not to the
        // search filter. Reject them so the caller is forced to canonicalize.
        assert!(validate_iso_date("2026-06-12T00:00:00Z", "date_from").is_err());
        assert!(validate_iso_date("2026-06-12 00:00:00", "date_from").is_err());
    }

    #[test]
    fn validate_name_contains_passes_normal_input() {
        let result = validate_name_contains(Some("Renaissance")).unwrap();
        assert_eq!(result.as_deref(), Some("Renaissance"));
    }

    #[test]
    fn validate_name_contains_trims_whitespace() {
        let result = validate_name_contains(Some("  jazz   ")).unwrap();
        assert_eq!(result.as_deref(), Some("jazz"));
    }

    #[test]
    fn validate_name_contains_returns_none_for_absent_input() {
        assert_eq!(validate_name_contains(None).unwrap(), None);
    }

    #[test]
    fn validate_name_contains_returns_none_for_empty_and_whitespace() {
        // Empty and whitespace-only should be treated identically to absent —
        // nothing to filter on, skip the LIKE clause.
        assert_eq!(validate_name_contains(Some("")).unwrap(), None);
        assert_eq!(validate_name_contains(Some("   ")).unwrap(), None);
        assert_eq!(validate_name_contains(Some("\t\n")).unwrap(), None);
    }

    #[test]
    fn validate_name_contains_rejects_oversized_input() {
        let big = "x".repeat(NAME_CONTAINS_MAX_LEN + 1);
        let err = validate_name_contains(Some(&big)).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_name_contains_allows_max_length_exactly() {
        let exact = "x".repeat(NAME_CONTAINS_MAX_LEN);
        let result = validate_name_contains(Some(&exact)).unwrap();
        assert_eq!(
            result.as_deref().map(|s| s.len()),
            Some(NAME_CONTAINS_MAX_LEN)
        );
    }

    #[test]
    fn validate_name_contains_does_not_escape_internally() {
        // The validator only trims and length-caps. LIKE-escaping is the
        // context layer's responsibility (so callers without LIKE semantics
        // get the raw string). This test pins that contract: special chars
        // pass through validation untouched.
        let result = validate_name_contains(Some("100% music_fest")).unwrap();
        assert_eq!(result.as_deref(), Some("100% music_fest"));
    }

    #[test]
    fn validate_sort_param_defaults_to_name_when_absent() {
        assert_eq!(validate_sort_param(None).unwrap(), EventSortOrder::Name);
    }

    #[test]
    fn validate_sort_param_defaults_to_name_when_empty_or_whitespace() {
        assert_eq!(validate_sort_param(Some("")).unwrap(), EventSortOrder::Name);
        assert_eq!(
            validate_sort_param(Some("   ")).unwrap(),
            EventSortOrder::Name
        );
    }

    #[test]
    fn validate_sort_param_accepts_three_legal_values() {
        assert_eq!(
            validate_sort_param(Some("name")).unwrap(),
            EventSortOrder::Name
        );
        assert_eq!(
            validate_sort_param(Some("date")).unwrap(),
            EventSortOrder::Date
        );
        assert_eq!(
            validate_sort_param(Some("distance")).unwrap(),
            EventSortOrder::Distance
        );
    }

    #[test]
    fn validate_sort_param_rejects_typos_loudly() {
        // The whole point of validating instead of silent-defaulting: a
        // typo should be visible, not invisibly wrong (caller thinks
        // they're sorting by date, getting name).
        let err = validate_sort_param(Some("naem")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_sort_param_is_case_sensitive() {
        // Lowercase only — matches our REST convention. If we later want
        // case-insensitive, the test pins us to consider that explicitly.
        assert!(validate_sort_param(Some("Name")).is_err());
        assert!(validate_sort_param(Some("DATE")).is_err());
    }

    #[test]
    fn validate_event_type_ids_both_absent_returns_empty() {
        assert_eq!(
            validate_event_type_ids(None, None).unwrap(),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn validate_event_type_ids_single_only() {
        assert_eq!(validate_event_type_ids(Some(5), None).unwrap(), vec![5]);
    }

    #[test]
    fn validate_event_type_ids_multi_only_parses_csv() {
        assert_eq!(
            validate_event_type_ids(None, Some("1,2,3")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn validate_event_type_ids_multi_tolerates_whitespace() {
        // Curl-pasted lists often have spaces; accept them rather than 400.
        assert_eq!(
            validate_event_type_ids(None, Some("1, 2 ,  3")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn validate_event_type_ids_skips_empty_entries() {
        // Trailing comma, leading comma, double comma. Common copy-paste
        // mistake; tolerating it beats erroring.
        assert_eq!(
            validate_event_type_ids(None, Some(",1,,2,3,")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn validate_event_type_ids_deduplicates_preserving_first_appearance() {
        assert_eq!(
            validate_event_type_ids(None, Some("1,2,1,3,2")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn validate_event_type_ids_unions_single_and_multi() {
        // Both present → union. Single appears first because it's processed
        // first. Order matters here because the SQL IN-clause respects it.
        assert_eq!(
            validate_event_type_ids(Some(5), Some("1,2,5,3")).unwrap(),
            vec![5, 1, 2, 3]
        );
    }

    #[test]
    fn validate_event_type_ids_rejects_non_integers() {
        let err = validate_event_type_ids(None, Some("1,abc,3")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    // `validate_pagination` tests moved to `src/util.rs::tests` alongside
    // the function itself.

    #[test]
    fn validate_event_type_ids_rejects_pathologically_long_lists() {
        // Build a CSV of 200 ids — well over the 100-id cap. We don't
        // care about the SQL impact; we care that the cap is enforced.
        let csv = (1..=200)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let err = validate_event_type_ids(None, Some(&csv)).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    // -------------------------------------------------------------
    // advance_event_to_next_occurrence — pure cadence arithmetic on
    // the date_info dates
    // -------------------------------------------------------------
    //
    // The shift is approximate (festivals usually fall on a weekend,
    // not a fixed calendar date), so `date_verified` is always
    // cleared. The pure-function shape lets us exercise the leap-day
    // and day-clamp edge cases without spinning up a DB.

    use chrono::TimeZone;

    fn dt(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    #[test]
    fn advance_annual_event_advances_both_dates_by_one_year() {
        let mut event = make_event("Annual Fest", "desc");
        event.date_info.start_date = Some(dt(2026, 7, 4));
        event.date_info.end_date = Some(dt(2026, 7, 6));
        event.recurring_annual = true;
        event.date_verified = true;

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2027, 7, 4)));
        assert_eq!(advanced.date_info.end_date, Some(dt(2027, 7, 6)));
        // Auto-roll always clears verification — the new date is a
        // guess and the owner must reconfirm.
        assert!(!advanced.date_verified);
    }

    #[test]
    fn advance_annual_leap_day_falls_back_to_feb_28() {
        // 2024-02-29 → 2025-02-29 doesn't exist; conventional
        // behavior is to land on Feb 28 of the new year.
        let mut event = make_event("Leap Day Fest", "desc");
        event.date_info.start_date = Some(dt(2024, 2, 29));
        event.date_info.end_date = Some(dt(2024, 2, 29));
        event.recurring_annual = true;

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2025, 2, 28)));
        assert_eq!(advanced.date_info.end_date, Some(dt(2025, 2, 28)));
    }

    #[test]
    fn advance_preserves_null_dates() {
        // Some seasonal events have NULL start/end_date (the
        // Renaissance Faire cluster, etc.). The shift should leave
        // them alone — there's nothing to advance — and still flip
        // date_verified off so the owner is prompted to fill them in.
        let mut event = make_event("No-Date Fest", "desc");
        event.date_info.start_date = None;
        event.date_info.end_date = None;
        event.recurring_annual = true;
        event.date_verified = true;

        let advanced = advance_event_to_next_occurrence(event);
        assert!(advanced.date_info.start_date.is_none());
        assert!(advanced.date_info.end_date.is_none());
        assert!(!advanced.date_verified);
    }

    #[test]
    fn advance_handles_start_without_end() {
        // Single-day events: start_date set, end_date null. The shift
        // should advance start and leave end null.
        let mut event = make_event("One Day Fest", "desc");
        event.date_info.start_date = Some(dt(2025, 11, 15));
        event.date_info.end_date = None;
        event.recurring_annual = true;

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2026, 11, 15)));
        assert!(advanced.date_info.end_date.is_none());
    }

    #[test]
    fn advance_biennial_event_adds_two_years() {
        // An explicit { year, 2 } cadence advances by two calendar years.
        let mut event = make_event("Biennale", "desc");
        event.date_info.start_date = Some(dt(2026, 5, 1));
        event.date_info.end_date = Some(dt(2026, 11, 1));
        event.recurrence_interval = Some(RecurrenceInterval::new(RecurrenceUnit::Year, 2));

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2028, 5, 1)));
        assert_eq!(advanced.date_info.end_date, Some(dt(2028, 11, 1)));
    }

    #[test]
    fn advance_monthly_event_clamps_overflowing_day() {
        // { month, 1 } from Jan 31 must clamp to the last valid day of
        // the target month (Feb 28 in 2026, a common year) rather than
        // spilling into March.
        let mut event = make_event("Monthly Meetup", "desc");
        event.date_info.start_date = Some(dt(2026, 1, 31));
        event.date_info.end_date = None;
        event.recurrence_interval = Some(RecurrenceInterval::new(RecurrenceUnit::Month, 1));

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2026, 2, 28)));
    }

    #[test]
    fn advance_weekly_event_adds_seven_days_per_count() {
        // { week, 3 } advances by 21 days, crossing the month boundary.
        let mut event = make_event("Triweekly Market", "desc");
        event.date_info.start_date = Some(dt(2026, 6, 15));
        event.date_info.end_date = Some(dt(2026, 6, 15));
        event.recurrence_interval = Some(RecurrenceInterval::new(RecurrenceUnit::Week, 3));

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2026, 7, 6)));
        assert_eq!(advanced.date_info.end_date, Some(dt(2026, 7, 6)));
    }

    #[test]
    fn recurrence_interval_wins_over_recurring_annual() {
        // When both are set, the explicit interval is the source of
        // truth — a { year, 2 } event must not collapse to the legacy
        // one-year shift implied by recurring_annual.
        let mut event = make_event("Both Flags", "desc");
        event.date_info.start_date = Some(dt(2026, 3, 10));
        event.recurring_annual = true;
        event.recurrence_interval = Some(RecurrenceInterval::new(RecurrenceUnit::Year, 2));

        let advanced = advance_event_to_next_occurrence(event);
        assert_eq!(advanced.date_info.start_date, Some(dt(2028, 3, 10)));
    }

    #[test]
    fn effective_interval_resolution_order() {
        // 1. explicit interval wins over recurring_annual.
        let mut event = make_event("E", "desc");
        event.recurring_annual = true;
        event.recurrence_interval = Some(RecurrenceInterval::new(RecurrenceUnit::Month, 6));
        assert_eq!(
            effective_interval(&event),
            Some(RecurrenceInterval::new(RecurrenceUnit::Month, 6))
        );

        // 2. recurring_annual alone → one calendar year.
        let mut event = make_event("E", "desc");
        event.recurring_annual = true;
        assert_eq!(
            effective_interval(&event),
            Some(RecurrenceInterval::annual())
        );

        // 3. neither flag → None.
        let event = make_event("E", "desc");
        assert_eq!(effective_interval(&event), None);
    }
}
