# Test catalog and plan approval

Phase 8 adds the versioned test catalog, authenticated plan read/update APIs,
optimistic revision checks, and an immutable approval snapshot. The UI presents
the model's recommendations, rationale, prerequisites, risks, configuration, and
tool availability before a user records an approval.

Approval is a review decision, not target authorization or execution. No run is
started by these endpoints. The job status is not advanced by plan approval;
Phase 9/run creation will define that transition.

## Versioned test catalog

`packages/test-catalog/catalog.json` is the server-owned catalog. Schema version
`1`, catalog version `0.3.0`, and stable lowercase IDs are embedded by both the
TypeScript worker and the Rust API.

Each entry contains:

- stable ID, name, category, and risk;
- description;
- prerequisites, tool requirements, and target requirements;
- timeout in seconds;
- evidence expectations;
- whether explicit confirmation is required.

The parser rejects unknown fields, executable fields, control characters,
duplicate IDs, unsupported categories/risks, and values outside bounded lengths
or counts. Rust uses a separate `deny_unknown_fields` parser over the same file
and validates the same bounds.

### Adapter availability

Phase 10 owns execution adapters. The API reports `available: true` only for
catalog IDs with a vetted adapter in the shared `execution-adapters` registry.
Initially that is `http.headers.missing` and `http.cors.misconfiguration`
(`http_probe` 0.1.0). Every other entry reports `available: false` with an
explicit note. Entry availability is computed from the registry, not from model
output, repository text, or a shared tool-requirement string. Users may still
select catalog-supported tests and record an approval; approval does not start a
run, and run creation/dispatch remains deferred.

### Load scenario review and versioning

Load plans are reviewed in the same plan-review surface. Each scenario card shows
its ordered steps, dependencies, secret-backed header names (never values),
extraction variables, assertions, body-template presence, bounded profile, and
closed stop conditions. Selection remains the only editable field.

A scenario is versioned by the parent `test_plans.version`. Approval revalidates
each selected scenario against the current contract, requires the stored row name
to match, and writes the exact scenario JSON and profile into the immutable
approval snapshot. Editing a scenario therefore requires a new plan version and a
new approval; there is no independent scenario-edit endpoint. Approval never
sends load traffic. See [LOAD_SCENARIOS.md](LOAD_SCENARIOS.md).

## API surface

All routes are authenticated and return `no-store` responses.

| Method | Path | Permission | Purpose |
|---|---|---|---|
| `GET` | `/api/test-catalog` | Any authenticated user | Versioned catalog with availability |
| `GET` | `/api/jobs/{jobId}/plan` | Any project member | Current plan for a job |
| `GET` | `/api/plans/{planId}` | Any project member | Plan detail by ID |
| `PATCH` | `/api/plans/{planId}` | Owner, admin, collaborator | Update selected test/scenario IDs |
| `POST` | `/api/plans/{planId}/approve` | Owner, admin, collaborator | Record an immutable approval |

`GET` responses never cross a project boundary. An outsider receives a not-found
response; a viewer may read but cannot edit or approve.

### PATCH selection

```json
{
  "expectedRevision": 2,
  "selectedTestIds": ["<planned-test-uuid>"],
  "selectedLoadScenarioIds": []
}
```

Only `planned_tests.selected` and `load_scenarios.selected` change. Names,
rationale, risk, configuration, catalog IDs, and executable strings are not
editable. Unknown fields, duplicate IDs, IDs from another plan, and IDs that do
not match the plan goal are rejected.

If `expectedRevision` is stale, the API returns `409 plan_conflict` without
changing anything. The UI preserves the conflict state and offers a reload.

### Approval

```json
{
  "expectedRevision": 3,
  "confirmed": true
}
```

Approval requires at least one selected test or scenario and explicit
confirmation. It locks the plan and job, rechecks the revision and current-plan
pointer, and writes:

- `plan_approvals.approved_test_ids`;
- `plan_approvals.approved_load_scenario_ids`;
- `plan_approvals.plan_revision`;
- a bounded `approval_snapshot` containing the exact selected tests/scenarios,
  risks, configuration, rationale, catalog metadata, catalog version, approver,
  and timestamp;
- an immutable `plan.approved` audit event.

The plan becomes `approved`; older proposed plans for the same job become
`superseded`. The snapshot cannot be updated or deleted because of the
`plan_approvals_immutable` trigger.

`test_runs.plan_approval_id` is `NOT NULL`, and migration `0009` adds a composite
foreign key from `(plan_approval_id, plan_id, plan_version)` to the matching
approval. A run cannot cite an approval for another plan or version.

Run creation revalidates each selected test's adapter configuration before any
run is inserted. A plan whose HTTP test lacks a safe `route` or `path` returns
`422 invalid_adapter_configuration` and queues nothing; regenerate the plan
instead of editing the approval snapshot.

## Optimistic concurrency

`test_plans.revision` starts at zero and increments on each selection change.
Both PATCH and approval require the revision the user reviewed. Concurrent
requests serialize on the plan row; exactly one succeeds and the other receives
`409 plan_conflict`. A newer plan for the same job also produces
`409 plan_superseded`.

The UI never treats an optimistic local selection as saved or approved. It
updates its state only from the server response.

## UI behavior

The project workspace lists analysis jobs and opens a focused plan review when a
job has a current plan.

The review shows:

- plan summary, confidence, catalog version, provider/model, required inputs, and
  limitations;
- each planned test with rationale, risk label, prerequisites, target
  requirements, expected evidence, configuration, catalog metadata, unavailable
  adapter/tool warnings, and a missing-route warning for available HTTP tests;
- load scenarios with method/path, expected statuses, sizing, ramp, duration,
  request caps, and credential reference;
- a review sidebar with selected counts, save, explicit confirmation, and approve
  actions;
- approval details and immutable snapshot identifiers after approval.

Viewers see a read-only review. Owners, admins, and collaborators may save a
selection and approve it. Selection controls are disabled after approval.

The UI shows real loading, empty, error, conflict, and success states. It does not
invent progress percentages or claim that an approval started a run.

## Database changes

- `migrations/0008_plan_review.sql` adds plan revisions, approval revision
  evidence, and rationale references.
- `migrations/0009_approval_run_match.sql` adds the approval/plan/version
  composite constraint for runs.

## Verification

See `docs/phases/08_plan_approval_report.md` for the Phase 8 gate evidence and
commands. Tests use inert plans and fixtures; no target, scanner, or load test is
contacted.
