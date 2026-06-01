// src/context/microevent_context.rs

use crate::errors::AppError;
use crate::models::microevents_models::Microevent;
use chrono::Utc;
use sqlx::SqlitePool;

/// See event_context.rs for the rationale. Cap on `IN (?, ?, …)` batch size.
const MAX_BATCH_SIZE: usize = 500;

pub struct MicroeventContext {
    pool: SqlitePool,
}

impl MicroeventContext {
    // ... existing new() method ...
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Admin-facing list of all microevents. Paginated via the shared
    /// `util::validate_pagination` contract on the logic layer. ORDER
    /// BY id DESC so newest entries come first (matches the audit-log
    /// and admin /event list conventions); the prior ORDER BY start_time
    /// is not pagination-stable when start_time is NULL for many rows.
    pub async fn find_all(&self, limit: i64, offset: i64) -> Result<Vec<Microevent>, AppError> {
        let rows = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description,
             start_time, end_time, created_at, updated_at
             FROM microevents
             ORDER BY id DESC
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_by_id(&self, id: i64) -> Result<Microevent, AppError> {
        let row = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description,
             start_time, end_time, created_at, updated_at
             FROM microevents
             WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_by_id_list(&self, input: Vec<i64>) -> Result<Vec<Microevent>, AppError> {
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
            e.id, e.event_id, e.user_id, e.name,  e.archive, e.description,
             e.start_time, e.end_time, e.created_at, e.updated_at
         FROM microevents e
         WHERE e.id IN ({})",
            placeholders
        );

        let mut query = sqlx::query_as::<_, Microevent>(&query_str);
        for id in input {
            query = query.bind(id);
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn find_by_event(&self, event_id: i64) -> Result<Vec<Microevent>, AppError> {
        let rows = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description,
             start_time, end_time, created_at, updated_at
             FROM microevents
             WHERE event_id = ?
             ORDER BY start_time",
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_by_user(&self, user_id: i64) -> Result<Vec<Microevent>, AppError> {
        let rows = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description,
             start_time, end_time, created_at, updated_at
             FROM microevents
             WHERE user_id = ?
             ORDER BY start_time",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn create(&self, microevent: &Microevent) -> Result<i64, AppError> {
        let result = sqlx::query(
            "INSERT INTO microevents (event_id, user_id, name, archive, description, 
             start_time, end_time, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(microevent.event_id)
        .bind(&microevent.user_id)
        .bind(&microevent.name)
        .bind(microevent.archive)
        .bind(&microevent.description)
        .bind(microevent.start_time.map(|dt| dt.to_rfc3339()))
        .bind(microevent.end_time.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn update(&self, id: i64, microevent: &Microevent) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE microevents 
             SET event_id = ?, user_id = ?, name = ?, archive = ?, description = ?,
                 start_time = ?, end_time = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(microevent.event_id)
        .bind(&microevent.user_id)
        .bind(&microevent.name)
        .bind(microevent.archive)
        .bind(&microevent.description)
        .bind(microevent.start_time.map(|dt| dt.to_rfc3339()))
        .bind(microevent.end_time.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM microevents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn archive(&self, id: i64) -> Result<bool, AppError> {
        let result =
            sqlx::query("UPDATE microevents SET archive = true, updated_at = ? WHERE id = ?")
                .bind(Utc::now().to_rfc3339())
                .bind(id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn unarchive(&self, id: i64) -> Result<bool, AppError> {
        let result =
            sqlx::query("UPDATE microevents SET archive = false, updated_at = ? WHERE id = ?")
                .bind(Utc::now().to_rfc3339())
                .bind(id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Sweep microevents whose end time has passed and flip their
    /// `archive` flag. Microevents have no `recurring` concept — each
    /// one is tied to a specific occurrence of a parent event, so once
    /// `end_time < now` it's done and should disappear from the active
    /// list. Returns the number of rows archived.
    ///
    /// Idempotent: re-running the sweep does nothing for already-archived
    /// rows. Microevents with NULL end_time are skipped — TBA microevents
    /// stay visible until their owner fills the time in.
    pub async fn auto_archive_past(&self) -> Result<u64, AppError> {
        let result = sqlx::query(
            "UPDATE microevents \
             SET archive = true, updated_at = ? \
             WHERE archive = false \
               AND end_time IS NOT NULL \
               AND end_time < ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn find_active(&self) -> Result<Vec<Microevent>, AppError> {
        let rows = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description,
             start_time, end_time, created_at, updated_at
             FROM microevents
             WHERE archive = false
             ORDER BY start_time",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests with an in-memory SQLite pool. Run via
    //! `cargo test --package rust_nomad_events_api`.
    //!
    //! This is the canonical example for the context-layer test pattern
    //! the project ROADMAP calls for. The shape — per-test pool +
    //! `setup_pool` helper + per-test seed — should be cloned for the
    //! other contexts as they get their own coverage.

    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
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
        // The migrations enable FK enforcement (`PRAGMA foreign_keys`).
        // Microevents have FKs to `events.id` and `users.id`, so inserting
        // a bare microevent without seeded parents would fail. The
        // context-layer tests target the context's own behavior — they
        // don't need cross-table integrity — so we relax FK checks for
        // these tests only. Cross-table integrity is covered by route /
        // logic-layer tests where the request flow seeds real parents.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement for context tests");
        pool
    }

    fn sample_microevent(name: &str, event_id: i64, user_id: &str) -> Microevent {
        Microevent {
            id: 0,
            event_id,
            user_id: user_id.to_string(),
            name: name.to_string(),
            archive: false,
            description: Some("test description".to_string()),
            start_time: Some(Utc.with_ymd_and_hms(2026, 6, 15, 14, 0, 0).unwrap()),
            end_time: Some(Utc.with_ymd_and_hms(2026, 6, 15, 16, 0, 0).unwrap()),
            created_at: None,
            updated_at: None,
        }
    }

    // -----------------------------------------------------------------
    // create + find_by_id round-trip — the "happy path"
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_then_find_by_id_returns_inserted_row() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let id = ctx
            .create(&sample_microevent("Jousting", 1, "user-1"))
            .await
            .expect("create");
        assert!(id > 0);

        let found = ctx.find_by_id(id).await.expect("find_by_id");
        assert_eq!(found.id, id);
        assert_eq!(found.name, "Jousting");
        assert_eq!(found.event_id, 1);
        assert_eq!(found.user_id, "user-1");
        assert!(!found.archive);
        assert!(found.created_at.is_some());
        assert!(found.updated_at.is_some());
    }

    #[tokio::test]
    async fn find_by_id_missing_row_errors() {
        // `fetch_one` returns `RowNotFound` for a missing id; we expect
        // the propagation to surface that as a DatabaseError. The exact
        // variant matters less than "returns an Err" — callers should
        // handle either DatabaseError or NotFound uniformly.
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let err = ctx.find_by_id(99_999).await;
        assert!(err.is_err());
    }

    // -----------------------------------------------------------------
    // update — happy path + missing-row case
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_modifies_row_and_returns_true() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let id = ctx
            .create(&sample_microevent("Original", 1, "user-1"))
            .await
            .expect("create");

        let mut updated = sample_microevent("New Name", 2, "user-2");
        updated.archive = true;
        updated.description = Some("Updated description".to_string());

        let ok = ctx.update(id, &updated).await.expect("update");
        assert!(ok, "update must report rows_affected > 0");

        let found = ctx.find_by_id(id).await.expect("find");
        assert_eq!(found.name, "New Name");
        assert_eq!(found.event_id, 2);
        assert_eq!(found.user_id, "user-2");
        assert!(found.archive);
        assert_eq!(found.description.as_deref(), Some("Updated description"));
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        // Pinned contract: `update` returns Ok(false) (not Err) when no
        // rows matched. Logic-layer callers use this to distinguish
        // "DB error" from "row didn't exist".
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let ok = ctx
            .update(99_999, &sample_microevent("ghost", 1, "u"))
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
        let ctx = MicroeventContext::new(pool);

        let id = ctx
            .create(&sample_microevent("To Be Deleted", 1, "u"))
            .await
            .expect("create");

        let deleted = ctx.delete(id).await.expect("delete");
        assert!(deleted);

        // Subsequent find errors — row is gone.
        assert!(ctx.find_by_id(id).await.is_err());
    }

    #[tokio::test]
    async fn delete_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let deleted = ctx.delete(99_999).await.expect("delete");
        assert!(!deleted);
    }

    // -----------------------------------------------------------------
    // find_all — pagination + ordering
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_all_returns_newest_first() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let id1 = ctx
            .create(&sample_microevent("First", 1, "u"))
            .await
            .unwrap();
        let id2 = ctx
            .create(&sample_microevent("Second", 1, "u"))
            .await
            .unwrap();
        let id3 = ctx
            .create(&sample_microevent("Third", 1, "u"))
            .await
            .unwrap();

        let all = ctx.find_all(10, 0).await.expect("find_all");
        assert_eq!(all.len(), 3);
        // ORDER BY id DESC — newest inserted comes first.
        assert_eq!(all[0].id, id3);
        assert_eq!(all[1].id, id2);
        assert_eq!(all[2].id, id1);
    }

    #[tokio::test]
    async fn find_all_paginates_via_limit_and_offset() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        for i in 1..=5 {
            ctx.create(&sample_microevent(&format!("M{}", i), 1, "u"))
                .await
                .unwrap();
        }

        // Page 1: 2 rows, newest first (ids 5, 4).
        let p1 = ctx.find_all(2, 0).await.expect("p1");
        assert_eq!(p1.len(), 2);
        assert!(p1[0].name == "M5" && p1[1].name == "M4");

        // Page 2: skip 2 → (ids 3, 2).
        let p2 = ctx.find_all(2, 2).await.expect("p2");
        assert_eq!(p2.len(), 2);
        assert!(p2[0].name == "M3" && p2[1].name == "M2");

        // Tail.
        let p3 = ctx.find_all(2, 4).await.expect("p3");
        assert_eq!(p3.len(), 1);
        assert_eq!(p3[0].name, "M1");
    }

    // -----------------------------------------------------------------
    // find_by_event / find_by_user — filter correctness
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_by_event_returns_only_matching_event() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        ctx.create(&sample_microevent("A", 1, "u")).await.unwrap();
        ctx.create(&sample_microevent("B", 1, "u")).await.unwrap();
        ctx.create(&sample_microevent("C", 2, "u")).await.unwrap();

        let event_1 = ctx.find_by_event(1).await.expect("find");
        assert_eq!(event_1.len(), 2);
        let names: Vec<_> = event_1.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));

        let event_2 = ctx.find_by_event(2).await.expect("find");
        assert_eq!(event_2.len(), 1);
        assert_eq!(event_2[0].name, "C");
    }

    #[tokio::test]
    async fn find_by_event_empty_when_no_matches() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);
        ctx.create(&sample_microevent("A", 1, "u")).await.unwrap();

        let no_matches = ctx.find_by_event(999).await.expect("find");
        assert_eq!(no_matches.len(), 0);
    }

    // -----------------------------------------------------------------
    // get_by_id_list — batch lookup constraints
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_by_id_list_empty_input_returns_empty_no_query() {
        // Empty input must not execute any SQL — the short-circuit
        // documented at the top of get_by_id_list. Pinned because a
        // refactor that always builds the `IN ()` query would generate
        // invalid SQL and 500.
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let result = ctx.get_by_id_list(vec![]).await.expect("empty list");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn get_by_id_list_returns_matching_subset() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);
        let mut ids: Vec<i64> = Vec::new();
        for i in 1..=3 {
            let id = ctx
                .create(&sample_microevent(&format!("M{}", i), 1, "u"))
                .await
                .unwrap();
            ids.push(id);
        }

        // Look up just two of the three.
        let subset = ctx.get_by_id_list(vec![ids[0], ids[2]]).await.expect("get");
        assert_eq!(subset.len(), 2);
        let returned_ids: Vec<i64> = subset.iter().map(|r| r.id).collect();
        assert!(returned_ids.contains(&ids[0]));
        assert!(returned_ids.contains(&ids[2]));
    }

    #[tokio::test]
    async fn get_by_id_list_rejects_oversize() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        // MAX_BATCH_SIZE is 500. 501 should reject as BadRequest before
        // attempting any SQL.
        let oversize: Vec<i64> = (1..=501).collect();
        let err = ctx.get_by_id_list(oversize).await.unwrap_err();
        match err {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("500"), "expected cap mention, got: {}", msg);
            }
            other => panic!("expected BadRequest, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // archive / unarchive / find_active
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn archive_flips_flag() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);
        let id = ctx
            .create(&sample_microevent("Active", 1, "u"))
            .await
            .unwrap();

        let archived = ctx.archive(id).await.expect("archive");
        assert!(archived);

        let found = ctx.find_by_id(id).await.expect("find");
        assert!(found.archive);
    }

    #[tokio::test]
    async fn unarchive_clears_flag() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);
        let id = ctx.create(&sample_microevent("X", 1, "u")).await.unwrap();
        ctx.archive(id).await.expect("archive");

        let unarchived = ctx.unarchive(id).await.expect("unarchive");
        assert!(unarchived);
        assert!(!ctx.find_by_id(id).await.unwrap().archive);
    }

    #[tokio::test]
    async fn find_active_excludes_archived() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);
        let a = ctx.create(&sample_microevent("A", 1, "u")).await.unwrap();
        let b = ctx.create(&sample_microevent("B", 1, "u")).await.unwrap();
        ctx.archive(a).await.expect("archive");

        let active = ctx.find_active().await.expect("find_active");
        let ids: Vec<i64> = active.iter().map(|r| r.id).collect();
        assert!(!ids.contains(&a), "archived row leaked");
        assert!(ids.contains(&b), "active row missing");
    }

    // -----------------------------------------------------------------
    // auto_archive_past — the sweep that retires past microevents
    // -----------------------------------------------------------------

    /// Build a microevent with explicit timestamps for testing the
    /// auto-archive sweep. The standard `sample_microevent` helper
    /// hardcodes a 2026 time which is the wrong era for the past/future
    /// distinction we're pinning here.
    fn microevent_with_times(
        name: &str,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Microevent {
        Microevent {
            id: 0,
            event_id: 1,
            user_id: "user-1".to_string(),
            name: name.to_string(),
            archive: false,
            // `description` is NOT NULL in the schema — pass a placeholder
            // so the INSERT doesn't 1299 on the unrelated constraint.
            description: Some("test microevent".to_string()),
            start_time: start,
            end_time: end,
            created_at: None,
            updated_at: None,
        }
    }

    #[tokio::test]
    async fn auto_archive_past_targets_only_past_microevents() {
        let pool = setup_pool().await;
        let ctx = MicroeventContext::new(pool);

        let past = ctx
            .create(&microevent_with_times(
                "Past Slot",
                Some(Utc.with_ymd_and_hms(2020, 6, 15, 14, 0, 0).unwrap()),
                Some(Utc.with_ymd_and_hms(2020, 6, 15, 16, 0, 0).unwrap()),
            ))
            .await
            .unwrap();
        let future = ctx
            .create(&microevent_with_times(
                "Future Slot",
                Some(Utc.with_ymd_and_hms(2099, 8, 1, 10, 0, 0).unwrap()),
                Some(Utc.with_ymd_and_hms(2099, 8, 1, 12, 0, 0).unwrap()),
            ))
            .await
            .unwrap();
        let tba = ctx
            .create(&microevent_with_times("TBA Slot", None, None))
            .await
            .unwrap();

        let n = ctx.auto_archive_past().await.expect("auto-archive");
        assert_eq!(n, 1, "only the past microevent should be archived");

        assert!(
            ctx.find_by_id(past).await.unwrap().archive,
            "past microevent must be archived"
        );
        assert!(
            !ctx.find_by_id(future).await.unwrap().archive,
            "future microevent must stay active"
        );
        assert!(
            !ctx.find_by_id(tba).await.unwrap().archive,
            "TBA microevent (NULL end_time) must stay active"
        );

        // Idempotent: re-sweep is a no-op.
        let again = ctx.auto_archive_past().await.expect("second sweep");
        assert_eq!(again, 0);
    }
}
