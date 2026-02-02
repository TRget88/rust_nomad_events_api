use crate::errors::AppError;
use crate::models::database_models::{UserEventDataRow, UserRow};
use crate::models::user::UserRole;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct UserContext {
    pool: SqlitePool,
}

impl UserContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Authentication & Authorization
    // ========================================================================

    /// Ensures user exists in database (for OAuth flow)
    /// Creates new user or updates last_login timestamp
    pub async fn ensure_user_exists(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
        user_name: &str,
        email: Option<String>,
        profile_picture_url: Option<String>,
    ) -> Result<UserRow, AppError> {
        self.find_by_oauth(&oauth_id, &oauth_provider).await

        //let user_id = Uuid::new_v4().to_string();
        //let now = Utc::now();

        //This is not trying to find a user, it is trying to update a user. This seems wrong for now.
        // Try to find existing user
        //if let Ok(existing_user) = self.find_by_oauth(oauth_id, oauth_provider).await {
        //// Update last login and login count
        //sqlx::query(
        //"UPDATE users
        //SET last_login_at = ?1,
        //login_count = login_count + 1,
        //updated_at = ?1
        //WHERE id = ?2",
        //)
        //.bind(now)
        //.bind(&existing_user.id)
        //.execute(&self.pool)
        //.await
        //.map_err(|e| AppError::DatabaseError(e.to_string()))?;
        //
        //return self.find_by_id(&existing_user.id).await;
        //}

        // Create new user -- Obviously this was created by mistake, we are not trying to make a new user here.
        //sqlx::query(
        //"INSERT INTO users (
        //id, oauth_id, oauth_provider, user_name, email,
        //profile_picture_url, email_verified, locked_out, role,
        //created_at, updated_at, last_login_at, login_count,
        //events_created_count, microevents_created_count,
        //favorite_events_count, favorite_microevents_count,
        //saved_events_count, saved_microevents_count
        //) VALUES (
        //?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
        //)",
        //)
        //.bind(&user_id)
        //.bind(oauth_id)
        //.bind(oauth_provider)
        //.bind(user_name)
        //.bind(&email)
        //.bind(&profile_picture_url)
        //.bind(false) // email_verified
        //.bind(false) // locked_out
        //.bind("user") // role
        //.bind(now) // created_at
        //.bind(now) // updated_at
        //.bind(now) // last_login_at
        //.bind(1) // login_count
        //.bind(0) // events_created_count
        //.bind(0) // microevents_created_count
        //.bind(0) // favorite_events_count
        //.bind(0) // favorite_microevents_count
        //.bind(0) // saved_events_count
        //.bind(0) // saved_microevents_count
        //.execute(&self.pool)
        //.await
        //.map_err(|e| AppError::DatabaseError(e.to_string()))?;

        //self.find_by_id(&user_id).await
    }

    pub async fn get_user_role(&self, user_id: &str) -> Result<UserRole, AppError> {
        let result = sqlx::query_scalar::<_, String>(
            "SELECT role FROM users WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        match result.as_deref() {
            Some("admin") => Ok(UserRole::Admin),
            Some("super_admin") => Ok(UserRole::SuperAdmin),
            Some("user") | Some(_) => Ok(UserRole::User),
            None => Err(AppError::NotFound("User not found".to_string())),
        }
    }

    pub async fn update_user_role(&self, user_id: &str, role: UserRole) -> Result<(), AppError> {
        let role_str = match role {
            UserRole::User => "user",
            UserRole::Admin => "admin",
            UserRole::SuperAdmin => "super_admin",
        };

        let result = sqlx::query(
            "UPDATE users 
             SET role = ?1, updated_at = ?2 
             WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(role_str)
        .bind(Utc::now())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    /// Authenticate existing user (login)
    /// Returns error if user doesn't exist
    pub async fn authenticate_user(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
    ) -> Result<UserRow, AppError> {
        let user = self.find_by_oauth(oauth_id, oauth_provider).await?;

        // Check if locked out
        if user.locked_out {
            // Check if temporary lockout expired
            if let Some(until) = user.lockout_until {
                if until > Utc::now() {
                    return Err(AppError::Forbidden(format!(
                        "Account locked until {}",
                        until
                    )));
                }
                // Lockout expired, auto-unlock
                self.unlock_user(&user.id).await?;
            } else {
                // Permanent lockout
                return Err(AppError::Forbidden(
                    user.lockout_reason
                        .unwrap_or_else(|| "Account locked".to_string()),
                ));
            }
        }

        // Update login stats
        sqlx::query(
            "UPDATE users 
             SET last_login_at = ?1, 
                 login_count = login_count + 1,
                 updated_at = ?1
             WHERE id = ?2",
        )
        .bind(Utc::now())
        .bind(&user.id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        self.find_by_id(&user.id).await
    }

    /// Check if OAuth account already has a registered user
    pub async fn user_exists(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM users 
             WHERE oauth_id = ?1 AND oauth_provider = ?2 AND deleted_at IS NULL",
        )
        .bind(oauth_id)
        .bind(oauth_provider)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result > 0)
    }

    // ========================================================================
    // CRUD Operations
    // ========================================================================
    /// Create a new user account (explicit signup)
    /// Returns error if user already exists
    pub async fn create_user(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
        user_name: &str,
        email: Option<String>,
        profile_picture_url: Option<String>,
    ) -> Result<UserRow, AppError> {
        // Check if user already exists
        if self.find_by_oauth(oauth_id, oauth_provider).await.is_ok() {
            return Err(AppError::Conflict("User already exists".to_string()));
        }

        // Check if email already taken
        if let Some(ref email_addr) = email {
            if self.find_by_email(email_addr).await.is_ok() {
                return Err(AppError::Conflict("Email already registered".to_string()));
            }
        }

        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO users (
                id, oauth_id, oauth_provider, user_name, email, 
                profile_picture_url, email_verified, locked_out, role,
                created_at, updated_at, last_login_at, login_count,
                events_created_count, microevents_created_count,
                favorite_events_count, favorite_microevents_count,
                saved_events_count, saved_microevents_count
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )",
        )
        .bind(&user_id)
        .bind(oauth_id)
        .bind(oauth_provider)
        .bind(user_name)
        .bind(&email)
        .bind(&profile_picture_url)
        .bind(false) // email_verified
        .bind(false) // locked_out
        .bind("user") // role
        .bind(now) // created_at
        .bind(now) // updated_at
        .bind(now) // last_login_at
        .bind(1) // login_count
        .bind(0) // events_created_count
        .bind(0) // microevents_created_count
        .bind(0) // favorite_events_count
        .bind(0) // favorite_microevents_count
        .bind(0) // saved_events_count
        .bind(0) // saved_microevents_count
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        self.find_by_id(&user_id).await
    }

    /// Get all active users (not deleted)
    pub async fn get_all(&self) -> Result<Vec<UserRow>, AppError> {
        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE deleted_at IS NULL
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(rows)
    }

    /// Find user by ID
    pub async fn find_by_id(&self, id: &str) -> Result<UserRow, AppError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(row)
    }

    /// Find user by OAuth credentials
    pub async fn find_by_oauth(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
    ) -> Result<UserRow, AppError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE oauth_id = ?1 AND oauth_provider = ?2 AND deleted_at IS NULL",
        )
        .bind(oauth_id)
        .bind(oauth_provider)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(row)
    }

    /// Find user by email
    pub async fn find_by_email(&self, email: &str) -> Result<UserRow, AppError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE email = ?1 AND deleted_at IS NULL",
        )
        .bind(email)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(row)
    }

    /// Update user profile
    pub async fn update(
        &self,
        user_id: &str,
        user_name: Option<&str>,
        email: Option<&str>,
        timezone: Option<&str>,
        language: Option<&str>,
    ) -> Result<bool, AppError> {
        let mut query = "UPDATE users SET updated_at = ?1".to_string();
        let mut param_count = 2;

        if user_name.is_some() {
            query.push_str(&format!(", user_name = ?{}", param_count));
            param_count += 1;
        }
        if email.is_some() {
            query.push_str(&format!(", email = ?{}", param_count));
            param_count += 1;
        }
        if timezone.is_some() {
            query.push_str(&format!(", timezone = ?{}", param_count));
            param_count += 1;
        }
        if language.is_some() {
            query.push_str(&format!(", language = ?{}", param_count));
            param_count += 1;
        }

        query.push_str(&format!(
            " WHERE id = ?{} AND deleted_at IS NULL",
            param_count
        ));

        let mut q = sqlx::query(&query).bind(Utc::now());

        if let Some(name) = user_name {
            q = q.bind(name);
        }
        if let Some(e) = email {
            q = q.bind(e);
        }
        if let Some(tz) = timezone {
            q = q.bind(tz);
        }
        if let Some(lang) = language {
            q = q.bind(lang);
        }

        q = q.bind(user_id);

        let result = q
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Soft delete user
    pub async fn delete(&self, user_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE users 
             SET deleted_at = ?1, updated_at = ?1 
             WHERE id = ?2 AND deleted_at IS NULL",
        )
        .bind(Utc::now())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Hard delete user (permanent)
    pub async fn hard_delete(&self, user_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    // ========================================================================
    // User Lockout Management
    // ========================================================================

    /// Lock out a user (ban)
    pub async fn lockout_user(
        &self,
        user_id: &str,
        reason: &str,
        until: Option<DateTime<Utc>>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE users 
             SET locked_out = ?1, 
                 lockout_reason = ?2, 
                 lockout_until = ?3,
                 updated_at = ?4
             WHERE id = ?5 AND deleted_at IS NULL",
        )
        .bind(true)
        .bind(reason)
        .bind(until)
        .bind(Utc::now())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Unlock a user
    pub async fn unlock_user(&self, user_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE users 
             SET locked_out = ?1, 
                 lockout_reason = NULL, 
                 lockout_until = NULL,
                 updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(false)
        .bind(Utc::now())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Check if user is currently locked out
    pub async fn is_locked_out(&self, user_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query_scalar::<_, bool>(
            "SELECT locked_out FROM users 
             WHERE id = ?1 
             AND deleted_at IS NULL
             AND (lockout_until IS NULL OR lockout_until > datetime('now'))",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(result.unwrap_or(false))
    }

    // ========================================================================
    // User Event Data (Favorites, Saves, Created)
    // ========================================================================

    /// Get all user's event-related data in one query
    //pub async fn get_user_event_data(&self, user_id: &str) -> Result<UserEventDataRow, AppError> {
    //// Get favorite events
    //let favorite_events: Vec<String> =
    //sqlx::query_scalar("SELECT event_id FROM user_favorite_events WHERE user_id = ?1")
    //.bind(user_id)
    //.fetch_all(&self.pool)
    //.await
    //.map_err(|e| AppError::DatabaseError(e.to_string()))?
    //.into_iter()
    //.map(|id: i64| id.to_string())
    //.collect();
    //
    //// Get favorite microevents
    //let favorite_microevents: Vec<String> = sqlx::query_scalar(
    //"SELECT microevent_id FROM user_favorite_microevents WHERE user_id = ?1",
    //)
    //.bind(user_id)
    //.fetch_all(&self.pool)
    //.await
    //.map_err(|e| AppError::DatabaseError(e.to_string()))?
    //.into_iter()
    //.map(|id: i64| id.to_string())
    //.collect();
    //
    //// Get saved events
    //let saved_events: Vec<String> =
    //sqlx::query_scalar("SELECT event_id FROM user_saved_events WHERE user_id = ?1")
    //.bind(user_id)
    //.fetch_all(&self.pool)
    //.await
    //.map_err(|e| AppError::DatabaseError(e.to_string()))?
    //.into_iter()
    //.map(|id: i64| id.to_string())
    //.collect();
    //
    //// Get saved microevents
    //let saved_microevents: Vec<String> = sqlx::query_scalar(
    //"SELECT microevent_id FROM user_saved_microevents WHERE user_id = ?1",
    //)
    //.bind(user_id)
    //.fetch_all(&self.pool)
    //.await
    //.map_err(|e| AppError::DatabaseError(e.to_string()))?
    //.into_iter()
    //.map(|id: i64| id.to_string())
    //.collect();
    //
    //// Get created events (if you track this - might need separate table)
    //let created_events: Vec<String> = vec![]; // TODO: Implement if needed
    //
    //// Get created microevents
    //let created_microevents: Vec<String> =
    //sqlx::query_scalar("SELECT id FROM microevents WHERE user_id = ?1 AND archive = 0")
    //.bind(user_id)
    //.fetch_all(&self.pool)
    //.await
    //.map_err(|e| AppError::DatabaseError(e.to_string()))?
    //.into_iter()
    //.map(|id: i64| id.to_string())
    //.collect();
    //
    //Ok(UserEventDataRow {
    //id: 0,
    //user_id: user_id.to_string(),
    //favorite_events,
    //favorite_microevents,
    //saved_events,
    //saved_microevents,
    //created_events,
    //created_microevents,
    //})
    //}

    // ========================================================================
    // Search & Filtering
    // ========================================================================

    /// Search users by username or email
    pub async fn search(&self, query: &str) -> Result<Vec<UserRow>, AppError> {
        let search_pattern = format!("%{}%", query);

        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE deleted_at IS NULL
             AND (user_name LIKE ?1 OR email LIKE ?1)
             ORDER BY created_at DESC
             LIMIT 50",
        )
        .bind(&search_pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(rows)
    }

    /// Get recently active users
    pub async fn get_recently_active(&self, limit: i32) -> Result<Vec<UserRow>, AppError> {
        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE deleted_at IS NULL
             AND last_login_at IS NOT NULL
             ORDER BY last_login_at DESC
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(rows)
    }

    /// Get newly registered users
    pub async fn get_recent_signups(&self, limit: i32) -> Result<Vec<UserRow>, AppError> {
        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, oauth_id, oauth_provider, user_name, email, 
                    email_verified, profile_picture_url, locked_out, 
                    lockout_reason, lockout_until, role, created_at, 
                    updated_at, last_login_at, deleted_at, login_count,
                    events_created_count, microevents_created_count,
                    favorite_events_count, favorite_microevents_count,
                    saved_events_count, saved_microevents_count,
                    timezone, language, notification_preferences
             FROM users
             WHERE deleted_at IS NULL
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(rows)
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    /// Get total user count (excluding deleted)
    pub async fn count_total(&self) -> Result<i64, AppError> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    /// Get count of users by role
    pub async fn count_by_role(&self, role: &str) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM users WHERE role = ?1 AND deleted_at IS NULL",
        )
        .bind(role)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(count)
    }
}
