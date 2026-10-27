# 15a — Executable adapter configuration (post-phase fix)

**Date:** 2026-10-26
**Status:** PASS
**Decision:** [D-048](../DECISION_LOG.md)

## Problem

Every vulnerability execution in the local database failed before any network
request with:

```text
Test configuration must contain exactly one route or path value.
error_code = invalid_adapter_configuration
```

The two runs existed, carried `outcome = 'error'` results, and still appeared as
`status = 'completed'`, so the failure looked like a successful run with an
inconclusive test. All 42 stored `planned_tests` rows had `configuration = {}`.

## Root cause

`route_from_configuration` correctly requires exactly one of `route` or `path`,
but nothing upstream produced it:

- the planning prompt never mentioned `configuration`;
- `fallbackTest` omitted it, so the deterministic fallback always stored `{}`;
- worker validation only required `configuration` to be an object;
- run creation did not validate the adapter contract;
- the end-to-end tests hand-seeded `{"route": "/"}`, which masked the gap.

## Fix

| Layer | File | Change |
|---|---|---|
| Adapter | `crates/execution-adapters/src/lib.rs` | `validate_configuration(catalog_test_id, configuration)` reuses `route_from_configuration` for adapter-backed IDs and accepts skipped, unsupported IDs. |
| API | `services/api/src/runs.rs` | `POST /api/plans/{planId}/runs` validates every selected vulnerability test before inserting a run, returning `422 invalid_adapter_configuration` and queueing nothing. |
| Worker | `services/analysis-worker/src/agent/fallback.ts` | HTTP entries receive `{"route": "<first sorted literal GET/HEAD path>"}`; entries are omitted when no safe literal route exists. The load fallback reuses the same selector. |
| Worker | `services/analysis-worker/src/agent/validate.ts` | Rejects missing, ambiguous, non-string, unsafe, undiscovered, or extra-key configuration with `invalid_adapter_configuration`. A truncated route inventory skips only the discovery-match check. |
| Worker | `services/analysis-worker/src/agent/planning-input.ts` | Prompt requires `configuration.route` from `discovery_list_routes` and forbids invented routes and other keys. |
| UI | `apps/web/src/PlanReview.tsx` | Warns when an available HTTP test has no `route`/`path`. |
| Tests | adapter, worker, API | Added direct adapter-contract tests, fallback/validation tests, and a run-creation rejection test. Removed reliance on hand-seeded configuration in the run fixture. |

The rule is intentionally duplicated at the planning and run-creation layers:
planning rejects bad proposals before an approval exists, and run creation fails
closed for plans approved before this fix. The authoritative scope check at
connection time is unchanged.

## Verification

| Check | Command | Result |
|---|---|---|
| Rust format | `cargo fmt --all -- --check` | PASS |
| Adapter tests | `cargo test -p probe-forge-execution-adapters` | 8 passed |
| Run creation tests | `bun scripts/run-with-env.mjs cargo test --target-dir target/test-build -p probe-forge-api --test runs_lifecycle` | 4 passed, including `rejects_plans_whose_tests_cannot_be_configured` |
| Rust lint | `cargo clippy --target-dir target/test-build -p probe-forge-execution-adapters -p probe-forge-api --all-targets -- -D warnings` | PASS |
| Worker typecheck | `bun run --cwd services/analysis-worker typecheck` | PASS |
| Worker tests | `bun test` (analysis worker) | 138 passed, 10 skipped |
| Web typecheck | `bun run --cwd apps/web typecheck` | PASS |
| Web tests | `bun run --cwd apps/web test` | 19 passed |
| Biome | `bunx biome check <changed TS/TSX files>` | PASS |
| Docs contract | `bun scripts/check-docs.mjs` | PASS |

A separate target directory was used for Rust checks because the local API,
scheduler, and execution-worker binaries were running and locked the default
`target/debug` artifacts. The full workspace suite was not re-run for that
reason; the two directly affected crates and their integration tests were run.

The first web test invocation hit a Vitest forks-worker timeout while a Rust
check was running concurrently. Re-running alone passed 19/19, including the new
missing-route warning assertion.

## Operator recovery

1. Rebuild and restart the local services; the running processes still execute
   the pre-fix binaries.
2. Existing approved plans with `{}` cannot create runs: the API returns
   `422 invalid_adapter_configuration` before any queue row or network call.
   Start a fresh analysis job (or otherwise regenerate the plan) so the plan
   carries a discovered route, then review and approve it.
3. No database rows were modified during this fix.

## Known limitations (unchanged)

- An errored test still belongs to a `completed` run; run-level status semantics
  were not changed.
- Analysis jobs remain `awaiting_approval` after runs finish because no
  `job.completed` producer exists.
- The fallback can still propose catalog entries that have no adapter; those
  runs are skipped as `unsupported` at execution time.
- Adapter availability is not part of the shared catalog file, so the fallback
  cannot filter on it without a new shared mapping.
