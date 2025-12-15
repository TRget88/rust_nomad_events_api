// ============================================================================
// Service: src/services/camping_service.rs
// ============================================================================

use crate::models::event_models::CampingProfile;
use crate::repositories::CampingRepository;
use crate::errors::AppError;

pub struct CampingService {
    repository: CampingRepository,
}

impl CampingService {
    pub fn new(repository: CampingRepository) -> Self {
        Self { repository }
    }

    pub async fn get_all_profiles(&self) -> Result<Vec<CampingProfile>, AppError> {
        self.repository.find_all().await
    }

    pub async fn get_profile_by_id(&self, id: i64) -> Result<CampingProfile, AppError> {
        self.repository.find_by_id(id).await
    }

    pub async fn create_profile(&self, profile: CampingProfile) -> Result<i64, AppError> {
        // Validate profile name is not empty
        if profile.profile_name.trim().is_empty() {
            return Err(AppError::ValidationError("Profile name cannot be empty".to_string()));
        }

        self.repository.create(&profile).await
    }

    pub async fn update_profile(&self, id: i64, profile: CampingProfile) -> Result<(), AppError> {
        if profile.profile_name.trim().is_empty() {
            return Err(AppError::ValidationError("Profile name cannot be empty".to_string()));
        }

        let updated = self.repository.update(id, &profile).await?;
        
        if !updated {
            return Err(AppError::NotFound("Camping profile not found".to_string()));
        }

        Ok(())
    }

    pub async fn delete_profile(&self, id: i64) -> Result<(), AppError> {
        let deleted = self.repository.delete(id).await?;
        
        if !deleted {
            return Err(AppError::NotFound("Camping profile not found".to_string()));
        }

        Ok(())
    }
}