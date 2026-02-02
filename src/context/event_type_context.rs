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
