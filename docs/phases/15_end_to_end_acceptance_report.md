# Phase 15 report — End-to-end acceptance using local fixtures

**Status:** PHASE 15: PASS

**Date:** 2026-10-24

**Scope:** Wired and accepted both end-to-end workflows against inert fixtures
and a local mock target. Added run materialization and cancellation from an
approved plan, an operator-invoked execution dispatch (vulnerability adapters and
the constrained load runner), and an operator-invoked analysis job loop with the
deterministic fallback planner. No real security scan or load test was executed.

## What changed

### Run lifecycle (`services/api/src/runs.rs`)

- `POST /api/plans/{plan_id}/runs` creates one queued run per approved planned
  test (vulnerability) or load scenario, from an approved plan and an active,
  in-project target authorization. Idempotent per `(approval, selection)`; each
  new run appends `run.queued` to the job stream and the audit log.
- `POST /api/runs/{run_id}/cancel` cancels pre-execution states and refuses a
  leased running run. Owner/admin/collaborator may act; viewers are forbidden.

### Execution dispatch (`services/execution-worker`)

- `dispatch.rs` claims a due run (`claim_due_run_fair`), loads the run's
  authorization scope/limits and target environment, and executes:
  - vulnerability via the vetted adapters, mapping the normalized adapter result
    to a `test_results` row (outcome, finding status, severity, confidence) plus
    redacted `evidence` rows;
  - load via `build_runner_spec` + `run_load` + `persist_metrics`.
  - Completion/failure uses the fenced queue transitions; transient target
    failures retry with backoff, scope violations fail closed.
- `src/bin/dispatch.rs` is the operator-run loop (`--once` for drain-and-exit).

### Analysis job loop (`services/analysis-worker`)

- `job-loop.ts` claims a queued job, ingests the pinned repository read-only,
  runs deterministic discovery, and generates a draft plan. With no configured
  model it uses the deterministic fallback planner, then moves the job to
  `awaiting_approval`; retryable outcomes return the job to the queue.
- `src/job-cli.ts` is the operator entry point (`bun run plan`).

### Acceptance tests

- `services/api/tests/end_to_end.rs`: vulnerability workflow (approved plan → run
  → adapter → result/evidence → report) and load workflow (approved scenario →
  run → runner → metrics) against a local mock HTTP target, plus out-of-scope
  rejection with no request sent.
- `services/api/tests/runs_lifecycle.rs`: run creation idempotency, role and
  authorization checks, and cancellation.
- `services/analysis-worker/src/job-loop.integration.test.ts`: queued job →
  deterministic plan → `awaiting_approval`, then idle.

### Documentation

- New `docs/RELEASE_READINESS.md`; decisions D-039–D-041; checklist and index
  updates. `scripts/db-check.mjs` now runs the end-to-end and analysis suites.

## Exit gate evidence

- **All automated checks pass.** See the verification table.
- **Both end-to-end workflows work with safe local fixtures.** The vulnerability
  and load `end_to_end` tests drive the full chain against an inert fixture and a
  loopback mock; the analysis integration test drives job → plan.
- **No real security scan or load test was executed.** Every target is
  `127.0.0.1` mock code started inside the test; repository fixtures are inert.
  The only network is loopback, and out-of-scope runs assert zero requests.
- **Documentation describes what is and is not supported.**
  `docs/RELEASE_READINESS.md` lists supported/unsupported behavior, the runbook,
  and the security invariants.
- **No duplicate dangerous execution.** Run creation is idempotent; analysis and
  run claims use `FOR UPDATE SKIP LOCKED` with leases.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL`.

| Command | Result |
|---|---|
| `DATABASE_URL=... cargo test -p probe-forge-api --test end_to_end` | PASS — 3 tests (vulnerability, load, out-of-scope). |
| `DATABASE_URL=... cargo test -p probe-forge-api --test runs_lifecycle` | PASS — 3 tests. |
| `PROBE_FORGE_TEST_DATABASE_URL=... bun test src/job-loop.integration.test.ts` | PASS — 1 test. |
| `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check` | PASS — docs 97/97 required files, 18 linked docs, 16 phases; fmt; clippy `-D warnings`; workspace unit tests; Biome; typecheck; contracts 55, test-catalog 6, analysis-worker 113 pass / 10 skip, web 10. |
| `DATABASE_URL=... bun run check:db` | PASS — auth 5, jobs 2, migrations 1, operations 6, plans 6, results 4, runs_lifecycle 3, end_to_end 3, targets 5, queue 8, analysis job loop 1. |
| `bun run check:services` | PASS — 5/5 healthy after rebuild; ports released afterward. |
| `DATABASE_URL=... bun run db:backup:check` | PASS — backup/restore roundtrip. |
| `git diff --check` | PASS — no whitespace errors. |

Failures observed and resolved during verification:

- A clippy `let_unit_value` in the mock handler was simplified.
- A load metric assertion compared `SUM(bigint)` (numeric) to `i64`; cast to
  `::bigint`.

## Known limitations

- Dispatch and analysis are operator-invoked binaries, not supervised daemons;
  there is no scheduler-driven queue drain. This is deliberate so no work runs
  without an operator, but an external supervisor is required for continuous
  operation.
- The recommendation generator remains deterministic; no model-backed
  recommendations are wired.
- Retention deletes but does not archive immutable rows.
- A cancelled/failed load dispatch retries only for transient target errors.

## Next permitted phase

None. All phases 0–15 are complete. A separate, explicitly approved staging
smoke test may now be performed per `docs/RELEASE_READINESS.md`.
