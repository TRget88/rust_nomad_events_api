use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    User,
    Admin,
    SuperAdmin,
}

/// Stable wire-format strings for `UserRole`. Match the serde `rename_all =
/// "snake_case"` mapping AND the sqlx column values, so a `Display` print
/// roundtrips cleanly with serde and sqlx. Used by the JWT-stamping path
/// and the role-probe test in auth_middleware.
impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            UserRole::User => "user",
            UserRole::Admin => "admin",
            UserRole::SuperAdmin => "super_admin",
        };
        f.write_str(s)
    }
}

impl UserRole {
    /// Parse a role string from the `users.role` column. Defensively falls
    /// back to `UserRole::User` on unknown strings rather than erroring —
    /// the DB column has `NOT NULL DEFAULT 'user'` so the unknown case
    /// shouldn't happen in production, but failing closed (least privilege)
    /// on corrupted data is safer than failing open or panicking.
    pub fn from_db_string(s: &str) -> Self {
        match s {
            "admin" => UserRole::Admin,
            "super_admin" => UserRole::SuperAdmin,
            _ => UserRole::User,
        }
    }
}

//-- In your users table, the user_role column will contain:
//'user'         -- for UserRole::User
//'admin'        -- for UserRole::Admin
//'super_admin'  -- for UserRole::SuperAdmin

#[derive(Debug, Deserialize)]
pub struct GoogleIdToken {
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub sub: String, // Google user ID
    pub email_verified: bool,
    /// OIDC nonce echoed back by Google from the original authentication
    /// request. Per OIDC Core §3.1.2.1 the client picks a value at sign-in
    /// time, passes it via `<GoogleLogin nonce={...}>`, and the server
    /// verifies the round-trip survived. `#[serde(default)]` keeps the
    /// deserialize tolerant of older ID tokens that didn't carry it; the
    /// verifier enforces the match.
    #[serde(default)]
    pub nonce: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,      // "subject" - the user ID (who the token is about)
    pub email: String,    // Custom claim - user's email
    pub username: String, // Custom claim - user's username
    /// User's role. Strongly typed (`UserRole` enum, not `String`) so
    /// downstream comparisons are exhaustive `match`/`matches!` checks —
    /// a typo like `claims.role == "admmin"` would have been a silent
    /// always-false comparison; now it's a compile error. The enum
    /// roundtrips through serde as `"user"`/`"admin"`/`"super_admin"` to
    /// match the column values and the prior wire format exactly, so
    /// existing tokens stay compatible.
    pub role: UserRole,
    pub exp: usize, // "expiration" - when token expires (Unix timestamp)
    pub iat: usize, // "issued at" - when token was created (Unix timestamp)
    /// "JWT id" — a UUID stamped at issue time, used as the key into the
    /// `jwt_revocations` table so an explicit logout can invalidate this
    /// specific token. `#[serde(default)]` lets the deserializer succeed
    /// when the field is missing; the auth middleware then rejects the
    /// empty value with a clear 401 ("Token is missing required claim")
    /// instead of the generic serde "missing field" parse error. Every
    /// real, currently-issued token carries a UUID here.
    #[serde(default)]
    pub jti: String,
    /// "Audience" — identifies which service the token was issued for.
    /// Stamped as `"festurah-api"` on every new token. Defends against
    /// shared-secret confusion: if Festurah ever runs a second service
    /// using the same `JWT_SECRET`, a token minted for the API can't be
    /// silently accepted by the other (and vice versa).
    ///
    /// `#[serde(default)]` lets a missing field deserialize to the empty
    /// string; the auth middleware then rejects an empty aud with the
    /// same 401 path as a wrong-audience aud. Same shape as `jti` above.
    #[serde(default)]
    pub aud: String,
}

// `Claims::get_role()` previously parsed a stringly-typed role into
// `UserRole`. Now `role` is `UserRole` natively; the parse happens at
// serde-decode time. Access via `claims.role` directly.

#[derive(Deserialize)]
pub struct GoogleLoginRequest {
    pub credential: String, // The ID token from Google
    /// OIDC nonce the frontend generated for this sign-in attempt and
    /// passed to `<GoogleLogin nonce={...}>`. Google echoes it back inside
    /// the ID token's `nonce` claim; the backend asserts the round-trip
    /// matches. Optional during the rollout — see `verify_google_id_token`
    /// for the four-case acceptance matrix.
    #[serde(default)]
    pub nonce: Option<String>,
}

/// Wire format from the frontend's Facebook sign-in. `facebook-oauth-react`
/// returns a short-lived **user access token** (opaque, not a JWT) after
/// successful OAuth. Unlike Google's ID token, Facebook tokens cannot be
/// verified offline — they must be exchanged against the Graph API.
#[derive(Deserialize)]
pub struct FacebookLoginRequest {
    pub access_token: String,
}

/// Facebook Graph API `/me` response shape. Requested fields:
///   `id,name,email,picture`
///
/// `email` is **optional** on Facebook — users can sign up with a phone
/// number, or revoke the email permission after install. The signin path
/// rejects responses without an email (same shape as the Google flow,
/// which rejects unverified emails) so every account in our DB has a
/// usable contact address.
///
/// `picture` is a nested object: `{"data": {"url": "https://..."}}`. The
/// flatten happens at consumer time so the wire model stays one-to-one
/// with the Graph API contract.
#[derive(Debug, Deserialize)]
pub struct FacebookUser {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub picture: Option<FacebookPicture>,
}

#[derive(Debug, Deserialize)]
pub struct FacebookPicture {
    pub data: FacebookPictureData,
}

#[derive(Debug, Deserialize)]
pub struct FacebookPictureData {
    pub url: String,
}

/// Auth response sent to the frontend after Google or Facebook sign-in.
///
/// Wire shape must stay 1:1 with the frontend's `AuthResponse` /
/// `User` types (`festurah_frontend/src/shared/dtos.ts` +
/// `festurah_frontend/src/shared/models.ts`). Keeping them in sync is a
/// manual contract — any field change here needs a paired edit there.
#[derive(Serialize)]
pub struct AuthResponse {
    /// Short-lived access token (JWT). Used as the `Bearer ...` value on
    /// every authenticated request. TTL is currently 24h (the
    /// pre-refresh-token default); the planned drop to 15 min is gated
    /// on the frontend handling a 401 → `/auth/refresh` → retry flow.
    pub token: String,
    /// Opaque long-lived refresh token (hex of 32 random bytes). Used
    /// only to mint a fresh access token via `POST /auth/refresh`.
    /// Optional in the wire format so this rolled out without forcing
    /// every existing client to update in lockstep — clients that don't
    /// understand it can ignore it and keep using the access token
    /// until it expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub user_name: Option<String>,
    pub picture_url: Option<String>,
    pub role: String,
    pub provider: String,
    pub provider_id: String, // Google's "sub" or Facebook's "id"
    pub created_at: String,  // ISO date string
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub user_name: Option<String>,
    pub email: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
}

/// Body for `PUT /admin/users/{id}/role`.
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    /// One of `"user"`, `"admin"`, `"super_admin"`.
    pub role: String,
}

/// Full data bundle returned by `GET /self/data-export`. Aggregates
/// everything tied to the authenticated user — profile, collection
/// (favorites/saves/created), and the corresponding full event /
/// microevent records — into one downloadable JSON blob for GDPR/CCPA
/// portability compliance.
///
/// `exported_at` is the server-side timestamp so the user can prove
/// when the export ran. The event/microevent collections are the same
/// shapes the API returns elsewhere, so a downstream tool reading the
/// JSON gets identical types it'd see live.
#[derive(serde::Serialize)]
pub struct UserDataExport {
    pub exported_at: chrono::DateTime<chrono::Utc>,
    pub user: UserInfo,
    /// The `created/favorite/saved` id arrays. Uses `UserEventDataRow`
    /// (the DB shape) rather than `UserCollection` (the route DTO)
    /// because that's what `user_collection_logic.get()` returns.
    pub collection: crate::models::database_models::UserEventDataRow,
    pub created_events: Vec<crate::models::dto::EventResponse>,
    /// Note: microevent collections use the storage type (`Microevent`)
    /// rather than `MicroeventResponse` because that's what the
    /// user_collection_logic getters already return. The two shapes carry
    /// the same field set today — switching to MicroeventResponse would
    /// require an unrelated refactor of the getters.
    pub created_microevents: Vec<crate::models::microevents_models::Microevent>,
    pub favorite_events: Vec<crate::models::dto::EventResponse>,
    pub favorite_microevents: Vec<crate::models::microevents_models::Microevent>,
    pub saved_events: Vec<crate::models::dto::EventResponse>,
    pub saved_microevents: Vec<crate::models::microevents_models::Microevent>,
}

/// Body for `POST /admin/users/{id}/lock`. `until` is optional — omit for a
/// permanent lockout, set for a temporary one (must be in the future).
#[derive(Debug, Deserialize)]
pub struct LockUserRequest {
    pub reason: String,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}
impl UserRole {
    pub fn can_manage_users(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::SuperAdmin)
    }

    pub fn can_manage_admins(&self) -> bool {
        matches!(self, UserRole::SuperAdmin)
    }

    pub fn can_delete_any_content(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::SuperAdmin)
    }
}
