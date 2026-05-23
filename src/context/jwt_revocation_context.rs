// src/context/jwt_revocation_context.rs

use crate::errors::AppError;
use sqlx::SqlitePool;

/// Data-access layer for the `jwt_revocations` table. Per the layered design
/// rule: contexts hold queries, nothing else. Validation and decisions
/// (e.g. "is this jti present in the user's token?") live in
/// `JwtRevocationLogic`.
pub struct JwtRevocationContext {
    pool: SqlitePool,
}

impl JwtRevocationContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Check whether a given jti is on the revocation list. Returns false for
    /// "not revoked or unknown" — the auth middleware uses this on every
    /// authenticated request, so it must be cheap (the `jti` PRIMARY KEY makes
    /// it a single index hit).
    pub async fn is_revoked(&self, jti: &str) -> Result<bool, AppError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM jwt_revocations WHERE jti = ?1 LIMIT 1")
                .bind(jti)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(row.is_some())
    }

    /// Add a jti to the revocation list. Idempotent — if the same jti is
    /// already present, INSERT OR IGNORE keeps the original revoked_at and
    /// returns Ok(()). A repeat logout for the same token shouldn't error.
    pub async fn revoke(&self, jti: &str, user_id: &str, expires_at: i64) -> Result<(), AppError> {
        sqlx::query(
            "INSERT OR IGNORE INTO jwt_revocations (jti, user_id, expires_at) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(jti)
        .bind(user_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Drop every revocation whose underlying token has already expired.
    /// Once `expires_at` is in the past, the token can no longer
    /// authenticate (the JWT verifier rejects it on its own), so keeping
    /// the row on the revocation list is pure noise. Called by a periodic
    /// retention sweep — see `JwtRevocationLogic::sweep_expired`.
    ///
    /// Returns the number of rows deleted so the caller can log
    /// "swept N expired revocations" without a separate count.
    pub async fn delete_expired(&self, now_unix_secs: i64) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM jwt_revocations WHERE expires_at < ?1")
            .bind(now_unix_secs)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool.
    //! Same pattern as `microevent_context::tests` — see the comments
    //! there for the rationale.

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
        // `jwt_revocations.user_id` references `users.id`. We're testing
        // the revocation context's own behavior; cross-table integrity
        // is covered at the auth-middleware layer. Disable FK checks
        // so the tests can use arbitrary user_id strings.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        pool
    }

    #[tokio::test]
    async fn is_revoked_returns_false_for_unknown_jti() {
        // Critical hot-path check — the auth middleware calls this on
        // every authenticated request. An unknown jti must answer
        // "not revoked" cheaply; the wrong default here would 401 every
        // request.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        assert!(!ctx.is_revoked("unknown-jti").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_then_is_revoked_round_trips() {
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        ctx.revoke("jti-1", "user-1", 9_999_999_999)
            .await
            .expect("revoke");
        assert!(ctx.is_revoked("jti-1").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_is_idempotent() {
        // Two logouts of the same token must not error — the second
        // call hits the INSERT OR IGNORE path. Pinned because a future
        // refactor to plain INSERT would surface a UNIQUE constraint
        // violation here and break the second logout.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        ctx.revoke("jti-1", "user-1", 9_999_999_999)
            .await
            .expect("first revoke");
        ctx.revoke("jti-1", "user-1", 9_999_999_999)
            .await
            .expect("second revoke must not error");
        assert!(ctx.is_revoked("jti-1").await.unwrap());
    }

    #[tokio::test]
    async fn revoked_one_jti_doesnt_affect_another() {
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        ctx.revoke("jti-1", "user-1", 9_999_999_999).await.unwrap();
        assert!(ctx.is_revoked("jti-1").await.unwrap());
        assert!(!ctx.is_revoked("jti-2").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_distinguishes_jti_per_user() {
        // Different users' tokens are independent — revoking one user's
        // token must not flip another user's jti-with-same-string.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        ctx.revoke("jti-alice", "user-alice", 1).await.unwrap();
        ctx.revoke("jti-bob", "user-bob", 1).await.unwrap();

        assert!(ctx.is_revoked("jti-alice").await.unwrap());
        assert!(ctx.is_revoked("jti-bob").await.unwrap());
        // Bob's jti shouldn't bleed into Alice's lookup or vice versa.
        assert!(!ctx.is_revoked("jti-carol").await.unwrap());
    }

    #[tokio::test]
    async fn is_revoked_empty_jti_returns_false() {
        // Defensive: the auth middleware now rejects empty-jti tokens
        // at the source, but a future caller might still pass an empty
        // string. Confirm we don't accidentally match a NULL/empty row.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        assert!(!ctx.is_revoked("").await.unwrap());
    }

    // -----------------------------------------------------------------
    // delete_expired — retention sweep
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn delete_expired_drops_only_past_rows() {
        // The sweep removes rows where `expires_at < cutoff`. Pin: rows
        // with cutoff exactly equal stay (strict `<`), and rows beyond
        // the cutoff stay. A regression that flipped to `<=` would
        // surface here.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        // Three rows: 1000 (past), 2000 (boundary), 3000 (future).
        ctx.revoke("jti-past", "u", 1000).await.unwrap();
        ctx.revoke("jti-boundary", "u", 2000).await.unwrap();
        ctx.revoke("jti-future", "u", 3000).await.unwrap();

        let deleted = ctx.delete_expired(2000).await.expect("delete_expired");
        assert_eq!(deleted, 1, "only the past row should be deleted");

        // jti-past gone, others still present.
        assert!(!ctx.is_revoked("jti-past").await.unwrap());
        assert!(ctx.is_revoked("jti-boundary").await.unwrap());
        assert!(ctx.is_revoked("jti-future").await.unwrap());
    }

    #[tokio::test]
    async fn delete_expired_on_empty_table_returns_zero() {
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        let deleted = ctx
            .delete_expired(9_999_999_999)
            .await
            .expect("delete_expired");
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn delete_expired_can_drop_every_row() {
        // Cutoff far in the future → everything's expired relative to it.
        let pool = setup_pool().await;
        let ctx = JwtRevocationContext::new(pool);

        ctx.revoke("a", "u", 100).await.unwrap();
        ctx.revoke("b", "u", 200).await.unwrap();
        ctx.revoke("c", "u", 300).await.unwrap();

        let deleted = ctx
            .delete_expired(9_999_999_999)
            .await
            .expect("delete_expired");
        assert_eq!(deleted, 3);
    }
}
