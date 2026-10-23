# Phase 8 report — Test catalog and plan approval UI

**Status:** PHASE 8: PASS

**Date:** 2026-10-23

**Scope:** Added a complete versioned test catalog, authenticated plan read/select/
approve APIs, optimistic revision conflict handling, immutable approval
snapshots, and a plan-review UI. No target was contacted, no execution adapter
was invoked, and no run was created.

## What changed

### Test catalog

- Replaced the empty catalog scaffold with schema `1` / catalog version `0.2.0`
  and seven inert entries covering HTTP headers, CORS, auth rate limiting, TLS
  baseline, static secret patterns, dependency advisories, and a load baseline.
- Each entry now carries a description, prerequisites, tool requirements, target
  requirements, timeout, evidence expectations, and explicit-confirmation policy.
- The TypeScript parser enforces exact fields, stable IDs, bounded arrays and
  text, no control characters, no duplicate IDs, and no executable/unknown fields.
- The Rust API embeds and parses the same JSON with `deny_unknown_fields` and
  equivalent bounds. Unit tests validate version, IDs, and every entry.
- Adapter availability is computed from a server-side allowlist. The allowlist is
  empty until Phase 10, so every entry is honestly reported as unavailable.

### Database

- `migrations/0008_plan_review.sql` adds `test_plans.revision`,
  `plan_approvals.plan_revision`, and `planned_tests.rationale_references`.
- `migrations/0009_approval_run_match.sql` adds a composite foreign key so a run
  can only cite the approval for its exact plan version.
- The existing `test_runs.plan_approval_id NOT NULL` and immutable
  `plan_approvals` trigger remain in force.

### API

- Added `services/api/src/catalog.rs` for embedded catalog parsing, validation,
  availability, and DTOs.
- Added `services/api/src/plans.rs` with:
  - `GET /api/test-catalog`
  - `GET /api/jobs/{jobId}/plan`
  - `GET /api/plans/{planId}`
  - `PATCH /api/plans/{planId}`
  - `POST /api/plans/{planId}/approve`
- Reads are project-scoped; outsiders receive not-found and viewers are read-only.
- PATCH changes only `planned_tests.selected` and `load_scenarios.selected`.
  Names, rationale, risk, configuration, and executable strings are not editable.
- Both PATCH and approval require `expectedRevision`; conflicts return
  `409 plan_conflict`, and non-current plans return `409 plan_superseded`.
- Approval requires `confirmed: true`, at least one selected item, a locked
  plan/job, a current plan pointer, and a matching goal selection. It snapshots
  the exact selected rows, catalog metadata, catalog version, approver, and time.
- On approval, the plan becomes `approved`, older proposed plans are superseded,
  and `plan.approved` is appended to the immutable audit log.
- Plan generation now persists rationale references and load profiles/scenarios
  so the review UI and approval snapshot use stable IDs.

### Frontend

- Extended `apps/web/src/api.ts` with job/plan/catalog types and review APIs.
- Added `apps/web/src/PlanReview.tsx`:
  - plan summary, confidence, catalog version, provider/model, required inputs,
    limitations, and evidence references;
  - risk-labelled tests with rationale, prerequisites, target requirements,
    expected evidence, configuration, tool requirements, and unavailable-adapter
    warnings;
  - load scenarios with requests, expected statuses, sizing, ramp, duration,
    request caps, and credential references;
  - selection checkboxes, explicit save, revision-aware conflicts, explicit
    approval confirmation, and immutable approval details.
- `ProjectsView` now lists analysis jobs and opens a focused plan-review view.
  Viewers can review but cannot change or approve. The UI uses only server
  responses and never treats local state as saved or approved.
- The existing warm-neutral/terracotta design tokens, keyboard focus, semantic
  controls, responsive layouts, and reduced-motion behavior are preserved.

### Tests

- `services/api/tests/plans_approval.rs`: tenant isolation, viewer denial,
  catalog metadata, selection update, unknown IDs/fields, stale revisions,
  concurrent PATCH conflict, approval confirmation, immutable snapshot, audit,
  load-scenario approval, and the no-run-without-approval constraint.
- `services/api/tests/migrations.rs`: revision columns and approval/run
  composite constraint.
- `services/api/src/catalog.rs` unit tests.
- `apps/web/src/App.test.tsx`: plan review, selection save, approval, conflict
  reload, and viewer read-only behavior.
- `packages/test-catalog/src/index.test.ts`: complete schema and rejection paths.

### Documentation

- Added `docs/PLAN_APPROVAL.md`.
- Updated `docs/README.md`, `docs/DECISION_LOG.md` (D-029),
  `docs/IMPLEMENTATION_CHECKLIST.md`, `docs/CONTRACTS.md`, `docs/DATABASE.md`,
  `README.md`, and `scripts/check-docs.mjs`.
- No real `.env`, secret, target detail, or scan evidence was added.

## Exit gate evidence

- **Review and select from a generated plan:** API integration tests read a
  seeded but contract-valid proposed plan, and the UI test opens the plan,
  reviews catalog metadata/risk/configuration, selects a test, saves, and
  observes the server revision. Viewers see the plan read-only.
- **Immutable versioned snapshot:** approval writes `plan_approvals` with the
  plan ID, version, reviewed revision, selected test/scenario IDs, and a bounded
  snapshot containing exact selections and catalog version. The immutable
  trigger rejects updates/deletes. A second approval is rejected.
- **Concurrent edits detected safely:** concurrent PATCH requests with the same
  `expectedRevision` serialize on the plan row; one succeeds and the other
  receives `409 plan_conflict` without overwriting the first. The UI shows the
  conflict and a reload action.
- **No run without approval:** `test_runs.plan_approval_id` is `NOT NULL`;
  migration `0009` additionally requires the cited approval to match the run's
  plan/version. The migration/API tests verify the constraint.
- **No arbitrary shell strings:** PATCH accepts only selection UUIDs and rejects
  unknown fields; plan names/rationale/configuration/executable fields remain
  immutable after generation.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL` unless noted.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | PASS — API 27 unit tests, 5 auth/project tests, 2 job/SSE tests, 1 migration test, 5 plan approval tests, and all queue/scheduler/contract tests. |
| `bun run check:docs` | PASS — 68/68 required files, 11 linked docs, 16 phase headings, `.env` hygiene, secret scan. |
| `bun run check` | PASS — docs, Rust fmt/clippy/tests, Biome, all TypeScript typechecks and tests. Analysis worker: 111 passed / 7 skipped / 579 assertions; contracts: 46 passed; test-catalog: 6 passed; web: 8 passed. |
| `bun run check:services` | PASS — 5/5 services healthy; ports 8080, 8081, 8082, 8090, and 5173 were closed after the check. |
| `bun run check:db` | PASS — migration, auth/project, jobs, plan approval, and queue tests in isolated schemas. |
| `bun run db:migrate` | PASS — migrations through `0009`. |
| Playwright against local API + Vite | PASS — logged in, opened the seeded inert plan, reviewed catalog/risk/configuration, selected a test, saved at revision 1, and rendered desktop 1440×1200 and mobile 390×844 layouts. Only the expected initial unauthenticated session-check 401s appeared in the browser console; no page errors. |

## Known limitations

- No execution adapter is available yet; every catalog entry reports
  `available: false`. Approval records intent and starts no run.
- Plan approval updates the plan status but does not advance the job status.
  Target authorization and run creation in later phases must define the next job
  transition; the UI shows the authoritative plan status separately.
- There is no job creation UI or authenticated worker claim/dispatch path. Plans
  must be generated by a worker/seed path outside the current HTTP surface.
- Approval snapshots are returned as summary counts/IDs in the ready API; the
  full immutable snapshot is stored in PostgreSQL for later audit/reporting.
- Load scenario/profile persistence is bounded by the existing plan and database
  limits; Phase 11 will define the full declarative scenario contract.
- The plan review UI is a single focused panel, not a separate routed page. It
  uses the existing project workspace shell.
- The catalog is embedded at compile time. Catalog changes require a rebuild of
  both the API and worker.
- The browser check covered the proposed and selected/saved states at desktop and
  mobile widths. The approved-state visual treatment is exercised by the
  component and API tests rather than a separate browser screenshot.
- No real target, scanner, load test, or repository execution was performed.

## Next permitted phase

**Phase 9 — Target validation and authorization.** Not started. It requires
explicit authorization after this report is reviewed.
