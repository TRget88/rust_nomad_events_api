// src/logic/jwt_revocation_logic.rs

use crate::context::JwtRevocationContext;
use crate::errors::AppError;
use crate::models::user::Claims;

/// Business logic for JWT revocation. Sits between routes (which extract the
/// Claims from the request) and the context (which performs the SQL).
/// Validates that the claim is actually revocable before touching the DB:
/// legacy tokens without a `jti` (issued before the rollout) can't be
/// revoked because there's no key to put on the list — we surface that as a
/// clear error so the caller can decide what to do.
pub struct JwtRevocationLogic {
    context: JwtRevocationContext,
}

impl JwtRevocationLogic {
    pub fn new(context: JwtRevocationContext) -> Self {
        Self { context }
    }

    pub async fn is_revoked(&self, jti: &str) -> Result<bool, AppError> {
        // Empty jti (legacy token) is never on the list — short-circuit
        // rather than firing a query that can never match.
        if jti.is_empty() {
            return Ok(false);
        }
        self.context.is_revoked(jti).await
    }

    /// Revoke the JWT described by `claims`. Pulls `jti`, `sub`, and `exp`
    /// from the verified claims — the middleware has already authenticated
    /// the token, so these are trustworthy. Returns 400 for legacy tokens
    /// without a jti so the client knows the revocation didn't take and can
    /// surface a "re-login required" path.
    pub async fn revoke_claims(&self, claims: &Claims) -> Result<(), AppError> {
        if claims.jti.is_empty() {
            return Err(AppError::BadRequest(
                "Token cannot be revoked (missing jti). Re-authenticate to get a revocable token."
                    .to_string(),
            ));
        }
        self.context
            .revoke(&claims.jti, &claims.sub, claims.exp as i64)
            .await
    }

    /// Retention sweep: drop revocation rows whose `expires_at` has already
    /// passed. Once a token is past `exp` the JWT verifier rejects it on
    /// its own (the `exp` claim is checked at decode time), so a stale
    /// revocation row is pure noise — it only ever costs an index seek
    /// on every authenticated request that happens to land on a
    /// since-expired jti.
    ///
    /// Wire to a cron job, an admin endpoint, or a one-shot binary
    /// invocation. The system-clock dependency is internal so callers
    /// don't have to compute the timestamp themselves; the trade-off is
    /// that the function isn't pure (testing the *now* boundary
    /// requires the context's lower-level `delete_expired` directly).
    pub async fn sweep_expired(&self) -> Result<u64, AppError> {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| AppError::InternalError(format!("System clock error: {}", e)))?
            .as_secs() as i64;
        let deleted = self.context.delete_expired(now_secs).await?;
        if deleted > 0 {
            tracing::info!(rows = deleted, "Swept expired JWT revocations");
        }
        Ok(deleted)
    }
}
