# Phase 11 report — Load scenario generation and validation

**Status:** PHASE 11: PASS

**Date:** 2026-10-24

**Scope:** Expanded the v1 load contract with ordered steps, stable step IDs,
bounded dependencies, path/header/body correlation templates, secret-backed
headers, extraction rules, assertions, and closed profile/stop-condition limits.
Added deterministic Rust/TypeScript validation, scope validation without network
access, Pi prompt/fallback guidance, approval revalidation and immutable snapshot
coverage, plan-review details, and a defense-in-depth database migration. No load
generator was started and no target was contacted.

## What changed

### Declarative load contract

- `crates/contracts/src/model.rs`, `validation.rs`;
  `packages/contracts/src/model.ts`, `validate.ts`;
  `packages/contracts/schemas/probeforge.v1.schema.json`:
  - optional step `id`;
  - `dependsOn` restricted to earlier step IDs, unique, no self/forward cycles;
  - `secretHeaders` mapping header names to `secret://` references only;
  - header/JSON-body `extract` rules with unique correlation variable names;
  - header/JSON-body `equals`, `contains`, and `exists` assertions;
  - `${name}` templates only from earlier extraction variables in paths, literal
    headers, and body strings;
  - required `expectedStatuses`;
  - GET/HEAD/OPTIONS bodies rejected;
  - per-step body cap of 64 KiB;
  - closed stop conditions (`errorRate`, `p95LatencyMs`, `maxFailedRequests`);
  - platform maxima: 100 VUs or 100 arrivals/s, 300 s ramp, 900 s duration,
    100,000 requests, 100 requests/s;
  - typed stop-condition deserialization rejects non-object values even when all
    fields are optional.
- Shared fixtures gained a valid correlated scenario plus invalid dependency,
  template-expression, loop, raw-secret-header, unknown-stop-condition, and
  platform-maxima cases. Rust and TypeScript run the same manifest.

### Generation and worker policy

- `services/analysis-worker/src/agent/planning-input.ts` now tells Pi to use
  discovered routes, stable step IDs, earlier-step correlation, secret references,
  bounded assertions, explicit statuses, and capped profiles. It explicitly
  forbids raw credentials, loops, repeat fields, executable fields, and arbitrary
  template expressions.
- The deterministic load fallback adds a stable `baseline` step ID and keeps its
  conservative profile.
- Worker tests cover valid correlated scenarios and rejection of unknown
  dependencies/templates, raw secret headers, and loop fields.

### Target scope and approval

- `crates/target-policy` adds `validate_load_scenario_scope`: checks every step
  against an approved scheme/host/port/path prefix without DNS or traffic,
  expands template placeholders only for the check, rejects GET/HEAD/OPTIONS
  bodies, and reports per-step issues. Phase 12 must call it at run creation.
- `services/api/src/plans.rs` revalidates each selected scenario against the
  current contract and requires the stored row name to match before approval.
  The immutable approval snapshot already contains the exact scenario JSON,
  profile, plan version, and revision; tests assert the new dependency and stop
  condition fields survive.
- Migration `0011_load_safety.sql` adds `NOT VALID` platform/stop-condition
  constraints to `load_profiles`. Historical rows are not rewritten, but new and
  updated rows are checked.

### Review UI

- `apps/web/src/PlanReview.tsx` now shows each step ID, method/path, expected
  statuses, dependencies, secret-backed header names, extraction and assertion
  summaries, body-template presence, state-changing method marker, profile
  maxima, stop conditions, and the plan version that approval will pin.
- Selection remains the only editable field; there is no scenario editor and no
  run control.

### Documentation

- New `docs/LOAD_SCENARIOS.md`.
- Decision D-032.
- Updated contracts, database, planning, plan-approval, documentation index,
  README, checklist, and contract-check inventory.

## Exit gate evidence

- **A scenario can be generated and validated without sending traffic.**
  - Pi can propose the expanded scenario through `plan_propose`; the fallback can
    build a conservative load draft; both use server-side validation.
  - `validate_load_scenario_scope` and all contract tests are pure/no-network.
  - No runner, scheduler dispatch, or live load endpoint exists.
- **Invalid paths, unbounded loops, arbitrary code, and excessive profiles are
  rejected.**
  - Shared fixtures reject traversal/separator paths, backslashes, malformed
    templates, forward/unknown dependencies, loop fields, raw credential headers,
    unknown stop conditions, and platform maxima violations.
  - Rust unit tests reject oversized bodies, script-expression templates, loops,
    raw secret references, missing statuses, and excessive profiles.
  - TypeScript unit tests cover the same boundaries through the shared validator.
- **User approval pins scenario version and load profile.**
  - Scenario versioning is the parent plan version. Approval revalidates selected
    scenarios, writes the exact JSON/profile into `plan_approvals.approval_snapshot`,
    and cannot be updated or deleted.
  - API tests verify the approved scenario dependency and stop condition appear in
    the immutable snapshot.
- **No load test is run against a live deployment.**
  - No k6/load generator, schedule execution, or traffic path was added.
  - All tests use inert data and local database schemas only.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL` for database and service checks.

| Command | Result |
|---|---|
| `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check` | PASS — docs 80/80 required files, 14 linked documents, 16 phase headings; Rust fmt/clippy/tests; Biome; TypeScript typecheck/tests. Contracts: 55 passed; analysis worker: 113 passed / 7 skipped / 586 assertions; test-catalog: 6 passed; web: 9 passed. |
| `DATABASE_URL=... CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check:db` | PASS — auth/projects (5), jobs (2), migrations (1), plan approval (6), targets (5), queue (5). |
| `bun run db:migrate` | PASS — migration `0011_load_safety` applied after `0010`; constraints present with `convalidated = false` (intentional `NOT VALID`). |
| `bun run check:services` | PASS — 5/5 healthy; ports 8080, 8081, 8082, 8090, and 5173 closed afterward. |
| `git diff --check` | PASS — no whitespace errors. |

Failures observed and resolved during verification:

- The first `check:db` run counted matching constraint names in both the isolated
  test schema and `public`, producing 14 instead of 7. The query now filters on
  `conrelid = 'load_profiles'::regclass`; the rerun passed.
- The first `check:services` attempt found stale local analysis-worker/Vite
  processes holding ports 8090/5173. They were stopped and the rerun passed.
- `bun run check` initially failed on formatter output and literal `${...}`
  lint warnings in strings. Formatting was applied and test literals were built
  from constants so the placeholder text no longer trips the lint rule.
- A later parallel Rust rebuild hit a Windows paging-file/linker out-of-memory
  failure (`LNK1102`), not a code failure. Rerunning with
  `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0` passed the full Rust and workspace
  checks. The database check was rerun with the same limit and passed.

## Known limitations

- No k6/load runner, scheduler execution, worker lease, concurrency quota,
  metrics bucket, cancellation path, or result persistence exists yet.
- Target-scope validation is a library function. Phase 12 must call it with the
  chosen target authorization before any traffic and must percent-encode extracted
  values before substitution so a value cannot add path segments.
- Secret references are validated but not resolved; no secret manager or runtime
  header injection exists yet.
- Migration `0011` constraints are `NOT VALID`, so historical rows that exceed
  the new maxima are not rewritten. They are enforced for new/updated rows. An
  operator should review existing rows before validating the constraints.
- State-changing methods are representable and marked in the UI; the v1 target
  scope has no method allowlist. Phase 12 must combine profile limits, target
  authorization, explicit approval, and runtime policy before executing them.
- The review UI supports selection-only approval, not scenario editing or
  independent scenario revisions. A changed scenario requires a new plan version.
- No load traffic, real target contact, or live deployment test was performed.

## Next permitted phase

**Phase 12 — Load runner and scheduler.** Not started. It requires explicit
authorization after this report is reviewed.
