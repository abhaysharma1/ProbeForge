# Phase 14 report — Observability, quotas, and operational hardening

**Status:** PHASE 14: PASS

**Date:** 2026-10-24

**Scope:** Added shared structured logging with correlation IDs and redaction, an
internal queue/worker metrics endpoint, server-enforced per-project quotas and
request budgets, API and expensive-operation rate limits, a single sanctioned
retention purge, a tested PostgreSQL backup/restore roundtrip, and an operator
runbook. No real target, scanner, or load traffic is involved.

## What changed

### Schema (migration `0014_observability.sql`)

- `project_quotas`: per-project `max_active_runs`, optional
  `max_requests_per_window`, and `window_seconds`.
- `api_rate_limits`: SHA-256-keyed fixed-window counters, separate from the
  authentication attempt counter.
- `reject_immutable_row` is replaced so it still rejects every UPDATE and DELETE
  except a DELETE inside a transaction that opts in with
  `SET LOCAL probeforge.retention_purge = 'on'`.
- Indexes for retention scans (`evidence.retention_until`, `job_events.occurred_at`).

### Shared observability crate (`crates/observability`)

- `init_tracing(service)` with `PROBE_FORGE_LOG_FORMAT=json` support, used by the
  API, scheduler, and execution worker.
- A validated `RequestId` (printable ASCII, bounded) and a `Redacted` wrapper.
- The single redaction implementation (`redact_json`/`redact_text`) moved out of
  `results.rs`, which now imports it.

### API (`services/api`)

- `http_middleware::request_id` attaches and echoes a correlation ID and carries
  it on a `http_request` tracing span.
- `http_middleware::rate_limit` enforces per-user API and expensive-operation
  limits using `rate_limit::enforce` (SHA-256 keys).
- `quotas.rs` adds `GET|PUT /api/projects/{id}/quota` with owner/admin writes,
  member reads, and a `project.quota_updated` audit record.
- `AuthConfig` carries the two rate limits; `Config::from_env` reads them.

### Queue (`crates/queue`)

- `queue_metrics` returns queue depth, active work, expired leases, recent
  failures, and completed-run duration.
- `set_project_quota` / `project_quota`.
- `claim_due_run_fair` overlays per-project active-run caps and refuses a claim
  whose project has exhausted its request budget for the window.
- `run_retention` performs the bounded, opt-in purge and reports counts.

### Scheduler and workers (`services/scheduler`, `services/execution-worker`)

- `GET /metricsz` exposes aggregate queue metrics on the internal scheduler.
- A retention loop runs the purge on `PROBE_FORGE_RETENTION_INTERVAL_SECONDS`.
- Both services initialize tracing through the observability crate.

### Tooling and docs

- `scripts/db-backup.mjs` (`backup`, `restore`, `check`) plus `db:backup`,
  `db:restore`, and `db:backup:check`; `restore` guards the primary database.
- New `docs/OPERATIONS.md`; decision log D-035–D-038; checklist and index
  updates; `.env.example` placeholders; `backups/` gitignored.

## Exit gate evidence

- **Operators can diagnose stuck jobs and failed workers.** `GET /metricsz`
  reports lease-expiry, queue depth, and recent failures, and
  `docs/OPERATIONS.md` includes the exact lease/failure SQL and an incident
  runbook.
- **Quotas are enforced server-side.** `project_quota_is_manageable_and_enforced_server_side`
  sets a one-active-run cap through the API and asserts a second concurrent
  claim returns none; `request_budget_blocks_claims_over_the_window_budget`
  asserts a claim is refused at a 50-request budget and allowed at 200.
- **Restore and retention procedures are documented and tested.**
  `bun run db:backup:check` dumps, restores into a scratch database, verifies the
  migration count, and drops the scratch database;
  `retention_purges_expired_data_and_preserves_immutability` asserts the purge
  works and that an ordinary DELETE is still rejected.
- **No secrets are emitted in logs.** Redaction is a single tested
  implementation; the Phase 13 redaction assertions still pass, and the
  observability crate adds marker/JWT tests.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL`.

| Command | Result |
|---|---|
| `cargo test -p probe-forge-observability` | PASS — 4 unit tests (request ID validation, key/marker/JWT redaction, wrapper). |
| `DATABASE_URL=... cargo test -p probe-forge-api --test operations` | PASS — 6 integration tests. |
| `bun run check:db` | PASS — api schema/auth/jobs/plans/targets (migrations 1, auth 5, jobs 2, plans 6, targets 5, results 4), operations (6), queue (8). |
| `bun run db:backup:check` | PASS — source and restored scratch each report 14 applied migrations. |
| `bun run db:migrate` | PASS — migration `0014_observability` applied after `0013`. |
| `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check` | PASS — docs 90/90 required files, 17 linked docs, 16 phase headings; fmt; clippy `-D warnings`; workspace tests (api lib 34, observability 4); Biome 150 files; typecheck; contracts 55, test-catalog 6, analysis-worker 113 pass / 7 skip, web 10. |
| `bun run check:services` | PASS — 5/5 healthy after rebuild; ports closed afterward. |
| live `GET http://127.0.0.1:8081/metricsz` | PASS — JSON queue metrics returned (`failed_window_seconds` 3600). |
| `git diff --check` | PASS — no whitespace errors. |

Failures observed and resolved during verification:

- The first `run_retention` reuse of the request `validate_limit` (max 500)
  rejected the 2000-row event/counter batches; added `validate_retention_batch`
  (1–100000).
- Inline `format!("... {Uuid::new_v4()}")` is invalid Rust; switched to
  positional arguments.
- Renamed the API middleware module to `http_middleware` to avoid colliding with
  `axum::middleware`.

## Known limitations

- No external metrics/alerts system is integrated; `/metricsz` and structured
  logs are the interfaces.
- The execution worker remains health-only, so quotas and leases are enforced
  and tested at the queue layer but not exercised end to end; Phase 15 wires the
  dispatch loop.
- Retention deletes but does not archive immutable rows before deletion.
- Rate-limit windows are per shared table; cross-host clock skew is not
  corrected.

## Next permitted phase

**Phase 15 — End-to-end acceptance using local fixtures.** Not started. It
requires explicit authorization after this report is reviewed.
