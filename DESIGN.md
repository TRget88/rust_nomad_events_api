# Design Notes

The **why** behind non-obvious decisions in this codebase. Inline code comments capture local context; this doc captures cross-cutting decisions and "don't change this because…" callouts that would be easy to miss when touching individual functions.

> **Convention:** when you make a non-obvious design choice, add an entry here. Index by `module::function` (or area when cross-cutting). Each entry: **Decision** → **Why** → **Don't change without**. Don't try to back-fill — record decisions when you make them.
>
> When the decision becomes false (e.g., the constraint that motivated it goes away), move the entry to `## Superseded` at the bottom rather than deleting. The note exists so a future contributor doesn't re-introduce the rejected alternative.

---

## Auth & authorization

### `src/custom_middleware/auth_middleware.rs::auth_middleware`

- **Re-reads `role` from DB on every authenticated request, not from the JWT.**
  - Why: a 24h JWT TTL means a demotion (super_admin → user) takes up to 24h to actually downgrade permissions if we trust the in-JWT claim. The query already fetches lockout state — adding one extra column is essentially free.
  - Don't change without: re-evaluating the staleness/perf tradeoff. The DB hit is on every auth'd request; caching it is the obvious optimization but introduces its own staleness window.

- **Empty `jti` is accepted with a warn log, not rejected.**
  - Why: backwards-compat with tokens issued before the jti rollout. The `#[serde(default)]` on `Claims::jti` means legacy tokens decode as empty string. They authenticate normally but can't be revoked (because there's no key to put on the revocation list).
  - Don't change without: confirming the legacy window has closed. Every JWT TTL after the deploy, the legacy population shrinks; after one full TTL (24h) no legacy tokens remain. Drop the carve-out then — see [ROADMAP.md](ROADMAP.md).

- **`AuthMiddlewareState` bundles `pool` + `Arc<JwtRevocationLogic>`, not just the pool.**
  - Why: lets the middleware route revocation checks through the logic layer instead of inlining the SQL. One canonical query, one place to change if the table shape evolves.
  - Don't change without: understanding the alternative had a documented SQL duplicate that quietly diverged from `JwtRevocationContext::is_revoked`. The state bundle exists to prevent that re-diverging.

### `src/models/user.rs::UserRole`

- **`Claims.role: UserRole`, not `String`.**
  - Why: pre-refactor, `claims.role == "admin" || claims.role == "super_admin"` was scattered across `event_logic.rs`, `microevent_logic.rs`, and auth_middleware (via `claims.get_role()`). A typo in one site (e.g., `"admmin"`) would silently fail to match and downgrade access without any compile-time warning. After: `matches!(claims.role, UserRole::Admin | UserRole::SuperAdmin)` is exhaustive — adding a new role variant becomes a compile error in every consumer, and typos can't happen.
  - Don't change without: keeping the wire-format strings stable. Serde's `#[serde(rename_all = "snake_case")]` and the `Display` impl both emit `"user"` / `"admin"` / `"super_admin"` — same as the old String values. Existing tokens, DB rows, and external integrations continue to round-trip cleanly.

- **`Display` impl emits the same strings as serde and sqlx.**
  - Why: three serializers (Display via `format!("{}")`, serde via `to_string`, sqlx via the `rename_all` derive) all need to agree, or telemetry/logs drift from on-the-wire form. The `Display` impl is the single source of truth; the test `parse_cache_control_max_age_extracts_from_real_google_header_shape` doesn't apply here, but the equivalent pin for UserRole is the JWT-decode-and-stringify roundtrip exercised by `build_role_probe_app`.

- **`UserRole::from_db_string(s)` defensively falls back to `User` on unknown input.**
  - Why: the `users.role` column has `NOT NULL DEFAULT 'user'` so the unknown case shouldn't happen in production. But if it ever does (corrupted row, manual SQL edit gone wrong), failing closed (least privilege) is safer than returning a `Result<UserRole, _>` and forcing every caller to handle the error path.
  - Don't change without: weighing the alternative. A strict-parse version would surface bad data loudly via 500 errors instead of silently downgrading — defensible, but the JWT-creation path is hot and we'd rather have a working signin than a 500 on every signin while ops investigate the bad row.

### `src/custom_middleware/auth_middleware.rs::require_admin` vs `require_super_admin`

- **Two-tier split: view/inspect/lock on Admin tier; destructive/role-change/catalog-mutation on SuperAdmin tier.**
  - Why: the role enum is 3-tier (User | Admin | SuperAdmin). Pre-split every admin route required SuperAdmin, making Admin functionally equivalent to User. Splitting gives Admin real meaning and bounds the blast radius of an Admin compromise.
  - Don't change without: ensuring the audit log still covers any new Admin-tier mutating action. Admin actions that aren't audited become accountability gaps.
  - Specifically: Admin can list/view/update users (non-role fields), lock/unlock, view audit log. Admin cannot delete users, change roles, or mutate the catalog (event types, camping profiles). Pinned by `require_admin_*` tests.

### `src/logic/user_logic.rs::create_jwt_for_user`

- **`jti` is `Uuid::new_v4()` per token, not derived from anything.**
  - Why: each issued token must be individually revocable. UUID v4 is collision-safe at our issuance rate and doesn't leak any info about the user/session.
  - Don't change without: replacing the revocation table key column type to match.

- **`aud` is stamped from `auth_middleware::EXPECTED_AUDIENCE`, not a string literal at the call site.**
  - Why: the audience value is consumed in two places (here when minting, in `auth_middleware` when validating). Pinning as a module-level constant ensures they can't drift — a typo in one place becomes a typo in both.
  - Don't change without: also updating the validation site. Better: don't change it.

### Audience claim (`aud`)

- **`Validation::default()` has `validate_aud = true` — we explicitly disable it.**
  - Why: jsonwebtoken's built-in audience check fails any token with an `aud` field when no audience is registered on the validator. That would block every aud-stamped token from reaching our three-case check (correct / legacy-empty / wrong). We set `validation.validate_aud = false` and do the check ourselves in middleware.
  - Don't change without: understanding that re-enabling jsonwebtoken's check would lock out legacy tokens immediately (before the rollout window closes) AND break our ability to log+accept legacy aud with a warn.

- **Three-case audience check, not strict match.**
  - Why: the three states are correct (`festurah-api`), legacy (empty — token issued before the rollout), and wrong (anything else, like a token minted for a sibling service that shares JWT_SECRET). The legacy branch logs a warn but accepts; the wrong branch rejects 401. The window self-closes one JWT TTL post-deploy.
  - Don't change without: removing the legacy branch first (after one TTL — tracked in ROADMAP.md as "Drop the legacy-aud carve-out").

### `src/logic/user_logic.rs::validate_email_format`

- **Strict-but-narrower than RFC 5322.**
  - Why: full RFC 5322 (quoted local parts, comments, escape syntax) is famously hard to get right and the rare-use addresses it permits are vanishingly rare in real users — most "weird-but-valid" addresses are actually bad data. Rejecting them catches more real bugs than it loses real addresses.
  - Don't change without: a concrete report of a real user blocked by the strictness.

- **Numeric TLDs rejected.**
  - Why: `user@example.c0m` (zero instead of `o`) is the most common typo this catches. Real TLDs are alphabetic.
  - Don't change without: someone pointing at a legitimate numeric TLD (none exist as of writing).

- **Hand-rolled, no `regex` crate.**
  - Why: adding a heavyweight dep for one validator wasn't worth it. The validator is small and tested; the cost is rebuilding it if we ever switch to a regex-based one.

- **ASCII-only, IDN domains accepted in punycode form only.**
  - Why: validating UTF-8 (RFC 6531) needs the `idna` crate and Uts46 normalization. Most international users encode to ACE/punycode at the client. Documented in [ROADMAP.md](ROADMAP.md) as a future extension.

---

## Search

### `src/context/event_context.rs::find_nearby`

- **Always appends `LIMIT ? OFFSET ?` — no unbounded mode.**
  - Why: server response size must be bounded regardless of caller input. Default `limit=200` (set by the logic layer) preserves current frontend behavior; the cap matters at scale.
  - Don't change without: ensuring the logic layer always passes a clamped pair. The context trusts that input.

- **Date filter uses interval-overlap, not "start_date BETWEEN".**
  - Why: a festival running Fri–Sun should match a search for the Saturday in the middle. Strict-equality on start_date would miss it. The SQL: `e.start_date <= date_to AND COALESCE(e.end_date, e.start_date) >= date_from`.
  - Don't change without: re-evaluating what "events on date X" means for the user. The current semantics matches "what's happening on day X."

- **Date comparison uses `substr(date, 1, 10)`.**
  - Why: stored format is inconsistent (`YYYY-MM-DD` from the curator path; `YYYY-MM-DD HH:MM:SS UTC` from the chrono datetime path). substr-10 normalizes both to date-only for comparison. Skips the start_date index but the bounding-box filter cuts the candidate set first.
  - Don't change without: canonicalizing date storage upstream (would let us use the index).

- **Distance sort uses squared Euclidean in lat/lon degrees, not real haversine.**
  - Why: within a single search radius (≤ 500 mi), local distortion is small enough that the *ordering* is correct. Reporting actual distances ("12 mi away") would need real haversine via SQLite SIN/COS extension. Currently we only sort, never report.
  - Don't change without: if we ever surface distances to clients, switch.

- **lat/lon are BIND parameters in ORDER BY, not interpolated via `format!()`.**
  - Why: the f64 type already rules out injection, but binding is the project convention. Interpolating one set of "safe" values sets a precedent that someone later might apply to less-safe values.
  - Don't change without: replacing the convention everywhere or accepting the precedent risk.

- **LIKE clause uses `ESCAPE '\\'` + escaped pattern.**
  - Why: user input `%off%` would otherwise wildcard-match every row. Escaping `%`/`_`/`\` makes the substring match literal.

### `src/util.rs::validate_pagination`

- **Lives in `util.rs`, not `logic/event_logic.rs`. Shared across endpoints.**
  - Why: pre-refactor the helper was in `event_logic.rs` because `/event/search` was the only paginated endpoint. As `/user` (admin) and `/admin/audit-log` joined, putting the helper in event_logic would have meant cross-importing from sibling logic modules — a layering smell. Hoisting to `util.rs` (the pure-helpers home) gives every endpoint a single canonical source for defaults, caps, and error-message text.
  - Don't change without: a real reason to want per-endpoint caps. If `/admin/audit-log` ever needed `max=1000` while `/event/search` stayed at 500, override via a constant in the calling logic module rather than diverge here.

- **Rejects out-of-bounds inputs loudly, doesn't silently clamp.**
  - Why: `?limit=-1` is a caller bug. Silent clamping (to 1 or 500) hides bugs and makes the contract murky. 400 lets the caller find their bug.
  - Don't change without: a real complaint about strictness from a real caller.

- **`limit` default is 200; max is 500.**
  - Why: 200 is wide enough that the frontend's pre-pagination behavior is preserved at realistic event densities. 500 matches `EventContext::get_by_id_list`'s existing cap so the two patterns agree. The roadmap originally suggested 50 default — too tight for a frontend without a "Load more" UI; documented in case a future tightening pass revisits.
  - Don't change without: confirming the frontend gracefully handles the new cap.

- **`offset` has no upper bound.**
  - Why: deep paging is legitimate. The query just returns empty once offset > total. Cursor-based pagination is on the roadmap for when drift-under-concurrent-inserts becomes a real problem.

- **`PaginationQuery` is in `models/dto.rs`, not `util.rs`.**
  - Why: `PaginationQuery` is a *data shape* (carries serde derive for query-param deserialization) — by the project's "data shapes in `models/`" rule it lives in `models/`. `util.rs` stays pure helpers. Endpoints with extra filter fields (`/event/search`) carry their own query struct with the same `limit`/`offset` shape; the duplication is intentional so each endpoint's full query surface is documented in one place.

### `src/logic/event_logic.rs::validate_event_type_ids`

- **Single + multi inputs merged; dedup preserves first-appearance order.**
  - Why: the SQL IN-clause is order-sensitive (well, not for matching, but for downstream sort consistency). First-appearance-order is the deterministic choice; pinned by a test.

### `src/logic/event_logic.rs::validate_name_contains`

- **Trims but does NOT escape internally.**
  - Why: LIKE-escaping is the context layer's responsibility. Callers without LIKE semantics get the raw string. Pinned by a test (`validate_name_contains_does_not_escape_internally`).

### `src/logic/event_logic.rs::validate_sort_param`

- **Lowercase only — case-sensitive.**
  - Why: matches REST convention. If we ever want case-insensitive, the test (`validate_sort_param_is_case_sensitive`) is there to make us consider it explicitly rather than silently flipping.

---

## Pagination

### Always-on pagination contract

- **Every `/event/search` response is bounded.**
  - Why: no "unbounded" mode means a malformed caller can't accidentally request the entire catalog. The logic layer always produces concrete `(limit, offset)`; the context layer trusts and binds.
  - Don't change without: a clear "internal-only, trusted caller, return everything" requirement. None exists today.

- **Offset-based, not cursor-based.**
  - Why: simpler v1. Cursor-based pagination is robust against concurrent inserts but more complex. Acceptable tradeoff until catalog drift causes real user-visible issues.

---

## Rate limiting

### Two `RateLimiter` instances, not one shared

- **`rate_limiter` (per-IP) and `user_rate_limiter` (per-user) are separate `Arc<RateLimiter>` instances.**
  - Why: separate `HashMap` buckets mean a key-namespace collision is structurally impossible, even if some future code path adds keys that look similar. Cost is one extra HashMap; benefit is "can't accidentally cross the streams."

- **Per-user limiter runs AFTER `auth_middleware`, not before.**
  - Why: needs `Claims` in request extensions to key on `claims.sub`. Layer order: auth_middleware (outermost, runs first) → user_rate_limit → handler. axum's last-`.layer()`-call-is-outermost rule means `user_rate_limit_middleware` is added first (innermost) and `auth_middleware` is added last (outermost).
  - Don't change without: understanding axum's layer ordering. Many subtle bugs hide here.

- **User keys prefixed `user:<uuid>`.**
  - Why: if some future code shares a `RateLimiter` between the two key spaces, the namespace prefix prevents IP-keyed entries from colliding with user-keyed ones. Defense-in-depth even though the two-instance design already prevents this.

---

## JWT revocation

### `migrations/00002_jwt_revocations.sql`

- **No FK on `user_id` to `users.id`.**
  - Why: the revocation list should outlive the user record. A hard-delete of a user (rare, but possible via migrations) shouldn't drop their revocation entries — the JWT they were holding is still technically valid until `exp`, so we keep it on the list until then.

- **`expires_at` is INTEGER (Unix seconds), not TEXT.**
  - Why: matches the `exp` claim in the JWT exactly. Background sweep of expired rows (roadmap) is a single `WHERE expires_at < strftime('%s', 'now')` query.

### `src/context/jwt_revocation_context.rs::revoke`

- **`INSERT OR IGNORE`, not `INSERT`.**
  - Why: idempotent — a repeat logout for the same token shouldn't error. The original `revoked_at` is preserved, which is the more useful "when was this token first invalidated?" timestamp.

---

## Audit log

### `migrations/00003_admin_audit_log.sql`

- **No FK on `actor_user_id` or `target_id`.**
  - Why: same as `jwt_revocations` — the audit trail must survive hard-deletes of users it references. Soft-delete (default via `deleted_at`) leaves the row in place anyway; the missing FK matters only for the rare hard-delete case.

- **`metadata` is TEXT (JSON) not JSONB.**
  - Why: SQLite doesn't have a JSONB type. JSON-as-TEXT works fine for our query patterns (we never query inside the JSON; it's read-back as-is). The context layer parses on read.

### `src/logic/audit_log_logic.rs::record_best_effort`

- **Best-effort: logs loudly on failure but doesn't roll back the underlying op.**
  - Why: when forced to choose between audit *completeness* (every op has a log entry) and audit *integrity* (every log entry corresponds to a real op), we choose integrity. A failed audit write produces an `error!` log; the operator can see the gap. Rolling back a successful admin op because we couldn't write a log row would be worse — admins would see "delete failed" when actually the delete succeeded.
  - Don't change without: implementing the queued "transactional audit logging" item, which wraps op + audit in a single transaction.

- **Method on `AuditLogLogic`, not a free helper in `routes/user.rs`.**
  - Why: originally a private helper in `routes/user.rs`. When audit coverage expanded to event/event_type/camping_profile handlers across multiple route modules, the helper got hoisted onto the logic struct so all callers can share one implementation. Per the layered design rule, cross-cutting helpers like this belong with the logic layer that owns the operation.
  - Don't change without: ensuring all route modules still have one canonical "log on failure, don't propagate" call site.

### Audit log coverage map (which routes audit, which don't)

- **Audited (writes to `admin_audit_log` after success):**
  - User admin: `update_role`, `lock`, `unlock`, `delete`
  - Event admin: `DELETE /event/{id}`
  - Event type catalog: `POST /eventtype`, `PUT /eventtype/{id}`, `DELETE /eventtype/{id}`
  - Camping profile catalog: `POST /campingprofile`, `PUT /campingprofile/{id}`, `DELETE /campingprofile/{id}`

- **Deliberately NOT audited:**
  - User-owned event/microevent CRUD (`POST /event`, `PUT /event/{id}`, etc.) — these are user-owned operations, already tracked via `user_event_data.created_events`. Auditing them would create noise.
  - Read-only routes (GET *) — listed in the read tier (`require_admin`) but don't mutate state. No audit value.
  - `/admin/audit-log` readback itself — auditing audit reads is recursive noise; if needed later, gate it behind a flag.

- **Why this distinction matters:** the audit log is for *consequential admin actions*. Filling it with routine user-owned CRUD would dilute its signal and break the "scan recent audit entries to see what admins did" use case.

---

## Database

### Migrations are append-only

- **Never edit a shipped migration file, even comments.**
  - Why: sqlx checksums every migration on boot. Editing a shipped file changes its checksum; sqlx refuses to run in any environment that already applied the old version.
  - Don't change without: deleting `events.db` (acceptable in dev only). In production it's an outage.

### `events.start_date` / `end_date` are TEXT

- **Stored as ISO 8601 strings, not as a typed date.**
  - Why: SQLite doesn't have a date type. Comparison works lexicographically as long as the format is consistent. Currently the format is inconsistent — the curator inserts `YYYY-MM-DD`, chrono's `to_string()` inserts `YYYY-MM-DD HH:MM:SS UTC`. The `find_nearby` filter normalizes via `substr(date, 1, 10)` to dodge the inconsistency.
  - Don't change without: settling on one canonical format upstream and migrating existing rows.

---

## JWKS cache

### `src/logic/user_logic.rs::get_google_jwks`

- **TTL is per-entry, parsed from the response's `Cache-Control: max-age`.**
  - Why: Google publishes `Cache-Control: max-age=21600` (~6h). Honoring it means we cache for as long as Google says it's safe to, instead of an artificial 1h floor. Each cache entry stores its own TTL (the one current at fetch time) because Google can change the advertised value between fetches.
  - The TTL is **capped at `JWKS_MAX_TTL` (1 day)** so a misbehaving upstream can't pin us to a stale JWKS for weeks via `max-age=2592000`.
  - Missing/unparseable `Cache-Control` headers fall back to `JWKS_FALLBACK_TTL` (1h) — the previous conservative default.
  - Don't change without: ensuring the per-entry TTL stays *per-entry*. A pinned test (`is_cache_fresh_uses_per_entry_ttl_not_a_global_constant`) catches accidental refactors back to a module constant — long-`max-age` entries would expire prematurely under that mistake.

- **`Cache-Control` header read BEFORE `.json()` consumes the response.**
  - Why: `reqwest::Response::json()` consumes the response, dropping the headers. Capture the TTL first; then deserialize the body.
  - Don't change without: re-architecting around `bytes()` + manual JSON parsing if you genuinely need the headers AFTER reading the body.

- **`parse_cache_control_max_age` is case-insensitive on the directive name.**
  - Why: RFC 9111 §5.2 specifies directive names as case-insensitive. Google sends lowercase; pinned against real-world variance just in case.

- **Force-refresh on unknown `kid` before failing.**
  - Why: Google can rotate keys mid-TTL. If we serve a cached JWKS that doesn't contain the incoming token's `kid`, we don't know if the kid is real (we have stale data) or fake (the token is forged). Force-refresh resolves the ambiguity: refreshed JWKS has the kid → real token, JWKS still doesn't → forged. One retry, not a loop.
  - Don't change without: weighing the rare-case extra Google fetch against the auth-fail false-positives we'd see otherwise.

- **Concurrent cold-cache requests both fetch.**
  - Why: holding the lock across the await would block all signins on a single fetch. Two fetches on a cold cache is a small Google traffic cost; the alternative is a serialization point that's worse on a slow Google response.

---

## Row-to-response conversion

### `src/logic/event_logic.rs::event_response_or_log`

- **Logs and drops malformed rows instead of erroring or silently filtering.**
  - Why: every list endpoint that produces `Vec<EventResponse>` was previously calling `.filter_map(|row| EventResponse::from_row(row).ok())` — the `.ok()` silently dropped any row whose `event_data` JSON failed to parse, leaving operators no way to detect or fix the corrupt row. Now the helper captures `row.id` before the move, calls `from_row`, and on failure emits a `tracing::warn!` with the event_id + error before returning None.
  - Don't change without: preserving the "silent drop is invisible corruption" principle. If you decide to instead propagate the error and fail the entire list query, you also need to handle the case where ONE bad row blocks every other event from being returned.

- **Lives in `event_logic.rs`, imported by `user_collection_logic.rs` — not duplicated.**
  - Why: 7 call sites across two logic modules use this. A `pub(crate)` free function in event_logic with a sibling-module import from user_collection_logic is the smallest non-duplicated form. Putting the helper as a method on `EventResponse` in `models/dto.rs` would pull `tracing` into the models layer, which violates the "models are pure data shapes" rule.
  - Don't change without: weighing the tradeoff. A new `logic/conversions.rs` module would be architecturally cleaner but adds a file for one helper. The current location is "good enough" until a second similar helper joins it.

---

## Request body extraction

### `src/extractors.rs::ApiJson`

- **Custom `ApiJson<T>` wraps `axum::Json<T>` so JSON rejections produce structured `AppError::BadRequest`.**
  - Why: axum's default `Json<T>` rejection response uses a separate envelope shape (`{"message": "..."}`) from every other handler error (`{"error": "..."}`). The mismatch confuses API consumers writing error handlers. Wrapping the extractor lets us route JSON-parse failures through the same `AppError → IntoResponse` pipeline as every other error.
  - Pinned by `response_uses_structured_error_envelope` test (asserts the `error` key, not the `message` key).

- **Friendly empty-body message: "Did you forget to send a body?"**
  - Why: by far the most common cause of `JsonSyntaxError` in practice is `curl -X POST` (or similar) without a body. axum's underlying message — "Failed to parse the request body as JSON: EOF while parsing a value at line 1 column 0" — is technically accurate but unhelpful. Pre-pending a concrete hint about the likely cause helps API consumers debug faster.
  - The underlying axum error is still appended in parens for cases where the body is non-empty but malformed.

- **All 15 production `Json<T>` call sites converted in one pass; no partial coverage.**
  - Why: half-converted would be worse than fully-converted or fully-original — users hitting some endpoints would get one envelope, others a different envelope. Confusing. Pinned by `Grep` confirming no remaining `Json<T>` body extractors in routes/.
  - Don't change without: ensuring the conversion stays consistent. New handlers should use `ApiJson<T>`; convention is documented in the extractor's doc comment.

---

## Logging

### `tracing` levels — when to use which

- **`tracing::info!`**: process lifecycle events (server start, shutdown, DB creation) and routine successful state changes that ops want to see in production logs (e.g., "JWT role differs from DB role" warnings already use `warn!`).
- **`tracing::warn!`**: a recoverable anomaly (stale-role drift, legacy-jti token, audit-log write failure that didn't abort the op).
- **`tracing::error!`**: an op-visible failure that needs investigation (DB query failed, JWKS fetch couldn't recover).
- **`tracing::debug!`**: dev-debug breadcrumbs (function-reached markers, row counts). Filtered out by default; flippable on via `RUST_LOG=debug`.

- **No `println!` / `eprintln!` in committed code.**
  - Why: tracing supports structured logging, levels, filtering via `RUST_LOG`, and JSON output for log aggregators. `println!` bypasses all of that — it can't be filtered, structured, or routed. The sweep replaced 8 ad-hoc prints; the rule is enforceable via `clippy` (queued as part of the strict-CI follow-up).
  - Don't change without: clippy lint allowance + a strong reason. If you want a print for transient debugging, use `dbg!` or `tracing::debug!`; if you need to land it, it's `info!`/`warn!`/`error!`.

- **Ad-hoc "reached this function" breadcrumbs demoted to `debug!`, not deleted.**
  - Why: deleting them removes the ability to flip them back on via `RUST_LOG=debug`. `debug!` keeps the breadcrumb available for anyone debugging without polluting production logs.
  - When the breadcrumb's purpose is gone (the bug it helped find is solved), delete it as part of the "delete commented-out blocks" cleanup pass.

---

## Code conventions

### Layered design

- **Routes → Logic → Context → DB. No skipping. Contexts never call logic.**
  - Why: makes the data flow direction unambiguous. Routes orchestrate; logic validates and applies rules; context just queries. Tested by "where does the new code go?" question being a 5-second answer.

- **All data types in `models/`. All business logic in `logic/`.**
  - Why: when you go looking for the canonical definition of a domain shape, there's exactly one place it lives. Same with logic — no orphan `pub struct FooResponse` in a route handler.
  - See: [CLAUDE.md](CLAUDE.md) — "The 'models in `models/`, logic in `logic/`' rule".

### Cross-cutting helpers

- **`src/util.rs` for stateless, no-I/O, no-domain-awareness helpers.**
  - Why: `escape_like_pattern` was originally private to `user_context.rs` and got duplicated when needed in `event_context.rs`. Extracting to `util.rs` gave one canonical location. Future helpers (`format_dtstamp`-equivalents for SQL etc.) belong here too.

---

## Superseded

_None yet._
