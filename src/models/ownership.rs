// ============================================================================
// src/models/ownership.rs
// ============================================================================
//
// Data shapes for the event-ownership-request workflow (migration 00008,
// table `event_ownership_requests`). Per the "models in `models/`" rule:
// the row + request DTO + logic-outcome types live here; the SQL is in
// `context/event_ownership_request_context.rs`; the business rules in
// `logic/event_ownership_request_logic.rs`; the HTTP handlers in
// `routes/event_ownership.rs`.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Canonical `status` column values. A request is born 'pending' (the
/// migration defaults it) and leaves that state exactly once.
pub mod status {
    pub const PENDING: &str = "pending";
    pub const APPROVED: &str = "approved";
    pub const REJECTED: &str = "rejected";
}

/// Canonical `resolution_method` column values — the "why" behind a
/// resolved request. NULL while pending. See the migration header for the
/// full narrative.
pub mod resolution {
    /// The event's current owner approved the transfer.
    pub const OWNER_APPROVAL: &str = "owner_approval";
    /// An Admin/SuperAdmin approved (fallback for seed/unowned events and
    /// general oversight of the queue).
    pub const ADMIN_APPROVAL: &str = "admin_approval";
    /// Auto-approved at request time: the requester's *verified* email
    /// domain matched the event website domain, so no human approved.
    pub const DOMAIN_AUTO: &str = "domain_auto";
    /// The owner declined.
    pub const OWNER_REJECTION: &str = "owner_rejection";
    /// An admin declined.
    pub const ADMIN_REJECTION: &str = "admin_rejection";
    /// Auto-retired as a side effect of *another* request for the same
    /// event being approved: ownership has already moved, so this still-
    /// pending sibling can never be granted. Stored with `status='rejected'`
    /// (it was not granted) while this method distinguishes it from a human
    /// `*_rejection`. `resolved_by_user_id` is the approver on the human
    /// paths, or NULL on the domain-auto path (no human acted). Closing the
    /// dual-ownership race documented in BUGS.md.
    pub const SUPERSEDED: &str = "superseded";
}

/// One row of `event_ownership_requests`. Serialized straight back to the
/// API (column names == JSON field names), so it doubles as the response
/// DTO — the shape is identical and there's nothing sensitive to hide.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct EventOwnershipRequestRow {
    pub id: i64,
    pub event_id: i64,
    pub requester_user_id: String,
    pub status: String,
    pub note: Option<String>,
    pub resolution_method: Option<String>,
    pub resolved_by_user_id: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

/// Request body for `POST /event/{id}/ownership-request`. The note is the
/// only client-supplied field — `event_id` comes from the path, the
/// requester from the JWT. Optional so a bare `{}` (or, via
/// `Option<ApiJson<_>>`, no body at all) is a valid "no note" request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OwnershipRequestInput {
    #[serde(default)]
    pub note: Option<String>,
}

/// Outcome of `request_ownership`. The request either parks pending a
/// human decision, or the verified-domain check fired and the transfer
/// completed immediately. The route uses this to emit the right audit
/// records (a plain `request`, or `request` + `auto_approve`).
#[derive(Debug, Clone)]
pub enum RequestOutcome {
    /// Parked. Awaiting owner or admin action.
    Pending(EventOwnershipRequestRow),
    /// Auto-approved via domain verification; ownership already moved.
    AutoApproved {
        request: EventOwnershipRequestRow,
        /// Previous owner whose `created_events` lost the event, or `None`
        /// if the event was unowned (seed / curator-added).
        previous_owner: Option<String>,
        /// The registrable domain that matched — recorded in the audit
        /// metadata so the auto-approval is explainable after the fact.
        matched_domain: String,
        /// Count of *other* pending requests for this event retired by the
        /// inline transfer (see `resolution::SUPERSEDED`). Usually 0.
        superseded_count: u64,
    },
}

/// Outcome of `approve_request` — a human (owner or admin) approved.
/// Ownership has already moved by the time this returns.
#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    pub request: EventOwnershipRequestRow,
    /// `resolution::OWNER_APPROVAL` or `resolution::ADMIN_APPROVAL`.
    pub method: &'static str,
    /// Previous owner whose ownership was removed, or `None` for an
    /// unowned event approved by an admin.
    pub previous_owner: Option<String>,
    /// The requester, who now owns the event.
    pub new_owner: String,
    /// How many *other* still-pending requests for the same event this
    /// approval retired as `resolution::SUPERSEDED`. Usually 0; >0 only
    /// when several users held open claims on one event.
    pub superseded_count: u64,
}

/// Outcome of `reject_request`.
#[derive(Debug, Clone)]
pub struct RejectionOutcome {
    pub request: EventOwnershipRequestRow,
    /// `resolution::OWNER_REJECTION` or `resolution::ADMIN_REJECTION`.
    pub method: &'static str,
}
