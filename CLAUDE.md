# rust_nomad_events_api

Rust REST API for nomad/festival event discovery. Built on **axum 0.8** + **sqlx (SQLite)** + **tokio**. JWT-protected with Google OAuth sign-in.

Serves the **festurah_frontend** SPA — see [Paired frontend](#paired-frontend) below.

## Paired frontend

The companion frontend lives at `D:\RandomProgrammingProjects\festurah_frontend` (not a sibling in this repo). Stack:

- **React 19** + **TypeScript** + **Vite 7**
- **Tailwind CSS 3** for styling
- **React Router 7** for navigation
- **leaflet** + **react-leaflet** for map views (events have lat/lon — see the `/event/search` radius query)
- **@react-oauth/google** for the Google sign-in flow that hits `POST /auth/google/signup` and `POST /auth/google/login`
- **facebook-oauth-react** is installed but the backend's Facebook flow is currently commented out in [src/routes/auth.rs](src/routes/auth.rs) and [src/main.rs](src/main.rs)

Backend implications when changing the API:

- **The frontend talks to this API over HTTPS via the URL in its `VITE_*` env vars** (`.env`, `.env.production` in the frontend repo). Add CORS origins to [src/main.rs](src/main.rs) accordingly before production — currently `Any` (see [ROADMAP.md](ROADMAP.md)).
- The frontend sends `X-API-Key` on public auth routes and `Authorization: Bearer <jwt>` on everything else. Don't rename these headers without coordinating a frontend change.
- The `AuthResponse` shape ([src/models/user.rs](src/models/user.rs) — `token` + `UserInfo` with `id`, `email`, `name`, `user_name`, `picture_url`, `role`, `provider`, `provider_id`, `created_at`, `updated_at`) is consumed directly by the frontend. Breaking field renames here need a coordinated frontend PR.
- Toggle endpoints (`/event/{id}/save`, `/favorite`, microevent equivalents) are `POST` (state-mutating per HTTP semantics). The current frontend never calls these directly — it syncs via `POST /usercollection/sync` instead — so the backend keeps them as a documented contract for future direct callers (admin tooling, external integrations).

The frontend has its own `CLAUDE.md` / `ROADMAP.md` / `BUGS.md` at its project root — read those when working there.

## Architecture

Layered design pattern. Each module type has one responsibility and lives in one directory:

- **routes/** — axum HTTP handlers. Extract `Claims` from request extensions, parse path/query/body, call into logic, return JSON.
- **logic/** — business logic. Validation, ownership checks, cross-resource orchestration.
- **context/** — data access. `sqlx` queries against the SQLite pool.
- **models/** — every data type the rest of the app touches.
- **errors/** — cross-cutting `AppError` and its `IntoResponse` / `From` impls.
- **custom_middleware/** — axum middleware (auth, api key, rate limit, etc.).

Layering rule: `routes → logic → context → DB`. Context never calls logic; routes never bypass logic.

### The "models in `models/`, logic in `logic/`" rule

**All data types live in [src/models/](src/models/).** That includes domain entities (`NomEvent`, `Microevent`, `User`-shaped types), DTOs (`*Request`, `*Response`), database row types (`*Row`), and enums representing data (`UserRole`). No data type definitions outside `models/`.

**All business logic lives in [src/logic/](src/logic/).** Validation, ownership checks, cross-resource orchestration, and anything that interprets data semantically. Tests for the validators are colocated as `#[cfg(test)]` mods.

Exempt from the rule (these are *services* and *framework infrastructure*, not data models — they have impl behavior, not just shape):

- Service structs that *own* their layer's behavior: `UserLogic`, `EventLogic`, etc. in `logic/`; `UserContext`, `EventContext`, etc. in `context/`.
- Framework types: `AppState` in `main.rs`, `AppError` in `errors/`, `RateLimiter` in `custom_middleware/`.

When adding a new domain object, the answer is always: type in `models/`, the code that operates on it in `logic/`, the queries that persist it in `context/`. Don't define `pub struct FooResponse {…}` inside a route handler — it gets its own module under `models/`.

The paired frontend mirrors this discipline: shared data shapes live in [festurah_frontend/src/shared/types.ts](../festurah_frontend/src/shared/types.ts), cross-cutting behavior in `src/services/` / `src/hooks/` / `src/context/`, presentation in `src/scenes/`. See the frontend's [shapes/behavior rule](../festurah_frontend/CLAUDE.md) for the exact analog and the list of contract types that must stay field-for-field aligned with `src/models/`.

`AppState` holds `Arc<Logic>` instances for every domain and is the single piece of axum state. The rate limiter is threaded separately via `from_fn_with_state` since it doesn't fit the per-domain pattern.

## Auth & authorization

- **API key** (`X-API-Key` header) gates only the public auth routes (`/auth/google/signup`, `/auth/google/login`). Validated against the SHA-256 of `API_KEY_HASH` env var.
- **Rate limiting** is keyed two ways depending on the route tier. Public routes (search, eventtype catalog, login/signup) go through `rate_limit_middleware` keyed on remote IP. `jwt_routes` and `admin_routes` go through `user_rate_limit_middleware` keyed on `claims.sub` (UUID-prefixed `user:<id>`), so a logged-in scraper rotating IPs hits a single bucket. Both buckets default to 100 requests/60s; tune via the `RateLimiter::new(...)` constructors in [src/main.rs](src/main.rs). The two limiters are separate instances so their buckets never collide.
- **Google JWKS** (`oauth2/v3/certs`) is fetched once and cached in-process for 1h via a `OnceLock<RwLock<...>>` in [src/logic/user_logic.rs](src/logic/user_logic.rs). The verifier force-refreshes once if the incoming token's `kid` isn't in the cache (handles Google mid-TTL key rotations). Skips a full HTTPS round-trip per signin.
- **JWT (Bearer)** gates `jwt_routes` and `admin_routes`. Verified in [src/custom_middleware/auth_middleware.rs](src/custom_middleware/auth_middleware.rs).
- JWT claims: `sub` (user UUID), `email`, `username`, `role` (`user` | `admin` | `super_admin`), `exp`, `iat`, `jti` (UUID stamped at issue time; key into the `jwt_revocations` table), and `aud` (always `"festurah-api"`; defends against shared-secret confusion if Festurah ever runs a second service with the same `JWT_SECRET`). Signed HS256 with `JWT_SECRET`.
- **`POST /auth/logout`** (behind the auth middleware) writes the caller's `jti` to `jwt_revocations`. Subsequent requests bearing the same token are rejected 401 by the middleware. Idempotent.
- Admin routes split by privilege tier (see `main.rs`):
  - **`admin_view_routes`** — gated by `require_admin` (Admin OR SuperAdmin). Listing users / events / microevents, viewing a single user, updating user profile (non-role fields), lock/unlock, and reading the audit log.
  - **`admin_super_routes`** — gated by `require_super_admin` (SuperAdmin only). User deletes, role changes, event_type / camping_profile catalog mutations, event deletes — everything irreversible or that affects global state.
  - The audit log records who did what, so the Admin tier comes with built-in oversight.
- **Role decisions re-read `role` from the DB on every authenticated request.** [auth_middleware](src/custom_middleware/auth_middleware.rs) extends the same `SELECT locked_out, lockout_until, role …` query and overwrites the JWT's `role` claim with the DB value before downstream handlers see it. Demotions, promotions, and role rotations therefore take effect on the next request, not at JWT TTL. A `tracing::warn!` fires when the two disagree so drift is visible in logs.
- **Ownership** of an event is tracked by the user's `user_event_data.created_events` JSON array, not by a column on `events`. Microevents *do* have `user_id` but ownership checks still consult the JSON blob. This asymmetry is on the roadmap to resolve.

## Database

SQLite, file `events.db` at the project root (auto-created on first run). Schema lives in [migrations/00001_initial.sql](migrations/00001_initial.sql); migrations run on startup via `sqlx::migrate!`. `seed_all` currently runs on **every** startup (see [BUGS.md](BUGS.md)).

Tables of note:

- `users` (UUID PK, OAuth fields, soft-delete via `deleted_at`, lockout fields, cached counts)
- `events` (no `creator_id` column — see ownership note above)
- `microevents` (has `user_id` FK to `users`)
- `user_event_data` (per-user JSON arrays for favorites / saves / created)
- `event_types`, `camping_profiles`
- `jwt_revocations` (per-token revocation list; key is the `jti` claim) — populated by `POST /auth/logout`, read by the auth middleware
- `admin_audit_log` (actor + action + target + JSON metadata + timestamp) — populated by the four user-admin handlers (`update_role`, `lock`, `unlock`, `delete`), readable via `GET /admin/audit-log?limit=N` (super-admin only)
- analytics tables (`daily_analytics`, `event_stats`, `creator_stats`) — currently unwired

Triggers on insert/delete of favorites/saves maintain the cached counts on `users`.

## Environment variables

| Var | Required | Purpose |
|---|---|---|
| `PORT` | yes | Listening port. App panics if unset. |
| `JWT_SECRET` | yes | HMAC key for JWT sign/verify. |
| `API_KEY_HASH` | yes | SHA-256 hex of the expected `X-API-Key` value. |
| `GOOGLE_CLIENT_ID` | yes | Audience for Google ID-token verification. |

Loaded via `dotenv` at startup, so a `.env` file in the project root works.

## Code conventions

- Rust `snake_case` for locals. Some pre-existing `camelCase` (`listeningPort`, `listenerAddress`) is queued for cleanup.
- Logging: use `tracing::{info,warn,error,debug}`. Don't add `println!` / `eprintln!`. `tracing-subscriber` is already initialized in `main`.
- Errors flow through `AppError` ([src/errors/app_error.rs](src/errors/app_error.rs)). Don't return `Box<dyn std::error::Error>` from handler-adjacent code — the blanket `From` impl collapses everything to `InternalError` (wrong HTTP status). Prefer concrete error types.
- SQL: parameterized queries only (`?1`, `?` placeholders). When using `LIKE`, escape `%` / `_` / `\` in user input.
- Don't silently swallow `from_row` errors with `filter_map(...).ok()` — at minimum log a `warn!` so corruption isn't invisible.

## Common tasks

- Run: `cargo run` (requires the env vars above; put them in `.env`).
- Test: `cargo test` — runs inline validator unit tests plus auth-middleware integration tests against an in-memory SQLite pool. CI runs the same command on every push + PR via [.github/workflows/ci.yml](.github/workflows/ci.yml) — gate today is `cargo build --locked` + `cargo test --all-features --locked`; fmt/clippy/audit are queued in ROADMAP.md.
- Seed: `SEED=1 cargo run` — `seed_all` only runs when the env var is set (and is internally idempotent — no-op if `event_types` already has rows).
- Build for release: `cargo build --release`.
- Docker: `docker build .` (Dockerfile has cache/security improvements queued — see [ROADMAP.md](ROADMAP.md)).
- DB inspection: a Windows `sqlite3.exe` is currently checked into the repo (also queued for removal).

## Keep these docs current

When you change code, update these four files in the **same** PR:

- **CLAUDE.md** (this file) — architecture, conventions, env vars, layering. Update the relevant section whenever it changes.
- **[ROADMAP.md](ROADMAP.md)** — new features, hardening work, refactors, deferred improvements. **Remove** items when you finish them (git is the record). Add new items when you start something or defer it.
- **[BUGS.md](BUGS.md)** — known broken runtime behavior. When you confirm a bug, add it under `## Open`. When you fix one, **move** the entry to `## Fixed` and append a paragraph covering symptom, root cause, fix, and files touched. Never just delete. `## Fixed` entries are institutional memory so the same regression doesn't get re-introduced.
- **[DESIGN.md](DESIGN.md)** — the **why** behind non-obvious decisions, indexed by function. When you make a tradeoff you wouldn't want a future contributor to silently undo (e.g., "validator rejects loud instead of clamping," "context query uses substr instead of date()"), add an entry. Inline code comments capture local context; this doc captures cross-cutting rationale that's easy to miss. Don't back-fill — record decisions when you make them. Superseded entries move to `## Superseded`, never delete.

**Bug fixes go in BUGS.md, not ROADMAP.md.** Roadmap is forward-looking improvements; bugs are *currently broken*.

If a code change contradicts something in these files (route added/removed, env var added, layer responsibility shifted), updating the docs is part of the change, not a follow-up.
