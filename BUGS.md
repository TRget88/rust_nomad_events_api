# Bugs

Tracking issues that have actually broken runtime behavior — both currently open and historically fixed. Improvements / hardening / refactors belong in [ROADMAP.md](ROADMAP.md), not here.

**Convention:**
- New confirmed bug → add to `## Open` with the symptom and where it lives.
- Bug gets fixed → **move** the entry to `## Fixed` and append a one-paragraph note covering symptom, root cause, fix, and the files touched. Don't just delete.
- Each `## Fixed` entry stays forever — it's the institutional memory so the same regression doesn't get re-introduced.

## Open

### Refresh-token rotation is non-atomic (revoke succeeds, insert can fail → user logged out)
[`src/logic/refresh_token_logic.rs:175-189`](src/logic/refresh_token_logic.rs) revokes the presented row, then inserts the rotated child in two separate SQL statements with no transaction. If the insert errors (DB write failure, FK violation, etc.) the user is left with their refresh token already revoked and no new token issued — silently logged out on the next access-token expiry with no recovery path. Wrap `revoke_by_id` + `insert` in a single `sqlx` transaction. Not a security regression (the old token *is* revoked which is the safer half) but a UX paper-cut waiting to happen.

### Rate-limiter HashMap grows unbounded
[`src/custom_middleware/rate_limit.rs`](src/custom_middleware/rate_limit.rs) never evicts entries for keys that stopped sending traffic. A distributed scraper spoofing IPs, or a frontend bug that rotates `X-API-Key` values, grows the map indefinitely. Per-entry timestamps are aged out of the sliding window, but the outer map's keys live forever. Add a periodic `.retain(|_, timestamps| !timestamps.is_empty())` sweep (or piggyback on the existing retention-sweep task in `main.rs`).

### Facebook app-ownership check silently disabled when env vars unset
[`src/logic/user_logic.rs::check_facebook_app_ownership`](src/logic/user_logic.rs) returns `Ok(())` when either `FACEBOOK_APP_ID` or `FACEBOOK_APP_SECRET` is missing/empty (only a `tracing::warn!`). In that state, a token minted for a *different* Facebook App will authenticate against this API. The check should fail-close: if Facebook auth is wired into the router but the env vars aren't set, refuse Facebook sign-ins with a clear 503 until configured.

### CORS fallback opens the API to any origin
[`src/main.rs::~204`](src/main.rs) — when `CORS_ORIGINS` is unset, the server logs a warning and falls back to `CorsLayer::new().allow_origin(Any)`. A production deploy that forgets the env var ships with CSRF-vulnerable defaults; the warning lands in a log nobody is tailing. Switch to fail-fast in `main.rs` when `RUST_ENV=production` (or panic outright if `CORS_ORIGINS` is empty in any env that isn't explicitly `development`).

### JWT algorithm pinning is implicit, not explicit
[`src/custom_middleware/auth_middleware.rs:~80`](src/custom_middleware/auth_middleware.rs) constructs `Validation::default()` and overrides `validate_aud = false`, but never calls `validation.set_algorithms(&[Algorithm::HS256])`. Today the library defaults reject `alg: none`, but a future jsonwebtoken bump that loosens defaults (or a copy-paste of this Validation builder to a new verifier) could open up an algorithm-confusion attack. Explicit pin: one line, defense in depth.

### `datetime('now')` vs `Utc::now()` precision mismatch on lockout boundary
[`src/context/user_context.rs::is_locked_out`](src/context/user_context.rs) uses `datetime('now')` (second precision); [`src/custom_middleware/auth_middleware.rs::~146`](src/custom_middleware/auth_middleware.rs) compares against `Utc::now()` (nanosecond precision). A user whose `lockout_until` lands on a second boundary can be treated as locked by one path and unlocked by the other within the same request. Practical impact is <1s of flakiness; pin both paths to a single bound parameter so the comparison source is unambiguous.

## Fixed

### Rate limiter middleware was a per-request no-op
`RateLimiter::new(100, 60)` was being constructed *inside* the per-request middleware function, so every request got a fresh empty bucket and nothing was actually rate-limited. Fixed by constructing the limiter once at startup, wrapping it in `Arc<RateLimiter>`, and threading it through `middleware::from_fn_with_state`. Files: [src/custom_middleware/rate_limit.rs](src/custom_middleware/rate_limit.rs), [src/main.rs](src/main.rs).

### `locked_out` check authenticated soft-deleted and non-existent users
The lockout query filtered `WHERE deleted_at IS NULL`, so a soft-deleted user (or a JWT carrying a `sub` that didn't exist at all) returned `None` → `.unwrap_or(false)` → request authenticated. The only thing actually gating requests was the JWT signature. Fixed by splitting into two queries: first reject if no row matches with `deleted_at IS NULL`, then compute the lockout state separately. File: [src/custom_middleware/auth_middleware.rs](src/custom_middleware/auth_middleware.rs).

### Expired temporary lockouts left `locked_out=1` in the DB forever
Same query as above also discarded rows where `lockout_until <= now()`, silently treating the user as unlocked — but the `locked_out=1` flag never cleared, so admin tooling reading the column saw permanently-stale state. Fixed by calling `unlock_user` from inside auth middleware when an expired temp lockout is detected. File: [src/custom_middleware/auth_middleware.rs](src/custom_middleware/auth_middleware.rs).

### JWT `iat` claim was set to `now + 24h` instead of `now`
`let issued = SystemTime::now()... + 3600 * 24;` — `iat` ended up equal to `exp`. `jsonwebtoken` doesn't validate `iat` by default so auth still worked, but any downstream consumer that did (audit log, future revocation logic) would reject every token. Fixed by dropping the offset. File: [src/logic/user_logic.rs](src/logic/user_logic.rs).

### `POST /profile` was a no-op echo
The route accepted an `UpdateProfileRequest` and returned it back as a `ProfileResponse` without ever calling the DB. Clients calling it thought they'd updated their profile and silently hadn't. Fixed by deleting the route entirely — `POST /self` in [routes/user.rs](src/routes/user.rs) handles real profile updates. The standalone `profile.rs` route module was removed at the same time, which also closed the orphan-data-types-outside-`models/` layering violation. Files: deleted `src/routes/profile.rs`, updated [src/main.rs](src/main.rs) and [src/routes/mod.rs](src/routes/mod.rs).

### `seed_all` ran on every startup
Despite an inline comment saying "uncomment to run once," the call was unconditional. Each restart re-ran the seed. The function itself was idempotent (returns early if `event_types` has rows), so no data corruption — but it added an avoidable DB query per boot and the comment lied. Fixed by gating behind `SEED=1` env var. File: [src/main.rs](src/main.rs).

### Frontend admin user-management UI hit four non-existent backend routes
[`festurah_frontend/src/scenes/user/index.tsx`](../festurah_frontend/src/scenes/user/index.tsx) called `PUT /admin/users/{id}/role`, `POST /admin/users/{id}/lock`, `POST /admin/users/{id}/unlock`, `DELETE /admin/users/{id}` — none existed on the backend. The frontend's `/admin/users/...` URLs also had leading slashes, producing `//admin/...` URLs that axum doesn't normalize. Fixed by wiring four new routes under `admin_routes` (the underlying `UserLogic::update_role` / `lockout_user` / `unlock_user` / `delete_user` methods already existed) plus stripping the leading slash on the frontend (and adding a defensive normalizer in `ApiService.request`). Files: [src/routes/user.rs](src/routes/user.rs), [src/main.rs](src/main.rs), [src/models/user.rs](src/models/user.rs), [src/logic/user_logic.rs](src/logic/user_logic.rs).
