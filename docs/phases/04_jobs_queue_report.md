# Phase 4 report — Job creation and PostgreSQL queue

- **Date:** 2026-10-23
- **Status:** `PHASE 4: PASS`
- **Scope:** authenticated job creation and cancellation, the shared PostgreSQL
  queue, leases and fencing, retries and stale-lease recovery, ordered job
  events, SSE replay, and scheduler due-run promotion/claim logic.

## Implemented behavior

### Job API

- `POST /api/jobs?projectId=<uuid>` accepts the unchanged versioned
  `jobCreation` contract for both `vulnerability_testing` and `load_testing`.
- Owner, admin, and collaborator roles may create and cancel jobs; viewers are
  read-only (`403`) and non-members receive `404`.
- Git URLs require HTTPS or SSH, reject embedded usernames/passwords, and accept
  credentials only through `credentialRef` (`secret://...`). A concrete
  `git.reference` is required, so Phase 4 never enqueues an unresolvable job.
- Project provider is derived from the URL host and stored in `git_sources`.
  Credential references are stored but never returned in job DTOs.
- `Idempotency-Key` is an optional header. An identical retry returns the
  existing job with `200`; the same key with a different payload returns
  `409 idempotency_key_reused`. The unique project/idempotency index is the
  second line of defense.
- Creation is one transaction: source upsert, job insert, `job.queued` event, and
  `job.created` audit record. Failure leaves no job or event.
- Job reads and bounded project listings are available to project members.
- Cancellation clears worker/lease state, appends `job.cancelled`, and audits the
  change. It is idempotent for an already cancelled job; failed/expired jobs
  return `409 job_terminal`.
- Unauthenticated unsafe requests now authenticate before CSRF and receive
  `401`; authenticated unsafe requests without a valid token still receive
  `403`.

### Shared queue

`crates/queue` is used by the API, tests, and scheduler:

- Atomic claim with `FOR UPDATE SKIP LOCKED` and a bounded candidate query.
- Claim, attempt increment, worker ID, heartbeat, lease, and `job.claimed` event
  commit before the worker does any work.
- Heartbeat and completion/failure require the same worker and an unexpired
  lease.
- Retryable failures return to `queued` with bounded exponential backoff (1, 2,
  4, ... seconds, capped at 300) until `max_attempts`; exhausted failures become
  terminal.
- Expired leases are recovered under `FOR UPDATE SKIP LOCKED`; jobs retry with
  backoff or become `failed`.
- Due `scheduled` runs are promoted to `queued`.
- `claim_due_run` atomically claims a due run for the future execution
  dispatcher and appends `run.started` to the parent job stream.
- The scheduler loop owns a database pool, promotes due runs, and recovers stale
  job leases. It does not claim runs until an execution dispatcher exists.

### Events and SSE

- Per-job immutable events with gap-free sequence numbers:
  `job.queued`, `job.claimed`, `job.retry_scheduled`, `job.failed`,
  `job.cancelled`, `plan.proposed`, and `run.started`.
- `GET /api/jobs/{id}/events` emits `text/event-stream` with the sequence as the
  SSE ID and the event type as the SSE event name. Data is the `jobEvent`
  contract JSON.
- `Last-Event-ID` is the primary reconnect cursor; `?after=` is an explicit
  fallback. Replay is exclusive (`after=1` starts at sequence 2).
- `?follow=false` returns a finite replay stream; the default follows until the
  job is terminal. Long-lived SSE streams bypass the normal 30-second API request
  timeout.
- Invalid cursors return `400`, unauthenticated requests `401`, and non-members
  `404`. A database failure during a live stream emits a safe `stream.error`
  event and closes the stream.

### Contracts

- Added `job.claimed` and `job.retry_scheduled` to the Rust event enum, TypeScript
  event union, and JSON Schema enum.
- Added `event.claimed.valid.json` and a shared manifest case; both validators
  now cover the new event type.

## Files changed

### New

| File | Purpose |
|---|---|
| `crates/queue/Cargo.toml` | Shared queue crate |
| `crates/queue/src/lib.rs` | Claims, heartbeats, fencing, retries, recovery, due-run promotion/claim |
| `crates/queue/tests/queue.rs` | Concurrent-claim, lease, retry, fencing, and scheduler integration tests |
| `services/api/src/jobs.rs` | Job API, idempotency, cancellation, event stream, SSE |
| `services/api/tests/jobs_queue.rs` | Job API, authorization, idempotency, cancellation, SSE replay tests |
| `packages/contracts/fixtures/v1/event.claimed.valid.json` | Cross-language event fixture |
| `docs/JOBS.md` | Job API, queue, event, and scheduler contract |
| `docs/phases/04_jobs_queue_report.md` | This report |

### Modified

| File | Why |
|---|---|
| `Cargo.toml`, `services/api/Cargo.toml`, `services/scheduler/Cargo.toml`, `Cargo.lock` | Queue crate, async-stream/futures-core, SQLx scheduler dependencies |
| `crates/contracts/src/model.rs`, `packages/contracts/src/model.ts`, JSON Schema | New event types and fixture |
| `services/api/src/lib.rs` | Protected job routes, auth-before-CSRF ordering, SSE-aware timeout |
| `services/api/src/error.rs` | Contract-validated 256 KiB JSON extractor |
| `services/api/src/projects.rs` | Job-creator role and shared project-role lookup |
| `services/api/tests/support/mod.rs` | Idempotency and raw SSE replay test helpers |
| `services/scheduler/src/lib.rs` | Database pool, DB-aware readiness, due-run promotion, stale-lease loop |
| `scripts/db-check.mjs` | Runs API migration/auth/job and queue DB suites |
| `scripts/services.mjs` | Scheduler readiness uses `/readyz` |
| `scripts/check-docs.mjs` | Requires queue, job API, report, fixture, and docs |
| `.env.example`, `README.md`, `docs/README.md`, `docs/AUTH.md`, `docs/CONTRACTS.md`, `docs/DATABASE.md`, `docs/DECISION_LOG.md`, `docs/IMPLEMENTATION_CHECKLIST.md` | Status, contracts, decisions, and role/queue documentation |

## Validation evidence

| # | Command | Actual outcome |
|---|---|---|
| 1 | `bun run format` | Biome formatted 79 files; `cargo fmt --all` completed |
| 2 | `cargo fmt --all -- --check` | Exit 0; no diffs |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0; no warnings |
| 4 | `cargo build --locked --workspace` | Finished successfully with both lockfiles |
| 5 | `cargo test --workspace` (no `DATABASE_URL`) | 40 non-DB tests passed: 24 API, 5 execution-worker, 3 queue unit, 5 scheduler, 3 contract. DB suites returned early by design; they are covered by `check:db`. |
| 6 | `bun run check:db` with `DATABASE_URL` and `PROBE_FORGE_REQUIRE_DB_TESTS=1` | 13 DB tests passed: 5 auth/RBAC, 2 job API/SSE, 1 migration, 5 queue/scheduler. |
| 7 | `bun run lint` | Biome checked 80 files; exit 0 |
| 8 | `bun run typecheck` | All four TypeScript workspaces exited 0 |
| 9 | `bun run test:ts` | 62 tests passed: contracts 46, test-catalog 4, analysis-worker 7, web 5 |
| 10 | `bun run build:web` | Vite built 19 modules; production assets emitted |
| 11 | Runtime smoke on `127.0.0.1:8180` / `8181` | API `/readyz` 200; unauthenticated job POST 401; unauthenticated SSE 401; scheduler `/readyz` 200 |
| 12 | `bun run check` | Full docs/format/clippy/tests/typecheck/test suite passed after the report was added |
| 13 | `bun run check:services` | **Not completed:** an existing user-started Vite server on `127.0.0.1:5173` made the fail-closed port guard exit 1. No service was stopped. |

### Database tests

`crates/queue/tests/queue.rs` covers:

- Two concurrent workers claim disjoint jobs; a third claim returns none.
- Heartbeats succeed only for the current unexpired lease owner.
- Expired leases recover with retry events and bounded attempts.
- A stale worker cannot complete or fail after a newer worker claims the job.
- Due scheduled runs promote to queued and one worker claims them; a second claim
  returns none and `run.started` is appended once.

`services/api/tests/jobs_queue.rs` covers:

- Contract validation, project roles, cross-tenant denial, and unauthenticated
  job creation denial.
- Idempotent retry, idempotency-key reuse conflict, and job/event/audit writes.
- Cancellation clearing leases and blocking stale heartbeats/completions.
- SSE replay with `Last-Event-ID`, finite `follow=false`, and outsider/unauth
  denial.

All tests use isolated schemas, inert users/projects/repositories, and local
database rows. No Git clone, scanner, load generator, target request, or model
call was performed.

## Security review

- **Authorization:** every job route re-checks project membership in the
  database. Viewers cannot create/cancel; outsiders receive `404`; unauthenticated
  unsafe requests receive `401`.
- **Untrusted input:** job creation uses the shared contract validator, the
  256 KiB cap, unknown-field rejection, and executable-field rejection. Git URLs
  cannot carry credentials. Idempotency keys and worker/error codes are bounded
  and control-character free.
- **Queue safety:** claims use `FOR UPDATE SKIP LOCKED` and commit before work.
  Worker IDs fence every heartbeat, completion, and failure. Expired leases are
  recovered with bounded retries; a stale worker cannot overwrite newer state.
- **Secret handling:** credential references are stored but never returned in job
  DTOs or SSE payloads; repository URLs and credentials are never logged.
- **Event integrity:** events are append-only, sequence-ordered, and scoped by
  job. SSE checks project membership before streaming and does not expose cross-
  project events.
- **No execution in Phase 4:** tests use local fixtures; the analysis worker still
  serves health endpoints only. The scheduler promotes/claims records but does not
  run scanners or load generators.
- **No weakened controls:** auth-before-CSRF ordering improves response
  consistency without disabling CSRF on authenticated mutations. The scheduler
  does not fake running runs without a dispatcher.

## Remaining issues

1. **`check:services` was not completed.** A user-started Vite server occupies
   `127.0.0.1:5173`. Stop it and rerun with `DATABASE_URL` for the full
   five-service live check.
2. **No analysis-worker polling yet.** Phase 5/7 wires safe Git ingestion, the Pi
   planning session, and worker-side claim/heartbeat calls.
3. **No execution dispatcher.** `claim_due_run` is implemented and tested, but the
   scheduler does not dispatch runs until Phases 10/12.
4. **No job UI.** The Phase 3 project workspace remains; jobs are API/SSE only.
5. **Polling instead of LISTEN/NOTIFY.** Scheduler and SSE use short polling with
   a database fallback; no wake-up hint is added yet.
6. **No job/event retention job.** Events remain append-only; a retention policy
   needs a separate design.
7. **No jobs beyond creation/queue.** Repository snapshots, plans, approvals,
   targets, scanners, load generation, and reports remain Phases 5-13.
8. **No CI/Git remote.** Local checks are the gate (D-011, Q-004).

## Gate check

- [x] **Concurrent worker integration tests show a job is not simultaneously
      claimed by two live workers.** `concurrent_workers_cannot_claim_the_same_job`
      passes with disjoint claims and no extra job.
- [x] **Expired leases can be recovered.** `retries_backoff_and_expiry_recovery_are_bounded`
      and the heartbeat/fencing test recover expired leases with backoff.
- [x] **A worker cannot complete a job after losing its lease.**
      `heartbeat_and_completion_are_fenced_by_the_lease_owner` and
      `stale_worker_cannot_fail_or_complete_after_a_new_claim` pass.
- [x] **SSE reconnects can replay events.**
      `cancellation_clears_leases_and_sse_replays_after_last_event_id` verifies
      `Last-Event-ID: 1` returns only sequence 2.
- [x] **All tests use local fixtures and do not invoke real scanners.** The API
      and queue tests use inert metadata and isolated schemas; no network target,
      Git clone, scanner, load runner, or model call is made.

**PHASE 4: PASS**

## Next permitted step

**Phase 5 — Read-only Git ingestion** may begin only after explicit user
authorization. Do not implement Phase 5 while resolving the live
`check:services` limitation above.
