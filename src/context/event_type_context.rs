// ============================================================================
// Repository: src/context/event_type_repository.rs
// ============================================================================

use crate::errors::AppError;
use crate::models::database_models::EventTypeRow;
use crate::models::event_models::EventType;
use sqlx::SqlitePool;

pub struct EventTypeContext {
    pool: SqlitePool,
}

impl EventTypeContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn find_all(&self) -> Result<Vec<EventType>, AppError> {
        let rows = sqlx::query_as::<_, EventTypeRow>(
            "SELECT id, name, description, map_indicator, category FROM event_types",
        )
        .fetch_all(&self.pool)
        .await?;

        let types: Vec<EventType> = rows
            .into_iter()
            .map(|row| EventType {
                id: Some(row.id),
                name: row.name,
                description: row.description,
                map_indicator: row.map_indicator,
                category: row.category,
            })
            .collect();

        Ok(types)
    }

    pub async fn find_by_id(&self, id: i64) -> Result<EventType, AppError> {
        let row = sqlx::query_as::<_, EventTypeRow>(
            "SELECT id, name, description, map_indicator, category FROM event_types WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(EventType {
            id: Some(row.id),
            name: row.name,
            description: row.description,
            map_indicator: row.map_indicator,
            category: row.category,
        })
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<EventType>, AppError> {
        let result = sqlx::query_as::<_, EventTypeRow>(
            "SELECT id, name, description, map_indicator, category FROM event_types WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|row| EventType {
            id: Some(row.id),
            name: row.name,
            description: row.description,
            map_indicator: row.map_indicator,
            category: row.category,
        }))
    }

    pub async fn find_by_category(&self, category: &str) -> Result<Vec<EventType>, AppError> {
        let rows = sqlx::query_as::<_, EventTypeRow>(
            "SELECT id, name, description, map_indicator, category FROM event_types WHERE category = ?"
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await?;

        let types: Vec<EventType> = rows
            .into_iter()
            .map(|row| EventType {
                id: Some(row.id),
                name: row.name,
                description: row.description,
                map_indicator: row.map_indicator,
                category: row.category,
            })
            .collect();

        Ok(types)
    }

    pub async fn create(&self, event_type: &EventType) -> Result<i64, AppError> {
        let result = sqlx::query(
            "INSERT INTO event_types (name, description, map_indicator, category) VALUES (?, ?, ?, ?)"
        )
        .bind(&event_type.name)
        .bind(&event_type.description)
        .bind(&event_type.map_indicator)
        .bind(&event_type.category)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn update(&self, id: i64, event_type: &EventType) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE event_types SET name = ?, description = ?, map_indicator = ?, category = ? WHERE id = ?"
        )
        .bind(&event_type.name)
        .bind(&event_type.description)
        .bind(&event_type.map_indicator)
        .bind(&event_type.category)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM event_types WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool.
    //! See `microevent_context::tests` for the canonical pattern.

    use super::*;
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
        // The `event_types` table has no outgoing FKs — incoming FKs
        // (events.event_type_id) only matter if a test inserts events,
        // which these don't. The PRAGMA is left off here as a
        // documentation point: not every context needs it.
        pool
    }

    /// Pool with `foreign_keys=ON` so the BEFORE DELETE triggers added
    /// in migration 00007 (reparent + sentinel protection) fire under
    /// the same FK regime as production. Use this for trigger tests
    /// only — the rest of the tests don't need FK enforcement.
    async fn setup_pool_with_fk() -> sqlx::SqlitePool {
        use sqlx::sqlite::SqliteConnectOptions;
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("parse url")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite with FK");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    /// Look up the sentinel id once per test rather than hardcoding.
    /// The sentinel is inserted by migration 00007 via INSERT OR
    /// IGNORE, so its rowid depends on insert order vs the seed
    /// types — never assume a particular value.
    async fn sentinel_id(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM event_types WHERE name = 'Uncategorized'",
        )
        .fetch_one(pool)
        .await
        .expect("sentinel exists")
    }

    fn sample_event_type(name: &str, category: &str) -> EventType {
        EventType {
            id: None,
            name: name.to_string(),
            description: format!("{} description", name),
            map_indicator: "📍".to_string(),
            category: category.to_string(),
        }
    }

    // -----------------------------------------------------------------
    // create + find_by_id round-trip
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_then_find_by_id_returns_inserted_row() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let id = ctx
            .create(&sample_event_type("Music Festival", "entertainment"))
            .await
            .expect("create");
        assert!(id > 0);

        let found = ctx.find_by_id(id).await.expect("find_by_id");
        assert_eq!(found.id, Some(id));
        assert_eq!(found.name, "Music Festival");
        assert_eq!(found.category, "entertainment");
    }

    #[tokio::test]
    async fn find_by_id_missing_row_errors() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        assert!(ctx.find_by_id(99_999).await.is_err());
    }

    // -----------------------------------------------------------------
    // update
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_modifies_row_and_returns_true() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let id = ctx
            .create(&sample_event_type("Old Name", "old-category"))
            .await
            .unwrap();

        let updated_input = sample_event_type("New Name", "new-category");
        let ok = ctx.update(id, &updated_input).await.expect("update");
        assert!(ok);

        let found = ctx.find_by_id(id).await.expect("find");
        assert_eq!(found.name, "New Name");
        assert_eq!(found.category, "new-category");
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let ok = ctx
            .update(99_999, &sample_event_type("ghost", "none"))
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
        let ctx = EventTypeContext::new(pool);

        let id = ctx
            .create(&sample_event_type("To Delete", "entertainment"))
            .await
            .unwrap();
        let deleted = ctx.delete(id).await.expect("delete");
        assert!(deleted);
        assert!(ctx.find_by_id(id).await.is_err());
    }

    #[tokio::test]
    async fn delete_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let deleted = ctx.delete(99_999).await.expect("delete");
        assert!(!deleted);
    }

    // -----------------------------------------------------------------
    // find_all
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_all_returns_every_row() {
        // event_types are catalog data — the table is small and unsorted
        // by the query. Pin that find_all returns every row, regardless
        // of order. Migration 00007 seeds the "Uncategorized" sentinel
        // unconditionally, so the count includes it (3 inserted + 1
        // sentinel = 4 total).
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let id1 = ctx
            .create(&sample_event_type("Music", "entertainment"))
            .await
            .unwrap();
        let id2 = ctx
            .create(&sample_event_type("Food", "food"))
            .await
            .unwrap();
        let id3 = ctx
            .create(&sample_event_type("Sports", "sports"))
            .await
            .unwrap();

        let all = ctx.find_all().await.expect("find_all");
        assert_eq!(all.len(), 4);
        let ids: Vec<i64> = all.iter().filter_map(|t| t.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
        // Pin: sentinel is present and discoverable by name.
        let names: Vec<_> = all.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Uncategorized"));
    }

    // -----------------------------------------------------------------
    // find_by_name — Option semantics
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_by_name_returns_some_for_existing() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);
        ctx.create(&sample_event_type("Renaissance Faire", "entertainment"))
            .await
            .unwrap();

        let found = ctx
            .find_by_name("Renaissance Faire")
            .await
            .expect("find_by_name");
        assert!(found.is_some(), "name lookup should find the row");
        assert_eq!(found.unwrap().category, "entertainment");
    }

    #[tokio::test]
    async fn find_by_name_returns_none_for_missing() {
        // The Option return distinguishes "not found" from "DB error".
        // Pinned: missing name is None, not Err.
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);

        let found = ctx
            .find_by_name("Nonexistent Type")
            .await
            .expect("find_by_name");
        assert!(found.is_none());
    }

    // -----------------------------------------------------------------
    // find_by_category
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_by_category_returns_matching_subset() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);
        ctx.create(&sample_event_type("Music", "entertainment"))
            .await
            .unwrap();
        ctx.create(&sample_event_type("Comedy", "entertainment"))
            .await
            .unwrap();
        ctx.create(&sample_event_type("Marathon", "sports"))
            .await
            .unwrap();

        let entertainment = ctx.find_by_category("entertainment").await.expect("find");
        assert_eq!(entertainment.len(), 2);
        let names: Vec<_> = entertainment.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Music"));
        assert!(names.contains(&"Comedy"));

        let sports = ctx.find_by_category("sports").await.expect("find");
        assert_eq!(sports.len(), 1);
        assert_eq!(sports[0].name, "Marathon");
    }

    #[tokio::test]
    async fn find_by_category_empty_when_no_matches() {
        let pool = setup_pool().await;
        let ctx = EventTypeContext::new(pool);
        ctx.create(&sample_event_type("Music", "entertainment"))
            .await
            .unwrap();

        let no_matches = ctx
            .find_by_category("unknown-category")
            .await
            .expect("find");
        assert_eq!(no_matches.len(), 0);
    }

    // -----------------------------------------------------------------
    // Migration 00007 — sentinel + reparenting trigger
    //
    // These tests run with `foreign_keys=ON` so the production FK
    // regime is exercised. They pin:
    //   1. The sentinel ("Uncategorized") exists after migrations run
    //   2. Deleting a non-sentinel event_type reparents its events to
    //      the sentinel rather than failing the ON DELETE RESTRICT
    //      check or orphaning rows
    //   3. The sentinel itself cannot be deleted — the trigger
    //      aborts with the documented message
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn sentinel_exists_after_migration() {
        let pool = setup_pool_with_fk().await;
        let sid = sentinel_id(&pool).await;
        assert!(sid > 0);

        // Pin metadata so the seed shape doesn't silently drift.
        let row = sqlx::query_as::<_, EventTypeRow>(
            "SELECT id, name, description, map_indicator, category FROM event_types WHERE id = ?",
        )
        .bind(sid)
        .fetch_one(&pool)
        .await
        .expect("fetch sentinel");
        assert_eq!(row.name, "Uncategorized");
        assert_eq!(row.category, "general");
    }

    #[tokio::test]
    async fn deleting_event_type_reparents_dependent_events_to_sentinel() {
        // The scenario: an admin deletes the "Music Festival" type
        // while two events still reference it. Pre-trigger, the
        // ON DELETE RESTRICT FK would block the DELETE; post-trigger,
        // the events re-parent to the sentinel and the DELETE
        // succeeds.
        let pool = setup_pool_with_fk().await;
        let ctx = EventTypeContext::new(pool.clone());
        let sid = sentinel_id(&pool).await;

        let music_id = ctx
            .create(&sample_event_type("Music Festival", "entertainment"))
            .await
            .expect("create music");

        // Insert two events referencing the "Music Festival" type
        // directly through the raw pool — we don't have an event
        // context here, but the schema only needs the FK + the
        // non-nullable columns.
        for n in 1..=2 {
            sqlx::query(
                "INSERT INTO events (name, description, event_type_id, event_data) \
                 VALUES (?, ?, ?, '{}')",
            )
            .bind(format!("Event {}", n))
            .bind("desc")
            .bind(music_id)
            .execute(&pool)
            .await
            .expect("insert event");
        }

        // Sanity: both events currently point at Music.
        let before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE event_type_id = ?",
        )
        .bind(music_id)
        .fetch_one(&pool)
        .await
        .expect("count before");
        assert_eq!(before, 2);

        // Delete the Music type — should succeed because the
        // BEFORE DELETE trigger reparents both events first.
        let deleted = ctx.delete(music_id).await.expect("delete music");
        assert!(deleted);

        // Music gone, sentinel now owns both events.
        let after_music: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE event_type_id = ?",
        )
        .bind(music_id)
        .fetch_one(&pool)
        .await
        .expect("count after");
        assert_eq!(after_music, 0);

        let after_sentinel: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE event_type_id = ?",
        )
        .bind(sid)
        .fetch_one(&pool)
        .await
        .expect("count sentinel");
        assert_eq!(after_sentinel, 2);
    }

    #[tokio::test]
    async fn deleting_sentinel_event_type_is_forbidden() {
        // The protective trigger aborts the DELETE with a RAISE.
        // Pin: the sentinel still exists afterward, and the error
        // surfaces (we don't care about the exact message — only
        // that it failed).
        let pool = setup_pool_with_fk().await;
        let ctx = EventTypeContext::new(pool.clone());
        let sid = sentinel_id(&pool).await;

        let result = ctx.delete(sid).await;
        assert!(result.is_err(), "deleting the sentinel must fail");

        // Sentinel still there.
        let still_there: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM event_types WHERE name = 'Uncategorized'",
        )
        .fetch_one(&pool)
        .await
        .expect("count sentinel");
        assert_eq!(still_there, 1);
    }
}
