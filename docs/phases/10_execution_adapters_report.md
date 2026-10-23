# Phase 10 report — Security execution adapters

**Status:** PHASE 10: PASS

**Date:** 2026-10-23

**Scope:** Added a shared, server-owned adapter registry and two vetted native
HTTP adapters (`http.headers.missing`, `http.cors.misconfiguration`) backed by
the Phase 9 guarded client and run budget. Added typed configuration, normalized
results, independent execution/security states, sanitized evidence, timeouts,
cancellation, response-size caps, and fail-closed behavior for unsupported
catalog IDs. No run creation, dispatcher, persistence, UI, real-target scan, or
load test was added.

## What changed

### Adapter library

- New `crates/execution-adapters` workspace crate:
  - `http_probe` 0.1.0 with two catalog mappings:
    - `http.headers.missing`
    - `http.cors.misconfiguration`
  - `AdapterSpec` registry with fixed catalog ID, adapter name, adapter version,
    kind, and 30-second timeout.
  - Typed route configuration (`route` or `path`) with unknown fields denied.
  - Safe absolute-path checks for traversal, query/fragment, backslash, double
    slash, control characters, and length.
  - Scope validation through `validate_request_url` before any request.
  - Requests only through `GuardedHttpClient`; CORS sends only the fixed
    harmless origin `https://probe.invalid`.
  - Common `AdapterResult` schema with separate `ExecutionStatus` and
    `SecurityOutcome`.
  - Severity/confidence, bounded summary, duration, evidence, and stable error
    codes.
  - No response bodies, cookies, arbitrary headers, or resolved addresses in
    evidence.
  - Unsupported catalog IDs return `unsupported` / `not_run` and make no request.
- `crates/target-policy/src/lib.rs`:
  - Added `GuardedHttpClient::get_with_headers`.
  - Added request-header sanitization with a max count/size, control-character
    rejection, and denial of `authorization`, `cookie`, `proxy-authorization`,
    `set-cookie`, `host`, `content-length`, `transfer-encoding`, and
    `connection`.
  - Added `TargetRequestError::InvalidHeader`.

### Catalog availability

- `services/api/src/catalog.rs` now derives `available` from the adapter
  registry by catalog ID rather than from a shared tool-requirement string.
- `http.headers.missing` and `http.cors.misconfiguration` report available with
  a note that run creation is not wired.
- All other catalog IDs remain unavailable, including `http.auth.rate-limit`,
  which shares the `http_probe` tool requirement but has no adapter.
- API/plan-review tests and the web plan-review fixture were updated to reflect
  the two available adapters. The UI still does not execute anything.

### Documentation

- New `docs/EXECUTION_ADAPTERS.md`.
- Decision D-031 records the native-adapter/no-shell choice and future external
  process requirements.
- Updated `docs/PLAN_APPROVAL.md`, `docs/README.md`, `README.md`,
  `docs/IMPLEMENTATION_CHECKLIST.md`, and `scripts/check-docs.mjs`.
- No database migration was required.

## Exit gate evidence

- **Each adapter has schema, timeout, cancellation, and output-normalization
  tests.**
  - Schema: both catalog IDs are exercised with an empty configuration, an
    unknown `command` field, and a traversal route; all fail before any request.
  - Timeout: both IDs are exercised against a delayed local mock server with a
    50 ms test timeout. Result: `timed_out` + `inconclusive`.
  - Cancellation: both IDs run with a cancelled `RunBudget`. Result:
    `cancelled` + `inconclusive`.
  - Output normalization: header missing/present behavior, reflected CORS with
    credentials, wildcard CORS with credentials, wildcard CORS without
    credentials, and 5xx inconclusive behavior.
  - Response-size cap: both IDs fail with `response_size_exceeded` for a declared
    body over 2 MiB.
  - Sanitization: serialized results do not contain mock response bodies or
    `Set-Cookie` values.
- **Unsupported test IDs fail closed.**
  - `static.secrets.exposure` returns `unsupported` / `not_run` with
    `unsupported_catalog_test`; the mock server received zero requests.
- **No arbitrary shell execution is possible through agent input.**
  - The adapter crate has no `std::process`, `Command::new`, shell, PowerShell,
    or executable-path code. Search: no process/shell execution patterns found.
  - Only a fixed catalog-to-adapter registry can execute.
  - Configuration is closed to `route`/`path`; an injected `command` field is
    rejected.
  - No external tool is selected in this phase, so process limits and child
    process-tree termination are not applicable. The design requires them for
    any future external adapter.
- **Real-target scans remain disabled during development.**
  - There is no run-creation or execution-dispatch API/worker path.
  - Adapter availability means "a vetted adapter exists", not "a run can start";
    the API note says run creation is not wired.
  - All adapter tests use a loopback mock HTTP server under the explicit local
    test policy. No real target or deployment was contacted.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL` for database and service checks.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | PASS — 6 adapter tests, 8 target-policy tests, 27 API unit tests, 5 plan approval integration tests, plus auth/jobs/migrations/targets/queue/contracts suites. |
| `bun run check` | PASS — docs 77/77 required files, 13 documents linked, 16 phase headings; Rust fmt/clippy/tests; Biome; TypeScript typecheck/tests. Analysis worker: 111 passed / 7 skipped / 579 assertions; contracts: 47 passed; test-catalog: 6 passed; web: 8 passed. |
| `bun run check:db` | PASS — auth/projects (5), jobs (2), migrations (1), plan approval (5), targets (5), queue (5). |
| `bun run db:migrate` | PASS — migrations through `0010`; no new migration. |
| `bun run check:services` | PASS — 5/5 healthy; ports 8080, 8081, 8082, 8090, and 5173 closed afterward. |
| `git diff --check` | PASS — no whitespace errors. |
| `rg -n "std::process\|Command::new\|sh -c\|cmd /C\|powershell\|exec\\(" crates/execution-adapters crates/target-policy services/api/src/catalog.rs` | No process/shell execution patterns found. |

Setup notes observed during verification:

- The first `check:services` attempt failed because a stale local
  `probe-forge-execution-worker` process held port 8082; it was stopped.
- The next attempt failed with `MissingDatabaseUrl`; rerunning with the
  command-scoped local `DATABASE_URL` passed 5/5.
- `bun run check` initially failed only because the phase report file did not
  exist yet; after the report was created, the full check passed.

## Known limitations

- Only two HTTP catalog IDs have adapters. Static secret/dependency, TLS,
  rate-limit, and load checks remain unavailable.
- The header adapter is a baseline check (CSP, `X-Content-Type-Options`,
  `Referrer-Policy`, plus HSTS only for HTTPS). The HTTPS/HSTS branch is not
  exercised by the loopback HTTP test server.
- CORS normalization uses one fixed, harmless probe origin. It does not test
  credentialed origins, preflight behavior, or application-specific policy.
- Adapter results are in memory only. Durable raw observations, evidence
  records, result pages, and reports belong to Phase 13.
- There is no run creation, worker claim/dispatch, execution UI, or scheduling
  integration for these adapters. No target was contacted.
- No external executable/process runner is shipped. A future external scanner
  or k6 adapter must add a fixed server-owned executable path, typed arguments,
  output parsing, process limits, and process-tree termination under the same
  registry and target policy.
- An "available" catalog entry currently means a vetted adapter exists, not that
  a production run can be started.

## Next permitted phase

**Phase 11 — Load scenario generation and validation.** Not started. It requires
explicit authorization after this report is reviewed.
