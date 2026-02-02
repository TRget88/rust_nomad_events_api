// src/context/event_context.rs

use crate::errors::AppError;
use crate::models::database_models::EventRow;
use crate::models::event_models::NomEvent;
use sqlx::SqlitePool;

pub struct EventContext {
    pool: SqlitePool,
}

impl EventContext {
    // ... existing new() method ...
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn find_all(&self) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT 
                e.id, e.name, e.description, e.website, e.event_type_id, 
                e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
                et.name as event_type_name,
                et.description as event_type_description,
                et.map_indicator as event_type_map_indicator,
                et.category as event_type_category
             FROM events e
             JOIN event_types et ON e.event_type_id = et.id",
        )
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
        // Convert Vec to comma-separated string for SQL IN clause
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

        // Bind each id individually
        for id in input {
            query = query.bind(id);
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn find_by_type(&self, event_type_id: i64) -> Result<Vec<EventRow>, AppError> {
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT 
                e.id, e.name, e.description, e.website, e.event_type_id,
                e.latitude, e.longitude, e.start_date, e.end_date, e.camping_allowed, e.event_data,
                et.name as event_type_name,
                et.description as event_type_description,
                et.map_indicator as event_type_map_indicator,
                et.category as event_type_category
             FROM events e
             JOIN event_types et ON e.event_type_id = et.id
             WHERE e.event_type_id = ?",
        )
        .bind(event_type_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_nearby(
        &self,
        lat: f64,
        lon: f64,
        radius_miles: f64,
    ) -> Result<Vec<EventRow>, AppError> {
        // Convert miles to degrees (rough approximation)
        // 1 degree latitude ≈ 69 miles
        // 1 degree longitude varies by latitude, but we'll use a simple approximation
        let lat_delta = radius_miles / 69.0;
        let lon_delta = radius_miles / (69.0 * f64::cos(lat.to_radians()));

        let min_lat = lat - lat_delta;
        let max_lat = lat + lat_delta;
        let min_lon = lon - lon_delta;
        let max_lon = lon + lon_delta;

        let query = r#"
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
        ORDER BY e.name
    "#;

        let rows = sqlx::query_as::<_, EventRow>(query)
            .bind(min_lat)
            .bind(max_lat)
            .bind(min_lon)
            .bind(max_lon)
            .fetch_all(&self.pool)
            .await?;

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
}
