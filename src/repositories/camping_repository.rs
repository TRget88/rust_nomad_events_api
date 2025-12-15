// ============================================================================
// Repository: src/repositories/camping_repository.rs
// ============================================================================

use sqlx::SqlitePool;
use crate::models::event_models::CampingProfile;
use crate::errors::AppError;

#[derive(sqlx::FromRow)]
pub struct CampingProfileRow {
    pub id: i64,
    pub profile_name: String,
    pub description: Option<String>,
    pub camping_data: String,
}

pub struct CampingRepository {
    pool: SqlitePool,
}

impl CampingRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn find_all(&self) -> Result<Vec<CampingProfile>, AppError> {
        let rows = sqlx::query_as::<_, CampingProfileRow>(
            "SELECT id, profile_name, description, camping_data FROM camping_profiles ORDER BY profile_name"
        )
        .fetch_all(&self.pool)
        .await?;

        let profiles: Vec<CampingProfile> = rows
            .into_iter()
            .filter_map(|row| {
                let mut profile: CampingProfile = serde_json::from_str(&row.camping_data).ok()?;
                profile.id = Some(row.id);
                Some(profile)
            })
            .collect();

        Ok(profiles)
    }

    pub async fn find_by_id(&self, id: i64) -> Result<CampingProfile, AppError> {
        let row = sqlx::query_as::<_, CampingProfileRow>(
            "SELECT id, profile_name, description, camping_data FROM camping_profiles WHERE id = ?"
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        let mut profile: CampingProfile = serde_json::from_str(&row.camping_data)?;
        profile.id = Some(row.id);
        Ok(profile)
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<CampingProfile>, AppError> {
        let result = sqlx::query_as::<_, CampingProfileRow>(
            "SELECT id, profile_name, description, camping_data FROM camping_profiles WHERE profile_name = ?"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        match result {
            Some(row) => {
                let mut profile: CampingProfile = serde_json::from_str(&row.camping_data)?;
                profile.id = Some(row.id);
                Ok(Some(profile))
            }
            None => Ok(None),
        }
    }

    pub async fn create(&self, profile: &CampingProfile) -> Result<i64, AppError> {
        let camping_json = serde_json::to_string(profile)?;

        let result = sqlx::query(
            "INSERT INTO camping_profiles (profile_name, description, camping_data) VALUES (?, ?, ?)"
        )
        .bind(&profile.profile_name)
        .bind(&profile.description)
        .bind(&camping_json)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn update(&self, id: i64, profile: &CampingProfile) -> Result<bool, AppError> {
        let camping_json = serde_json::to_string(profile)?;

        let result = sqlx::query(
            "UPDATE camping_profiles SET profile_name = ?, description = ?, camping_data = ? WHERE id = ?"
        )
        .bind(&profile.profile_name)
        .bind(&profile.description)
        .bind(&camping_json)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(&self, id: i64) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM camping_profiles WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}