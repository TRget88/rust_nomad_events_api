// ============================================================================
// src/logic/event_ownership_request_logic.rs - Business Logic Layer
// ============================================================================
//
// Orchestrates the "claim / request ownership of an existing event" flow.
// Ownership itself has no column — it is membership of an event id in a
// user's `user_event_data.created_events` list — so a *transfer* is
// `remove_event_ownership(old)` + `event_ownership(new)`. This layer owns
// the rules around when that transfer is allowed:
//
//   * request_ownership  — file a pending request, OR auto-approve on the
//                          spot if the requester's VERIFIED email domain
//                          matches the event website domain.
//   * approve_request    — the event's current owner, or any admin,
//                          approves; ownership transfers to the requester.
//   * reject_request     — owner or admin declines; no transfer.
//   * list_my_requests   — the requester's outgoing list.
//   * list_incoming_*    — the owner's review queue (admins: global).
//
// Audit recording happens at the ROUTE layer (mirroring `delete_event`),
// so this layer holds no `AuditLogLogic`; it returns rich outcome types
// (`RequestOutcome` / `ApprovalOutcome` / `RejectionOutcome`) the route
// turns into audit records.

use crate::context::{EventContext, EventOwnershipRequestContext, UserContext};
use crate::errors::AppError;
use crate::logic::UserCollectionLogic;
use crate::logic::domain_match::{domain_from_email, domains_match, registrable_domain};
use crate::models::ownership::{
    ApprovalOutcome, EventOwnershipRequestRow, RejectionOutcome, RequestOutcome, resolution, status,
};
use crate::models::user::{Claims, UserRole};
use std::sync::Arc;

/// Upper bound on the free-text note a requester may attach. Bounded here
/// (the API layer) rather than the DB so the error is a clean 400 instead
/// of a truncation or a constraint surprise.
const MAX_NOTE_LEN: usize = 1000;

pub struct EventOwnershipRequestLogic {
    repository: EventOwnershipRequestContext,
    // Shared with `EventLogic` — used to confirm the target event exists
    // and to read its `website` for the domain-verification check.
    events_context: Arc<EventContext>,
    // Owned (a fresh `UserContext`): the one `main.rs` builds first is
    // moved into `UserLogic`. Used to read the requester's `email` +
    // `email_verified` (the JWT `Claims` carry the email but NOT the
    // verified flag, so we must look it up).
    user_context: UserContext,
    // The ownership ledger. `find_event_owner` (reverse lookup),
    // `event_ownership` / `remove_event_ownership` (the transfer), and
    // `get` (to read a user's owned-event list) all live here.
    user_collection_logic: Arc<UserCollectionLogic>,
}

impl EventOwnershipRequestLogic {
    pub fn new(
        repository: EventOwnershipRequestContext,
        events_context: Arc<EventContext>,
        user_context: UserContext,
        user_collection_logic: Arc<UserCollectionLogic>,
    ) -> Self {
        Self {
            repository,
            events_context,
            user_context,
            user_collection_logic,
        }
    }

    /// File a request to own `event_id`. Returns `RequestOutcome::Pending`
    /// for the normal "awaiting a human" path, or
    /// `RequestOutcome::AutoApproved` when the requester's verified email
    /// domain matches the event website and the transfer happened inline.
    pub async fn request_ownership(
        &self,
        event_id: i64,
        claims: &Claims,
        note: Option<String>,
    ) -> Result<RequestOutcome, AppError> {
        // 1. Normalize + bound the note (empty/whitespace -> no note).
        let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if let Some(n) = &note
            && n.chars().count() > MAX_NOTE_LEN
        {
            return Err(AppError::ValidationError(format!(
                "Note must be at most {MAX_NOTE_LEN} characters"
            )));
        }

        // 2. The event must exist (NotFound otherwise).
        let event = self.events_context.find_by_id(event_id).await?;

        // 3. You can't request ownership of an event you already own.
        let current_owner = self
            .user_collection_logic
            .find_event_owner(event_id)
            .await?;
        if current_owner.as_deref() == Some(claims.sub.as_str()) {
            return Err(AppError::Conflict("You already own this event".to_string()));
        }

        // 4. File the pending request. A duplicate OPEN request for the
        //    same (event, requester) surfaces as Conflict via the partial
        //    unique index.
        let request = self
            .repository
            .create(event_id, &claims.sub, note.as_deref())
            .await?;

        // 5. Auto-approval: a VERIFIED email whose domain matches the event
        //    website transfers ownership with no human in the loop. Any
        //    weaker signal leaves the request pending for owner/admin review.
        let requester = self.user_context.find_by_id(&claims.sub).await?;
        let domain_ok = requester.email_verified
            && match (&requester.email, &event.website) {
                (Some(email), Some(website)) => domains_match(email, website),
                _ => false,
            };

        if domain_ok {
            // domains_match only returns true when both sides parse, so the
            // registrable domain is recoverable; default to "" for the audit
            // label rather than panicking on the (unreachable) None.
            let matched_domain = requester
                .email
                .as_deref()
                .and_then(domain_from_email)
                .and_then(|h| registrable_domain(&h))
                .unwrap_or_default();

            // CAS-first (mirrors approve_request): move ownership via the
            // authoritative creator_id column BEFORE marking the freshly
            // filed request approved. In the rare event two verified-domain
            // requesters race on the same event, the loser's CAS finds the
            // column already claimed and returns Conflict here; its pending
            // request is then superseded by the winner. See BUGS.md F1.
            self.transfer_ownership(event_id, current_owner.as_deref(), &claims.sub)
                .await?;
            self.repository
                .resolve(request.id, status::APPROVED, resolution::DOMAIN_AUTO, None)
                .await?;

            // The inline transfer moved ownership; any OTHER request still
            // pending on this event can never be granted now, so retire them.
            // resolved_by is None to mirror the no-human domain-auto path.
            let superseded_count = self
                .repository
                .supersede_other_pending_for_event(event_id, request.id, None)
                .await?;

            let resolved = self.repository.find_by_id(request.id).await?;
            return Ok(RequestOutcome::AutoApproved {
                request: resolved,
                previous_owner: current_owner,
                matched_domain,
                superseded_count,
            });
        }

        Ok(RequestOutcome::Pending(request))
    }

    /// Approve a pending request. The event's current owner OR any admin
    /// may approve; ownership transfers to the requester. Conflict if the
    /// request is already resolved (including a concurrent resolution that
    /// wins the compare-and-set).
    pub async fn approve_request(
        &self,
        request_id: i64,
        claims: &Claims,
    ) -> Result<ApprovalOutcome, AppError> {
        let request = self.repository.find_by_id(request_id).await?;
        if request.status != status::PENDING {
            return Err(AppError::Conflict(
                "This request has already been resolved".to_string(),
            ));
        }

        let current_owner = self
            .user_collection_logic
            .find_event_owner(request.event_id)
            .await?;
        let is_owner = current_owner.as_deref() == Some(claims.sub.as_str());
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if !is_owner && !is_admin {
            return Err(AppError::Forbidden(
                "Only the event owner or an admin can approve this request".to_string(),
            ));
        }
        // Owner approval is the more specific method when the actor is both.
        let method = if is_owner {
            resolution::OWNER_APPROVAL
        } else {
            resolution::ADMIN_APPROVAL
        };

        // CAS-first: move ownership via the authoritative creator_id column
        // BEFORE marking the request approved. If a concurrent approval
        // already moved ownership, the CAS inside transfer_ownership matches
        // zero rows and returns Conflict here — the request stays pending
        // (its resolve never runs) and is later superseded by whichever
        // approval won, keeping ownership single-valued and the audit trail
        // unambiguous (one approved, one superseded). See BUGS.md F1.
        self.transfer_ownership(
            request.event_id,
            current_owner.as_deref(),
            &request.requester_user_id,
        )
        .await?;

        let resolved_ok = self
            .repository
            .resolve(request_id, status::APPROVED, method, Some(&claims.sub))
            .await?;
        if !resolved_ok {
            // Ownership moved (our CAS won) but another actor resolved this
            // exact request first — e.g. a concurrent reject of the same id.
            // Astronomically rare; ownership stays single-valued, only the
            // labeling of this one request raced. Surface a Conflict.
            return Err(AppError::Conflict(
                "This request has already been resolved".to_string(),
            ));
        }

        // Ownership has moved to the requester; retire every OTHER request
        // still pending on this event so a stale sibling can't later be
        // approved into a second owner.
        let superseded_count = self
            .repository
            .supersede_other_pending_for_event(request.event_id, request_id, Some(&claims.sub))
            .await?;

        let resolved = self.repository.find_by_id(request_id).await?;
        Ok(ApprovalOutcome {
            request: resolved,
            method,
            previous_owner: current_owner,
            new_owner: request.requester_user_id,
            superseded_count,
        })
    }

    /// Reject a pending request. Owner or admin only. No ownership moves.
    pub async fn reject_request(
        &self,
        request_id: i64,
        claims: &Claims,
    ) -> Result<RejectionOutcome, AppError> {
        let request = self.repository.find_by_id(request_id).await?;
        if request.status != status::PENDING {
            return Err(AppError::Conflict(
                "This request has already been resolved".to_string(),
            ));
        }

        let current_owner = self
            .user_collection_logic
            .find_event_owner(request.event_id)
            .await?;
        let is_owner = current_owner.as_deref() == Some(claims.sub.as_str());
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if !is_owner && !is_admin {
            return Err(AppError::Forbidden(
                "Only the event owner or an admin can reject this request".to_string(),
            ));
        }
        let method = if is_owner {
            resolution::OWNER_REJECTION
        } else {
            resolution::ADMIN_REJECTION
        };

        let resolved_ok = self
            .repository
            .resolve(request_id, status::REJECTED, method, Some(&claims.sub))
            .await?;
        if !resolved_ok {
            return Err(AppError::Conflict(
                "This request has already been resolved".to_string(),
            ));
        }

        let resolved = self.repository.find_by_id(request_id).await?;
        Ok(RejectionOutcome {
            request: resolved,
            method,
        })
    }

    /// The requester's outgoing list — every request they've filed.
    pub async fn list_my_requests(
        &self,
        claims: &Claims,
    ) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        self.repository.list_by_requester(&claims.sub).await
    }

    /// The review queue. A regular user sees pending requests for events
    /// they own; an admin sees the global pending queue (which is the only
    /// way requests for unowned seed events ever get resolved).
    pub async fn list_incoming_requests(
        &self,
        claims: &Claims,
    ) -> Result<Vec<EventOwnershipRequestRow>, AppError> {
        let is_admin = matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin);
        if is_admin {
            return self.repository.list_all_pending().await;
        }
        let collection = self.user_collection_logic.get(&claims.sub).await?;
        self.repository
            .list_pending_for_events(&collection.created_events)
            .await
    }

    /// Move ownership of `event_id` from `previous_owner` (if any) to
    /// `new_owner`, gated by an atomic compare-and-set on the authoritative
    /// `events.creator_id` column (migration 00009).
    ///
    /// The CAS only flips the row while its current owner still equals
    /// `previous_owner` — the value the caller read moments earlier. If a
    /// concurrent approval moved ownership out from under us in that window,
    /// the CAS matches zero rows and we return `Conflict` instead of
    /// minting a second owner. This is the durable fix for the
    /// dual-ownership race (BUGS.md F1): ownership is single-valued at the
    /// DB level, not by a best-effort sequence of JSON-array edits.
    ///
    /// Only AFTER the CAS wins do we touch the derived `created_events`
    /// array cache: drop the id from the previous owner's list and add it
    /// to the new owner's. The add is idempotent (`event_ownership` pushes
    /// without dedup, so we skip it when the id is already present); its
    /// internal claim-if-null no-ops because the CAS just set creator_id.
    async fn transfer_ownership(
        &self,
        event_id: i64,
        previous_owner: Option<&str>,
        new_owner: &str,
    ) -> Result<(), AppError> {
        let won = self
            .events_context
            .claim_ownership(event_id, new_owner, previous_owner)
            .await?;
        if !won {
            return Err(AppError::Conflict(
                "Ownership of this event changed before the request could be approved".to_string(),
            ));
        }

        if let Some(prev) = previous_owner
            && prev != new_owner
        {
            self.user_collection_logic
                .remove_event_ownership(event_id, &prev.to_string())
                .await?;
        }
        let new_collection = self
            .user_collection_logic
            .get(&new_owner.to_string())
            .await?;
        if !new_collection.created_events.contains(&event_id) {
            self.user_collection_logic
                .event_ownership(event_id, &new_owner.to_string())
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Logic-layer integration tests. These construct the real logic over
    //! an in-memory pool and seed users/events directly, so they exercise
    //! the actual branches — auto-approval, RBAC, the ownership transfer —
    //! without the HTTP layer. Route wiring + audit emission is covered by
    //! the route tests in `routes/event_ownership.rs`.

    use super::EventOwnershipRequestLogic;
    use crate::context::{
        EventContext, EventOwnershipRequestContext, MicroeventContext, UserCollectionContext,
        UserContext,
    };
    use crate::errors::AppError;
    use crate::logic::UserCollectionLogic;
    use crate::models::ownership::{RequestOutcome, resolution, status};
    use crate::models::user::{Claims, UserRole};
    use sqlx::SqlitePool;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    // --- fixtures -----------------------------------------------------

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        // Seed an event_type so the EventContext JOIN resolves. Migrations
        // already seed an id=1 sentinel ("Uncategorized"), so use INSERT OR
        // IGNORE to keep event_type_id=1 valid without colliding on the PK.
        sqlx::query(
            "INSERT OR IGNORE INTO event_types (id, name, description, map_indicator, category) \
             VALUES (1, 'Festival', 'A festival', 'F', 'entertainment')",
        )
        .execute(&pool)
        .await
        .expect("seed event_type");
        pool
    }

    /// Build the logic plus the `UserCollectionLogic` (returned so tests
    /// can seed initial ownership and assert the final owner).
    fn make_logic(pool: &SqlitePool) -> (EventOwnershipRequestLogic, Arc<UserCollectionLogic>) {
        let events_context = Arc::new(EventContext::new(pool.clone()));
        let microevents_context = Arc::new(MicroeventContext::new(pool.clone()));
        let ucc = UserCollectionContext::new(pool.clone());
        let ucl = Arc::new(UserCollectionLogic::new(
            ucc,
            events_context.clone(),
            microevents_context.clone(),
        ));
        let user_context = UserContext::new(pool.clone());
        let eor_ctx = EventOwnershipRequestContext::new(pool.clone());
        let logic = EventOwnershipRequestLogic::new(
            eor_ctx,
            events_context.clone(),
            user_context,
            ucl.clone(),
        );
        (logic, ucl)
    }

    async fn seed_user(
        pool: &SqlitePool,
        id: &str,
        email: Option<&str>,
        verified: bool,
        role: &str,
    ) {
        sqlx::query(
            "INSERT INTO users \
             (id, oauth_id, oauth_provider, user_name, email, email_verified, role, created_at, updated_at) \
             VALUES (?1, ?2, 'google', ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
        )
        .bind(id)
        .bind(format!("oauth-{id}"))
        .bind(format!("name-{id}"))
        .bind(email)
        .bind(verified)
        .bind(role)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_event(pool: &SqlitePool, name: &str, website: Option<&str>) -> i64 {
        let event_data = serde_json::json!({ "name": name, "archive": false }).to_string();
        let res = sqlx::query(
            "INSERT INTO events (name, description, website, event_type_id, event_data) \
             VALUES (?1, ?2, ?3, 1, ?4)",
        )
        .bind(name)
        .bind(format!("{name} description"))
        .bind(website)
        .bind(event_data)
        .execute(pool)
        .await
        .expect("seed event");
        res.last_insert_rowid()
    }

    fn claims(sub: &str, email: &str, role: UserRole) -> Claims {
        Claims {
            sub: sub.to_string(),
            email: email.to_string(),
            username: "tester".to_string(),
            role,
            exp: 9_999_999_999,
            iat: 0,
            jti: "test-jti".to_string(),
            aud: "festurah-api".to_string(),
        }
    }

    // --- request_ownership: auto-approval branches --------------------

    #[tokio::test]
    async fn request_auto_approves_on_verified_domain_match() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", Some("a@old.example"), true, "user").await;
        seed_user(&pool, "band-b", Some("booking@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let outcome = logic
            .request_ownership(
                event,
                &claims("band-b", "booking@coolfest.com", UserRole::User),
                None,
            )
            .await
            .expect("request");

        match outcome {
            RequestOutcome::AutoApproved {
                request,
                previous_owner,
                matched_domain,
                superseded_count,
            } => {
                assert_eq!(superseded_count, 0, "no sibling claims to retire");
                assert_eq!(request.status, status::APPROVED);
                assert_eq!(
                    request.resolution_method.as_deref(),
                    Some(resolution::DOMAIN_AUTO)
                );
                assert!(request.resolved_by_user_id.is_none(), "no human approver");
                assert_eq!(previous_owner.as_deref(), Some("owner-a"));
                assert_eq!(matched_domain, "coolfest.com");
            }
            other => panic!("expected AutoApproved, got {:?}", other),
        }

        // Ownership actually moved from A to B.
        let owner = ucl.find_event_owner(event).await.expect("owner");
        assert_eq!(owner.as_deref(), Some("band-b"));
    }

    #[tokio::test]
    async fn request_auto_approves_and_claims_unowned_event() {
        // Seed/curated event nobody owns + a verified domain match ->
        // auto-approve with previous_owner None.
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "band-b", Some("info@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("http://www.coolfest.com/tickets")).await;

        let outcome = logic
            .request_ownership(
                event,
                &claims("band-b", "info@coolfest.com", UserRole::User),
                None,
            )
            .await
            .expect("request");

        match outcome {
            RequestOutcome::AutoApproved { previous_owner, .. } => {
                assert!(previous_owner.is_none());
            }
            other => panic!("expected AutoApproved, got {:?}", other),
        }
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("band-b")
        );
    }

    #[tokio::test]
    async fn request_stays_pending_when_email_unverified() {
        // Domain matches, but the email is NOT verified -> no auto-approve.
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("booking@coolfest.com"), false, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let outcome = logic
            .request_ownership(
                event,
                &claims("band-b", "booking@coolfest.com", UserRole::User),
                None,
            )
            .await
            .expect("request");

        assert!(matches!(outcome, RequestOutcome::Pending(_)));
        // Ownership unchanged.
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn request_stays_pending_on_domain_mismatch() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "rando-b", Some("me@gmail.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let outcome = logic
            .request_ownership(
                event,
                &claims("rando-b", "me@gmail.com", UserRole::User),
                None,
            )
            .await
            .expect("request");

        assert!(matches!(outcome, RequestOutcome::Pending(_)));
    }

    #[tokio::test]
    async fn request_stays_pending_when_event_has_no_website() {
        let pool = setup_pool().await;
        let (logic, _ucl) = make_logic(&pool);

        seed_user(&pool, "band-b", Some("booking@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;

        let outcome = logic
            .request_ownership(
                event,
                &claims("band-b", "booking@coolfest.com", UserRole::User),
                None,
            )
            .await
            .expect("request");

        assert!(matches!(outcome, RequestOutcome::Pending(_)));
    }

    // --- request_ownership: guards ------------------------------------

    #[tokio::test]
    async fn request_conflicts_when_already_owner() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-b", Some("b@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;
        ucl.event_ownership(event, &"owner-b".to_string())
            .await
            .expect("seed ownership");

        let err = logic
            .request_ownership(
                event,
                &claims("owner-b", "b@coolfest.com", UserRole::User),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn duplicate_pending_request_conflicts() {
        let pool = setup_pool().await;
        let (logic, _ucl) = make_logic(&pool);

        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        let c = claims("band-b", "b@nomatch.example", UserRole::User);

        // First request parks pending.
        logic
            .request_ownership(event, &c, None)
            .await
            .expect("first");
        // Second open request for the same pair -> Conflict.
        let err = logic.request_ownership(event, &c, None).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn request_unknown_event_is_not_found() {
        let pool = setup_pool().await;
        let (logic, _ucl) = make_logic(&pool);
        seed_user(&pool, "band-b", Some("b@x.example"), true, "user").await;

        let err = logic
            .request_ownership(
                999_999,
                &claims("band-b", "b@x.example", UserRole::User),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn request_rejects_overlong_note() {
        let pool = setup_pool().await;
        let (logic, _ucl) = make_logic(&pool);
        seed_user(&pool, "band-b", Some("b@x.example"), true, "user").await;
        let event = seed_event(&pool, "Fest", None).await;

        let huge = "x".repeat(1001);
        let err = logic
            .request_ownership(
                event,
                &claims("band-b", "b@x.example", UserRole::User),
                Some(huge),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)), "got {:?}", err);
    }

    // --- approve_request ----------------------------------------------

    #[tokio::test]
    async fn owner_approves_and_ownership_transfers() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        // B files a request that stays pending (no website -> no auto).
        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        // Owner A approves.
        let outcome = logic
            .approve_request(
                pending.id,
                &claims("owner-a", "a@x.example", UserRole::User),
            )
            .await
            .expect("approve");
        assert_eq!(outcome.method, resolution::OWNER_APPROVAL);
        assert_eq!(outcome.previous_owner.as_deref(), Some("owner-a"));
        assert_eq!(outcome.new_owner, "band-b");
        assert_eq!(outcome.request.status, status::APPROVED);

        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("band-b")
        );
    }

    #[tokio::test]
    async fn admin_approves_unowned_event_request() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "admin-c", None, true, "admin").await;
        let event = seed_event(&pool, "Seed Fest", None).await; // unowned

        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        let outcome = logic
            .approve_request(
                pending.id,
                &claims("admin-c", "c@x.example", UserRole::Admin),
            )
            .await
            .expect("approve");
        assert_eq!(outcome.method, resolution::ADMIN_APPROVAL);
        assert!(outcome.previous_owner.is_none());
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("band-b")
        );
    }

    #[tokio::test]
    async fn non_owner_non_admin_cannot_approve() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "rando-d", None, true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        let err = logic
            .approve_request(
                pending.id,
                &claims("rando-d", "d@x.example", UserRole::User),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "got {:?}", err);
        // Ownership untouched.
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn approving_already_resolved_request_conflicts() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };
        let owner_claims = claims("owner-a", "a@x.example", UserRole::User);
        logic
            .approve_request(pending.id, &owner_claims)
            .await
            .expect("first approve");

        // Second approve hits the already-resolved guard.
        let err = logic
            .approve_request(pending.id, &owner_claims)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn approving_one_request_supersedes_other_pending_for_same_event() {
        // Two different users file pending claims on the SAME event (allowed
        // — the partial unique index only blocks a *duplicate* (event, user)
        // pair, not two distinct users). The owner approves one; the other
        // must be retired as 'superseded' so it can NEVER later be approved
        // into a second owner. This is the dual-ownership avenue from BUGS.md.
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "band-c", Some("c@nomatch.example"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let req_b = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("b request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };
        let req_c = match logic
            .request_ownership(
                event,
                &claims("band-c", "c@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("c request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        // Owner approves B's request.
        let outcome = logic
            .approve_request(req_b.id, &claims("owner-a", "a@x.example", UserRole::User))
            .await
            .expect("approve b");
        assert_eq!(outcome.new_owner, "band-b");
        assert_eq!(outcome.superseded_count, 1, "C's pending claim was retired");

        // Ownership sits ONLY with B — C never became a co-owner.
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("band-b")
        );

        // C's request is now resolved as superseded (status rejected), not
        // pending.
        let (c_status, c_method): (String, Option<String>) = sqlx::query_as(
            "SELECT status, resolution_method FROM event_ownership_requests WHERE id = ?1",
        )
        .bind(req_c.id)
        .fetch_one(&pool)
        .await
        .expect("read c");
        assert_eq!(c_status, status::REJECTED);
        assert_eq!(c_method.as_deref(), Some(resolution::SUPERSEDED));

        // And re-approving C must Conflict rather than mint a second owner.
        let err = logic
            .approve_request(req_c.id, &claims("owner-a", "a@x.example", UserRole::User))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {:?}", err);
    }

    #[tokio::test]
    async fn auto_approval_supersedes_other_pending_for_same_event() {
        // A non-matching user parks a pending claim, then a verified-domain
        // requester auto-approves. The inline transfer must also retire the
        // earlier pending claim — same dual-ownership guard as the manual
        // approve path, on the no-human path.
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "rando-x", Some("x@gmail.com"), true, "user").await;
        seed_user(&pool, "band-b", Some("booking@coolfest.com"), true, "user").await;
        let event = seed_event(&pool, "Cool Fest", Some("https://coolfest.com")).await;

        // X files first — a gmail address can't auto-approve, so it parks.
        let req_x = match logic
            .request_ownership(
                event,
                &claims("rando-x", "x@gmail.com", UserRole::User),
                None,
            )
            .await
            .expect("x request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        // B's verified domain matches -> auto-approve + inline transfer.
        let outcome = logic
            .request_ownership(
                event,
                &claims("band-b", "booking@coolfest.com", UserRole::User),
                None,
            )
            .await
            .expect("b request");
        match outcome {
            RequestOutcome::AutoApproved {
                superseded_count, ..
            } => assert_eq!(superseded_count, 1, "X's pending claim was retired"),
            other => panic!("expected AutoApproved, got {:?}", other),
        }

        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("band-b")
        );

        // X's earlier claim is superseded, with a NULL resolver (no human
        // acted on the domain-auto path), and can't be approved later.
        let (x_status, x_method, x_resolver): (String, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT status, resolution_method, resolved_by_user_id \
                 FROM event_ownership_requests WHERE id = ?1",
            )
            .bind(req_x.id)
            .fetch_one(&pool)
            .await
            .expect("read x");
        assert_eq!(x_status, status::REJECTED);
        assert_eq!(x_method.as_deref(), Some(resolution::SUPERSEDED));
        assert!(
            x_resolver.is_none(),
            "auto-path supersede records no human resolver"
        );
    }

    // --- reject_request -----------------------------------------------

    #[tokio::test]
    async fn owner_rejects_without_transfer() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        let outcome = logic
            .reject_request(
                pending.id,
                &claims("owner-a", "a@x.example", UserRole::User),
            )
            .await
            .expect("reject");
        assert_eq!(outcome.method, resolution::OWNER_REJECTION);
        assert_eq!(outcome.request.status, status::REJECTED);
        // Ownership stays with A.
        assert_eq!(
            ucl.find_event_owner(event).await.expect("owner").as_deref(),
            Some("owner-a")
        );
    }

    #[tokio::test]
    async fn non_owner_cannot_reject() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "rando-d", None, true, "user").await;
        let event = seed_event(&pool, "No Website Fest", None).await;
        ucl.event_ownership(event, &"owner-a".to_string())
            .await
            .expect("seed ownership");

        let pending = match logic
            .request_ownership(
                event,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("request")
        {
            RequestOutcome::Pending(r) => r,
            other => panic!("expected Pending, got {:?}", other),
        };

        let err = logic
            .reject_request(
                pending.id,
                &claims("rando-d", "d@x.example", UserRole::User),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "got {:?}", err);
    }

    // --- listings -----------------------------------------------------

    #[tokio::test]
    async fn list_my_requests_returns_only_mine() {
        let pool = setup_pool().await;
        let (logic, _ucl) = make_logic(&pool);

        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "band-c", Some("c@nomatch.example"), true, "user").await;
        let e1 = seed_event(&pool, "Fest 1", None).await;
        let e2 = seed_event(&pool, "Fest 2", None).await;

        logic
            .request_ownership(
                e1,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("b->e1");
        logic
            .request_ownership(
                e2,
                &claims("band-c", "c@nomatch.example", UserRole::User),
                None,
            )
            .await
            .expect("c->e2");

        let mine = logic
            .list_my_requests(&claims("band-b", "b@nomatch.example", UserRole::User))
            .await
            .expect("list mine");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].event_id, e1);
    }

    #[tokio::test]
    async fn list_incoming_for_owner_scopes_to_owned_events() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "owner-z", None, true, "user").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "band-c", Some("c@nomatch.example"), true, "user").await;
        seed_user(&pool, "band-d", Some("d@nomatch.example"), true, "user").await;

        let e1 = seed_event(&pool, "A's Fest 1", None).await;
        let e2 = seed_event(&pool, "A's Fest 2", None).await;
        let e3 = seed_event(&pool, "Z's Fest", None).await;
        ucl.event_ownership(e1, &"owner-a".to_string())
            .await
            .unwrap();
        ucl.event_ownership(e2, &"owner-a".to_string())
            .await
            .unwrap();
        ucl.event_ownership(e3, &"owner-z".to_string())
            .await
            .unwrap();

        logic
            .request_ownership(
                e1,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .unwrap();
        logic
            .request_ownership(
                e2,
                &claims("band-c", "c@nomatch.example", UserRole::User),
                None,
            )
            .await
            .unwrap();
        logic
            .request_ownership(
                e3,
                &claims("band-d", "d@nomatch.example", UserRole::User),
                None,
            )
            .await
            .unwrap();

        let incoming = logic
            .list_incoming_requests(&claims("owner-a", "a@x.example", UserRole::User))
            .await
            .expect("incoming");
        let mut event_ids: Vec<i64> = incoming.iter().map(|r| r.event_id).collect();
        event_ids.sort();
        assert_eq!(event_ids, vec![e1, e2]); // NOT e3 (owned by Z)
    }

    #[tokio::test]
    async fn list_incoming_for_admin_returns_global_queue() {
        let pool = setup_pool().await;
        let (logic, ucl) = make_logic(&pool);

        seed_user(&pool, "owner-a", None, true, "user").await;
        seed_user(&pool, "admin-c", None, true, "admin").await;
        seed_user(&pool, "band-b", Some("b@nomatch.example"), true, "user").await;
        seed_user(&pool, "band-d", Some("d@nomatch.example"), true, "user").await;

        let owned = seed_event(&pool, "Owned Fest", None).await;
        let seed_evt = seed_event(&pool, "Seed Fest", None).await; // unowned
        ucl.event_ownership(owned, &"owner-a".to_string())
            .await
            .unwrap();

        logic
            .request_ownership(
                owned,
                &claims("band-b", "b@nomatch.example", UserRole::User),
                None,
            )
            .await
            .unwrap();
        logic
            .request_ownership(
                seed_evt,
                &claims("band-d", "d@nomatch.example", UserRole::User),
                None,
            )
            .await
            .unwrap();

        // Admin sees BOTH — including the request for the unowned seed event
        // that no regular user's incoming list would ever surface.
        let incoming = logic
            .list_incoming_requests(&claims("admin-c", "c@x.example", UserRole::Admin))
            .await
            .expect("incoming");
        assert_eq!(incoming.len(), 2);
    }
}
