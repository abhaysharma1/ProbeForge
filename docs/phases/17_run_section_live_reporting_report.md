# 17 — Run section live reporting (SSE status + report-ready)

**Date:** 2026-10-27
**Status:** PHASE 17: PASS for the agreed scope (run status + report-ready live reporting, full doc-design delivery).
**Decision:** [D-050](../DECISION_LOG.md)
**Scope note:** Result-table growth streaming, per-bucket metric streaming, and
per-request SSE channels remain out of scope per `02_broad_plan.md` §9.

## What was implemented

### Database (migration `0018_run_events.sql`, additive)

- Append-only `run_events` mirroring `job_events`: per-run monotonic sequence,
  `run_id`/`project_id` references, event-type format check, 64 KiB payload cap,
  `(run_id, sequence)` and `(project_id, created_at DESC)` indexes, and a
  `reject_immutable_row` trigger.

### Event dual-write (queue crate + call sites)

- `probe_forge_queue::append_run_event` (per-run `MAX(sequence)+1` under the run
  row lock) and `append_run_and_job_event` (run + job stream in one transaction).
- Dual-written at every run transition site: `promote_due_schedules`
  (`run.scheduled`), `cancel_schedule` (`run.cancelled`), `claim_due_run` /
  `claim_due_run_fair` (`run.started`), `complete_run` (`run.completed`),
  `fail_run` / `recover_expired_run_leases` (`run.failed`, retry and terminal),
  and the API's `insert_run` (`run.queued`) + `cancel_run` (`run.cancelled`).
- New emissions: `test.started` before adapter execution and
  `test.completed` (+ `finding.recorded` for confirmed/suspected findings) in
  the execution worker's result persistence — result rows, evidence, and
  events now commit in one transaction; and `report.completed` /
  `report.failed` in the API report persistence path.

### API (`services/api/src/results.rs`)

- `GET /api/runs/{runId}/events` modeled 1:1 on the job stream: membership
  gate via `load_run_for_user` (`404` for non-members), `Last-Event-ID` +
  `?after=` cursor validation (`400` on junk), 200-event batches, 1 s poll,
  15 s keep-alive, safe `stream.error` on database failure, and a drain-then-
  close gate on terminal run statuses (`completed`,
  `completed_with_report_error`, `failed`, `cancelled`, `expired`) so
  `report.*` events emitted at the terminal tail are still delivered.

### Contracts

- `runEvent` JSON Schema definition, `RunEvent`/`RunEventType` TypeScript types,
  Rust `RunEvent`/`RunEventType` + `validate_run_event`, and shared fixtures
  (`run-event.valid.json`, `run-event.report.valid.json`,
  `run-event.job-type.invalid.json`) wired into the cross-language manifest.
- Vocabulary drift fixed: `run.queued` added to the shared event vocabulary
  (the backend has always emitted it for operator-created runs; the frontend
  listener list previously could not hear it).

### Frontend (`apps/web`)

- `useJobEvents.ts` generalized into an entity-agnostic `useEventStreams` with
  `useJobEvents` and `useRunEvents` wrappers (8-stream cap, 5-error give-up,
  server-authoritative refetch pattern).
- `jobEvents.ts`: `LIVE_RUN_STATUSES`/`isLiveRun`, `RUN_EVENT_TYPES`,
  `parseRunEvent`, `runStatusForEvent`.
- `ProjectsView`: per-run streams for live runs, coalesced `requestRunsRefresh`
  guarded by the existing detail-generation check, "Live updates" indicator and
  a paused notice (`role="status"`) in the Runs panel.
- `RunReport`: subscribes while the selected run is live, refetches on status
  and report events, keeps the last authoritative snapshot on transient
  refetch failure, and shows the same live indicator plus an explicit Refresh
  affordance when the stream stops responding. Static indicator text — no
  motion, color-only status, or fabricated progress.

### Tests

- `services/api/tests/runs_lifecycle.rs`: replay including `Last-Event-ID` and
  `?after=`, dual-write parity assertion on `job_events`, cursor validation,
  non-member denial, and drain-then-terminal follow behavior with a
  `report.completed` event after the terminal status.
- `apps/web/src/App.test.tsx`: list-level live updates (status change via
  `run.completed`, stream closes at terminal), detail-level report event →
  coalesced refetch → completed state + closed stream, and the five-failure
  paused notice with Refresh.

## Checks run (actual results)

| Check | Command | Result |
|---|---|---|
| Queue + contracts unit/integration tests | `cargo test -p probe-forge-queue -p probeforge-contracts` | pass — 8 DB-backed queue tests against local Postgres with migration 0018 applied |
| Contracts cross-language fixtures | same run | pass — Rust fixture manifest includes the new `runEvent` cases |
| **Full workspace tests (all crates/services)** | `cargo test --workspace` with `DATABASE_URL` | **pass — exit 0, 40 suites ok, 0 failed** (includes the new run SSE integration tests and the `admin_settings` suite) |
| **Workspace clippy with warnings denied** | `cargo clippy --workspace --all-targets -- -D warnings` | **pass (exit 0)** |
| Formatting | `cargo fmt --all -- --check` | pass (exit 0) |
| API compile | `cargo check -p probe-forge-api` | pass |
| Run SSE integration tests | `cargo test -p probe-forge-api --test runs_lifecycle` | pass — 6/6 (4 pre-existing + 2 new) |
| Web/TS typecheck (all workspaces) | `bun run typecheck` | pass (exit 0) |
| TS lint (whole repo) | `bun biome check .` | pass — 169 files, no issues |
| All TS tests | `bun run test:ts` | pass — contracts 59, web 22 (19 pre-existing + 3 new SSE tests), analysis-worker 140, test-catalog 6 |
| Repository contract check | `bun scripts/check-docs.mjs` | PASS — 115 required files, links, phases, env hygiene, secret scan |

## Known limitations

- Verification was split by environment: an initial `admin_settings` failure
  was diagnosed as `PoolTimedOut` because the local Postgres container had
  stopped mid-session (Docker daemon down), not a code failure; after
  restarting the stack (`bun run db:up` equivalent), the full
  `cargo test --workspace` ran with exit 0.
- A concurrent session in this repository was using the same tree while some
  of these checks ran (static-scanner work); final workspace-level checks
  were re-run after that tree compiled and all passed.
- A completed run's report is generated manually from the UI (the generation
  response already updates the view), so `report.*` events mostly matter for
  API/scheduler-driven generation; the stream contract supports both today.
- The events are a refresh prompt, not a push of data: the UI never renders
  status or report state from event payloads alone.

## Phase gate

- [x] Scope matches the agreed "status + report-ready" selection; no result
      table or metric bucket streaming was added.
- [x] Security parity with the job stream: membership gate, cursor validation,
      bounded batches, safe failure events, no secret-bearing payloads.
- [x] Dual-write keeps the jobs stream behavior unchanged (regression covered
      by pre-existing queue tests plus a new parity assertion).
- [x] All documented checks were actually run and observed passing: workspace
      cargo tests, workspace clippy with `-D warnings`, fmt check, all TS
      suites, biome, typecheck, and the repository contract check.

**PHASE 17: PASS**
