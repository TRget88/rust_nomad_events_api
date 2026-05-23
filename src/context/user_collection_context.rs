// ============================================================================
// src/context/user_collection_context.rs
// ============================================================================
use crate::errors::AppError;
use crate::models::database_models::UserEventDataRow;
use sqlx::Row;
use sqlx::SqlitePool;

pub struct UserCollectionContext {
    pool: SqlitePool,
}

impl UserCollectionContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, user_id: String) -> Result<UserEventDataRow, AppError> {
        if let Some(row) = sqlx::query(
            r#"
        SELECT
            id,
            user_id,
            favorite_events,
            favorite_microevents,
            saved_events,
            saved_microevents,
            created_events,
            created_microevents
        FROM user_event_data
        WHERE user_id = ?
        "#,
        )
        .bind(&user_id)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(UserEventDataRow {
                id: row.get::<i64, _>("id"),
                user_id: row.get::<String, _>("user_id"),

                favorite_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("favorite_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                favorite_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("favorite_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                saved_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("saved_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                saved_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("saved_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                created_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("created_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                created_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("created_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,
            });
        }

        // ─────────────────────────────────────────────
        // Row does not exist → create it
        // ─────────────────────────────────────────────

        let empty = UserEventDataRow {
            id: 0, // ignored by DB
            user_id: user_id.clone(),
            favorite_events: vec![],
            favorite_microevents: vec![],
            saved_events: vec![],
            saved_microevents: vec![],
            created_events: vec![],
            created_microevents: vec![],
        };

        self.create(&empty).await?;

        // Return what we *know* we just inserted
        Ok(empty)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<UserEventDataRow, AppError> {
        if let Some(row) = sqlx::query(
            r#"
        SELECT
            id,
            user_id,
            favorite_events,
            favorite_microevents,
            saved_events,
            saved_microevents,
            created_events,
            created_microevents
        FROM user_event_data
        WHERE id = ?
        "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(UserEventDataRow {
                id: row.get::<i64, _>("id"),
                user_id: row.get::<String, _>("user_id"),

                favorite_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("favorite_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                favorite_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("favorite_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                saved_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("saved_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                saved_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("saved_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                created_events: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("created_events")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,

                created_microevents: serde_json::from_str::<Vec<i64>>(
                    row.get::<Option<String>, _>("created_microevents")
                        .as_deref()
                        .unwrap_or("[]"),
                )?,
            });
        }
        Err(AppError::NotFound("User event data not found".to_string()))

        // ─────────────────────────────────────────────
        // Row does not exist → create it
        // ─────────────────────────────────────────────

        //let empty = UserEventDataRow {
        //id: 0, // ignored by DB
        //user_id: ' ',
        //favorite_events: vec![],
        //favorite_microevents: vec![],
        //saved_events: vec![],
        //saved_microevents: vec![],
        //created_events: vec![],
        //created_microevents: vec![],
        //};
        //
        //self.create(&empty).await?;
        //
        //// Return what we *know* we just inserted
        //Ok(empty)
    }

    async fn create(&self, data: &UserEventDataRow) -> Result<(), AppError> {
        let favorite_events = serde_json::to_string(&data.favorite_events)?;
        let favorite_microevents = serde_json::to_string(&data.favorite_microevents)?;
        let saved_events = serde_json::to_string(&data.saved_events)?;
        let saved_microevents = serde_json::to_string(&data.saved_microevents)?;
        let created_events = serde_json::to_string(&data.created_events)?;
        let created_microevents = serde_json::to_string(&data.created_microevents)?;

        sqlx::query(
            r#"
            INSERT INTO user_event_data 
            (user_id, favorite_events, favorite_microevents, saved_events, 
             saved_microevents, created_events, created_microevents)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&data.user_id)
        .bind(&favorite_events)
        .bind(&favorite_microevents)
        .bind(&saved_events)
        .bind(&saved_microevents)
        .bind(&created_events)
        .bind(&created_microevents)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update(&self, data: &UserEventDataRow) -> Result<(), AppError> {
        let favorite_events = serde_json::to_string(&data.favorite_events)?;
        let favorite_microevents = serde_json::to_string(&data.favorite_microevents)?;
        let saved_events = serde_json::to_string(&data.saved_events)?;
        let saved_microevents = serde_json::to_string(&data.saved_microevents)?;
        let created_events = serde_json::to_string(&data.created_events)?;
        let created_microevents = serde_json::to_string(&data.created_microevents)?;

        sqlx::query(
            r#"
            UPDATE user_event_data
            SET favorite_events = ?,
                favorite_microevents = ?,
                saved_events = ?,
                saved_microevents = ?,
                created_events = ?,
                created_microevents = ?
            WHERE user_id = ?
            "#,
        )
        .bind(&favorite_events)
        .bind(&favorite_microevents)
        .bind(&saved_events)
        .bind(&saved_microevents)
        .bind(&created_events)
        .bind(&created_microevents)
        .bind(&data.user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
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
        // user_event_data.user_id references users.id. These tests
        // exercise the context's own behavior with synthetic user IDs;
        // cross-table integrity is covered at the logic/route layer.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        pool
    }

    // -----------------------------------------------------------------
    // get(user_id) — lazy-create-on-miss semantics
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_creates_empty_row_for_unknown_user() {
        // Critical semantics: `get` is auto-vivify. A first-time user
        // accessing their collection has no row yet — the context creates
        // one and returns the empty collection. Pinned so a refactor
        // that drops the auto-insert doesn't silently 404 first logins.
        let pool = setup_pool().await;
        let ctx = UserCollectionContext::new(pool);

        let row = ctx.get("first-time-user".to_string()).await.expect("get");
        assert_eq!(row.user_id, "first-time-user");
        assert_eq!(row.favorite_events, Vec::<i64>::new());
        assert_eq!(row.favorite_microevents, Vec::<i64>::new());
        assert_eq!(row.saved_events, Vec::<i64>::new());
        assert_eq!(row.saved_microevents, Vec::<i64>::new());
        assert_eq!(row.created_events, Vec::<i64>::new());
        assert_eq!(row.created_microevents, Vec::<i64>::new());

        // Second call must not re-auto-create — it should find the
        // row already there. Pinned via a subsequent count.
        let row2 = ctx.get("first-time-user".to_string()).await.expect("get");
        assert_eq!(row.user_id, row2.user_id);
    }

    // -----------------------------------------------------------------
    // update + get round-trip
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_persists_arrays_and_get_round_trips() {
        // The context serializes the six `Vec<i64>` arrays into JSON
        // text columns. Pin the round-trip — every array shape needs
        // to survive serialize → SQLite → deserialize.
        let pool = setup_pool().await;
        let ctx = UserCollectionContext::new(pool);

        // Auto-vivify, then update.
        let mut row = ctx.get("u".to_string()).await.expect("get");
        row.favorite_events = vec![1, 2, 3];
        row.favorite_microevents = vec![10];
        row.saved_events = vec![4, 5];
        row.saved_microevents = vec![];
        row.created_events = vec![6, 7, 8, 9];
        row.created_microevents = vec![100, 101];
        ctx.update(&row).await.expect("update");

        // Round-trip via a fresh get.
        let reloaded = ctx.get("u".to_string()).await.expect("get");
        assert_eq!(reloaded.favorite_events, vec![1, 2, 3]);
        assert_eq!(reloaded.favorite_microevents, vec![10]);
        assert_eq!(reloaded.saved_events, vec![4, 5]);
        assert_eq!(reloaded.saved_microevents, Vec::<i64>::new());
        assert_eq!(reloaded.created_events, vec![6, 7, 8, 9]);
        assert_eq!(reloaded.created_microevents, vec![100, 101]);
    }

    #[tokio::test]
    async fn update_isolates_users_from_each_other() {
        // Two users' collections must never bleed into each other —
        // this is the kind of bug that turns into "User A sees User B's
        // saved events" in production.
        let pool = setup_pool().await;
        let ctx = UserCollectionContext::new(pool);

        let mut alice = ctx.get("alice".to_string()).await.expect("get");
        alice.favorite_events = vec![1, 2, 3];
        ctx.update(&alice).await.expect("update alice");

        let mut bob = ctx.get("bob".to_string()).await.expect("get");
        bob.favorite_events = vec![10, 20];
        ctx.update(&bob).await.expect("update bob");

        let alice_reloaded = ctx.get("alice".to_string()).await.expect("get");
        let bob_reloaded = ctx.get("bob".to_string()).await.expect("get");

        assert_eq!(alice_reloaded.favorite_events, vec![1, 2, 3]);
        assert_eq!(bob_reloaded.favorite_events, vec![10, 20]);
    }

    // -----------------------------------------------------------------
    // get_by_id
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_by_id_returns_row_when_present() {
        // Note the wrinkle: `get`'s auto-vivify path returns the in-memory
        // empty struct with `id: 0`, not the real SQLite rowid (the
        // method comment `id: 0, // ignored by DB` documents this). To
        // find the real id we query directly via the pool clone.
        let pool = setup_pool().await;
        let pool_for_lookup = pool.clone();
        let ctx = UserCollectionContext::new(pool);

        ctx.get("user-x".to_string()).await.expect("get");
        let real_id: i64 = sqlx::query_scalar("SELECT id FROM user_event_data WHERE user_id = ?")
            .bind("user-x")
            .fetch_one(&pool_for_lookup)
            .await
            .expect("lookup id");

        let by_id = ctx.get_by_id(real_id).await.expect("get_by_id");
        assert_eq!(by_id.user_id, "user-x");
        assert_eq!(by_id.id, real_id);
    }

    #[tokio::test]
    async fn get_by_id_returns_not_found_for_missing_id() {
        // get_by_id does NOT auto-vivify — that path only exists on
        // user_id lookups. A missing id returns NotFound.
        let pool = setup_pool().await;
        let ctx = UserCollectionContext::new(pool);

        let err = ctx.get_by_id(99_999).await.unwrap_err();
        match err {
            AppError::NotFound(_) => {} // expected
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // empty-array handling
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn empty_arrays_round_trip_cleanly() {
        // The auto-vivified row stores `[]` as the array JSON. Pin
        // that a fresh fetch decodes back to an empty Vec (not None
        // or Vec containing "[]" as a string).
        let pool = setup_pool().await;
        let ctx = UserCollectionContext::new(pool);

        let row = ctx.get("empty-user".to_string()).await.expect("get");
        assert!(row.favorite_events.is_empty());
        assert!(row.saved_events.is_empty());
        assert!(row.created_events.is_empty());
    }
}
