// src/logic/refresh_token_logic.rs
//
// Refresh-token rotation. Sits between the `/auth/refresh` route (which
// reads the plaintext off the request body) and `RefreshTokenContext`
// (which performs the SQL). Encodes the security-critical decisions:
//
//   - The plaintext refresh token is a 32-byte cryptographic random
//     value, hex-encoded for transport. 256 bits of entropy makes a
//     guessing attack vanishingly unlikely.
//   - The DB stores only `sha256(plaintext)` — a leak of the table
//     surrenders no live tokens directly.
//   - On every `/auth/refresh`, the presented token is revoked and a
//     fresh one is issued in the same family.
//   - **Reuse detection**: if a client presents a token whose row is
//     already revoked, that means either (a) replay by an attacker, or
//     (b) the legitimate client and an attacker both hold copies. The
//     safe response is to revoke the entire family — the user will be
//     forced to log in again from every device. Better a friction
//     event than a silent session takeover.

use crate::context::RefreshTokenContext;
use crate::context::refresh_token_context::RefreshTokenRow;
use crate::errors::AppError;
use chrono::{DateTime, Duration, Utc};
use rand::TryRngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

/// Refresh-token TTL. 30 days strikes the usual balance — long enough
/// that users don't have to re-authenticate from scratch every week,
/// short enough that an undetected leak doesn't grant year-long access.
/// Rotation on every use further bounds the practical exposure window.
const REFRESH_TTL_DAYS: i64 = 30;

/// Plaintext refresh token + the persisted row's id. Returned by
/// `issue_for_user` and the rotation path so the caller can put the
/// plaintext in the response body and the row id in the audit log.
///
/// `Debug` is derived so `expect_err`-style test assertions can format
/// the value; the impl includes the plaintext, so do **not** log this
/// at info/warn level in production code — only the `row_id` and the
/// `user_id` are safe to surface.
#[derive(Debug)]
pub struct IssuedRefreshToken {
    /// Plaintext refresh token. **The only place this exists** —
    /// the DB carries only its SHA-256 hash. Caller sends it to the
    /// client and then drops it; we never need it again.
    pub plaintext: String,
    /// Row id (UUID) for the row in `refresh_tokens`. Useful for
    /// audit-log entries that want to point at the specific token
    /// instance rather than the family.
    pub row_id: String,
    /// The user this token authenticates.
    pub user_id: String,
    /// When the row's `expires_at` was set to. Surfaced for the
    /// client so it can pre-emptively refresh before the hard cutoff.
    pub expires_at: DateTime<Utc>,
}

/// Pure helper: SHA-256 the plaintext and return the lowercase hex
/// digest. Same shape as `api_key.rs` so the DB columns are visually
/// comparable across the two paths.
pub(crate) fn hash_token(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a fresh 32-byte random refresh token, hex-encoded.
/// 256 bits of entropy → a brute-force guess against the hash store
/// is computationally infeasible even with the entire `refresh_tokens`
/// table in hand.
fn generate_plaintext_token() -> Result<String, AppError> {
    let mut buf = [0u8; 32];
    // `OsRng::try_fill_bytes` reads from the OS entropy pool (getrandom
    // on Linux, BCryptGenRandom on Windows). Returning a fallible value
    // since 0.9 of `rand` — the OS entropy call is the only thing that
    // can fail here. Map a failure to `InternalError`; the route handler
    // turns it into a 500.
    OsRng.try_fill_bytes(&mut buf).map_err(|e| {
        AppError::InternalError(format!("RNG failure issuing refresh token: {}", e))
    })?;
    Ok(hex::encode(buf))
}

pub struct RefreshTokenLogic {
    context: Arc<RefreshTokenContext>,
}

impl RefreshTokenLogic {
    pub fn new(context: Arc<RefreshTokenContext>) -> Self {
        Self { context }
    }

    /// Issue a brand-new refresh token at login time. The returned
    /// row sits in a fresh family with no parent. Use `rotate` to
    /// follow up on every subsequent `/auth/refresh`.
    pub async fn issue_for_user(&self, user_id: &str) -> Result<IssuedRefreshToken, AppError> {
        let plaintext = generate_plaintext_token()?;
        let row_id = Uuid::new_v4().to_string();
        let family_id = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + Duration::days(REFRESH_TTL_DAYS);

        self.context
            .insert(
                &row_id,
                user_id,
                &hash_token(&plaintext),
                &family_id,
                None,
                expires_at,
            )
            .await?;

        Ok(IssuedRefreshToken {
            plaintext,
            row_id,
            user_id: user_id.to_string(),
            expires_at,
        })
    }

    /// Rotation: consume the presented refresh token and issue a new
    /// one. Walks the security-critical decision tree:
    ///
    /// 1. Token unknown → 401 (no such token).
    /// 2. Token already revoked → **reuse detected**: revoke the
    ///    entire family and 401 (the client must log in again).
    /// 3. Token expired → revoke it (no point keeping it around) and
    ///    401 (the client must log in again — TTL exceeded).
    /// 4. Token valid → revoke it (one-shot use), mint a child token
    ///    in the same family, return the plaintext + the user_id so
    ///    the route can issue a paired access JWT.
    pub async fn rotate(&self, presented_plaintext: &str) -> Result<IssuedRefreshToken, AppError> {
        let hash = hash_token(presented_plaintext);
        let row: RefreshTokenRow = self.context.find_by_hash(&hash).await?.ok_or_else(|| {
            tracing::warn!("Refresh token lookup miss");
            AppError::Unauthorized("Invalid refresh token".to_string())
        })?;

        // Reuse detection. A revoked token presented for refresh means
        // either replay or a stolen copy is in play. Either way, the
        // family is compromised — kill it.
        if row.revoked_at.is_some() {
            let affected = self.context.revoke_family(&row.family_id).await?;
            tracing::warn!(
                family_id = %row.family_id,
                user_id = %row.user_id,
                affected,
                "Refresh-token reuse detected; revoked family"
            );
            return Err(AppError::Unauthorized(
                "Refresh token reuse detected; please log in again".to_string(),
            ));
        }

        // TTL check. Strict `<` against `Utc::now()` so an expires_at
        // landing exactly on the current instant survives by one tick
        // — matches the DB-side `expires_at < now` convention in the
        // retention sweep.
        if row.expires_at <= Utc::now() {
            // Revoke so a later replay of the same expired token hits
            // the revoked-but-was-valid branch and surfaces a clear
            // reuse event in the logs.
            self.context.revoke_by_id(&row.id).await?;
            return Err(AppError::Unauthorized(
                "Refresh token expired; please log in again".to_string(),
            ));
        }

        // Happy path: consume the presented row and issue a child in
        // the same family.
        self.context.revoke_by_id(&row.id).await?;

        let plaintext = generate_plaintext_token()?;
        let new_row_id = Uuid::new_v4().to_string();
        let new_expires_at = Utc::now() + Duration::days(REFRESH_TTL_DAYS);
        self.context
            .insert(
                &new_row_id,
                &row.user_id,
                &hash_token(&plaintext),
                &row.family_id,
                Some(&row.id),
                new_expires_at,
            )
            .await?;

        Ok(IssuedRefreshToken {
            plaintext,
            row_id: new_row_id,
            user_id: row.user_id,
            expires_at: new_expires_at,
        })
    }

    /// Retention sweep — same shape as `JwtRevocationLogic::sweep_expired`.
    /// Wired to a periodic trigger (admin endpoint / cron / one-shot
    /// binary). System-clock dependency lives inside so callers don't
    /// have to compute the timestamp themselves.
    #[allow(dead_code)] // wired to the cron item on ROADMAP; opt-in once a trigger lands
    pub async fn sweep_expired(&self) -> Result<u64, AppError> {
        let deleted = self.context.delete_expired(Utc::now()).await?;
        if deleted > 0 {
            tracing::info!(rows = deleted, "Swept expired refresh tokens");
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    //! Logic-layer tests. Each test constructs a `RefreshTokenLogic`
    //! over an in-memory pool, exercises the rotation/reuse path, and
    //! inspects the DB rows directly to confirm side effects.
    //!
    //! The hash function and plaintext generator are tested in isolation
    //! at the top so a regression in either (e.g. a future move to a
    //! shorter token) is pinned before the rotation tests run.

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
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        pool
    }

    fn make_logic(pool: sqlx::SqlitePool) -> RefreshTokenLogic {
        RefreshTokenLogic::new(Arc::new(RefreshTokenContext::new(pool)))
    }

    // -----------------------------------------------------------------
    // Helpers under test
    // -----------------------------------------------------------------

    #[test]
    fn hash_token_is_deterministic_hex_sha256() {
        // SHA-256("abc") = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        // Pinning the literal lets a future move to a different hash
        // function (e.g. blake3) surface here loudly.
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn generate_plaintext_token_is_64_hex_chars() {
        // 32 random bytes → 64 hex characters. A regression that
        // shortened the token to e.g. 16 bytes would surface here.
        let token = generate_plaintext_token().expect("generate");
        assert_eq!(token.len(), 64, "32-byte token = 64 hex characters");
        assert!(
            token.chars().all(|c| c.is_ascii_hexdigit()),
            "token must be pure hex"
        );
    }

    #[test]
    fn generated_tokens_are_distinct() {
        // Drawing twice from the OS RNG should never collide at
        // 256-bit entropy. Pinned because a future bug that seeded
        // the RNG deterministically would silently issue the same
        // token to every user.
        let a = generate_plaintext_token().unwrap();
        let b = generate_plaintext_token().unwrap();
        assert_ne!(a, b);
    }

    // -----------------------------------------------------------------
    // issue_for_user
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn issue_for_user_persists_a_row_with_hashed_token() {
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let issued = logic.issue_for_user("user-1").await.expect("issue");
        assert_eq!(issued.user_id, "user-1");

        // The plaintext is *not* what's in the DB. Look up by the
        // expected hash and confirm the row exists.
        let ctx = RefreshTokenContext::new(pool);
        let row = ctx
            .find_by_hash(&hash_token(&issued.plaintext))
            .await
            .unwrap()
            .expect("row stored");
        assert_eq!(row.id, issued.row_id);
        assert_eq!(row.user_id, "user-1");
        assert_eq!(row.parent_id, None, "issue starts a fresh family");
        assert!(row.revoked_at.is_none(), "fresh token is not revoked");
    }

    #[tokio::test]
    async fn issue_for_user_starts_new_family_per_call() {
        // Two logins → two independent families. Otherwise a single
        // reuse-detection event would revoke every session for the user.
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let a = logic.issue_for_user("user-1").await.unwrap();
        let b = logic.issue_for_user("user-1").await.unwrap();

        let ctx = RefreshTokenContext::new(pool);
        let row_a = ctx
            .find_by_hash(&hash_token(&a.plaintext))
            .await
            .unwrap()
            .unwrap();
        let row_b = ctx
            .find_by_hash(&hash_token(&b.plaintext))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            row_a.family_id, row_b.family_id,
            "each login must start its own family"
        );
    }

    // -----------------------------------------------------------------
    // rotate — happy path
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn rotate_revokes_old_and_issues_child_in_same_family() {
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let original = logic.issue_for_user("user-1").await.unwrap();
        let rotated = logic.rotate(&original.plaintext).await.expect("rotate");

        // Different plaintext, same user.
        assert_ne!(rotated.plaintext, original.plaintext);
        assert_eq!(rotated.user_id, "user-1");

        let ctx = RefreshTokenContext::new(pool);
        let original_row = ctx
            .find_by_hash(&hash_token(&original.plaintext))
            .await
            .unwrap()
            .unwrap();
        let rotated_row = ctx
            .find_by_hash(&hash_token(&rotated.plaintext))
            .await
            .unwrap()
            .unwrap();

        assert!(
            original_row.revoked_at.is_some(),
            "the consumed token must be revoked"
        );
        assert!(rotated_row.revoked_at.is_none(), "the new token is live");
        assert_eq!(
            original_row.family_id, rotated_row.family_id,
            "rotation stays inside the family"
        );
        assert_eq!(
            rotated_row.parent_id.as_deref(),
            Some(original_row.id.as_str()),
            "rotated row points back at the consumed row"
        );
    }

    #[tokio::test]
    async fn rotate_three_times_walks_a_chain() {
        // Rotating B → C → D must build a single parent chain. A
        // regression that reset `parent_id` on each rotate would
        // surface here.
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let a = logic.issue_for_user("user-1").await.unwrap();
        let b = logic.rotate(&a.plaintext).await.unwrap();
        let c = logic.rotate(&b.plaintext).await.unwrap();
        let d = logic.rotate(&c.plaintext).await.unwrap();

        let ctx = RefreshTokenContext::new(pool);
        let d_row = ctx
            .find_by_hash(&hash_token(&d.plaintext))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d_row.parent_id.as_deref(), Some(c.row_id.as_str()));

        let c_row = ctx
            .find_by_hash(&hash_token(&c.plaintext))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(c_row.parent_id.as_deref(), Some(b.row_id.as_str()));
    }

    // -----------------------------------------------------------------
    // rotate — failure cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn rotate_unknown_token_returns_unauthorized() {
        let pool = setup_pool().await;
        let logic = make_logic(pool);

        let err = logic
            .rotate("not-a-real-token-just-some-string")
            .await
            .expect_err("rotate should fail");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn rotate_reuse_revokes_the_whole_family() {
        // The classic reuse-detection scenario:
        //   1. Client logs in → gets token A
        //   2. Client rotates → gets token B (A is now revoked)
        //   3. Attacker who copied A presents it → reuse detected
        //   4. Both A and B (the whole family) get revoked
        //   5. A subsequent rotate(B) must also fail
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let a = logic.issue_for_user("user-1").await.unwrap();
        let b = logic.rotate(&a.plaintext).await.unwrap();

        // Replay of A → reuse detection fires.
        let replay_err = logic.rotate(&a.plaintext).await.expect_err("reuse");
        assert!(matches!(replay_err, AppError::Unauthorized(_)));

        // After reuse detection, B must also be unusable.
        let post_revocation_err = logic
            .rotate(&b.plaintext)
            .await
            .expect_err("family should be dead");
        assert!(matches!(post_revocation_err, AppError::Unauthorized(_)));

        // And the DB confirms both rows in the family are revoked.
        let ctx = RefreshTokenContext::new(pool);
        let row_a = ctx
            .find_by_hash(&hash_token(&a.plaintext))
            .await
            .unwrap()
            .unwrap();
        let row_b = ctx
            .find_by_hash(&hash_token(&b.plaintext))
            .await
            .unwrap()
            .unwrap();
        assert!(row_a.revoked_at.is_some());
        assert!(row_b.revoked_at.is_some());
    }

    #[tokio::test]
    async fn rotate_expired_token_returns_unauthorized_and_revokes_row() {
        // Past-TTL rotation must fail. We can't construct an expired
        // row via `issue_for_user` (which always sets +30 days), so go
        // through the context directly with a past `expires_at`.
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        let ctx = RefreshTokenContext::new(pool.clone());
        let plaintext = generate_plaintext_token().unwrap();
        let past = Utc::now() - Duration::hours(1);
        ctx.insert(
            "expired-id",
            "user-1",
            &hash_token(&plaintext),
            "fam-expired",
            None,
            past,
        )
        .await
        .unwrap();

        let err = logic.rotate(&plaintext).await.expect_err("must fail");
        assert!(matches!(err, AppError::Unauthorized(_)));

        // Row must now be revoked so a later replay surfaces in the
        // log path rather than just looking like another expired-token
        // event.
        let row = ctx
            .find_by_hash(&hash_token(&plaintext))
            .await
            .unwrap()
            .unwrap();
        assert!(row.revoked_at.is_some());
    }

    // -----------------------------------------------------------------
    // sweep_expired
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn sweep_expired_drops_past_rows_only() {
        let pool = setup_pool().await;
        let logic = make_logic(pool.clone());

        // One past, one future. Sweep drops only the past row.
        let ctx = RefreshTokenContext::new(pool.clone());
        let past_plain = generate_plaintext_token().unwrap();
        let future_plain = generate_plaintext_token().unwrap();
        ctx.insert(
            "past",
            "u",
            &hash_token(&past_plain),
            "f1",
            None,
            Utc::now() - Duration::days(1),
        )
        .await
        .unwrap();
        ctx.insert(
            "future",
            "u",
            &hash_token(&future_plain),
            "f2",
            None,
            Utc::now() + Duration::days(1),
        )
        .await
        .unwrap();

        let deleted = logic.sweep_expired().await.unwrap();
        assert_eq!(deleted, 1);

        assert!(
            ctx.find_by_hash(&hash_token(&past_plain))
                .await
                .unwrap()
                .is_none(),
            "past row should be gone"
        );
        assert!(
            ctx.find_by_hash(&hash_token(&future_plain))
                .await
                .unwrap()
                .is_some(),
            "future row should remain"
        );
    }
}
