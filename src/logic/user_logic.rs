use crate::context::UserContext;
use crate::errors::app_error::AppError;
use crate::models::database_models::UserRow;
use crate::models::user::{
    AuthResponse, Claims, GoogleIdToken, GoogleLoginRequest, UserInfo, UserRole,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct UserLogic {
    context: UserContext,
}

impl UserLogic {
    pub fn new(context: UserContext) -> Self {
        Self { context }
    }

    pub async fn verify_google_login(
        &self,
        payload: &GoogleLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        // Verify Google's ID token
        let google_claims = Self::verify_google_id_token(&payload.credential)
            .await
            .map_err(|e| {
                eprintln!("Failed to verify Google token: {:?}", e);
                AppError::Unauthorized("Invalid Google token".to_string())
            })?;

        // Ensure user exists and get their data
        let exists = self
            .context
            .user_exists(&google_claims.sub, "google")
            .await?;

        if !exists {
            return Err(AppError::Conflict(
                "An account with these credentials does not exist. Please create an account instead."
                    .to_string(),
            ));
        }

        let user_data = self
            .context
            .find_by_oauth(&google_claims.sub, "google")
            .await?;

        // Create your own JWT
        let jwt = self.create_jwt_for_user(
            &user_data.id,
            &google_claims.email,
            &user_data.role,
            &user_data.user_name,
        )?;

        //Ok(AuthResponse {
        //token: jwt,
        //user: UserInfo {
        //id: google_claims.sub,
        //email: google_claims.email,
        //name: google_claims.name,
        //picture: google_claims.picture,
        ////role,
        //},
        //})
        Ok(AuthResponse {
            token: jwt,
            user: UserInfo {
                id: user_data.id,
                email: google_claims.email,
                name: google_claims.name,
                user_name: Some(user_data.user_name),
                picture_url: google_claims.picture,
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: google_claims.sub,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    pub async fn verify_google_account_creation(
        &self,
        payload: &GoogleLoginRequest,
    ) -> Result<AuthResponse, AppError> {
        // Verify Google's ID token
        let google_claims = Self::verify_google_id_token(&payload.credential)
            .await
            .map_err(|e| {
                eprintln!("Failed to verify Google token: {:?}", e);
                AppError::Unauthorized("Invalid Google token".to_string())
            })?;

        // create user and set their role
        // Ensure user exists and get their data
        let exists = self
            .context
            .user_exists(&google_claims.sub, "google")
            .await?;

        ///if the user does not exist, create the user
        if exists {
            return Err(AppError::Conflict(
                "An account with these credentials already exists. Please use login instead."
                    .to_string(),
            ));
        }

        let user_data = self
            .context
            .create_user(
                &google_claims.sub,
                "google",
                &google_claims.name.as_deref().unwrap_or("Unknown"),
                Some(google_claims.email.clone()),
                google_claims.picture.clone(),
            )
            .await?;

        // Create your own JWT
        let jwt = self.create_jwt_for_user(
            &user_data.id,
            &google_claims.email,
            &user_data.role,
            &user_data.user_name,
        )?;

        Ok(AuthResponse {
            token: jwt,
            user: UserInfo {
                id: user_data.id,
                email: google_claims.email,
                name: google_claims.name,
                user_name: Some(user_data.user_name),
                picture_url: google_claims.picture,
                role: user_data.role,
                provider: user_data.oauth_provider,
                provider_id: google_claims.sub,
                created_at: user_data.created_at.to_string(),
                updated_at: user_data.updated_at.to_string(),
            },
        })
    }

    async fn verify_google_id_token(
        id_token: &str,
    ) -> Result<GoogleIdToken, Box<dyn std::error::Error>> {
        let google_client_id = std::env::var("GOOGLE_CLIENT_ID")?;

        // Decode header to get the kid (key ID)
        let header = decode_header(id_token)?;
        let kid = header.kid.ok_or("No kid in token header")?;

        // Fetch Google's public keys
        let client = reqwest::Client::new();
        let jwks: serde_json::Value = client
            .get("https://www.googleapis.com/oauth2/v3/certs")
            .send()
            .await?
            .json()
            .await?;

        // Find the matching key
        let key = jwks["keys"]
            .as_array()
            .ok_or("No keys in JWKS")?
            .iter()
            .find(|k| k["kid"] == kid)
            .ok_or("Key not found in JWKS")?;

        // Extract RSA components
        let n = key["n"].as_str().ok_or("No n in key")?;
        let e = key["e"].as_str().ok_or("No e in key")?;

        // Create decoding key
        let decoding_key = DecodingKey::from_rsa_components(n, e)?;

        // Verify the token
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&google_client_id]);
        validation.set_issuer(&["https://accounts.google.com"]);

        let token_data = decode::<GoogleIdToken>(id_token, &decoding_key, &validation)?;

        Ok(token_data.claims)
    }

    fn create_jwt_for_user(
        &self,
        user_id: &str,
        email: &str,
        role: &str,
        user_name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let jwt_secret = std::env::var("JWT_SECRET")?;
        let expiration = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3600 * 24;
        let issued = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3600 * 24;

        let claims = Claims {
            sub: user_id.to_string(),
            email: email.to_string(),
            role: role.to_string(),
            exp: expiration as usize,
            username: user_name.to_string(),
            iat: issued as usize,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    pub async fn get_all(&self) -> Result<Vec<UserRow>, AppError> {
        let rows = self.context.get_all().await?;

        let events: Vec<UserRow> = rows
            .into_iter()
            //.filter_map(|row| User::from_row(row).ok())
            .collect();

        Ok(events)
    }

    pub async fn get_self(&self, id: Uuid) -> Result<UserRow, AppError> {
        let row = self.context.find_by_id(&id.to_string()).await?;
        //let event = Microevent::from_row(row)?;
        //Ok(event)
        Ok(row)
    }

    pub async fn get(&self, id: Uuid) -> Result<UserRow, AppError> {
        let row = self.context.find_by_id(&id.to_string()).await?;
        //let event = Microevent::from_row(row)?;
        //Ok(event)
        Ok(row)
    }
    // ========================================================================
    // User Management (Update)
    // ========================================================================

    pub async fn update_profile(
        &self,
        user_id: &str,
        user_name: Option<&str>,
        email: Option<&str>,
        timezone: Option<&str>,
        language: Option<&str>,
    ) -> Result<(), AppError> {
        // Validate email if provided
        if let Some(email) = email {
            self.validate_email(email)?;
        }

        let updated = self
            .context
            .update(user_id, user_name, email, timezone, language)
            .await?;

        if !updated {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // For the update_self handler that passes UserRow
    pub async fn update(&self, user_id: Uuid, user_data: UserRow) -> Result<(), AppError> {
        // Extract the fields you want to allow updating
        let updated = self
            .context
            .update(
                &user_id.to_string(),
                Some(&user_data.user_name),
                user_data.email.as_deref(),
                user_data.timezone.as_deref(),
                user_data.language.as_deref(),
            )
            .await?;

        if !updated {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // ========================================================================
    // User Management (Delete)
    // ========================================================================

    pub async fn delete_user(&self, user_id: &str) -> Result<(), AppError> {
        let deleted = self.context.delete(user_id).await?;

        if !deleted {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    // ========================================================================
    // Security & Moderation
    // ========================================================================

    pub async fn lockout_user(
        &self,
        user_id: &str,
        reason: &str,
        until: Option<DateTime<Utc>>,
    ) -> Result<(), AppError> {
        // Validate reason
        if reason.trim().is_empty() {
            return Err(AppError::BadRequest("Lockout reason required".to_string()));
        }

        let locked = self.context.lockout_user(user_id, reason, until).await?;

        if !locked {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    pub async fn unlock_user(&self, user_id: &str) -> Result<(), AppError> {
        let unlocked = self.context.unlock_user(user_id).await?;

        if !unlocked {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        Ok(())
    }

    pub async fn is_locked_out(&self, user_id: &str) -> Result<bool, AppError> {
        self.context.is_locked_out(user_id).await
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    pub async fn count_total_users(&self) -> Result<i64, AppError> {
        self.context.count_total().await
    }

    pub async fn count_users_by_role(&self, role: &str) -> Result<i64, AppError> {
        self.context.count_by_role(role).await
    }

    // ========================================================================
    // Validation Helpers
    // ========================================================================

    fn validate_email(&self, email: &str) -> Result<(), AppError> {
        // Basic email validation
        if !email.contains('@') || !email.contains('.') {
            return Err(AppError::BadRequest("Invalid email format".to_string()));
        }

        if email.len() < 5 || email.len() > 254 {
            return Err(AppError::BadRequest("Email length invalid".to_string()));
        }

        Ok(())
    }

    fn validate_username(&self, username: &str) -> Result<(), AppError> {
        if username.trim().is_empty() {
            return Err(AppError::BadRequest("Username cannot be empty".to_string()));
        }

        if username.len() < 2 || username.len() > 50 {
            return Err(AppError::BadRequest(
                "Username must be 2-50 characters".to_string(),
            ));
        }

        Ok(())
    }
}
//pub async fn create(&self, event: UserRow) -> Result<i64, AppError> {
//// Business logic: validate event data
////self.validate_event(&event)?;
//
//let id = self.context.create(&event).await?;
//Ok(id)
//}
//
//pub async fn update(&self, id: Uuid, event: UserRow) -> Result<(), AppError> {
//let updated = self.context.update(&id.to_string(), &event).await?;
//
//if !updated {
//return Err(AppError::NotFound("Event not found".to_string()));
//}
//
//Ok(())
//}
