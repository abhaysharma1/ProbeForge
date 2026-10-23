# Phase 9 report — Target validation and authorization

**Status:** PHASE 9: PASS

**Date:** 2026-10-23

**Scope:** Added project-scoped target registration, server-owned URL/DNS/IP
policy, explicit private-network mode, redirect and scope enforcement,
connection-time destination checks, per-run budgets, target validation evidence,
and time-bounded authorization records. No test or scan was sent to a real
target. The only network test uses a loopback mock server under explicit private
mode.

## What changed

### Target policy and guarded client

- `crates/target-policy/src/lib.rs`:
  - target origin normalization (scheme, host, canonical port, no credentials/
    path/query/fragment);
  - allowed host/path/port normalization and validation;
  - default-deny IP classification for loopback, RFC1918/CGNAT, ULA, link-local,
    multicast, reserved, documentation, unspecified, IPv4-mapped private, and
    known cloud metadata addresses;
  - explicit private-network policy that still blocks metadata/link-local/
    reserved ranges;
  - injectable DNS resolver with a system resolver;
  - request scope checks for scheme, host, port, path prefix, encoded traversal,
    credentials, fragments, and disallowed ports;
  - redirect validation with max five hops, scope checks, and HTTPS downgrade
    rejection;
  - a `GuardedHttpClient` that disables proxies/connection reuse/automatic
    redirects, pins validated IPs per request with `resolve_to_addrs`,
    re-resolves each hop, caps response size and total bytes, and honors
    cancellation;
  - `RunBudget` with rate spacing, concurrency semaphore, duration deadline,
    request count, response-size, total-byte, and cancellation limits;
  - authorization expiry helpers (24-hour default, 7-day maximum).
- `crates/contracts` exposes `validate_target_scope` and
  `validate_target_limits`, and normalizes bracketed IPv6 origin comparison.

### API and persistence

- `services/api/src/targets.rs` adds:
  - `GET`/`POST /api/projects/{projectId}/targets`
  - `GET /api/targets/{targetId}`
  - `POST /api/targets/{targetId}/validate`
  - `GET`/`POST /api/targets/{targetId}/authorizations`
  - `POST /api/target-authorizations/{authorizationId}/revoke`
- Target and authorization writes are limited to owner/admin/collaborator;
  viewers are read-only and outsiders receive not-found.
- Target validation resolves and records addresses, rejects unsafe/empty/
  failed answers, and updates verification state.
- Authorization creation requires a verified target, explicit confirmation,
  bounded limits, and a future expiry. Authorization-time revalidation refreshes
  the target evidence.
- Revocation writes `revoked_at` once; scope/limits/confirmation remain immutable.
- Audit records: `target.created`, `target.validated`,
  `target.validation_failed`, `target.authorization_created`,
  `target.authorization_revoked`.
- `PROBE_FORGE_ALLOW_PRIVATE_TARGETS` is false by default. A separate
  `TargetPolicy::local_test` is used only by fixtures.

### Migrations

- `0010_target_validation.sql` adds `targets.verification_checked_at`,
  `targets.verified_addresses`, and `targets.allowed_ports` with bounded JSONB
  constraints.

### Tests and tooling

- `target_policy` unit tests cover address classification, private mode, origin
  normalization, invalid schemes, sensitive ports, path/scope escapes, redirect
  downgrade/escape, DNS change to a blocked address, budgets, cancellation, and
  a loopback mock HTTP client.
- `services/api/tests/targets_authorization.rs` covers target registration,
  validation, tenant isolation, viewer denial, default private-target denial,
  authorization confirmation/limits, expiry/revocation, and audit records.
- `scripts/db-check.mjs` now includes target authorization tests.
- Documentation: `docs/TARGET_VALIDATION.md`, decision D-030, contracts/database
  docs, checklist, README, and `.env.example`.

## Exit gate evidence

- **Invalid schemes:** `ftp`, credentials, path/query origins, malformed paths,
  sensitive ports, and production HTTP are rejected by tests.
- **Redirect escapes:** `validate_redirect` rejects out-of-scope paths/hosts/
  ports and HTTPS-to-HTTP downgrades; `GuardedHttpClient` follows a safe
  loopback redirect and blocks a scope escape before a second request.
- **Private/link-local/metadata:** unit tests assert loopback, RFC1918, CGNAT,
  link-local, metadata, IPv6 ULA/link-local/IMDS, multicast, reserved,
  documentation, and unspecified addresses are blocked by default. Private mode
  allows loopback/RFC1918/ULA while still blocking metadata/link-local.
- **DNS changes:** a static resolver that returns a public address then a
  loopback address fails closed; each guarded request re-resolves and pins the
  validated answer set.
- **Disallowed ports:** ports 22/5432/6379/9200 and the sensitive-port list are
  rejected even when explicitly listed.
- **Scope escapes:** host, port, scheme, path prefix, encoded traversal, and
  redirect escapes are rejected.
- **Server-side authority:** model output cannot create targets, widen scope,
  alter budgets, or call the guarded client; all target policy is deterministic
  Rust policy and database state.
- **No real target traffic:** API tests resolve literal loopback/public fixture
  addresses only; the guarded client test uses a local mock HTTP server under
  explicit private mode.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL` unless noted.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | PASS — 34 API unit tests, 5 target authorization integration tests, 5 plan approval tests, migration/auth/job/queue/contract suites. |
| `bun run check:db` | PASS — migration, auth/project, jobs, plan approval, target authorization, queue. |
| `bun run check` | PASS — docs, Rust fmt/clippy/tests, Biome, TypeScript. Analysis worker: 111 passed / 7 skipped / 579 assertions; contracts: 47 passed; test-catalog: 6 passed; web: 8 passed. |
| `bun run check:services` | PASS — 5/5 services healthy; ports 8080, 8081, 8082, 8090, and 5173 were closed after the check. |
| `bun run db:migrate` | PASS — migrations through `0010`. |

## Known limitations

- There is no target registration/authorization UI yet. The API is ready for the
  run workflow; audit actions have labels in the existing project workspace.
- Ownership verification is not implemented. Only explicit confirmation is
  accepted; the contract-level ownership method is rejected by the API.
- A real execution adapter has not been wired to `GuardedHttpClient`; Phase 10
  must use this path and not a raw client.
- The system resolver is used through `tokio::net::lookup_host`; a future
  deployment should add a pinned resolver/egress policy appropriate to its
  environment.
- Response-size and total-byte ceilings are platform constants rather than
  per-authorization fields because the v1 target contract has no such fields.
- Private-target mode is server-config only and should not be enabled for an
  untrusted multi-tenant deployment without network isolation.
- No target UI, scan, load test, or real target traffic was performed.

## Next permitted phase

**Phase 10 — Security execution adapters.** Not started. It requires explicit
authorization after this report is reviewed.
