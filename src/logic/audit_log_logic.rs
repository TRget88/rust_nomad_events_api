// src/logic/audit_log_logic.rs

use crate::context::AuditLogContext;
use crate::errors::AppError;
use crate::models::audit::{AuditEntry, AuditRecord};

/// Thin wrapper over the context — the audit log doesn't currently need
/// validation or cross-resource orchestration. It exists as its own layer so
/// the route layer follows the project's `routes → logic → context` rule and
/// future audit-policy logic (retention, redaction, signing) has a natural
/// home.
pub struct AuditLogLogic {
    context: AuditLogContext,
}

impl AuditLogLogic {
    pub fn new(context: AuditLogContext) -> Self {
        Self { context }
    }

    /// Record an admin action. If the write itself fails, the caller has
    /// usually already completed the underlying operation — we propagate the
    /// error so the route handler can log loudly (the action succeeded but
    /// its audit trail is missing), but we deliberately don't roll the
    /// operation back. Audit integrity is preferred over audit completeness
    /// when forced to choose; getting both means switching to a transaction-
    /// wrapped pattern, which is on the roadmap.
    pub async fn record(&self, record: AuditRecord) -> Result<(), AppError> {
        self.context.record(&record).await
    }

    /// Best-effort version: called by route handlers after their underlying
    /// op has already succeeded. If the audit write itself fails we log
    /// loudly via `tracing::error!` (so ops can see the gap in the trail)
    /// but DO NOT propagate the error to the caller — the user-visible
    /// outcome is what they requested, and rolling back a successful admin
    /// op because we couldn't write a log row is worse. Transactional
    /// audit logging (which would roll back) is the queued improvement.
    pub async fn record_best_effort(&self, record: AuditRecord) {
        if let Err(e) = self.context.record(&record).await {
            // `AppError` derives `Debug` but not `Display` — use `?e`.
            tracing::error!(
                error = ?e,
                action = %record.action,
                target_id = %record.target_id,
                "Admin action succeeded but audit log write failed",
            );
        }
    }

    pub async fn list_recent(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<AuditEntry>, AppError> {
        let (l, o) = crate::util::validate_pagination(limit, offset)?;
        self.context.list_recent(l, o).await
    }
}
