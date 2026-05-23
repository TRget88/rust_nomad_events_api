// ============================================================================
// Repository: src/repositories/camping_repository.rs
// ============================================================================

use crate::errors::AppError;
use crate::models::database_models::CampingProfileRow;
use crate::models::event_models::CampingProfile;
use sqlx::SqlitePool;

pub struct CampingProfileContext {
    pool: SqlitePool,
}

impl CampingProfileContext {
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
            "SELECT id, profile_name, description, camping_data FROM camping_profiles WHERE id = ?",
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

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool.
    //! See `microevent_context::tests` for the canonical pattern.

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
        // camping_profiles has no FK constraints to worry about for
        // these tests — left at default (FKs on) to confirm nothing
        // surprising lives in the schema.
        pool
    }

    /// Build a CampingProfile via JSON so `#[serde(default)]` fills
    /// the nested structs (`rv_camping`, `vehicle_camping`, etc.)
    /// without listing every boolean by hand.
    fn sample_profile(name: &str, camping_allowed: bool) -> CampingProfile {
        serde_json::from_value(json!({
            "profile_name": name,
            "description": format!("{} description", name),
            "camping_allowed": camping_allowed,
            "tent_camping": camping_allowed,
        }))
        .expect("construct sample profile")
    }

    // -----------------------------------------------------------------
    // create + find_by_id round-trip
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_then_find_by_id_returns_inserted_row() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let id = ctx
            .create(&sample_profile("Music Festival Camping", true))
            .await
            .expect("create");
        assert!(id > 0);

        let found = ctx.find_by_id(id).await.expect("find_by_id");
        assert_eq!(found.id, Some(id));
        assert_eq!(found.profile_name, "Music Festival Camping");
        assert!(found.camping_allowed);
        assert!(found.tent_camping);
    }

    #[tokio::test]
    async fn create_round_trips_nested_camping_data_via_json() {
        // Critical: the context stores the full profile in `camping_data`
        // as serialized JSON, then deserializes it back. Pin that the
        // nested `rv_camping` / `vehicle_camping` structs survive the
        // round-trip when explicit fields are populated.
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let profile: CampingProfile = serde_json::from_value(json!({
            "profile_name": "RV-Heavy",
            "camping_allowed": true,
            "rv_camping": {
                "allowed": true,
                "class_a_allowed": true,
                "max_length_feet": 40,
                "dump_station": true,
            },
            "vehicle_camping": {
                "van_camping": true,
                "rooftop_tent_allowed": true,
            },
        }))
        .expect("construct");

        let id = ctx.create(&profile).await.expect("create");
        let found = ctx.find_by_id(id).await.expect("find");
        assert!(found.rv_camping.allowed);
        assert!(found.rv_camping.class_a_allowed);
        assert_eq!(found.rv_camping.max_length_feet, Some(40));
        assert!(found.vehicle_camping.van_camping);
        assert!(found.vehicle_camping.rooftop_tent_allowed);
    }

    #[tokio::test]
    async fn find_by_id_missing_row_errors() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        assert!(ctx.find_by_id(99_999).await.is_err());
    }

    // -----------------------------------------------------------------
    // update
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_modifies_row_and_returns_true() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let id = ctx
            .create(&sample_profile("Original", false))
            .await
            .unwrap();

        let updated = sample_profile("Renamed", true);
        let ok = ctx.update(id, &updated).await.expect("update");
        assert!(ok);

        let found = ctx.find_by_id(id).await.expect("find");
        assert_eq!(found.profile_name, "Renamed");
        assert!(found.camping_allowed);
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let ok = ctx
            .update(99_999, &sample_profile("ghost", false))
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
        let ctx = CampingProfileContext::new(pool);

        let id = ctx
            .create(&sample_profile("To Delete", true))
            .await
            .unwrap();
        let deleted = ctx.delete(id).await.expect("delete");
        assert!(deleted);
        assert!(ctx.find_by_id(id).await.is_err());
    }

    #[tokio::test]
    async fn delete_missing_row_returns_false() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let deleted = ctx.delete(99_999).await.expect("delete");
        assert!(!deleted);
    }

    // -----------------------------------------------------------------
    // find_all + find_by_name
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_all_returns_every_row_ordered_by_name() {
        // The query ORDER BY profile_name — pin the alphabetical sort
        // so a future refactor can't silently change the wire-order
        // the frontend depends on.
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        ctx.create(&sample_profile("Zebra", true)).await.unwrap();
        ctx.create(&sample_profile("Alpha", true)).await.unwrap();
        ctx.create(&sample_profile("Middle", true)).await.unwrap();

        let all = ctx.find_all().await.expect("find_all");
        let names: Vec<_> = all.iter().map(|p| p.profile_name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "Middle", "Zebra"]);
    }

    #[tokio::test]
    async fn find_by_name_returns_some_for_existing() {
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);
        ctx.create(&sample_profile("Renaissance Camping", true))
            .await
            .unwrap();

        let found = ctx
            .find_by_name("Renaissance Camping")
            .await
            .expect("find_by_name");
        assert!(found.is_some());
        assert_eq!(found.unwrap().profile_name, "Renaissance Camping");
    }

    #[tokio::test]
    async fn find_by_name_returns_none_for_missing() {
        // Distinguishes "not found" from "DB error". Pinned: missing
        // is Ok(None), not Err.
        let pool = setup_pool().await;
        let ctx = CampingProfileContext::new(pool);

        let found = ctx
            .find_by_name("Nonexistent Profile")
            .await
            .expect("find_by_name");
        assert!(found.is_none());
    }
}
