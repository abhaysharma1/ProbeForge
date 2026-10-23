# Phase 7 report — Pi agent integration and plan generation

**Status:** PHASE 7: PASS

**Date:** 2026-10-23

**Scope:** Added a bounded Pi planning library to the TypeScript analysis worker,
a constrained read-only/proposal tool registry, strict server-side plan
validation, deterministic fallback generation, and a narrow PostgreSQL plan
writer. No proposed test, target request, load test, repository script, or
external scan was executed. The worker HTTP surface remains health-only.

## What changed

### Planning modules

- `services/analysis-worker/src/agent/limits.ts` — server-owned defaults and
  fail-closed validation for model calls, tool calls, turns, retries, response
  tokens, wall time, prompt bytes, tool output, plan size, repository reads,
  catalog entries, planned tests, and load scenarios.
- `services/analysis-worker/src/agent/model.ts` — builds a Pi runtime from the
  pinned SDK model catalog, clamps response tokens, uses `temperature: 0`, and
  resolves provider credentials inside Pi.
- `services/analysis-worker/src/agent/planning-input.ts` — trusted input
  validation, commit/snapshot/inventory consistency, catalog validation, a
  summary-only prompt, and the planning system prompt.
- `services/analysis-worker/src/agent/tools.ts` — the only model-facing actions:
  bounded repository metadata/read, discovery summary/routes/surfaces/
  unsupported, auth/schema/middleware inspection, catalog listing/get,
  `plan_validate`, and `plan_propose`.
- `services/analysis-worker/src/agent/validate.ts` — strict v1 contract
  validation plus catalog allowlisting, name/risk/category checks, confirmation
  policy, duplicate detection, reference checks, credential-material rejection,
  size limits, and non-removable server `requiredInputs`.
- `services/analysis-worker/src/agent/fallback.ts` — deterministic catalog-only
  vulnerability fallback and literal-route load fallback, both low confidence.
- `services/analysis-worker/src/agent/store.ts` — `PlanStore` contract,
  `InMemoryPlanStore`, session state rules, and bounded metadata handling.
- `services/analysis-worker/src/agent/postgres-store.ts` — transactional
  session start/finish, plan/test persistence, job-row-locked version
  allocation, `current_plan_id` update, and audit writes.
- `services/analysis-worker/src/agent/generate.ts` — attempt orchestration,
  retries, wall-time abort, budget enforcement, proposal capture, fallback,
  cancellation, and recoverable results.
- `services/analysis-worker/src/agent/index.ts` — public exports.

### Configuration and dependencies

- Added `PROBE_FORGE_PLANNING_PROVIDER`, `PROBE_FORGE_PLANNING_MODEL`, and
  bounded planning-limit environment variables to worker configuration and
  `.env.example` placeholders. Provider/model must be set together.
- Added `@earendil-works/pi-ai@1.4.0` and
  `@probe-forge/test-catalog@workspace:*` as analysis-worker dependencies.
  `@earendil-works/pi-agent-core` remains pinned at `1.4.0`.

### Tests

- `validate.test.ts` — valid plans, strict schema rejection, catalog
  allowlisting, risk/name/category/confirmation policy, duplicates, references,
  credential material, executable fields, unsafe load paths, size limits, and
  bounded inputs.
- `fallback.test.ts` — catalog-only vulnerability fallback, literal-route load
  fallback, no-safe-fallback behavior.
- `tools.test.ts` — exact tool registry, file metadata/content separation,
  traversal rejection, invalid proposal issues, plan validation without
  submission, unknown/forbidden/oversized arguments, tool/output budgets, catalog
  lookup failure.
- `generate.test.ts` — Pi faux-provider integration for a valid plan, repeated
  plan versioning, invalid output fallback, unconfigured model fallback, model
  outage fallback, hung-model wall-time abort and fallback, unknown-tool
  rejection, no-fallback retryable failure, cancellation, and load fallback.
- `planning-input.test.ts` — summary-only prompt, catalog truncation, prompt
  limit, commit/provenance/catalog validation, nil UUID rejection.
- `limits.test.ts` / `config.test.ts` — budget validation and planning
  configuration.
- `postgres-store.integration.test.ts` — transactional persistence, planned-test
  rows, job pointer, audit event, concurrent version allocation, terminal
  session rejection, and terminal-job rejection. It runs only when
  `PROBE_FORGE_TEST_DATABASE_URL` is set.

### Documentation

- Added `docs/PLANNING.md`.
- Updated `docs/README.md`, `docs/IMPLEMENTATION_CHECKLIST.md`,
  `docs/DECISION_LOG.md` (D-028), `README.md`, `.env.example`, and
  `scripts/check-docs.mjs`.
- No real `.env`, provider credential, token, private key, target detail, or
  scan evidence was added.

## Exit gate evidence

- **Valid plan from a model fixture:** `generate.test.ts` drives the pinned Pi
  faux provider through `plan_propose`; the result is persisted by
  `InMemoryPlanStore` with version 1 and `status = 'completed'`. The database
  integration test independently persists a valid v1 plan into PostgreSQL.
- **Invalid or malicious arguments rejected:** tests cover unknown tools,
  traversal paths, nested executable keys, oversized arguments, extra tool
  arguments, invented catalog IDs, duplicate tests, mismatched names, risk
  downgrades, missing confirmation, unknown references, and credential material.
- **Model outages leave the job recoverable:** a faux `error` response produces
  a `fallback` result with a persisted deterministic draft; a hung provider is
  aborted at the wall-time limit and also falls back; when no safe fallback
  exists the result is `retryable_failure`, not a job failure. No job row is
  marked failed by the planning library.
- **Prompt injection cannot grant tools or alter policy:** the registry is
  fixed and contains no shell, filesystem write, network, database, secret, or
  execution tool. Raw repository text is excluded from the prompt and appears
  only in bounded tool results marked as untrusted. A faux model that requests
  `shell_exec` is rejected without executing anything (`stats.toolCalls === 0`).
  Server validation, catalog allowlisting, budgets, and persistence are outside
  the model's control.
- **No proposed tests executed:** the planner has no execution adapter or target
  client; tests assert only plan generation and persistence. No scan or load
  test was run.

## Verification

Commands were run from `D:\Projects\ProbeForge\services\analysis-worker` unless
noted.

| Command | Result |
|---|---|
| `bun run typecheck` (analysis worker) | PASS |
| `bun test src/agent` | PASS — 43 passed, 6 skipped (database tests require an explicit URL) |
| `PROBE_FORGE_TEST_DATABASE_URL=... bun test src/agent` | PASS — 47 tests / 187 assertions (all agent and database integration tests) |
| `PROBE_FORGE_TEST_DATABASE_URL=... bun test src/agent/postgres-store.integration.test.ts` | PASS — 4 tests / 20 assertions |
| `bunx biome check --write` on changed TypeScript | PASS — no diagnostics |
| `bun run check:docs` (root) | PASS — 64/64 required files, links, 16 phase headings, `.env` hygiene, secret scan |
| `bun run check` (root) | PASS — docs, Rust fmt/clippy/tests, Biome, all TypeScript typechecks and tests. Analysis worker: 111 passed / 6 skipped / 577 assertions; contracts: 46 passed; test-catalog: 4 passed; web: 5 passed. |
| `DATABASE_URL=... bun run check:services` (root) | PASS — 5/5 local services healthy; ports 8080, 8081, 8082, 8090, and 5173 were closed after the check |

## Known limitations

- Planning is a library, not an authenticated job dispatcher or HTTP endpoint.
  Phase 8 must add the plan read/update/approval API and the worker claim path.
- `packages/test-catalog/catalog.json` still has no entries. A
  vulnerability-testing fallback is therefore unavailable until Phase 8 adds
  vetted entries.
- Load fallback only uses sorted literal `GET`/`HEAD` routes and a conservative
  baseline profile. It is not a capacity assessment.
- Persistence writes `test_plans` and `planned_tests`; load scenarios remain in
  the plan payload because the execution/scenario tables belong to later phases.
- Plan version allocation is serialized by a job-row lock. This is correct for
  the current single-database design but is not a distributed lock.
- If a persistence transaction commits but its response is lost, a caller retry
  creates a new plan version instead of returning the existing draft. Callers
  should reconcile by job/session/version records; versioning prevents overwrite
  but does not provide exactly-once delivery.
- The model/tool path is tested with the faux provider, not a live provider.
  Provider-specific capability differences (for example whether a model supports
  tool calls or accepts `temperature`) remain an operational validation item.
- The database integration test uses the operator's local disposable database
  and temporarily disables the audit immutability trigger only to remove its own
  fixture rows; the trigger is re-enabled in a `finally` block.
- Canceled model sessions are marked `cancelled`, but the future dispatcher must
  also observe job cancellation and signal the planner. Phase 7 has no dispatcher.
- The analysis-worker HTTP surface is still health-only. No frontend work was
  performed.

## Next permitted phase

**Phase 8 — Test catalog and plan approval UI.** Not started. It requires
explicit authorization after this report is reviewed.
