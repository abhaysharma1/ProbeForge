# Phase 3 report — Authentication, projects, and authorization

- **Date:** 2026-10-23
- **Status:** `PHASE 3: PASS`
- **Scope:** local authentication, opaque server-side sessions, CSRF, login rate
  limiting, project membership roles, backend authorization, audit events, the
  first API database pool, database-aware readiness, and the authenticated web
  workspace.

## Implemented behavior

### Authentication and sessions

- Local email/password authentication with Argon2id PHC hashes via the
  maintained `argon2` 0.5.3 crate. Passwords are bounded to 12-256 bytes and
  never stored or logged in plaintext.
- Operator-only account provisioning through
  `cargo run -p probe-forge-api --bin create-user -- <email> "<display name>"`.
  The password is prompted twice and is never accepted as a command argument.
  There is no public registration endpoint.
- 256-bit opaque session tokens. Only a SHA-256 digest is stored in
  `auth_sessions.token_hash`; the plaintext token exists only in the HttpOnly
  cookie and the login response.
- Session cookies are `HttpOnly`, `SameSite=Strict`, `Path=/`, and `Secure`
  when `PROBE_FORGE_COOKIE_SECURE=true` (the default). Absolute expiry defaults
  to eight hours.
- `POST /api/auth/logout` revokes the current session; `POST
  /api/auth/logout-all` revokes every session; `GET /api/auth/sessions` lists
  the caller's sessions; `DELETE /api/auth/sessions/{id}` revokes one owned
  session. Every request re-checks `revoked_at IS NULL`, expiry, and the user's
  active status.
- Double-submit CSRF: the `pf_csrf` cookie must match `X-CSRF-Token` on unsafe
  requests. `Sec-Fetch-Site: cross-site` and non-allowlisted `Origin` values are
  rejected. Safe methods remain readable so the client can bootstrap.
- Database-backed login limits: five attempts per normalized account plus direct
  peer address and fifty attempts per peer in a fifteen-minute window. Keys are
  SHA-256 digests; raw emails and IP addresses are not stored in the limit table.
  Successful login clears the keys. `Retry-After` is returned on `429`.
- The login path performs a dummy Argon2 verification for unknown accounts and
  malformed passwords so timing does not reveal account existence.
- Application errors use a stable `{ error: { code, message } }` envelope without
  stack traces, SQL details, hashes, tokens, or internal paths.

### Projects and authorization

- Project create/read/update, membership listing, member addition, role changes,
  member removal, ownership transfer, and project audit viewing.
- Roles: owner, admin, collaborator, viewer. Owners and admins can manage the
  project; only owners can add or modify admins and transfer ownership; nobody
  can remove or change the owner through the member endpoints.
- Every project handler requires authentication and re-checks membership/role in
  the database inside the transaction. Non-members receive `404` to avoid
  disclosing project existence.
- Cross-project reads and writes are denied by an explicit membership lookup, not
  by trusting a path or UI state.
- Sensitive changes write immutable `audit_log` events in the same transaction:
  project creation/update, member add/role change/removal, ownership transfer,
  login, logout, logout-all, and session revocation. Metadata contains IDs and
  roles only.
- Hard project deletion is deliberately absent. Phase 2's immutable audit,
  approval, and evidence triggers block a cascading delete, so deletion is
  deferred to an explicit archive/retention design (D-022). The triggers were not
  weakened.

### API and database runtime

- `DATABASE_URL` is now required for API startup. The API owns a SQLx PostgreSQL
  pool (10 connections, five-second acquire timeout) and fails fast if the
  database is unavailable.
- `/readyz` checks that at least one successful SQLx migration is recorded; it
  returns `503` when the database is unreachable or unmigrated.
- `migrations/0007_auth_sessions.sql` adds `users.password_hash`,
  `auth_sessions`, and `auth_login_attempts`, with format, length, expiry, and
  revocation constraints plus lookup indexes.
- All project and auth routes are nested under `/api`; the protected router wraps
  `/auth/session`, logout, session management, and all project routes in
  `require_auth`. No job or approval endpoint exists yet, so no unauthenticated
  path can create or approve one.

### Web workspace

- Signed-out login view and signed-in projects workspace using the palette,
  typography, spacing, borders, focus states, and reduced-motion behavior from
  `docs/06_frontend_design.md`.
- The client stores the CSRF token in memory only. It never uses `localStorage` or
  `sessionStorage` for tokens, and the session cookie is not readable by
  JavaScript.
- Project list, create form, member list, role-gated add-member form, audit trail,
  loading/empty/error/retry states, and sign-out.
- Viewer and collaborator controls are hidden, but the server remains
  authoritative.
- Vite proxies `/api`, `/healthz`, and `/readyz` to `127.0.0.1:8080`, so the
  browser talks to the API on the same origin during development. The browser has
  no PostgreSQL credentials or direct database access.

## Files changed

### New

| File | Purpose |
|---|---|
| `migrations/0007_auth_sessions.sql` | Password hash, session, and login-attempt schema |
| `services/api/src/auth/mod.rs` | Login, session, CSRF, logout, revocation handlers |
| `services/api/src/auth/password.rs` | Argon2id hashing, verification, email normalization, password bounds |
| `services/api/src/auth/session.rs` | Token generation/hashing, session lookup helpers, rate-limit keys |
| `services/api/src/cookies.rs` | Hardened session and CSRF cookie construction |
| `services/api/src/audit.rs` | Transactional audit insertion |
| `services/api/src/error.rs` | Stable error envelope, bounded JSON/path/query extractors |
| `services/api/src/projects.rs` | Project/member RBAC, ownership transfer, audit listing |
| `services/api/src/bin/create-user.rs` | Operator-only account provisioning |
| `services/api/tests/auth_projects.rs` | DB-backed auth, CSRF, rate-limit, RBAC, cross-project, audit tests |
| `services/api/tests/support/mod.rs` | Schema-isolated test app harness and HTTP helpers |
| `apps/web/src/api.ts` | Typed same-origin API client and role helpers |
| `apps/web/src/LoginView.tsx` | Signed-out authentication UI |
| `apps/web/src/ProjectsView.tsx` | Authenticated projects, members, and audit UI |
| `docs/AUTH.md` | Authentication/RBAC implementation contract |
| `docs/phases/03_auth_projects_report.md` | This report |

### Modified

| File | Why |
|---|---|
| `Cargo.toml`, `services/api/Cargo.toml`, `Cargo.lock` | Auth, cookie, hashing, session, and test dependencies |
| `services/api/src/lib.rs` | Config, pool, readiness, protected routing, middleware, startup |
| `services/api/tests/migrations.rs` | Auth table/index/password-column assertions |
| `apps/web/src/App.tsx` | Session bootstrap, login, sign-out, retry, view switching |
| `apps/web/src/App.test.tsx` | Login, rate-limit, project detail, viewer gating, sign-out tests |
| `apps/web/src/index.css` | Design tokens, light theme, focus, reduced motion |
| `apps/web/vite.config.ts` | Same-origin `/api`, health, and readiness proxy |
| `biome.json` | Tailwind CSS directive parsing |
| `.env.example` | Database, cookie, origin, session, and account-provisioning guidance |
| `.gitignore` | Ignore the local CodeGraph index |
| `package.json` | `user:create` script |
| `scripts/db-check.mjs` | Run migration and auth/RBAC DB suites |
| `scripts/check-docs.mjs` | Require the Phase 3 docs, report, migration, and tests |
| `README.md`, `docs/README.md`, `docs/DATABASE.md`, `docs/DECISION_LOG.md`, `docs/IMPLEMENTATION_CHECKLIST.md` | Phase status, schema, decisions, and links |

## Validation evidence

Commands were run from `D:\Projects\ProbeForge`. PowerShell may render native
stderr as error records; the observed exit codes and summaries below are the
authoritative results.

| # | Command | Actual outcome |
|---|---|---|
| 1 | `bun run format` | Biome formatted 78 files; `cargo fmt --all` completed |
| 2 | `cargo fmt --all -- --check` | Exit 0, no diffs |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 after collapsing two nested `if` blocks; no warnings |
| 4 | `cargo build --locked --workspace` | Finished successfully with the lockfile; no dependency drift |
| 5 | `cargo test --workspace` (no `DATABASE_URL`) | 34 non-DB tests passed: 21 API unit, 5 execution-worker, 5 scheduler, 3 contract. The DB suites returned early by design; they are not counted as DB coverage in this run. |
| 6 | `bun run check:db` with `DATABASE_URL` and `PROBE_FORGE_REQUIRE_DB_TESTS=1` | 5 auth/RBAC tests and 1 migration test passed; repeated runs varied with Argon2 cost (for example 11.38 s and 0.97 s) |
| 7 | `bun run db:migrate` with `DATABASE_URL` | Applied the forward migration set to local `public`; 0007 is now current |
| 8 | Local API smoke on `127.0.0.1:8180` with `DATABASE_URL` and `PROBE_FORGE_COOKIE_SECURE=false` | `/readyz` 200; unauthenticated `/api/projects` 401; login without CSRF 403; `/api/auth/csrf` returned a token |
| 9 | `bun run lint` | Biome checked 79 files; exit 0 |
| 10 | `bun run typecheck` | All four TypeScript workspaces exited 0 |
| 11 | `bun run test:ts` | 61 tests passed: contracts 45, test-catalog 4, analysis-worker 7, web 5 |
| 12 | `bun run build:web` | Vite built 19 modules; production assets emitted |
| 13 | `bun run check:docs` | 49/50 required files before the report existed; after adding the report, see the final `bun run check` row |
| 14 | `bun run check` | Full repository contract, Rust fmt/clippy/tests, Biome, typecheck, and TypeScript tests passed (see below) |
| 15 | `bun run check:services` | **Not completed:** an existing user-started Vite server on `127.0.0.1:5173` made the fail-closed port guard exit 1. No service was stopped and no second web server was started. |

### Web behavior tests

`apps/web/src/App.test.tsx` covers:

1. Signed-out bootstrap, login, and the empty projects state.
2. A `429` login response rendered as an alert without leaving the login view.
3. Signed-in project selection, member loading, and audit-trail rendering.
4. Viewer role receiving no add-member form and no audit section.
5. Sign-out returning to the login view.

### Database behavior tests

`services/api/tests/auth_projects.rs` creates a UUID-named isolated schema per
test and covers:

- Authentication required on project routes.
- Login success, session cookie, CSRF token, logout, and revocation.
- CSRF cookie/header mismatch, cross-site, and origin rejection.
- Login rate limiting and reset after success.
- Owner/admin/collaborator/viewer permissions.
- Cross-project reads and writes denied.
- Ownership transfer and audit events.
- Database-aware readiness.

The tests use inert users/projects only. No scanner, load generator, external
target, or project code was executed.

## Security review

- **No project execution.** Phase 3 adds no repository ingestion, scanning, load
  generation, or child-process execution. Tests create database rows and call the
  local API only.
- **Approval boundary.** No job or approval endpoint exists. All future
  project-scoped routes must join the same `require_auth` protected router before
  they can create or approve work.
- **Least privilege.** Project authorization is re-checked in the database on
  every request and inside mutation transactions. The UI hides controls but never
  substitutes for server checks. Ownership transfer and admin management are
  narrower than general member management.
- **Credential handling.** Argon2id hashes are stored; plaintext passwords are not.
  Session tokens are stored only as SHA-256 digests. Cookies are HttpOnly and
  SameSite=Strict; CSRF tokens are memory-only. The account-provisioning CLI
  prompts for the password and never accepts it as an argument. Tokens, hashes,
  and passwords are not logged.
- **Rate limiting.** Database-backed limits resist process restarts and do not
  store raw account identifiers or IP addresses in the limit table. Forwarded
  headers are ignored, so a client cannot spoof the direct peer identity.
- **Error handling.** Responses use stable codes and generic messages. Database
  errors are logged server-side; clients receive `internal_error` without SQL or
  internal path details.
- **Browser boundary.** The browser uses same-origin `/api` requests through the
  Vite proxy, keeps tokens out of browser storage, and has no database
  credentials.
- **Data integrity.** Audit writes commit with the state change. Immutable
  audit/approval/evidence triggers remain intact; project deletion was deferred
  rather than weakening them.
- **Test isolation.** DB tests use random schemas and drop them; they never touch
  `public`. `PROBE_FORGE_REQUIRE_DB_TESTS=1` prevents a silent skip in
  `check:db`.

## Remaining issues

1. **`check:services` was not completed.** A user-started Vite server occupies
   `127.0.0.1:5173`. Stop it and rerun `bun run check:services` with
   `DATABASE_URL` and a free API port for a full five-service live check.
2. **No OIDC, MFA, password reset, email verification, or public registration.**
   Phase 3 uses local Argon2id accounts provisioned by an operator.
3. **No trusted reverse-proxy client-IP design.** Per-IP login limits use the
   direct peer address; deployments behind a proxy need explicit trusted-proxy
   configuration before those limits are meaningful.
4. **Hard project deletion is deferred (D-022).** Immutable audit/approval/evidence
   records intentionally prevent cascading destructive deletes. Archiving and
   retention need a dedicated design.
5. **No per-member approval grant.** Owner/admin are the approval-capable roles
   for future approval work; the workflow itself is not implemented yet.
6. **The web UI is intentionally smaller than the API.** It covers project
   creation, member addition, audit viewing, and sign-out. Project rename,
   ownership transfer, member role change/removal, and session list/revoke are
   implemented and tested in the backend but not surfaced yet.
7. **No session/rate-limit retention job.** Expired/revoked sessions and old
   login-attempt rows remain until a future cleanup job. Authentication queries
   reject expired/revoked sessions, so this is not an auth bypass.
8. **No jobs, targets, scans, load tests, or reports.** Phases 4-13.
9. **No CI and no Git remote.** The documented local checks are the gate (D-011,
   Q-004).

## Gate check

- [x] **Authentication and project RBAC have automated tests.** 5 DB-backed auth
      and RBAC tests plus 21 API unit tests passed; web behavior tests cover
      login, rate limiting, role gating, and sign-out.
- [x] **Cross-project reads and writes are denied.** The RBAC integration test
      exercises reads and mutations against a project the caller does not belong
      to and receives denial/not-found.
- [x] **Session invalidation and logout work.** Logout and session revocation are
      tested; the auth middleware rejects revoked and expired sessions.
- [x] **No unauthenticated endpoint can create or approve jobs.** No job or
      approval endpoint exists in Phase 3. Project routes require authentication;
      the runtime smoke returned `401` for unauthenticated `/api/projects`.

**PHASE 3: PASS**

## Next permitted step

**Phase 4 — Job creation and PostgreSQL queue** may begin only after explicit
user authorization. Do not implement Phase 4 while resolving the live
`check:services` limitation above.
