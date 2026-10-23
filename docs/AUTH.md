# ProbeForge authentication and authorization

Phase 3 implements local password authentication, opaque server-side sessions, project
membership roles, backend authorization, and audit events. This document is the
implementation contract for those controls.

## Authentication model

- **Passwords:** Argon2id through the maintained RustCrypto `argon2` crate. The
  PHC string is stored in `users.password_hash`; plaintext passwords are never
  persisted or logged. Passwords must be 12-256 bytes.
- **Sessions:** 256-bit random opaque tokens. Only a SHA-256 digest is stored in
  `auth_sessions.token_hash`; the plaintext token exists only in the HttpOnly
  cookie and the immediate response.
- **Cookies:** `pf_session` is `HttpOnly`, `SameSite=Strict`, `Path=/`, and
  `Secure` when `PROBE_FORGE_COOKIE_SECURE=true`. It has an absolute expiry
  (default 8 hours). Logout and session revocation set `revoked_at`; the
  authentication middleware requires `revoked_at IS NULL` and an unexpired
  session.
- **CSRF:** Double-submit token. The readable `pf_csrf` cookie must match the
  `X-CSRF-Token` header on every state-changing request. The CSRF token is not an
  authentication credential and is never placed in local storage. The guard also
  rejects `Sec-Fetch-Site: cross-site` and origins outside
  `PROBE_FORGE_ALLOWED_ORIGINS`.
- **Login rate limits:** Five failed attempts per normalized email plus direct
  peer address in a fifteen-minute window, and fifty attempts per peer address.
  Attempts are stored as SHA-256 key digests; raw emails and IP addresses are not
  stored in the rate-limit table. A successful login clears both keys.
- **No public registration:** Accounts are provisioned by an operator with
  `bun run user:create`. This avoids an unauthenticated account-creation surface
  in Phase 3.

## Account provisioning

1. Apply migrations with `DATABASE_URL` set: `bun run db:migrate`.
2. Create an account. The password is prompted and never passed as an argument:

   ```bash
   bun run user:create -- owner@example.test "Owner"
   ```

3. Start the API and web app. The API requires `DATABASE_URL`; it fails fast if
   the database is unavailable. `/readyz` returns `ready` only after at least one
   successful migration is recorded.

## Project roles

| Capability | Owner | Admin | Collaborator | Viewer |
|---|---:|---:|---:|---:|
| View project and members | yes | yes | yes | yes |
| Update project name/slug | yes | yes | no | no |
| Add/change/remove collaborators and viewers | yes | yes | no | no |
| Add/change/remove admins | yes | no | no | no |
| Change/remove the owner | no | no | no | no |
| Transfer ownership | yes | no | no | no |
| View project audit trail | yes | yes | no | no |

Ownership transfer is a dedicated operation: the current owner must already be a
member and the new owner must already be a project member. The former owner
becomes an admin; the new owner becomes the owner. Non-members receive `404` for
project routes so project existence is not disclosed.

Collaborators are intended to create and edit jobs and plans in later phases.
They do not receive approval authority in Phase 3. Per-member approval grants are
deferred until the approval workflow is implemented.

## API surface

| Method | Path | Access |
|---|---|---|
| `GET` | `/api/auth/csrf` | public; sets the CSRF cookie |
| `POST` | `/api/auth/login` | public; requires CSRF; sets session and CSRF cookies |
| `GET` | `/api/auth/session` | authenticated |
| `POST` | `/api/auth/logout` | authenticated; revokes the current session |
| `POST` | `/api/auth/logout-all` | authenticated; revokes all user sessions |
| `GET` | `/api/auth/sessions` | authenticated; lists own sessions |
| `DELETE` | `/api/auth/sessions/{id}` | authenticated; revokes an own session |
| `GET` | `/api/projects` | authenticated; lists memberships |
| `POST` | `/api/projects` | authenticated; creator becomes owner |
| `GET` | `/api/projects/{id}` | project member |
| `PATCH` | `/api/projects/{id}` | owner or admin |
| `GET` | `/api/projects/{id}/members` | project member |
| `POST` | `/api/projects/{id}/members` | owner or admin, role-dependent |
| `PATCH` | `/api/projects/{id}/members/{userId}` | owner or admin, role-dependent |
| `DELETE` | `/api/projects/{id}/members/{userId}` | owner or admin, role-dependent |
| `POST` | `/api/projects/{id}/transfer-owner` | owner |
| `GET` | `/api/projects/{id}/audit` | owner or admin; latest 50 by default |

All `/api` state-changing routes pass through the CSRF guard. Every project route
requiring authentication is layered behind `require_auth`; project handlers also
re-check membership and role in the database. Future job, approval, target, and
report routes must be added to the same protected router.

## Audit events

Sensitive changes insert an immutable `audit_log` row in the same transaction as
the state change. Phase 3 records login, logout, session revocation, project
creation/update, member addition/role change/removal, and ownership transfer. Audit
metadata contains identifiers and roles, not passwords, session tokens, CSRF
tokens, or email addresses.

## Configuration

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | required | PostgreSQL connection string; never logged |
| `PROBE_FORGE_COOKIE_SECURE` | `true` | Local HTTP development sets `false`; production must use `true` over HTTPS |
| `PROBE_FORGE_ALLOWED_ORIGINS` | `http://127.0.0.1:5173,http://localhost:5173` | Comma-separated exact origins; no paths |
| `PROBE_FORGE_SESSION_TTL_SECONDS` | `28800` | Allowed range 300-604800 |

The web client calls the API through the Vite `/api` proxy in development and
same-origin routing in deployment. The browser never connects to PostgreSQL and
never stores an access token in local storage.

## Verification

```bash
bun run check       # docs, Rust lint/tests, TypeScript lint/type/tests
bun run check:db    # migration + auth/RBAC integration tests (requires DATABASE_URL)
bun run build       # Rust workspace and web production build
```

The integration suite creates isolated `pf_test_*` schemas. It covers login,
logout, session revocation, CSRF and origin rejection, rate limiting, role
permissions, cross-project reads/writes, audit writes, and database readiness.

## Known limitations and deferred work

- No public registration, email verification, password reset, MFA, or OIDC
  provider yet. The maintained Argon2id library is the Phase 3 authentication
  approach.
- No trusted reverse-proxy client-IP configuration. Rate limiting uses the direct
  peer address; deployments behind a proxy must add an explicit trusted-proxy
  design before relying on per-IP limits.
- No hard project deletion. Phase 2's immutable audit, approval, and evidence
  triggers intentionally block cascading destructive deletes. Archiving and
  retention require a dedicated design.
- No per-member approval grant; owner and admin are the approval-capable roles
  when approvals are implemented.
- No platform-wide administrator role. Access is project-scoped.
- Expired/revoked sessions and old login-attempt rows are retained until a
  future retention/cleanup job. Queries only accept unexpired, unrevoked
  sessions, so retention is an operational concern rather than an auth bypass.
- The Phase 3 web UI exposes project creation, member addition, audit viewing,
  and sign-out. Project rename, ownership transfer, member role change/removal,
  and the session-list/revoke endpoints exist and are tested but are not yet
  surfaced in the UI.
