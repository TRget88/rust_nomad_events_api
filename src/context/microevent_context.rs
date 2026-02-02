// src/context/event_context.rs

use crate::errors::AppError;
use crate::models::microevents_models::Microevent;
use chrono::Utc;
use sqlx::SqlitePool;

pub struct MicroeventContext {
    pool: SqlitePool,
}

impl MicroeventContext {
    // ... existing new() method ...
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn find_all(&self) -> Result<Vec<Microevent>, AppError> {
        let rows = sqlx::query_as::<_, Microevent>(
            "SELECT id, event_id, user_id, name, archive, description, 
             start_time, end_time, created_at, updated_at
             FROM microevents
             ORDER BY start_time",
        )
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
        // Convert Vec to comma-separated string for SQL IN clause
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

        // Bind each id individually
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
