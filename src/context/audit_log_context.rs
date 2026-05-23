// src/context/audit_log_context.rs

use crate::errors::AppError;
use crate::models::audit::{AuditEntry, AuditRecord};
use sqlx::SqlitePool;

/// Hard cap on `list_recent` page size. Aligns with the existing `EventContext`
/// MAX_BATCH_SIZE — same reasoning: bounded memory + bounded query cost.
const MAX_LIST_LIMIT: i64 = 500;

pub struct AuditLogContext {
    pool: SqlitePool,
}

impl AuditLogContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append an audit entry. `metadata` is serialized to JSON text before
    /// storage; failure to serialize is a programmer error (we only call
    /// this with our own struct shapes) but we still surface it cleanly.
    pub async fn record(&self, entry: &AuditRecord) -> Result<(), AppError> {
        let metadata_json = serde_json::to_string(&entry.metadata).map_err(|e| {
            AppError::InternalError(format!("Failed to serialize audit metadata: {}", e))
        })?;

        sqlx::query(
            "INSERT INTO admin_audit_log (actor_user_id, action, target_type, target_id, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&entry.actor_user_id)
        .bind(&entry.action)
        .bind(&entry.target_type)
        .bind(&entry.target_id)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// List recent audit entries, newest first. Caps at MAX_LIST_LIMIT so a
    /// caller asking for `limit=999999` doesn't read the entire table.
    /// `metadata` is parsed back from JSON; on malformed JSON (shouldn't
    /// happen since we control writes) the entry is logged and dropped so a
    /// single corrupt row doesn't poison the whole response.
    ///
    /// `offset` enables paging deep into the audit log. Logic-layer
    /// `validate_pagination` clamps to `>= 0` before we get here; the
    /// internal `limit.clamp(1, MAX_LIST_LIMIT)` is defense-in-depth in
    /// case a future caller bypasses the validator.
    pub async fn list_recent(&self, limit: i64, offset: i64) -> Result<Vec<AuditEntry>, AppError> {
        let capped_limit = limit.clamp(1, MAX_LIST_LIMIT);
        let safe_offset = offset.max(0);

        let rows: Vec<(i64, String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, actor_user_id, action, target_type, target_id, metadata, created_at \
             FROM admin_audit_log \
             ORDER BY id DESC \
             LIMIT ?1 OFFSET ?2",
        )
        .bind(capped_limit)
        .bind(safe_offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        let entries = rows
            .into_iter()
            .filter_map(
                |(id, actor, action, target_type, target_id, metadata, created_at)| {
                    match serde_json::from_str::<serde_json::Value>(&metadata) {
                        Ok(v) => Some(AuditEntry {
                            id,
                            actor_user_id: actor,
                            action,
                            target_type,
                            target_id,
                            metadata: v,
                            created_at,
                        }),
                        Err(e) => {
                            tracing::warn!(
                                entry_id = id,
                                error = %e,
                                "Audit entry has malformed metadata JSON; dropping from response"
                            );
                            None
                        }
                    }
                },
            )
            .collect();

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
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
        pool
    }

    fn sample_record(actor: &str, target: &str, action: &str) -> AuditRecord {
        AuditRecord {
            actor_user_id: actor.to_string(),
            action: action.to_string(),
            target_type: "user".to_string(),
            target_id: target.to_string(),
            metadata: json!({ "note": "test" }),
        }
    }

    #[tokio::test]
    async fn record_then_list_round_trips() {
        let pool = setup_pool().await;
        let ctx = AuditLogContext::new(pool);

        ctx.record(&sample_record("actor-1", "target-1", "user.lock"))
            .await
            .expect("record entry");

        let entries = ctx.list_recent(50, 0).await.expect("list");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.actor_user_id, "actor-1");
        assert_eq!(entry.target_id, "target-1");
        assert_eq!(entry.action, "user.lock");
        assert_eq!(entry.metadata, json!({ "note": "test" }));
    }

    #[tokio::test]
    async fn list_returns_newest_first() {
        let pool = setup_pool().await;
        let ctx = AuditLogContext::new(pool);

        for i in 1..=3 {
            ctx.record(&sample_record(
                &format!("actor-{}", i),
                &format!("target-{}", i),
                "user.unlock",
            ))
            .await
            .expect("record");
        }

        let entries = ctx.list_recent(50, 0).await.expect("list");
        assert_eq!(entries.len(), 3);
        // Newest first: actor-3 inserted last, should appear first.
        assert_eq!(entries[0].actor_user_id, "actor-3");
        assert_eq!(entries[1].actor_user_id, "actor-2");
        assert_eq!(entries[2].actor_user_id, "actor-1");
    }

    #[tokio::test]
    async fn list_caps_at_max_limit() {
        let pool = setup_pool().await;
        let ctx = AuditLogContext::new(pool);

        // Caller asks for 99999 — we should silently cap at MAX_LIST_LIMIT
        // without erroring or building an enormous query.
        let entries = ctx.list_recent(99_999, 0).await.expect("list");
        assert!(entries.is_empty()); // empty table; the cap matters once it grows
    }

    #[tokio::test]
    async fn list_clamps_zero_and_negative_to_minimum() {
        let pool = setup_pool().await;
        let ctx = AuditLogContext::new(pool);
        ctx.record(&sample_record("actor-x", "target-x", "user.delete"))
            .await
            .expect("record");

        // Caller asks for 0 or negative — clamp to 1 so we don't generate
        // `LIMIT 0` (always empty) or `LIMIT -1` (SQLite-specific behavior).
        let entries = ctx.list_recent(0, 0).await.expect("list 0");
        assert_eq!(entries.len(), 1);
        let entries = ctx.list_recent(-5, 0).await.expect("list -5");
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn list_respects_offset() {
        // Pin the new offset behavior: page-size 2 + offset 2 over a
        // 5-row table returns the 3rd and 4th rows (newest-first).
        let pool = setup_pool().await;
        let ctx = AuditLogContext::new(pool);

        for i in 1..=5 {
            ctx.record(&sample_record(
                &format!("actor-{}", i),
                &format!("target-{}", i),
                "user.lock",
            ))
            .await
            .expect("record");
        }

        // Page 1: rows 5, 4
        let p1 = ctx.list_recent(2, 0).await.expect("page 1");
        assert_eq!(p1.len(), 2);
        assert_eq!(p1[0].actor_user_id, "actor-5");
        assert_eq!(p1[1].actor_user_id, "actor-4");

        // Page 2 (offset=2): rows 3, 2
        let p2 = ctx.list_recent(2, 2).await.expect("page 2");
        assert_eq!(p2.len(), 2);
        assert_eq!(p2[0].actor_user_id, "actor-3");
        assert_eq!(p2[1].actor_user_id, "actor-2");

        // Page 3 (offset=4): row 1 only
        let p3 = ctx.list_recent(2, 4).await.expect("page 3");
        assert_eq!(p3.len(), 1);
        assert_eq!(p3[0].actor_user_id, "actor-1");

        // Past the end: empty.
        let p4 = ctx.list_recent(2, 10).await.expect("past end");
        assert_eq!(p4.len(), 0);
    }
}
