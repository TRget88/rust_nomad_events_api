// ============================================================================
// src/context/event_ownership_request_context.rs
// ============================================================================
//
// SQL layer for the event-ownership-request workflow (migration 00008).
// Data shapes live in `models/ownership.rs`; business rules in
// `logic/event_ownership_request_logic.rs`.

use crate::errors::AppError;
use crate::models::ownership::{EventOwnershipRequestRow, resolution, status};
use sqlx::SqlitePool;

/// Hard cap on the admin "all pending" sweep / multi-event lookup so a
/// runaway queue can't load unboundedly. Matches the `AuditLogContext` /
/// `EventContext` MAX_* convention and stays well under SQLite's
/// SQLITE_MAX_VARIABLE_NUMBER (999) for the `IN (...)` path.
const MAX_LIST_LIMIT: usize = 500;

/// Full column list, in struct-field order, shared by every SELECT so
/// `query_as::<_, EventOwnershipRequestRow>` maps cleanly by name.
const SELECT_COLUMNS: &str = "id, event_id, requester_user_id, status, note, \
     resolution_method, resolved_by_user_id, created_at, resolved_at";

pub struct EventOwnershipRequestContext {
    pool: SqlitePool,
}

impl EventOwnershipRequestContext {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a new pending request and return the stored row. Relies on
    /// column defaults (`status='pending'`, `created_at=datetime('now')`).
    ///
    /// A second pending request for the same (event, requester) trips the
    /// `idx_eor_one_pending_per_user_event` partial unique index. We let
    /// that error propagate via `?` (deliberately NOT `map_err`) so the
    /// `From<sqlx::Error>` impl maps the UNIQUE violation to
    /// `AppError::Conflict` (409) instead of a sanitized 500.
    pub async fn create(
        &self,
        event_id: i64,
        requester_user_id: &str,
        note: Option<&str>,
    ) -> Result<EventOwnershipRequestRow, AppError> {
        sqlx::query(
            "INSERT INTO event_ownership_requests (event_id, requester_user_id, note) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(event_id)
        .bind(requester_user_id)
        .bind(note)
        .execute(&self.pool)
        .await?;

        // Re-read via the pending unique key. The partial index guarantees
        // exactly one pending row for this pair, so this is unambiguous and
        // avoids a `last_insert_rowid()` round-trip that could race across
        // pooled connections.
        let row = sqlx::query_as::<_, EventOwnershipRequestRow>(&format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests \
             WHERE event_id = ?1 AND requester_user_id = ?2 AND status = 'pending'"
        ))
        .bind(event_id)
        .bind(requester_user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Fetch one request by id. NotFound when it doesn't exist.
    pub async fn find_by_id(&self, id: i64) -> Result<EventOwnershipRequestRow, AppError> {
        let row = sqlx::query_as::<_, EventOwnershipRequestRow>(&format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests WHERE id = ?1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        row.ok_or_else(|| AppError::NotFound("Ownership request not found".to_string()))
    }

    /// Every request filed by one user, newest first. Backs the
    /// requester's outgoing "my requests" list.
    pub async fn list_by_requester(
        &self,
        requester_user_id: &str,
    ) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        let rows = sqlx::query_as::<_, EventOwnershipRequestRow>(&format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests \
             WHERE requester_user_id = ?1 \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(requester_user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Pending requests for one event, oldest first (FIFO review order).
    pub async fn list_pending_for_event(
        &self,
        event_id: i64,
    ) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        let rows = sqlx::query_as::<_, EventOwnershipRequestRow>(&format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests \
             WHERE event_id = ?1 AND status = 'pending' \
             ORDER BY created_at ASC, id ASC"
        ))
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Pending requests across a set of events — the owner's "incoming"
    /// review queue spanning every event they own. Empty input short-
    /// circuits (an `IN ()` is a syntax error).
    pub async fn list_pending_for_events(
        &self,
        event_ids: &[i64],
    ) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        if event_ids.len() > MAX_LIST_LIMIT {
            return Err(AppError::BadRequest(format!(
                "Too many events requested (max {})",
                MAX_LIST_LIMIT
            )));
        }
        let placeholders = event_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests \
             WHERE status = 'pending' AND event_id IN ({placeholders}) \
             ORDER BY created_at ASC, id ASC"
        );
        let mut query = sqlx::query_as::<_, EventOwnershipRequestRow>(&sql);
        for id in event_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Every pending request in the system, newest first — the admin
    /// global queue. Capped at `MAX_LIST_LIMIT`.
    pub async fn list_all_pending(&self) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        let rows = sqlx::query_as::<_, EventOwnershipRequestRow>(&format!(
            "SELECT {SELECT_COLUMNS} FROM event_ownership_requests \
             WHERE status = 'pending' \
             ORDER BY created_at DESC, id DESC \
             LIMIT ?1"
        ))
        .bind(MAX_LIST_LIMIT as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Atomically resolve a pending request. The `AND status = 'pending'`
    /// guard makes this a compare-and-set: a second concurrent resolution
    /// (two admins, or owner + admin racing) affects zero rows and returns
    /// `false`, so the caller treats it as "already resolved" without a
    /// separate lock. `resolved_by` is NULL for the domain-auto path.
    pub async fn resolve(
        &self,
        id: i64,
        new_status: &str,
        method: &str,
        resolved_by: Option<&str>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE event_ownership_requests \
             SET status = ?1, \
                 resolution_method = ?2, \
                 resolved_by_user_id = ?3, \
                 resolved_at = datetime('now') \
             WHERE id = ?4 AND status = 'pending'",
        )
        .bind(new_status)
        .bind(method)
        .bind(resolved_by)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Retire every OTHER still-pending request for `event_id` — called right
    /// after an approval (or domain-auto) transfer moves ownership, so a
    /// stale sibling request can't later be approved and hand the same event
    /// a second owner. The just-approved request is spared via
    /// `keep_request_id`. Rows are flipped to `status='rejected'` with
    /// `resolution_method='superseded'` (distinct from a human rejection) and
    /// stamped with `resolved_by` (the approver, or NULL on the domain-auto
    /// path). Returns the number retired.
    ///
    /// This is a single atomic `UPDATE`, so it closes the *sequential* path:
    /// once one approval has committed, any later approval of a sibling sees
    /// `status != 'pending'` and Conflicts. It does NOT fully close two
    /// approvals whose read+compare-and-set interleave within the same
    /// instant — each still wins its own row. Fully closing that needs
    /// single-column ownership (`events.creator_id`) or an IMMEDIATE
    /// transaction wrapping read+resolve+transfer. Tracked in BUGS.md.
    pub async fn supersede_other_pending_for_event(
        &self,
        event_id: i64,
        keep_request_id: i64,
        resolved_by: Option<&str>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            "UPDATE event_ownership_requests \
             SET status = ?1, \
                 resolution_method = ?2, \
                 resolved_by_user_id = ?3, \
                 resolved_at = datetime('now') \
             WHERE event_id = ?4 AND id != ?5 AND status = 'pending'",
        )
        .bind(status::REJECTED)
        .bind(resolution::SUPERSEDED)
        .bind(resolved_by)
        .bind(event_id)
        .bind(keep_request_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    //! Context-layer integration tests with an in-memory pool. FK
    //! enforcement is OFF — these tests exercise the context's own SQL
    //! with synthetic event/user ids; cross-table integrity is covered at
    //! the logic/route layer. Mirrors `user_collection_context::tests`.

    use super::*;
    use crate::models::ownership::{resolution, status};
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

    // -----------------------------------------------------------------
    // create / find_by_id
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_then_find_round_trips() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let created = ctx
            .create(42, "requester-1", Some("I run the festival"))
            .await
            .expect("create");
        assert_eq!(created.event_id, 42);
        assert_eq!(created.requester_user_id, "requester-1");
        assert_eq!(created.status, status::PENDING);
        assert_eq!(created.note.as_deref(), Some("I run the festival"));
        // Pending rows carry no resolution yet.
        assert!(created.resolution_method.is_none());
        assert!(created.resolved_by_user_id.is_none());
        assert!(created.resolved_at.is_none());

        let found = ctx.find_by_id(created.id).await.expect("find_by_id");
        assert_eq!(found.id, created.id);
        assert_eq!(found.event_id, 42);
    }

    #[tokio::test]
    async fn create_accepts_null_note() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let created = ctx.create(7, "requester-x", None).await.expect("create");
        assert!(created.note.is_none());
    }

    #[tokio::test]
    async fn find_by_id_missing_is_not_found() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let err = ctx.find_by_id(999_999).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "got {:?}", err);
    }

    // -----------------------------------------------------------------
    // partial-unique-index behavior
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn duplicate_pending_request_conflicts() {
        // Two open requests for the same (event, requester) trip the
        // partial unique index -> Conflict. This is the 409 the API
        // surfaces when a user double-submits.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        ctx.create(100, "dup-user", None).await.expect("first");
        let err = ctx.create(100, "dup-user", None).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn second_request_allowed_after_resolution() {
        // The partial index only constrains pending rows. Once a prior
        // request is resolved (here: rejected), the user may file again —
        // re-requesting after a rejection must NOT be blocked.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let first = ctx.create(200, "retry-user", None).await.expect("first");
        let resolved = ctx
            .resolve(
                first.id,
                status::REJECTED,
                resolution::OWNER_REJECTION,
                Some("owner-1"),
            )
            .await
            .expect("resolve");
        assert!(resolved);

        // Now a fresh pending request for the same pair succeeds, and the
        // re-read unambiguously returns the new pending row (not the old
        // rejected one).
        let second = ctx.create(200, "retry-user", None).await.expect("second");
        assert_ne!(second.id, first.id);
        assert_eq!(second.status, status::PENDING);
    }

    // -----------------------------------------------------------------
    // resolve — compare-and-set semantics
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn resolve_sets_all_resolution_fields() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let req = ctx.create(300, "req-user", None).await.expect("create");
        let ok = ctx
            .resolve(
                req.id,
                status::APPROVED,
                resolution::ADMIN_APPROVAL,
                Some("admin-9"),
            )
            .await
            .expect("resolve");
        assert!(ok);

        let after = ctx.find_by_id(req.id).await.expect("find");
        assert_eq!(after.status, status::APPROVED);
        assert_eq!(
            after.resolution_method.as_deref(),
            Some(resolution::ADMIN_APPROVAL)
        );
        assert_eq!(after.resolved_by_user_id.as_deref(), Some("admin-9"));
        assert!(after.resolved_at.is_some(), "resolved_at should be stamped");
    }

    #[tokio::test]
    async fn resolve_allows_null_resolver_for_domain_auto() {
        // The auto-approval path has no human approver — resolved_by is
        // NULL while resolution_method records 'domain_auto'.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let req = ctx.create(310, "band-user", None).await.expect("create");
        let ok = ctx
            .resolve(req.id, status::APPROVED, resolution::DOMAIN_AUTO, None)
            .await
            .expect("resolve");
        assert!(ok);

        let after = ctx.find_by_id(req.id).await.expect("find");
        assert_eq!(
            after.resolution_method.as_deref(),
            Some(resolution::DOMAIN_AUTO)
        );
        assert!(after.resolved_by_user_id.is_none());
    }

    #[tokio::test]
    async fn resolve_is_idempotent_after_first() {
        // Second resolve of an already-resolved request affects zero rows
        // (the `status='pending'` guard) and returns false. This is the
        // race guard for two approvers acting at once.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let req = ctx.create(320, "race-user", None).await.expect("create");
        let first = ctx
            .resolve(
                req.id,
                status::APPROVED,
                resolution::OWNER_APPROVAL,
                Some("o"),
            )
            .await
            .expect("first resolve");
        assert!(first);

        let second = ctx
            .resolve(
                req.id,
                status::REJECTED,
                resolution::ADMIN_REJECTION,
                Some("a"),
            )
            .await
            .expect("second resolve");
        assert!(!second, "second resolve must be a no-op");

        // State reflects the FIRST resolution only.
        let after = ctx.find_by_id(req.id).await.expect("find");
        assert_eq!(after.status, status::APPROVED);
        assert_eq!(
            after.resolution_method.as_deref(),
            Some(resolution::OWNER_APPROVAL)
        );
    }

    #[tokio::test]
    async fn resolve_unknown_id_returns_false() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let ok = ctx
            .resolve(
                404_404,
                status::APPROVED,
                resolution::ADMIN_APPROVAL,
                Some("a"),
            )
            .await
            .expect("resolve");
        assert!(!ok);
    }

    #[tokio::test]
    async fn supersede_other_pending_flips_only_sibling_pending() {
        // Two users with open claims on the SAME event, plus an unrelated
        // claim on a different event. Superseding (keeping one) must retire
        // ONLY the sibling pending row for the same event — never the kept
        // row, never another event's queue.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let keep = ctx.create(600, "user-a", None).await.expect("a");
        let sibling = ctx.create(600, "user-b", None).await.expect("b");
        let other_event = ctx.create(601, "user-c", None).await.expect("c");

        let superseded = ctx
            .supersede_other_pending_for_event(600, keep.id, Some("approver-9"))
            .await
            .expect("supersede");
        assert_eq!(superseded, 1, "only the one sibling on event 600");

        // The sibling is now resolved as 'superseded' (status rejected),
        // stamped with the approver and a resolved_at.
        let after_sibling = ctx.find_by_id(sibling.id).await.expect("sibling");
        assert_eq!(after_sibling.status, status::REJECTED);
        assert_eq!(
            after_sibling.resolution_method.as_deref(),
            Some(resolution::SUPERSEDED)
        );
        assert_eq!(
            after_sibling.resolved_by_user_id.as_deref(),
            Some("approver-9")
        );
        assert!(after_sibling.resolved_at.is_some());

        // The kept row and the other event's queue are untouched.
        assert_eq!(
            ctx.find_by_id(keep.id).await.expect("keep").status,
            status::PENDING
        );
        assert_eq!(
            ctx.find_by_id(other_event.id).await.expect("other").status,
            status::PENDING
        );

        // Idempotent: nothing pending left to supersede on event 600.
        let again = ctx
            .supersede_other_pending_for_event(600, keep.id, Some("approver-9"))
            .await
            .expect("again");
        assert_eq!(again, 0);
    }

    #[tokio::test]
    async fn supersede_allows_null_resolver_for_domain_auto() {
        // The domain-auto transfer has no human approver, so the retired
        // siblings carry a NULL resolved_by — same shape the auto-approved
        // row itself uses.
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let keep = ctx.create(700, "band-b", None).await.expect("keep");
        let sibling = ctx.create(700, "rando-x", None).await.expect("sibling");

        let n = ctx
            .supersede_other_pending_for_event(700, keep.id, None)
            .await
            .expect("supersede");
        assert_eq!(n, 1);

        let after = ctx.find_by_id(sibling.id).await.expect("sibling");
        assert_eq!(
            after.resolution_method.as_deref(),
            Some(resolution::SUPERSEDED)
        );
        assert!(after.resolved_by_user_id.is_none());
    }

    // -----------------------------------------------------------------
    // listing queries
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn list_by_requester_returns_newest_first() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        // Same requester, three different events (distinct pairs, so all
        // three may be pending simultaneously).
        let r1 = ctx.create(1, "lister", None).await.expect("c1");
        let r2 = ctx.create(2, "lister", None).await.expect("c2");
        let r3 = ctx.create(3, "lister", None).await.expect("c3");

        let list = ctx.list_by_requester("lister").await.expect("list");
        assert_eq!(list.len(), 3);
        // created_at ties at one-second resolution -> id DESC tie-break.
        assert_eq!(list[0].id, r3.id);
        assert_eq!(list[1].id, r2.id);
        assert_eq!(list[2].id, r1.id);
    }

    #[tokio::test]
    async fn list_pending_for_event_excludes_resolved() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        // One event, two requesters -> two pending rows.
        let a = ctx.create(500, "user-a", None).await.expect("a");
        let _b = ctx.create(500, "user-b", None).await.expect("b");
        // Resolve one of them.
        ctx.resolve(
            a.id,
            status::REJECTED,
            resolution::OWNER_REJECTION,
            Some("o"),
        )
        .await
        .expect("resolve a");

        let pending = ctx.list_pending_for_event(500).await.expect("list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].requester_user_id, "user-b");
    }

    #[tokio::test]
    async fn list_pending_for_events_spans_only_requested_ids() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        ctx.create(10, "u1", None).await.expect("10");
        ctx.create(11, "u2", None).await.expect("11");
        ctx.create(12, "u3", None).await.expect("12");

        // Ask for 10 and 12 — must exclude 11.
        let rows = ctx.list_pending_for_events(&[10, 12]).await.expect("list");
        let mut event_ids: Vec<i64> = rows.iter().map(|r| r.event_id).collect();
        event_ids.sort();
        assert_eq!(event_ids, vec![10, 12]);
    }

    #[tokio::test]
    async fn list_pending_for_events_empty_input_is_empty() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);
        // Even with rows present, an empty id set returns empty without a
        // malformed `IN ()` query.
        ctx.create(13, "u", None).await.expect("seed");
        let rows = ctx.list_pending_for_events(&[]).await.expect("list");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_all_pending_excludes_resolved_and_orders_newest_first() {
        let pool = setup_pool().await;
        let ctx = EventOwnershipRequestContext::new(pool);

        let r1 = ctx.create(20, "g1", None).await.expect("r1");
        let _r2 = ctx.create(21, "g2", None).await.expect("r2");
        let r3 = ctx.create(22, "g3", None).await.expect("r3");
        // Resolve the middle one; it should drop out of the pending sweep.
        let r2_again = ctx.create(23, "g4", None).await.expect("r2b");
        ctx.resolve(
            r2_again.id,
            status::APPROVED,
            resolution::ADMIN_APPROVAL,
            Some("a"),
        )
        .await
        .expect("resolve");

        let all = ctx.list_all_pending().await.expect("all pending");
        let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
        // r2_again resolved -> excluded. The rest present, newest id first.
        assert!(!ids.contains(&r2_again.id));
        assert_eq!(ids.first(), Some(&r3.id));
        assert!(ids.contains(&r1.id));
        assert_eq!(all.len(), 3);
    }
}
