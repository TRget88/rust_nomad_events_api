// ============================================================================
// src/repositories/event_repository.rs - Data Access Layer
// ============================================================================
use sqlx::SqlitePool;
use crate::models::event_models::NomEvent;
use crate::errors::AppError;

#[derive(sqlx::FromRow)]
pub struct EventRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub website: Option<String>,
    pub event_type: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub camping_allowed: Option<bool>,
    pub event_data: String,
}

pub struct EventRepository {
    pool: SqlitePool,
}

impl EventRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn find_all(&self) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT id, name, description, website, event_type, latitude, longitude, 
             start_date, end_date, camping_allowed, event_data FROM events"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_by_id(&self, id: i64) -> Result<EventRow, AppError> {
        let row = sqlx::query_as::<_, EventRow>(
            "SELECT id, name, description, website, event_type, latitude, longitude, 
             start_date, end_date, camping_allowed, event_data FROM events WHERE id = ?"
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn find_by_type(&self, event_type: &str) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT id, name, description, website, event_type, latitude, longitude, 
             start_date, end_date, camping_allowed, event_data FROM events 
             WHERE event_type = ?"
        )
        .bind(event_type)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_nearby(
        &self, 
        lat: f64, 
        lon: f64, 
        radius_miles: f64
    ) -> Result<Vec<EventRow>, AppError> {
        let query = r#"
            SELECT id, name, description, website, event_type, latitude, longitude, 
                   start_date, end_date, camping_allowed, event_data,
                   (3959 * acos(cos(radians(?)) * cos(radians(latitude)) * 
                    cos(radians(longitude) - radians(?)) + sin(radians(?)) * 
                    sin(radians(latitude)))) AS distance
            FROM events
            WHERE latitude IS NOT NULL AND longitude IS NOT NULL
            HAVING distance < ?
            ORDER BY distance
        "#;

        let rows = sqlx::query_as::<_, EventRow>(query)
            .bind(lat)
            .bind(lon)
            .bind(lat)
            .bind(radius_miles)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    pub async fn create(&self, event: &NomEvent) -> Result<i64, AppError> {
        let event_json = serde_json::to_string(event)?;

        let result = sqlx::query(
            "INSERT INTO events (name, description, website, event_type, latitude, longitude, 
             start_date, end_date, camping_allowed, event_data) 
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&event.name)
        .bind(&event.description)
        .bind(&event.website)
        .bind(&event.event_type.name)
        .bind(event.location_info.latitude)
        .bind(event.location_info.longitude)
        .bind(event.date_info.start_date.to_string())
        .bind(event.date_info.end_date.map(|d| d.to_string()))
        .bind(event.camping_info.as_ref().map(|c| c.camping_allowed).unwrap_or(false))
        .bind(&event_json)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn update(&self, id: i64, event: &NomEvent) -> Result<bool, AppError> {
        let event_json = serde_json::to_string(event)?;

        let result = sqlx::query(
            "UPDATE events SET name = ?, description = ?, website = ?, event_type = ?, 
             latitude = ?, longitude = ?, start_date = ?, end_date = ?, camping_allowed = ?, 
             event_data = ? WHERE id = ?"
        )
        .bind(&event.name)
        .bind(&event.description)
        .bind(&event.website)
        .bind(&event.event_type.name)
        .bind(event.location_info.latitude)
        .bind(event.location_info.longitude)
        .bind(event.date_info.start_date.to_string())
        .bind(event.date_info.end_date.map(|d| d.to_string()))
        .bind(event.camping_info.as_ref().map(|c| c.camping_allowed).unwrap_or(false))
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
}