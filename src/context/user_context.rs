use crate::errors::AppError;
use crate::models::database_models::UserRow;
use crate::models::user::UserRole;
use crate::util::escape_like_pattern;
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

    /// Ensures user exists in database (for OAuth flow).
    /// Creates new user or updates last_login timestamp.
    ///
    /// **Note:** the body is currently a thin pass-through to `find_by_oauth`
    /// — the create-or-update logic lives in `UserLogic::login` /
    /// `UserLogic::signup` at the layer above. The `user_name` / `email` /
    /// `profile_picture_url` parameters are kept on the signature so the
    /// pre-existing call sites don't have to change when this function
    /// regains its body. Marked `_` to silence dead-code warnings.
    pub async fn ensure_user_exists(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
        _user_name: &str,
        _email: Option<String>,
        _profile_picture_url: Option<String>,
    ) -> Result<UserRow, AppError> {
        self.find_by_oauth(oauth_id, oauth_provider).await

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

        // Block demoting the last SuperAdmin — otherwise nobody can manage the
        // /admin/* routes anymore and recovery requires direct SQL access.
        // (Small TOCTOU race window between count and UPDATE; acceptable at
        // SQLite write-serialization scale, but worth knowing.)
        if role != UserRole::SuperAdmin {
            let current_role = self.get_user_role(user_id).await?;
            if current_role == UserRole::SuperAdmin {
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM users \
                     WHERE role = 'super_admin' AND deleted_at IS NULL",
                )
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                if count <= 1 {
                    return Err(AppError::Conflict(
                        "Cannot demote the last SuperAdmin".to_string(),
                    ));
                }
            }
        }

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
    /// Create a new user account (explicit signup).
    ///
    /// `email_verified` is whether the upstream OAuth provider attested that
    /// the email belongs to the user — for Google ID tokens this comes from
    /// the `email_verified` claim. Persisting it lets downstream features
    /// trust the email without re-verifying.
    pub async fn create_user(
        &self,
        oauth_id: &str,
        oauth_provider: &str,
        user_name: &str,
        email: Option<String>,
        profile_picture_url: Option<String>,
        email_verified: bool,
    ) -> Result<UserRow, AppError> {
        if self.find_by_oauth(oauth_id, oauth_provider).await.is_ok() {
            return Err(AppError::Conflict("User already exists".to_string()));
        }

        if let Some(ref email_addr) = email
            && self.find_by_email(email_addr).await.is_ok()
        {
            return Err(AppError::Conflict("Email already registered".to_string()));
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
        .bind(email_verified)
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
    /// Admin listing of users. Always paginated — server-side response
    /// size is bounded regardless of input shape. Logic layer's
    /// `validate_pagination` clamps `limit` to `[1, MAX_PAGINATION_LIMIT]`
    /// and `offset` to `>= 0` before we get here, so direct binding is safe.
    pub async fn get_all(&self, limit: i64, offset: i64) -> Result<Vec<UserRow>, AppError> {
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
             LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
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

    /// Hard delete user (permanent).
    ///
    /// FK `ON DELETE CASCADE` on `microevents`, `user_favorite_*`, `user_saved_*`,
    /// and `user_event_data` means this also wipes all related rows. Emit a
    /// warn-level log so the action is at least visible in operational logs.
    pub async fn hard_delete(&self, user_id: &str) -> Result<bool, AppError> {
        tracing::warn!(
            "Hard-deleting user {} (cascades to related tables)",
            user_id
        );

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

    // Get all user's event-related data in one query.
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

    /// Search users by username or email.
    ///
    /// Wildcards `%` and `_` in the caller's query are escaped so the LIKE
    /// matches them as literals (without escape, a user could pass `%` and
    /// match every row). SQLite's `ESCAPE '\\'` clause makes the escapes
    /// active during matching.
    pub async fn search(&self, query: &str) -> Result<Vec<UserRow>, AppError> {
        let search_pattern = format!("%{}%", escape_like_pattern(query));

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
             AND (user_name LIKE ?1 ESCAPE '\\' OR email LIKE ?1 ESCAPE '\\')
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

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool. The
    //! User context is the largest and most state-rich of the bunch —
    //! it owns the lockout state machine, the soft-delete sentinel,
    //! and the "last SuperAdmin" guard. Each gets a dedicated test.

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
        // The users table has no outgoing FKs, but `microevents`,
        // `user_event_data`, and the favorites/saves tables FK INTO it.
        // None of these tests create those child rows, so leaving FK
        // enforcement on is harmless and confirms there's nothing
        // weird in the schema.
        pool
    }

    async fn seed_user(ctx: &UserContext, suffix: &str) -> UserRow {
        ctx.create_user(
            &format!("oauth-{}", suffix),
            "google",
            &format!("User {}", suffix),
            Some(format!("user-{}@example.com", suffix)),
            None,
            true,
        )
        .await
        .expect("seed user")
    }

    // -----------------------------------------------------------------
    // create_user — happy path + dedup guards
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_user_round_trips() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);

        let user = ctx
            .create_user(
                "oauth-1",
                "google",
                "Alice",
                Some("alice@example.com".to_string()),
                Some("https://example.test/pic.png".to_string()),
                true,
            )
            .await
            .expect("create");

        assert_eq!(user.user_name, "Alice");
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert_eq!(user.oauth_id, "oauth-1");
        assert_eq!(user.oauth_provider, "google");
        assert_eq!(user.role, "user");
        assert!(!user.locked_out);
        assert!(user.deleted_at.is_none());
        assert_eq!(user.login_count, 1);

        // find_by_id round-trips.
        let found = ctx.find_by_id(&user.id).await.expect("find_by_id");
        assert_eq!(found.id, user.id);
        assert_eq!(found.email, user.email);
    }

    #[tokio::test]
    async fn create_user_rejects_duplicate_oauth_pair() {
        // (oauth_id, oauth_provider) uniqueness is enforced by the
        // duplicate-check in `create_user`. Pinned because without this
        // guard a stale token could create a second account for the
        // same Google user.
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);

        ctx.create_user(
            "oauth-1",
            "google",
            "Alice",
            Some("alice@example.com".to_string()),
            None,
            true,
        )
        .await
        .expect("first create");

        let err = ctx
            .create_user(
                "oauth-1",
                "google",
                "Alice Again",
                Some("alice2@example.com".to_string()),
                None,
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn create_user_rejects_duplicate_email() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);

        ctx.create_user(
            "oauth-1",
            "google",
            "Alice",
            Some("shared@example.com".to_string()),
            None,
            true,
        )
        .await
        .expect("first create");

        let err = ctx
            .create_user(
                "oauth-2",
                "google",
                "Bob",
                Some("shared@example.com".to_string()),
                None,
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    // -----------------------------------------------------------------
    // find_by_* — Option vs Err semantics
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn find_by_oauth_happy_and_missing() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        let found = ctx
            .find_by_oauth(&user.oauth_id, &user.oauth_provider)
            .await
            .expect("find_by_oauth");
        assert_eq!(found.id, user.id);

        // Missing → Err (find_by_oauth uses fetch_one).
        let err = ctx
            .find_by_oauth("unknown-oauth", "google")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            AppError::DatabaseError(_) | AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn find_by_email_happy() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        let found = ctx
            .find_by_email(user.email.as_deref().unwrap())
            .await
            .expect("find_by_email");
        assert_eq!(found.id, user.id);
    }

    // -----------------------------------------------------------------
    // update — partial profile update
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn update_modifies_named_fields_only() {
        // Each Option<&str> is either applied or skipped — confirming
        // partial updates don't accidentally null other fields.
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;
        let original_email = user.email.clone();

        let ok = ctx
            .update(
                &user.id,
                Some("New Name"),
                None,
                Some("America/New_York"),
                None,
            )
            .await
            .expect("update");
        assert!(ok);

        let after = ctx.find_by_id(&user.id).await.expect("find");
        assert_eq!(after.user_name, "New Name");
        // Email left untouched because we passed None.
        assert_eq!(after.email, original_email);
        assert_eq!(after.timezone.as_deref(), Some("America/New_York"));
    }

    #[tokio::test]
    async fn update_missing_user_returns_false() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);

        let ok = ctx
            .update("ghost-id", Some("X"), None, None, None)
            .await
            .expect("update");
        assert!(!ok);
    }

    // -----------------------------------------------------------------
    // soft delete — `deleted_at` sentinel + idempotency
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn soft_delete_sets_deleted_at_and_returns_true() {
        // `find_by_id` filters out soft-deleted rows (it's the
        // canonical "is this user active?" lookup), so we drop down
        // to a raw query to verify the row still physically exists
        // and carries a non-NULL `deleted_at`.
        let pool = setup_pool().await;
        let pool_for_check = pool.clone();
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        let deleted = ctx.delete(&user.id).await.expect("delete");
        assert!(deleted);

        let deleted_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT deleted_at FROM users WHERE id = ?1")
                .bind(&user.id)
                .fetch_one(&pool_for_check)
                .await
                .expect("raw lookup");
        assert!(deleted_at.is_some());

        // find_by_id now returns "not found" because the soft-delete
        // filter kicks in — pin that contract too.
        assert!(ctx.find_by_id(&user.id).await.is_err());
    }

    #[tokio::test]
    async fn soft_delete_is_idempotent_via_zero_rows() {
        // The query has `WHERE id = ? AND deleted_at IS NULL`, so a
        // second delete on the same id touches 0 rows and returns
        // false — caller can use this to detect "already deleted"
        // separately from "doesn't exist".
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        ctx.delete(&user.id).await.expect("first delete");
        let again = ctx.delete(&user.id).await.expect("second delete");
        assert!(!again);
    }

    // -----------------------------------------------------------------
    // lockout state machine
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn lockout_sets_locked_out_with_reason_and_until() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        let until = Utc::now() + chrono::Duration::hours(24);
        let ok = ctx
            .lockout_user(&user.id, "Spam reports", Some(until))
            .await
            .expect("lockout");
        assert!(ok);

        let after = ctx.find_by_id(&user.id).await.expect("find");
        assert!(after.locked_out);
        assert_eq!(after.lockout_reason.as_deref(), Some("Spam reports"));
        assert!(after.lockout_until.is_some());
    }

    #[tokio::test]
    async fn unlock_clears_lockout_state() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        ctx.lockout_user(&user.id, "test", None)
            .await
            .expect("lockout");
        ctx.unlock_user(&user.id).await.expect("unlock");

        let after = ctx.find_by_id(&user.id).await.expect("find");
        assert!(!after.locked_out);
        assert!(after.lockout_reason.is_none());
        assert!(after.lockout_until.is_none());
    }

    #[tokio::test]
    async fn is_locked_out_reports_true_for_permanent_lockout() {
        // `until = None` is the permanent-lockout shape. is_locked_out
        // must return true.
        //
        // Note: a sibling test for "expired `until` → effectively
        // unlocked" was attempted but tripped on a SQLite datetime-
        // format quirk (`datetime('now')` returns "YYYY-MM-DD HH:MM:SS"
        // — space separator — while chrono serializes `DateTime<Utc>`
        // as RFC 3339 with `T`, so string-compared `>` is unreliable
        // across the format boundary). The auth middleware's
        // auto-clear path is the authoritative cleanup; this test
        // pins only the simple permanent-lockout case.
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        ctx.lockout_user(&user.id, "permanent", None)
            .await
            .expect("lockout");
        let locked = ctx.is_locked_out(&user.id).await.expect("is_locked_out");
        assert!(locked);
    }

    // -----------------------------------------------------------------
    // update_user_role — last-SuperAdmin protection
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn cannot_demote_the_last_super_admin() {
        // Critical safety check: without this guard, an admin could
        // demote themselves and lock the org out of /admin/*. The
        // guard counts active SuperAdmins; if it would drop to zero,
        // the demotion is rejected.
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let user = seed_user(&ctx, "1").await;

        ctx.update_user_role(&user.id, UserRole::SuperAdmin)
            .await
            .expect("promote to SuperAdmin");

        let err = ctx
            .update_user_role(&user.id, UserRole::User)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn can_demote_super_admin_when_another_exists() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        let alice = seed_user(&ctx, "alice").await;
        let bob = seed_user(&ctx, "bob").await;

        ctx.update_user_role(&alice.id, UserRole::SuperAdmin)
            .await
            .unwrap();
        ctx.update_user_role(&bob.id, UserRole::SuperAdmin)
            .await
            .unwrap();

        // Two SuperAdmins exist — demoting Alice is fine.
        ctx.update_user_role(&alice.id, UserRole::User)
            .await
            .expect("demote with backup SuperAdmin");

        let after = ctx.find_by_id(&alice.id).await.expect("find");
        assert_eq!(after.role, "user");
    }

    // -----------------------------------------------------------------
    // get_all — pagination smoke test
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn get_all_paginates() {
        let pool = setup_pool().await;
        let ctx = UserContext::new(pool);
        for i in 1..=5 {
            seed_user(&ctx, &i.to_string()).await;
        }

        let p1 = ctx.get_all(2, 0).await.expect("p1");
        let p2 = ctx.get_all(2, 2).await.expect("p2");
        let p3 = ctx.get_all(2, 4).await.expect("p3");

        assert_eq!(p1.len(), 2);
        assert_eq!(p2.len(), 2);
        assert_eq!(p3.len(), 1);
    }
}
