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
        .bind(&id)
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
