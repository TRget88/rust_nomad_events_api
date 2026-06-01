// src/models/audit.rs
//
// Data types for the admin audit log. Per the "models in `models/`" rule:
// data shapes live here; the logic that writes/reads these rows is in
// `logic/audit_log_logic.rs`; the SQL is in `context/audit_log_context.rs`.

use serde::{Deserialize, Serialize};

/// Canonical action strings used in `admin_audit_log.action`. Centralized
/// as constants so a typo at the call site is a compile error rather than
/// a silent log poisoning. Convention: `<resource>.<verb>`.
pub mod actions {
    // User admin
    pub const USER_UPDATE_ROLE: &str = "user.update_role";
    pub const USER_LOCK: &str = "user.lock";
    pub const USER_UNLOCK: &str = "user.unlock";
    pub const USER_DELETE: &str = "user.delete";

    // Event admin (delete only — create/update are user-owned operations
    // tracked separately via user_event_data; only the admin delete is
    // an irreversible cross-user action worth auditing).
    pub const EVENT_DELETE: &str = "event.delete";

    // Event type catalog (SuperAdmin-only). All three verbs change global
    // catalog state visible to every user, so all three get audited.
    pub const EVENT_TYPE_CREATE: &str = "event_type.create";
    pub const EVENT_TYPE_UPDATE: &str = "event_type.update";
    pub const EVENT_TYPE_DELETE: &str = "event_type.delete";

    // Camping profile catalog (SuperAdmin-only). Same rationale as event_type.
    pub const CAMPING_PROFILE_CREATE: &str = "camping_profile.create";
    pub const CAMPING_PROFILE_UPDATE: &str = "camping_profile.update";
    pub const CAMPING_PROFILE_DELETE: &str = "camping_profile.delete";
}

/// Canonical target_type strings. Same logic as the actions module.
pub mod target_types {
    pub const USER: &str = "user";
    pub const EVENT: &str = "event";
    pub const EVENT_TYPE: &str = "event_type";
    pub const CAMPING_PROFILE: &str = "camping_profile";
}

/// What an admin caller wants to record. Constructed at the route layer
/// where the actor's UUID (from `Claims`), the action, and the target are
/// known. `metadata` is a free-form JSON value — the schema convention by
/// action is documented in the migration comment.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub actor_user_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub metadata: serde_json::Value,
}

/// What `GET /admin/audit-log` returns per row. `metadata` is parsed back
/// from JSON so consumers don't have to do it themselves.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub actor_user_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

/// Query params for `GET /admin/audit-log`. Lives here per the
/// "data types in `models/`" rule. Limit/offset semantics match the
/// shared `util::validate_pagination` contract: defaults are
/// `DEFAULT_PAGINATION_LIMIT` / 0, caps at `MAX_PAGINATION_LIMIT`,
/// rejects out-of-range with 400.
#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
