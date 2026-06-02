// src/context/event_context.rs

use crate::errors::AppError;
use crate::models::database_models::EventRow;
use crate::models::dto::EventSortOrder;
use crate::models::event_models::NomEvent;
use crate::util::escape_like_pattern;
use sqlx::SqlitePool;

/// Hard cap on `IN (?, ?, …)` batch lookups. SQLite's default
/// SQLITE_MAX_VARIABLE_NUMBER is 999; staying well under that keeps us safe
/// across SQLite versions and avoids accidentally building enormous queries.
const MAX_BATCH_SIZE: usize = 500;

/// Shared WHERE-clause fragment that hides past one-time events from
/// listing endpoints. The rule: keep a row if any of these hold —
///
///   1. `recurring` is true or missing (default-true behavior). Annual /
///      biennial / "happens again" events stay visible forever; the
///      auto-roll-year task rolls annual recurring ones forward so users
///      always see the next occurrence.
///   2. The event has no start date yet (TBA). Stale TBA events are a
///      data-quality problem to be solved with curator passes, not by
///      hiding them from users who might have inside info on dates.
///   3. The event hasn't ended yet (`end_date >= today`, falling back to
///      `start_date` for single-day events without an explicit end).
///
/// The inverse (what gets filtered out) is precisely one-time events
/// that have already happened — exactly the "no longer shown if it
/// isn't recurring" UX the user wants. Detail-page lookups
/// (`find_by_id`, `get_by_id_list`) deliberately do NOT apply this
/// filter — saved-event links to past one-time events should still
/// resolve so the frontend can show a "this event has ended" notice.
const RECURRING_VISIBLE_FILTER: &str = "(\
    COALESCE(json_extract(e.event_data, '$.recurring'), 1) != 0 \
    OR e.start_date IS NULL \
    OR substr(COALESCE(e.end_date, e.start_date), 1, 10) >= date('now')\
)";

/// Hide archived events from listing endpoints. The `archive` flag is
/// stored inside the `event_data` JSON blob (not a top-level column).
/// `COALESCE(..., 0)` treats a missing key as "not archived" so legacy
/// rows from before the field existed continue to surface normally.
/// Detail-page lookups (`find_by_id`, `get_by_id_list`) deliberately do
/// NOT apply this filter — admins editing an archived event still need
/// to load it, and saved-event links should resolve to a "this event
/// has been archived" notice instead of a 404.
const NOT_ARCHIVED_FILTER: &str = "COALESCE(json_extract(e.event_data, '$.archive'), 0) != 1";

pub struct EventContext {
    pool: SqlitePool,
}

impl EventContext {
    // ... existing new() method ...
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    /// Admin-facing list of all events. Paginated via the shared
    /// `util::validate_pagination` contract on the logic layer: `limit`
    /// is clamped to `[1, MAX_PAGINATION_LIMIT]` and `offset` to `>= 0`
    /// before we get here. ORDER BY id DESC so the newest events come
    /// first — matches the admin-audit-log convention.
    pub async fn find_all(&self, limit: i64, offset: i64) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(&format!(
            "SELECT
                e.id, e.name, e.description, e.website, e.event_type_id,
                e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
                et.name as event_type_name,
                et.description as event_type_description,
                et.map_indicator as event_type_map_indicator,
                et.category as event_type_category
             FROM events e
             JOIN event_types et ON e.event_type_id = et.id
             WHERE {RECURRING_VISIBLE_FILTER}
               AND {NOT_ARCHIVED_FILTER}
             ORDER BY e.id DESC
             LIMIT ?1 OFFSET ?2"
        ))
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_by_id(&self, id: i64) -> Result<EventRow, AppError> {
        let row = sqlx::query_as::<_, EventRow>(
            "SELECT 
                e.id, e.name, e.description, e.website, e.event_type_id,
                e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
                et.name as event_type_name,
                et.description as event_type_description,
                et.map_indicator as event_type_map_indicator,
                et.category as event_type_category
             FROM events e
             JOIN event_types et ON e.event_type_id = et.id
             WHERE e.id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_by_id_list(&self, input: Vec<i64>) -> Result<Vec<EventRow>, AppError> {
        // Empty input would build `IN ()` — a SQL syntax error. Short-circuit
        // with an empty result so callers don't have to special-case it.
        if input.is_empty() {
            return Ok(Vec::new());
        }
        if input.len() > MAX_BATCH_SIZE {
            return Err(AppError::BadRequest(format!(
                "Too many ids requested (max {})",
                MAX_BATCH_SIZE
            )));
        }

        let placeholders = input.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let query_str = format!(
            "SELECT
            e.id, e.name, e.description, e.website, e.event_type_id,
            e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
            et.name as event_type_name,
            et.description as event_type_description,
            et.map_indicator as event_type_map_indicator,
            et.category as event_type_category
         FROM events e
         JOIN event_types et ON e.event_type_id = et.id
         WHERE e.id IN ({})",
            placeholders
        );

        let mut query = sqlx::query_as::<_, EventRow>(&query_str);
        for id in input {
            query = query.bind(id);
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn find_by_type(&self, event_type_id: i64) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(&format!(
            "SELECT
                e.id, e.name, e.description, e.website, e.event_type_id,
                e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
                et.name as event_type_name,
                et.description as event_type_description,
                et.map_indicator as event_type_map_indicator,
                et.category as event_type_category
             FROM events e
             JOIN event_types et ON e.event_type_id = et.id
             WHERE e.event_type_id = ?
               AND {RECURRING_VISIBLE_FILTER}
               AND {NOT_ARCHIVED_FILTER}"
        ))
        .bind(event_type_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Clippy flags 12-arg signatures as smelly; the params here all flow
    // straight from `EventQueryParams` 1:1 and a struct-wrap would just
    // shift the deconstruction one layer up. Suppress at the function.
    #[allow(clippy::too_many_arguments)]
    pub async fn find_nearby(
        &self,
        lat: f64,
        lon: f64,
        radius_miles: f64,
        event_type_ids: Vec<i64>,
        date_from: Option<String>,
        date_to: Option<String>,
        name_contains: Option<String>,
        camping_allowed: Option<bool>,
        sort: EventSortOrder,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<EventRow>, AppError> {
        // Convert miles to degrees (rough approximation):
        //   1 degree latitude ≈ 69 miles
        //   longitude varies with cos(lat); fine within ±89° (poles rejected upstream)
        let lat_delta = radius_miles / 69.0;
        let lon_delta = radius_miles / (69.0 * f64::cos(lat.to_radians()));

        let min_lat = lat - lat_delta;
        let max_lat = lat + lat_delta;
        let min_lon = lon - lon_delta;
        let max_lon = lon + lon_delta;

        // Build the WHERE clauses dynamically. We could've used a match on every
        // combination of optional filters, but with three independent flags
        // (event_type, date_from, date_to) the cartesian gets to 2^3 = 8 arms.
        // Build-then-bind keeps the surface area small and obvious.
        let mut sql = format!(
            "
        SELECT
            e.id, e.name, e.description, e.website, e.event_type_id,
            e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
            et.name as event_type_name,
            et.description as event_type_description,
            et.map_indicator as event_type_map_indicator,
            et.category as event_type_category
        FROM events e
        JOIN event_types et ON e.event_type_id = et.id
        WHERE e.latitude IS NOT NULL
        AND e.longitude IS NOT NULL
        AND e.latitude BETWEEN ? AND ?
        AND e.longitude BETWEEN ? AND ?
        AND {RECURRING_VISIBLE_FILTER}
        AND {NOT_ARCHIVED_FILTER}
        "
        );

        // Type filter. Empty vec = no filter; one element uses `= ?` (cleaner
        // EXPLAIN plan than `IN (?)`); two-plus uses `IN (?, ?, …)`. Caller
        // is responsible for deduping and capping the list size — the logic
        // layer's validator does both.
        let event_type_placeholders = if event_type_ids.is_empty() {
            String::new()
        } else if event_type_ids.len() == 1 {
            String::from(" AND e.event_type_id = ?")
        } else {
            let placeholders = event_type_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            format!(" AND e.event_type_id IN ({})", placeholders)
        };
        sql.push_str(&event_type_placeholders);

        // Date filtering uses interval-overlap semantics so that an event
        // running Fri–Sun matches a search for the Saturday between. We
        // truncate the stored datetime to YYYY-MM-DD with `substr(..., 1, 10)`
        // so the comparison works regardless of whether the column holds bare
        // dates ("2026-06-12"), full datetimes ("2026-06-12 12:00:00 UTC"), or
        // RFC3339 ("2026-06-12T12:00:00Z"). Skips the start_date index but
        // the preceding bounding-box filter cuts the candidate set first.
        //
        // When *any* date filter is active we also exclude TBA events
        // (start_date IS NULL); otherwise undated rows leak through because
        // they don't fail any inequality we added.
        if date_from.is_some() || date_to.is_some() {
            sql.push_str(" AND e.start_date IS NOT NULL");
        }
        if date_from.is_some() {
            sql.push_str(" AND substr(COALESCE(e.end_date, e.start_date), 1, 10) >= ?");
        }
        if date_to.is_some() {
            sql.push_str(" AND substr(e.start_date, 1, 10) <= ?");
        }

        // Free-text filter on name + description. Wraps the escaped pattern
        // in `%…%` for substring match. `ESCAPE '\'` tells SQLite which char
        // is the escape so `%`/`_`/`\` in user input match literally.
        // LIKE in SQLite is case-insensitive for ASCII by default — fine for
        // English-language event names; tracked as a known limitation if
        // we ever index non-ASCII content.
        let name_like_pattern = name_contains
            .as_deref()
            .map(|s| format!("%{}%", escape_like_pattern(s)));
        if name_like_pattern.is_some() {
            sql.push_str(" AND (e.name LIKE ? ESCAPE '\\' OR e.description LIKE ? ESCAPE '\\')");
        }

        // Camping toggle. Stored as INTEGER (0/1) per migration; sqlx binds
        // a Rust bool into the right shape. When omitted, no clause is
        // appended — both camping and non-camping events are returned.
        if camping_allowed.is_some() {
            sql.push_str(" AND e.camping_allowed = ?");
        }

        // Ordering. `name` is the historical default. `date` puts upcoming
        // events first and TBA-dated events (NULL start_date) at the end —
        // SQLite's default null-ordering sorts NULLs FIRST in ASC, which is
        // the wrong UX (users don't want TBA events first), so we hoist
        // `start_date IS NULL` into a leading sort key. `distance` uses
        // squared Euclidean in lat/lon degrees — within a single search's
        // bounding box the local distortion is small enough that the
        // *ordering* is correct even though the magnitudes aren't real
        // distances. Real haversine in SQL needs SIN/COS extension on
        // SQLite; not worth the dependency for ordering. lat/lon for the
        // distance comparison are bound (not interpolated) — the f64 type
        // already rules out injection, but binding is the project's
        // convention and avoids tempting future maintainers to interpolate
        // less-safe values into ORDER BY clauses.
        match sort {
            EventSortOrder::Name => sql.push_str(" ORDER BY e.name"),
            EventSortOrder::Date => {
                sql.push_str(" ORDER BY e.start_date IS NULL, e.start_date ASC, e.name ASC")
            }
            EventSortOrder::Distance => sql.push_str(
                " ORDER BY ((e.latitude - ?) * (e.latitude - ?) \
                         + (e.longitude - ?) * (e.longitude - ?)) ASC, \
                          e.name ASC",
            ),
        }

        // Pagination tail. Always appended (logic layer always produces a
        // limit/offset, defaulting if the caller didn't pass them) so the
        // server-side response size is bounded regardless of input.
        sql.push_str(" LIMIT ? OFFSET ?");

        let mut q = sqlx::query_as::<_, EventRow>(&sql)
            .bind(min_lat)
            .bind(max_lat)
            .bind(min_lon)
            .bind(max_lon);
        for type_id in &event_type_ids {
            q = q.bind(*type_id);
        }
        if let Some(df) = date_from.as_ref() {
            q = q.bind(df);
        }
        if let Some(dt) = date_to.as_ref() {
            q = q.bind(dt);
        }
        if let Some(pattern) = name_like_pattern.as_ref() {
            // The pattern is bound twice — once per LIKE clause (name and
            // description). Both clauses use the identical escaped string.
            q = q.bind(pattern).bind(pattern);
        }
        if let Some(camping) = camping_allowed {
            q = q.bind(camping);
        }
        if matches!(sort, EventSortOrder::Distance) {
            // lat/lon bound four times in the ORDER BY expression:
            // (lat - ?) * (lat - ?) + (lon - ?) * (lon - ?)
            q = q.bind(lat).bind(lat).bind(lon).bind(lon);
        }

        // Pagination tail. LIMIT/OFFSET must come after ORDER BY in
        // SQLite. Logic-layer `validate_pagination` already clamped
        // `limit` to [1, 500] and `offset` to [0, ...], so binding the
        // raw values here is safe — no sanitization needed at this layer.
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;

        Ok(rows)
    }

    // create, update, delete methods stay the same...
    pub async fn create(&self, event: &NomEvent) -> Result<i64, AppError> {
        let event_json = serde_json::to_string(event)?;

        let result = sqlx::query(
            "INSERT INTO events (name, description, website, event_type_id, latitude, longitude, 
             start_date, end_date, camping_allowed, event_data) 
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.name)
        .bind(&event.description)
        .bind(&event.website)
        .bind(event.event_type_id) // Changed: now uses event_type_id
        .bind(event.location_info.latitude)
        .bind(event.location_info.longitude)
        .bind(event.date_info.start_date.map(|d| d.to_string()))
        .bind(event.date_info.end_date.map(|d| d.to_string()))
        .bind(
            event
                .camping_info
                .as_ref()
                .map(|c| c.camping_allowed)
                .unwrap_or(false),
        )
        .bind(&event_json)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Atomic compare-and-set on `events.creator_id`, the authoritative
    /// single-column event-ownership field (migration 00009). Sets
    /// `creator_id = new_owner` ONLY when the row's current owner equals
    /// `expected_previous`, and reports whether the row actually moved.
    ///
    /// `IS` is SQLite's null-safe equality, so this transparently handles
    /// the unowned case: pass `expected_previous = None` to claim an event
    /// only while `creator_id IS NULL` (the initial-assignment path used by
    /// `UserCollectionLogic::event_ownership`), or `Some(prev)` to move an
    /// event only while `prev` still holds it (the transfer path).
    ///
    /// This CAS is the durable fix for the dual-ownership race (BUGS.md F1):
    /// when two ownership approvals run concurrently, each calls this with
    /// the owner it read; the first matches `expected_previous` and wins,
    /// the second finds the column already moved, affects zero rows, and
    /// returns `false` — which the caller turns into a 409 instead of
    /// minting a second owner. Ownership stays single-valued at the DB
    /// level, not by a best-effort sequence of JSON-array edits.
    pub async fn claim_ownership(
        &self,
        event_id: i64,
        new_owner: &str,
        expected_previous: Option<&str>,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("UPDATE events SET creator_id = ?1 WHERE id = ?2 AND creator_id IS ?3")
                .bind(new_owner)
                .bind(event_id)
                .bind(expected_previous)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update(&self, id: i64, event: &NomEvent) -> Result<bool, AppError> {
        let event_json = serde_json::to_string(event)?;

        let result = sqlx::query(
            "UPDATE events SET name = ?, description = ?, website = ?, event_type_id = ?, 
             latitude = ?, longitude = ?, start_date = ?, end_date = ?, camping_allowed = ?, 
             event_data = ? WHERE id = ?",
        )
        .bind(&event.name)
        .bind(&event.description)
        .bind(&event.website)
        .bind(event.event_type_id) // Changed: now uses event_type_id
        .bind(event.location_info.latitude)
        .bind(event.location_info.longitude)
        .bind(event.date_info.start_date.map(|d| d.to_string()))
        .bind(event.date_info.end_date.map(|d| d.to_string()))
        .bind(
            event
                .camping_info
                .as_ref()
                .map(|c| c.camping_allowed)
                .unwrap_or(false),
        )
        .bind(&event_json)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM events WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Find every event whose date has already passed AND whose
    /// `event_data.recurring_annual` flag is set. Returned rows are
    /// candidates for `EventLogic::roll_past_recurring_events`, which
    /// bumps each row's start/end year by one and clears
    /// `date_verified` so the UI can prompt the user to confirm.
    ///
    /// Date comparison uses `substr(date, 1, 10)` against `date('now')`
    /// to dodge the documented format inconsistency in the date
    /// columns (`YYYY-MM-DD` from the curator path vs
    /// `YYYY-MM-DD HH:MM:SS UTC` from the chrono datetime path).
    /// `COALESCE(end_date, start_date)` lets single-day events that
    /// only filled the start date still match — and when both columns
    /// are NULL the COALESCE returns NULL, substr-of-NULL is NULL,
    /// and the row is correctly excluded.
    /// Flip the `archive` flag inside the `event_data` JSON to `true`.
    /// Returns true on a state change, false if the row was already in
    /// the requested state (or doesn't exist). Mutates the JSON via
    /// `json_set` so the rest of the blob — date_info, location_info,
    /// recurring flags — is preserved untouched.
    pub async fn archive(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE events \
             SET event_data = json_set(event_data, '$.archive', json('true')) \
             WHERE id = ? AND COALESCE(json_extract(event_data, '$.archive'), 0) != 1",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Inverse of `archive`. Returns true on a state change.
    pub async fn unarchive(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE events \
             SET event_data = json_set(event_data, '$.archive', json('false')) \
             WHERE id = ? AND COALESCE(json_extract(event_data, '$.archive'), 0) = 1",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Sweep one-time events whose end date has passed and flip their
    /// `archive` flag. Returns the number of rows archived.
    ///
    /// Mirror of `find_past_recurring` but for the opposite case: that
    /// function finds recurring rows to roll forward, this one finds
    /// non-recurring rows to retire. Together they keep the catalog
    /// from accumulating stale rows.
    ///
    /// Predicate (all must hold):
    ///   - `recurring = false` (an event with `recurring=true` or no
    ///     flag is annual / biennial and shouldn't be auto-archived;
    ///     the roll-year sweep handles annual ones, and biennial ones
    ///     are still legitimately upcoming)
    ///   - `archive != 1` (idempotent — re-running the sweep is a no-op)
    ///   - end date (or start date when single-day) is strictly before
    ///     today (we keep events that end today visible — they might
    ///     still be in their final hours)
    pub async fn auto_archive_past_non_recurring(&self) -> Result<u64, AppError> {
        let result = sqlx::query(
            "UPDATE events \
             SET event_data = json_set(event_data, '$.archive', json('true')) \
             WHERE json_extract(event_data, '$.recurring') = 0 \
               AND COALESCE(json_extract(event_data, '$.archive'), 0) != 1 \
               AND substr(COALESCE(end_date, start_date), 1, 10) < date('now')",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Past-dated events the auto-roll should advance. An event qualifies
    /// when its latest date is before today AND it is flagged recurring on
    /// either path: the legacy `recurring_annual` boolean, or the newer
    /// `recurrence_interval` object (any non-null value). `EventLogic`
    /// resolves which cadence to advance by — see `effective_interval`.
    pub async fn find_past_recurring(&self) -> Result<Vec<EventRow>, AppError> {
        // Column order matches `EventRow` (sqlx::FromRow) — 11 native
        // event columns followed by the 4 JOINed event_type columns.
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT e.id, e.name, e.description, e.website, e.event_type_id, \
                    e.latitude, e.longitude, e.start_date, e.end_date, \
                    e.camping_allowed, e.event_data, \
                    et.name AS event_type_name, \
                    et.description AS event_type_description, \
                    et.map_indicator AS event_type_map_indicator, \
                    et.category AS event_type_category \
             FROM events e \
             JOIN event_types et ON e.event_type_id = et.id \
             WHERE substr(COALESCE(e.end_date, e.start_date), 1, 10) < date('now') \
               AND (json_extract(e.event_data, '$.recurring_annual') = 1 \
                    OR json_extract(e.event_data, '$.recurrence_interval') IS NOT NULL)",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    //! Context-layer integration tests for `EventContext` — the largest
    //! of the contexts. Uses the canonical in-memory-pool pattern with
    //! `PRAGMA foreign_keys = OFF` so we don't have to seed the `events`
    //! table's `event_type_id` FK parent for every test (and so the
    //! JSON `event_data` round-trip stays the focus). Cross-table
    //! integrity is covered at the route layer.

    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        // Seed one event_type so the `find_all` JOIN against `event_types`
        // returns rows. Even with FKs off, the JOIN is INNER — without
        // a matching event_types row, joined queries would silently
        // return no rows.
        sqlx::query(
            "INSERT INTO event_types (id, name, description, map_indicator, category) \
             VALUES (1000, 'Festival', 'A festival', 'F', 'entertainment')",
        )
        .execute(&pool)
        .await
        .expect("seed event_type");
        pool
    }

    /// Construct a NomEvent via JSON so the nested `date_info` /
    /// `location_info` / `camping_info` structs get their `serde(default)`
    /// shapes without listing every field by hand.
    fn sample_event(name: &str, lat: f64, lon: f64) -> NomEvent {
        serde_json::from_value(json!({
            "name": name,
            "description": format!("{} description", name),
            "event_type_id": 1000,
            "website": "https://example.test/event",
            "date_info": {
                "start_date": "2026-06-15",
                "end_date": "2026-06-17",
                "single_day": false,
            },
            "location_info": {
                "address": "123 Main St",
                "latitude": lat,
                "longitude": lon,
                "venue_name": "Test Venue",
            },
            "camping_info": {
                "camping_allowed": false,
                "tent_camping": false,
                "rv_camping": {},
                "vehicle_camping": {},
            },
            "archive": false,
        }))
        .expect("construct sample event")
    }

    // -----------------------------------------------------------------
    // create + find_by_id — round-trip via JSON event_data
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_then_find_by_id_round_trips_json_blob() {
        // Critical: events store the full NomEvent in `event_data` JSON
        // and also denormalize a few hot columns (name, latitude, etc.).
        // The `event_data` is what the Rust API reads on detail views,
        // so the round-trip has to preserve nested location_info,
        // date_info, and camping_info.
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);

        let id = ctx
            .create(&sample_event("Atlanta Festival", 33.74, -84.39))
            .await
            .expect("create");
        assert!(id > 0);

        let found = ctx.find_by_id(id).await.expect("find_by_id");
        assert_eq!(found.name, "Atlanta Festival");
        // Denormalized top-level coords match.
        assert_eq!(found.latitude, Some(33.74));
        assert_eq!(found.longitude, Some(-84.39));
        // event_data JSON survives the round-trip — pull out the nested
        // location info to prove it.
        let payload: serde_json::Value =
            serde_json::from_str(&found.event_data).expect("parse event_data");
        assert_eq!(payload["location_info"]["address"], "123 Main St");
        assert_eq!(payload["location_info"]["venue_name"], "Test Venue");
        // The deserializer canonicalizes bare-date strings into full
        // `DateTime<Utc>` values (`"YYYY-MM-DD"` → `"YYYY-MM-DDT00:00:00Z"`).
        // Pin that canonical form rather than the input form so a refactor
        // can't silently drop the normalization step.
        assert_eq!(payload["date_info"]["start_date"], "2026-06-15T00:00:00Z");
    }

    #[tokio::test]
    async fn find_by_id_missing_row_errors() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        assert!(ctx.find_by_id(99_999).await.is_err());
    }

    // -----------------------------------------------------------------
    // update
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_modifies_columns_and_json_returns_true() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let id = ctx
            .create(&sample_event("Original", 33.0, -84.0))
            .await
            .unwrap();

        let mut updated = sample_event("Renamed", 40.0, -75.0);
        updated.description = "new description".to_string();
        let ok = ctx.update(id, &updated).await.expect("update");
        assert!(ok);

        let after = ctx.find_by_id(id).await.expect("find");
        assert_eq!(after.name, "Renamed");
        assert_eq!(after.latitude, Some(40.0));
        let payload: serde_json::Value = serde_json::from_str(&after.event_data).expect("parse");
        assert_eq!(payload["description"], "new description");
        assert_eq!(payload["location_info"]["latitude"], 40.0);
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let ok = ctx
            .update(99_999, &sample_event("ghost", 0.1, 0.1))
            .await
            .expect("update");
        assert!(!ok);
    }

    // -----------------------------------------------------------------
    // delete
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn delete_removes_row_and_returns_true() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let id = ctx
            .create(&sample_event("To Delete", 33.0, -84.0))
            .await
            .unwrap();

        let deleted = ctx.delete(id).await.expect("delete");
        assert!(deleted);
        assert!(ctx.find_by_id(id).await.is_err());
    }

    #[tokio::test]
    async fn delete_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let deleted = ctx.delete(99_999).await.expect("delete");
        assert!(!deleted);
    }

    // -----------------------------------------------------------------
    // find_all — pagination + ordering
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_all_returns_newest_first_and_paginates() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let mut ids = Vec::new();
        for i in 1..=5 {
            ids.push(
                ctx.create(&sample_event(&format!("E{}", i), 33.0 + i as f64, -84.0))
                    .await
                    .unwrap(),
            );
        }

        // ORDER BY e.id DESC — newest first.
        let p1 = ctx.find_all(2, 0).await.expect("p1");
        assert_eq!(p1.len(), 2);
        assert_eq!(p1[0].id, ids[4]);
        assert_eq!(p1[1].id, ids[3]);

        let p2 = ctx.find_all(2, 2).await.expect("p2");
        assert_eq!(p2.len(), 2);
        assert_eq!(p2[0].id, ids[2]);
        assert_eq!(p2[1].id, ids[1]);

        // Past end → empty.
        let p_end = ctx.find_all(10, 100).await.expect("past end");
        assert_eq!(p_end.len(), 0);
    }

    // -----------------------------------------------------------------
    // find_by_type
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_by_type_returns_only_matching() {
        let pool = setup_pool().await;
        // Seed a second event_type so we can distinguish primary (1000)
        // from secondary (1001). Primary id 1000 was chosen to sit
        // well above the auto-seeded "Uncategorized" sentinel (id 1)
        // introduced in migration 00007.
        sqlx::query(
            "INSERT INTO event_types (id, name, description, map_indicator, category) \
             VALUES (1001, 'Concert', 'A concert', 'C', 'entertainment')",
        )
        .execute(&pool)
        .await
        .expect("seed second event_type");
        let ctx = EventContext::new(pool);

        let e1 = sample_event("Festival Event", 33.0, -84.0);
        let mut e2 = sample_event("Concert Event", 34.0, -85.0);
        e2.event_type_id = 1001;
        ctx.create(&e1).await.unwrap();
        ctx.create(&e2).await.unwrap();

        let by_type_1 = ctx.find_by_type(1000).await.expect("find_by_type 1000");
        assert_eq!(by_type_1.len(), 1);
        assert_eq!(by_type_1[0].name, "Festival Event");

        let by_type_2 = ctx.find_by_type(1001).await.expect("find_by_type 1001");
        assert_eq!(by_type_2.len(), 1);
        assert_eq!(by_type_2[0].name, "Concert Event");
    }

    #[tokio::test]
    async fn find_by_type_empty_when_no_matches() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        ctx.create(&sample_event("Only Event", 33.0, -84.0))
            .await
            .unwrap();

        let none = ctx.find_by_type(999).await.expect("find_by_type");
        assert!(none.is_empty());
    }

    // -----------------------------------------------------------------
    // get_by_id_list — batch lookup
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_by_id_list_empty_input_returns_empty_no_query() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let result = ctx.get_by_id_list(vec![]).await.expect("empty list");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn get_by_id_list_returns_subset() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let mut ids = Vec::new();
        for i in 1..=3 {
            ids.push(
                ctx.create(&sample_event(&format!("E{}", i), 33.0 + i as f64, -84.0))
                    .await
                    .unwrap(),
            );
        }

        let subset = ctx
            .get_by_id_list(vec![ids[0], ids[2]])
            .await
            .expect("get_by_id_list");
        assert_eq!(subset.len(), 2);
        let returned: Vec<i64> = subset.iter().map(|r| r.id).collect();
        assert!(returned.contains(&ids[0]));
        assert!(returned.contains(&ids[2]));
    }

    #[tokio::test]
    async fn get_by_id_list_rejects_oversize() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool);
        let oversize: Vec<i64> = (1..=501).collect();
        let err = ctx.get_by_id_list(oversize).await.unwrap_err();
        match err {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("500"), "expected cap mention: {}", msg);
            }
            other => panic!("expected BadRequest, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // Recurring-visibility filter — `find_all`, `find_by_type`,
    // `find_nearby` must drop past one-time events but keep:
    //   - past recurring events (they'll be auto-rolled forward, and in
    //     the gap between "now" and the next roll they should still
    //     appear)
    //   - undated (TBA) events of any recurring flag (data-quality
    //     placeholders, not stale finals)
    //   - future events of any recurring flag
    // -----------------------------------------------------------------

    /// Build a NomEvent with explicit `recurring` flag and date range,
    /// then insert via raw SQL to bypass the create()-level JSON shape
    /// (so we can set start_date/end_date to historical values that the
    /// stricter logic-layer validators would reject).
    async fn insert_event_with_dates(
        pool: &sqlx::SqlitePool,
        name: &str,
        start: Option<&str>,
        end: Option<&str>,
        recurring: bool,
    ) -> i64 {
        let event_data = json!({
            "name": name,
            "description": format!("{} description", name),
            "event_type_id": 1000,
            "recurring": recurring,
            "date_info": {
                "start_date": start.map(|s| format!("{}T00:00:00Z", s)),
                "end_date": end.map(|s| format!("{}T00:00:00Z", s)),
                "single_day": end.is_none() || start == end,
            },
            "location_info": {
                "address": "Test Address",
                "latitude": 40.0,
                "longitude": -100.0,
                "venue_name": "Test Venue",
            },
        });
        let result = sqlx::query(
            "INSERT INTO events (name, description, website, event_type_id, latitude, longitude, \
             start_date, end_date, camping_allowed, event_data) \
             VALUES (?, ?, NULL, 1000, 40.0, -100.0, ?, ?, 0, ?)",
        )
        .bind(name)
        .bind("")
        .bind(start.map(|s| format!("{} 00:00:00 UTC", s)))
        .bind(end.map(|s| format!("{} 00:00:00 UTC", s)))
        .bind(event_data.to_string())
        .execute(pool)
        .await
        .expect("insert event with dates");
        result.last_insert_rowid()
    }

    #[tokio::test]
    async fn find_all_hides_past_non_recurring_events() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let past_one_time = insert_event_with_dates(
            &pool,
            "Past OneTime",
            Some("2020-06-01"),
            Some("2020-06-03"),
            false,
        )
        .await;
        let past_recurring = insert_event_with_dates(
            &pool,
            "Past Recurring",
            Some("2020-07-01"),
            Some("2020-07-03"),
            true,
        )
        .await;
        let future_one_time = insert_event_with_dates(
            &pool,
            "Future OneTime",
            Some("2099-08-01"),
            Some("2099-08-03"),
            false,
        )
        .await;
        let tba_one_time = insert_event_with_dates(&pool, "TBA OneTime", None, None, false).await;

        let all = ctx.find_all(100, 0).await.expect("find_all");
        let ids: Vec<i64> = all.iter().map(|r| r.id).collect();

        assert!(
            !ids.contains(&past_one_time),
            "past non-recurring event {} should be hidden",
            past_one_time
        );
        assert!(
            ids.contains(&past_recurring),
            "past recurring event {} should still be visible (auto-roll will catch up)",
            past_recurring
        );
        assert!(
            ids.contains(&future_one_time),
            "future non-recurring event {} should be visible",
            future_one_time
        );
        assert!(
            ids.contains(&tba_one_time),
            "TBA non-recurring event {} should be visible (data-quality, not stale)",
            tba_one_time
        );
    }

    #[tokio::test]
    async fn find_by_type_hides_past_non_recurring_events() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let past_one_time = insert_event_with_dates(
            &pool,
            "Past Solo",
            Some("2020-06-01"),
            Some("2020-06-03"),
            false,
        )
        .await;
        let future_one_time = insert_event_with_dates(
            &pool,
            "Future Solo",
            Some("2099-08-01"),
            Some("2099-08-03"),
            false,
        )
        .await;

        let rows = ctx.find_by_type(1000).await.expect("find_by_type");
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert!(!ids.contains(&past_one_time), "past non-recurring hidden");
        assert!(
            ids.contains(&future_one_time),
            "future non-recurring visible"
        );
    }

    #[tokio::test]
    async fn find_nearby_hides_past_non_recurring_events() {
        use crate::models::dto::EventSortOrder;
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let past_one_time = insert_event_with_dates(
            &pool,
            "Past Local",
            Some("2020-06-01"),
            Some("2020-06-03"),
            false,
        )
        .await;
        let future_one_time = insert_event_with_dates(
            &pool,
            "Future Local",
            Some("2099-08-01"),
            Some("2099-08-03"),
            false,
        )
        .await;

        // Center on (40, -100) — same as our inserts — with a wide
        // radius so the bounding-box filter is non-restrictive.
        let rows = ctx
            .find_nearby(
                40.0,
                -100.0,
                500.0,
                vec![],
                None,
                None,
                None,
                None,
                EventSortOrder::Name,
                100,
                0,
            )
            .await
            .expect("find_nearby");
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert!(!ids.contains(&past_one_time), "past non-recurring hidden");
        assert!(
            ids.contains(&future_one_time),
            "future non-recurring visible"
        );
    }

    // -----------------------------------------------------------------
    // Archive: hand-flip + auto-archive sweep + listing filter
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn archive_flips_event_data_flag_and_hides_from_listings() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let id = ctx
            .create(&sample_event("Active Festival", 33.0, -84.0))
            .await
            .expect("create");

        // Sanity: visible before archive.
        let visible = ctx.find_all(100, 0).await.expect("find_all pre");
        assert!(visible.iter().any(|r| r.id == id));

        let archived = ctx.archive(id).await.expect("archive");
        assert!(archived, "first archive call should report state change");

        // Idempotent: second call is a no-op.
        let archived_again = ctx.archive(id).await.expect("archive again");
        assert!(!archived_again, "re-archive should be a no-op");

        // JSON blob updated.
        let row = ctx.find_by_id(id).await.expect("find_by_id");
        let payload: serde_json::Value = serde_json::from_str(&row.event_data).expect("parse JSON");
        assert_eq!(payload["archive"], serde_json::Value::Bool(true));

        // Now hidden from listings.
        let visible_after = ctx.find_all(100, 0).await.expect("find_all post");
        assert!(
            !visible_after.iter().any(|r| r.id == id),
            "archived event should not appear in find_all"
        );

        // Detail lookup still works (saved links + admin restore path).
        let direct = ctx.find_by_id(id).await.expect("find_by_id post-archive");
        assert_eq!(direct.id, id);
    }

    #[tokio::test]
    async fn unarchive_restores_visibility() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let id = ctx
            .create(&sample_event("Toggle Festival", 33.0, -84.0))
            .await
            .expect("create");
        ctx.archive(id).await.expect("archive");

        let restored = ctx.unarchive(id).await.expect("unarchive");
        assert!(restored, "first unarchive should report state change");
        let restored_again = ctx.unarchive(id).await.expect("unarchive again");
        assert!(!restored_again, "re-unarchive should be a no-op");

        let visible = ctx.find_all(100, 0).await.expect("find_all");
        assert!(visible.iter().any(|r| r.id == id));
    }

    #[tokio::test]
    async fn auto_archive_past_non_recurring_targets_only_the_right_rows() {
        // The sweep must archive past non-recurring events and leave
        // recurring / future / TBA / already-archived rows alone.
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let past_one_time = insert_event_with_dates(
            &pool,
            "Past OneTime",
            Some("2020-06-01"),
            Some("2020-06-03"),
            false,
        )
        .await;
        let past_recurring = insert_event_with_dates(
            &pool,
            "Past Recurring",
            Some("2020-07-01"),
            Some("2020-07-03"),
            true,
        )
        .await;
        let future_one_time = insert_event_with_dates(
            &pool,
            "Future OneTime",
            Some("2099-08-01"),
            Some("2099-08-03"),
            false,
        )
        .await;
        let tba_one_time = insert_event_with_dates(&pool, "TBA OneTime", None, None, false).await;

        let archived_count = ctx
            .auto_archive_past_non_recurring()
            .await
            .expect("auto-archive");
        assert_eq!(
            archived_count, 1,
            "exactly the past-one-time row should be archived"
        );

        // Inline check rather than a closure — async closures can't
        // capture &ctx through .await with stable Rust, and the move
        // capture would consume ctx on first call.
        async fn is_archived(ctx: &EventContext, id: i64) -> bool {
            let row = ctx.find_by_id(id).await.expect("find_by_id");
            let payload: serde_json::Value =
                serde_json::from_str(&row.event_data).expect("parse JSON");
            payload["archive"] == serde_json::Value::Bool(true)
        }
        assert!(
            is_archived(&ctx, past_one_time).await,
            "past one-time should be archived"
        );
        assert!(
            !is_archived(&ctx, past_recurring).await,
            "past recurring stays active — roll-year sweep will move it forward"
        );
        assert!(
            !is_archived(&ctx, future_one_time).await,
            "future one-time stays active"
        );
        assert!(
            !is_archived(&ctx, tba_one_time).await,
            "TBA one-time stays active until it has a date"
        );

        // Idempotent: second sweep is a no-op.
        let second = ctx
            .auto_archive_past_non_recurring()
            .await
            .expect("second sweep");
        assert_eq!(second, 0, "re-sweep should be a no-op");
    }

    #[tokio::test]
    async fn find_by_id_still_returns_past_non_recurring() {
        // Detail lookups deliberately bypass the visibility filter so
        // saved-event links to a one-time past event still resolve and
        // the frontend can show a "this event has ended" notice.
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        let past_one_time = insert_event_with_dates(
            &pool,
            "Past Direct",
            Some("2020-06-01"),
            Some("2020-06-03"),
            false,
        )
        .await;

        let direct = ctx.find_by_id(past_one_time).await.expect("find_by_id");
        assert_eq!(direct.id, past_one_time);
    }

    // -----------------------------------------------------------------
    // claim_ownership — atomic compare-and-set on creator_id. This is the
    // single-ownership gate (migration 00009); the dual-ownership race is
    // only reachable under true concurrency, so we prove the CAS contract
    // directly here rather than through the (always-re-reads-the-owner)
    // approve path.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn claim_ownership_is_an_atomic_compare_and_set() {
        let pool = setup_pool().await;
        let ctx = EventContext::new(pool.clone());

        // Fresh events are created unowned — create() never sets creator_id.
        let id = ctx
            .create(&sample_event("Race Fest", 33.0, -84.0))
            .await
            .expect("create");

        async fn owner(pool: &sqlx::SqlitePool, id: i64) -> Option<String> {
            sqlx::query_scalar::<_, Option<String>>("SELECT creator_id FROM events WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("read creator_id")
        }

        assert_eq!(
            owner(&pool, id).await,
            None,
            "create() leaves event unowned"
        );

        // Claim-if-unowned: expected_previous = None matches creator_id IS NULL.
        let won = ctx
            .claim_ownership(id, "user-a", None)
            .await
            .expect("claim");
        assert!(won, "claiming an unowned event wins");
        assert_eq!(owner(&pool, id).await.as_deref(), Some("user-a"));

        // A second claim-if-unowned no-ops — the row is no longer NULL, so the
        // CAS matches zero rows. This is what makes routing initial assignment
        // through event_ownership's claim-if-null safe on an already-owned
        // event: it can never clobber the existing owner.
        let again = ctx
            .claim_ownership(id, "user-b", None)
            .await
            .expect("claim again");
        assert!(!again, "claim-if-null is a no-op once owned");
        assert_eq!(
            owner(&pool, id).await.as_deref(),
            Some("user-a"),
            "owner unchanged"
        );

        // Stale transfer: a CAS that expects the WRONG previous owner is
        // refused — this is the losing side of the concurrent-approval race.
        let stale = ctx
            .claim_ownership(id, "user-c", Some("not-the-owner"))
            .await
            .expect("stale cas");
        assert!(!stale, "CAS with a stale expected owner is refused");
        assert_eq!(
            owner(&pool, id).await.as_deref(),
            Some("user-a"),
            "owner unchanged after stale CAS"
        );

        // Matching transfer: a CAS that expects the CURRENT owner wins and
        // moves the row — the winning side of the race.
        let moved = ctx
            .claim_ownership(id, "user-c", Some("user-a"))
            .await
            .expect("matching cas");
        assert!(moved, "CAS with the correct expected owner wins");
        assert_eq!(owner(&pool, id).await.as_deref(), Some("user-c"));

        // Unknown event id: nothing matches, no row moves.
        let missing = ctx
            .claim_ownership(999_999, "user-z", None)
            .await
            .expect("missing");
        assert!(!missing, "claiming a nonexistent event affects no rows");
    }
}
