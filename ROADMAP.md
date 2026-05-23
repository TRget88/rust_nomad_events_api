# Roadmap

Forward-looking improvements: hardening, refactors, new features. Bugs (broken runtime behavior) live in [BUGS.md](BUGS.md).

**Convention:** when an item is completed, remove it. Git history is the audit trail. Don't keep a "Done" section — it rots.

## Security & auth

- [ ] **Consider RS256/ES256 over HS256** for issued JWTs. Asymmetric keys let verifiers run without the signing secret — relevant if you ever split auth into a separate service.
- [ ] **Call Google's revoke endpoint on logout.** Only meaningful once we hold a Google access/refresh token (we don't today — we only get an ID token), but worth tracking if we ever upgrade to Authorization Code + PKCE for Google API access.
- [ ] **Internationalized (RFC 6531) email support.** `validate_email_format` in [src/logic/user_logic.rs](src/logic/user_logic.rs) accepts the punycode (ACE) form of IDN domains but rejects raw UTF-8 emails (`用户@例え.jp`). If non-ASCII addresses become a real need, add an opt-in path that runs `idna` crate's `Uts46` translation, or store the punycode form server-side and surface it client-side via a parallel display field.

## Data model

- [ ] **Add `events.creator_id`** with `FOREIGN KEY ... REFERENCES users(id)`. Drop the `created_events` / `created_microevents` JSON arrays in `user_event_data`. Switch ownership checks to a direct `WHERE creator_id = ?` clause. (Microevent ownership already uses the `microevents.user_id` column directly — see `MicroeventLogic::require_owner_or_admin`. Events still need the same treatment, which requires the schema change above.)

## Product features (LOW PRIORITY — paired with frontend)

These back the items in the [frontend Product features section](../festurah_frontend/ROADMAP.md). All low priority — none move revenue, and each requires a coordinated change in two repos.

- [ ] **`pending_events` table + admin approval flow.** Paired with curator update-mode + frontend submission flow. Only worth doing once curator pending-review actually writes there. New table mirroring `events` plus `proposed_by` (`user_id` or `"curator"`), `proposed_at`, `sources`, `status`. `POST /event/submit` (any authenticated user), `POST /admin/events/{id}/approve` and `/admin/events/{id}/reject` (SuperAdmin).
- [ ] **Real haversine for distance sort.** `/event/search?sort=distance` orders by squared Euclidean in lat/lon degrees ([src/context/event_context.rs](src/context/event_context.rs)). Inside a single radius the local distortion is negligible for *ordering*. Only matters once we surface the computed distance to clients (e.g., "12 mi away" badges).
- [ ] **Image storage for events.** Pick S3/R2/similar; `event_images` table; `POST/DELETE /event/{id}/images`. Defer until traffic justifies the ops cost.

## API hygiene

- [ ] **Switch `/event/search` from offset to cursor-based pagination.** Offset is simple but drifts under concurrent inserts — a row added between page 1 and page 2 either shifts later rows up (user sees a duplicate) or pushes them out of the window (user skips a row). Cursor pagination (e.g. `?cursor=<last_id>` or `?cursor=<opaque_token>`) is robust against this. Worth doing once the catalog is large enough that drift matters.

## Operations

These are infrastructure items the system needs to run reliably in production. The app is already deployed; these harden it.

- [ ] **Email infrastructure.** Pick a provider (Postmark / Resend / SES) and wire transactional emails: welcome on signup, "your saved event starts in 24 hours" reminder, weekly digest of new events in the user's saved regions. Pattern this after the curator service — separate process, own env, independent deploy.
- [ ] **Analytics implementation.** The schema has `daily_analytics`, `event_stats`, `creator_stats` tables (see [migrations/00001_initial.sql](migrations/00001_initial.sql)) but **nothing populates them.** Pick the simplest path: a nightly batch job that aggregates from the `events` / `users` / `user_*` tables. Same deploy pattern as the curator (Linux container, cron-driven).
- [ ] **Capture before-state in audit log entries.** Currently `metadata` carries only the input parameters of an admin action. Diffing "what changed" requires fetching the row before mutation. Add this when the audit-log readback UI demands it.
- [ ] **Transactional audit logging.** Audit writes currently log loudly on failure but don't roll back the underlying op. Wrap action + audit in a single `BEGIN; ... COMMIT;` so an op-with-missing-audit-entry can't happen.
- [ ] **Sentry (or equivalent) error reporting.** `tracing::error!` calls are only visible to whoever tails server logs. Pick a real error-tracking service for the `AppError::InternalError` / `DatabaseError` paths.
- [ ] **Uptime monitoring.** `/health` exists and pings the DB now; nothing pings it externally. Wire UptimeRobot / BetterStack / Pingdom.
- [ ] **Scheduled backups for `events.db`.** SQLite + single host = one drive failure away from total loss. Daily `sqlite3 .backup` → off-host storage (S3 / Backblaze / a different VPS). Pattern: another small Linux container, same shape as the curator, runs `sqlite3 .backup` + uploads.
- [ ] **Extend the retention sweep to soft-deleted users + curator artifacts + `pending_events`.** [`main.rs`](src/main.rs) now spawns a tokio task on `RETENTION_SWEEP_INTERVAL_SECS` (default 3600s, `0` disables) that calls `JwtRevocationLogic::sweep_expired` + `RefreshTokenLogic::sweep_expired` on every tick — both tables are covered, both log row counts at info, both swallow errors and retry next tick. Outstanding: add equivalents for soft-deleted users past their retention window, the curator `runs/*.json` archive (curator-side, age out files past N days), and `pending_events` once that table lands.

## Reliability / ops

- [ ] **Plan PostgreSQL migration.** SQLite + 5 connections is fine for now; concurrent write load will hit `SQLITE_BUSY`.

## Testing

A baseline test scaffold is in place (validator unit tests in [src/logic/event_logic.rs](src/logic/event_logic.rs) and integration tests against an in-memory SQLite pool in [src/custom_middleware/auth_middleware.rs](src/custom_middleware/auth_middleware.rs)). Run with `cargo test`. Outstanding additions:

- [ ] **Extend route-layer tests beyond `/event/search`, `/eventtype`, `POST /event`, `/auth/refresh`.** Canonical pattern (`setup_pool` + `make_app_state` + `oneshot`-driven Router) is used in [src/routes/events.rs](src/routes/events.rs) (`/event/search` + `POST /event` claims-extraction gate + happy-path create), [src/routes/event_type.rs](src/routes/event_type.rs) (list / get / 404 / create / audit-log write / delete), and [src/routes/auth.rs](src/routes/auth.rs) (`/auth/refresh` empty-body, unknown-token, happy-path rotation, reuse-detection family revoke). Remaining: `DELETE /event/{id}` requires SuperAdmin (via the auth middleware), JSON error shape for `AppError::ValidationError`, and one admin route per module (`/admin/users`, `/admin/audit-log`).
- [ ] **`#[sqlx::test]` macro** is a less-noisy alternative for per-test pools — evaluate after a few more tests are written.

## Code quality

