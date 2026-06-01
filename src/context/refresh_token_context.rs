// src/context/refresh_token_context.rs

use crate::errors::AppError;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

/// One row in `refresh_tokens`. The plaintext refresh token is never
/// stored — `token_hash` carries `sha256(plaintext)` in hex. The
/// `revoked_at` column drives both rotation ("we used this one, mark
/// it revoked") and reuse-detection ("the client presented an already-
/// revoked token — revoke its whole family").
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefreshTokenRow {
    pub id: String,
    pub user_id: String,
    #[allow(dead_code)] // returned for completeness; the lookup uses the hash directly
    pub token_hash: String,
    pub family_id: String,
    #[allow(dead_code)] // chain traversal isn't wired yet; carries audit-trail value
    pub parent_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    #[allow(dead_code)] // returned for completeness
    pub created_at: DateTime<Utc>,
}

/// Data-access for the `refresh_tokens` table. Per the layered design
/// rule: contexts hold queries, nothing else. Decisions like "is this
/// token still valid" / "should we revoke the family" live in
/// `RefreshTokenLogic`.
pub struct RefreshTokenContext {
    pool: SqlitePool,
}

impl RefreshTokenContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist a freshly-issued refresh token. `id` and `family_id` are
    /// UUIDs minted by the caller; `token_hash` is `sha256(plaintext)`
    /// hex; `parent_id` is the previous token in the rotation chain,
    /// `None` for the row at login time. The plaintext itself is never
    /// passed in — a leaked stack trace shouldn't include it.
    pub async fn insert(
        &self,
        id: &str,
        user_id: &str,
        token_hash: &str,
        family_id: &str,
        parent_id: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO refresh_tokens \
             (id, user_id, token_hash, family_id, parent_id, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(family_id)
        .bind(parent_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Look up a row by its hash. The `/auth/refresh` route hashes the
    /// plaintext the client sent and calls through here. Returns
    /// `Ok(None)` for an unknown hash — that's a "no such token"
    /// outcome the logic layer maps to 401.
    pub async fn find_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>, AppError> {
        let row: Option<RefreshTokenRow> = sqlx::query_as(
            "SELECT id, user_id, token_hash, family_id, parent_id, \
                    expires_at, revoked_at, created_at \
             FROM refresh_tokens WHERE token_hash = ?1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(row)
    }

    /// Mark a single row as revoked. Used during normal rotation:
    /// "this client presented token A; we're issuing token B; A is now
    /// spent." Idempotent — setting `revoked_at` twice on the same row
    /// is harmless because we always carry the same UTC `now`.
    pub async fn revoke_by_id(&self, id: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE refresh_tokens \
             SET revoked_at = ?1 \
             WHERE id = ?2 AND revoked_at IS NULL",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Revoke every row in a family. Called when reuse-detection fires:
    /// a client presented a token that was already revoked, which means
    /// either the same token is being replayed by an attacker, or the
    /// legitimate client and an attacker both hold copies. Either way,
    /// the safe response is to invalidate the whole chain and force
    /// the user to log in again.
    ///
    /// Returns the number of rows updated so the caller can log
    /// "revoked N tokens in family X" without an extra count query.
    pub async fn revoke_family(&self, family_id: &str) -> Result<u64, AppError> {
        let result = sqlx::query(
            "UPDATE refresh_tokens \
             SET revoked_at = ?1 \
             WHERE family_id = ?2 AND revoked_at IS NULL",
        )
        .bind(Utc::now())
        .bind(family_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(result.rows_affected())
    }

    /// Retention sweep: drop every row whose `expires_at` has already
    /// passed. After the TTL elapses the token can't authenticate
    /// anyway (the logic layer enforces the check), so the row carries
    /// no security value — keeping it is pure noise.
    ///
    /// Returns the number of rows deleted, mirroring
    /// `JwtRevocationContext::delete_expired`. Wired to the same cron
    /// sweep tracked in ROADMAP.md.
    pub async fn delete_expired(&self, now: DateTime<Utc>) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < ?1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool.
    //! Same pattern as `jwt_revocation_context::tests` — see there for
    //! the rationale. Decisions like "is this token valid" live one
    //! layer up; these tests pin only the queries.

    use super::*;
    use chrono::Duration;
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
        // The `user_id` column references `users.id`. The context tests
        // construct synthetic user_ids; cross-table integrity belongs to
        // the route-layer tests where users are real.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .expect("disable FK enforcement");
        pool
    }

    fn future() -> DateTime<Utc> {
        Utc::now() + Duration::days(30)
    }

    #[tokio::test]
    async fn insert_then_find_round_trips() {
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        ctx.insert("id-1", "user-1", "hash-1", "fam-1", None, future())
            .await
            .expect("insert");
        let row = ctx
            .find_by_hash("hash-1")
            .await
            .expect("find")
            .expect("row present");

        assert_eq!(row.id, "id-1");
        assert_eq!(row.user_id, "user-1");
        assert_eq!(row.family_id, "fam-1");
        assert_eq!(row.parent_id, None);
        assert!(row.revoked_at.is_none(), "fresh row must not be revoked");
    }

    #[tokio::test]
    async fn find_by_unknown_hash_returns_none() {
        // The route maps `Ok(None)` → 401. Pinning this means a refactor
        // that bubbles "not found" as an error would surface here.
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        let result = ctx.find_by_hash("never-issued").await.expect("find");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn insert_with_parent_records_the_chain() {
        // The rotation chain is reconstructable via `parent_id`. Pin
        // that the column round-trips so a future audit-view can walk
        // backwards from any row to the original login token.
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        ctx.insert("root", "user-1", "hash-root", "fam-1", None, future())
            .await
            .unwrap();
        ctx.insert(
            "child",
            "user-1",
            "hash-child",
            "fam-1",
            Some("root"),
            future(),
        )
        .await
        .unwrap();

        let child = ctx.find_by_hash("hash-child").await.unwrap().unwrap();
        assert_eq!(child.parent_id.as_deref(), Some("root"));
    }

    #[tokio::test]
    async fn revoke_by_id_sets_revoked_at() {
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        ctx.insert("id-1", "user-1", "hash-1", "fam-1", None, future())
            .await
            .unwrap();
        ctx.revoke_by_id("id-1").await.unwrap();

        let row = ctx.find_by_hash("hash-1").await.unwrap().unwrap();
        assert!(row.revoked_at.is_some(), "revoke_by_id must set revoked_at");
    }

    #[tokio::test]
    async fn revoke_by_id_is_idempotent() {
        // Double-revoke (e.g. a retry on a flaky client) must not error
        // or shift the revoked_at timestamp once it's set. Pinned because
        // a future refactor that dropped `AND revoked_at IS NULL` would
        // silently slide the timestamp forward on every call.
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        ctx.insert("id-1", "user-1", "hash-1", "fam-1", None, future())
            .await
            .unwrap();
        ctx.revoke_by_id("id-1").await.unwrap();
        let first_revoked_at = ctx
            .find_by_hash("hash-1")
            .await
            .unwrap()
            .unwrap()
            .revoked_at;

        ctx.revoke_by_id("id-1").await.unwrap();
        let second_revoked_at = ctx
            .find_by_hash("hash-1")
            .await
            .unwrap()
            .unwrap()
            .revoked_at;

        assert_eq!(
            first_revoked_at, second_revoked_at,
            "second revoke must not slide the timestamp"
        );
    }

    #[tokio::test]
    async fn revoke_family_marks_every_row_in_the_family() {
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        // Three rows in family A, one in family B. revoke_family("A")
        // must hit all three of A and none of B.
        ctx.insert("a1", "user-1", "ha1", "fam-a", None, future())
            .await
            .unwrap();
        ctx.insert("a2", "user-1", "ha2", "fam-a", Some("a1"), future())
            .await
            .unwrap();
        ctx.insert("a3", "user-1", "ha3", "fam-a", Some("a2"), future())
            .await
            .unwrap();
        ctx.insert("b1", "user-1", "hb1", "fam-b", None, future())
            .await
            .unwrap();

        let affected = ctx.revoke_family("fam-a").await.unwrap();
        assert_eq!(affected, 3, "exactly three rows in fam-a");

        assert!(
            ctx.find_by_hash("ha1")
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        assert!(
            ctx.find_by_hash("ha2")
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        assert!(
            ctx.find_by_hash("ha3")
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        assert!(
            ctx.find_by_hash("hb1")
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_none()
        );
    }

    #[tokio::test]
    async fn revoke_family_skips_already_revoked_rows() {
        // If half the family was already revoked (e.g. normal rotation
        // happened earlier), revoke_family only flips the remaining
        // unrevoked rows. Returns the count *changed* — the rotation
        // log already says "the others were already revoked."
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        ctx.insert("a1", "user-1", "ha1", "fam-a", None, future())
            .await
            .unwrap();
        ctx.insert("a2", "user-1", "ha2", "fam-a", Some("a1"), future())
            .await
            .unwrap();
        ctx.revoke_by_id("a1").await.unwrap();

        let affected = ctx.revoke_family("fam-a").await.unwrap();
        assert_eq!(
            affected, 1,
            "only the still-active row gets a fresh revoked_at"
        );
    }

    #[tokio::test]
    async fn delete_expired_drops_only_past_rows() {
        // Boundary mirror of `JwtRevocationContext::delete_expired`:
        // strict `<` semantics, so rows whose `expires_at` equals the
        // cutoff survive.
        let pool = setup_pool().await;
        let ctx = RefreshTokenContext::new(pool);

        let past = Utc::now() - Duration::days(1);
        let boundary = Utc::now();
        let future_ts = Utc::now() + Duration::days(7);

        ctx.insert("p", "user-1", "hp", "fam-1", None, past)
            .await
            .unwrap();
        ctx.insert("b", "user-1", "hb", "fam-1", None, boundary)
            .await
            .unwrap();
        ctx.insert("f", "user-1", "hf", "fam-1", None, future_ts)
            .await
            .unwrap();

        let deleted = ctx.delete_expired(boundary).await.unwrap();
        assert_eq!(deleted, 1, "only the past row is dropped");

        assert!(ctx.find_by_hash("hp").await.unwrap().is_none());
        assert!(ctx.find_by_hash("hb").await.unwrap().is_some());
        assert!(ctx.find_by_hash("hf").await.unwrap().is_some());
    }
}
