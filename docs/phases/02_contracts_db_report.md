# Phase 2 report - Contracts, configuration, and database migrations

- **Phase:** 2 of 15
- **Date:** 2026-10-23
- **Status:** PHASE 2: PASS
- **Next permitted phase:** Phase 3 - Authentication, projects, and authorization (after explicit authorization)

## Implemented

1. **Versioned contract bundle.** `packages/contracts/schemas/probeforge.v1.schema.json` defines JSON Schema 2020-12 payloads for job creation, plans, planned tests, load profiles/scenarios, target authorization, run requests, test results, and job events. It closes every object to unknown fields and fixes the two job goals to `vulnerability_testing` and `load_testing`.
2. **TypeScript contracts and validation.** `packages/contracts/src/model.ts` mirrors the schema; `src/validate.ts` compiles the schema with Ajv 8.20.0, adds bounded UUID/URI/date-time formats and semantic checks, and is exposed only through `@probe-forge/contracts/validate` so the browser bundle does not pull in Ajv. The existing browser-safe root entry point still exports the job goals and health contracts.
3. **Rust contracts and validation.** `crates/contracts` contains serde types, a `Validate` trait, deterministic semantic checks, a 256 KiB bounded parser, JSON validators, and stable `ContractError`/`ValidationIssue` types. Errors report paths and rule descriptions, not offending values.
4. **Shared fixtures.** `packages/contracts/fixtures/v1/manifest.json` contains 38 valid/invalid cases covering goal rejection, unknown fields, executable configuration fields, credential-bearing or malformed load headers, non-object structured fields, unsafe Git URLs/references, authorization scope mismatches, run-shape violations, UUID/limit failures, event types/sequences, and result severity rules. The same manifest runs in TypeScript and Rust.
5. **Core PostgreSQL schema.** Six forward-only migrations create 21 domain tables plus `_sqlx_migrations`: identity/projects, Git sources/snapshots/jobs/events, plans/planned tests/approvals, targets/authorizations, and load/run/result/evidence/metric/schedule/agent/audit records.
6. **Database invariants.** Check constraints enforce the two job goals, load-profile ranges, run target shapes, failed-result severity, and JSONB payload ceilings. Queue, lease, idempotency, event-order, schedule, run, result, and project-listing indexes are present. Triggers make job events, approvals, evidence, and audit records append-only; target authorization scope/limits are immutable, with revocation as the only permitted update.
7. **Migration runner and checks.** `services/api/src/bin/migrate.rs` applies SQLx migrations from `DATABASE_URL` without logging the connection string. `scripts/db-check.mjs` runs the isolated migration test and fails when `DATABASE_URL` is absent. Root commands `db:migrate`, `db:test`, `check:db`, and `db:reset` were added; `.env.example` documents the Phase 2 database usage.
8. **Migration integration test.** `services/api/tests/migrations.rs` creates a random `pf_test_*` schema, sets `search_path`, applies and re-applies all migrations, verifies tables/indexes, seeds a complete fixture chain, tests invalid/immutable records, and drops the schema without touching `public`.
9. **Documentation.** Added `docs/CONTRACTS.md`, `docs/DATABASE.md`, `docs/frontend_design.md` (pointer to the canonical `06_frontend_design.md`), this report, and the Phase 2 decisions D-017-D-020. README, docs index, checklist, `.env.example`, and the contract checker were updated.

## Files changed

| Area | Files |
|---|---|
| Workspace and dependencies | `Cargo.toml`, `Cargo.lock`, `bun.lock`, `package.json`, `packages/contracts/package.json`, `services/api/Cargo.toml` |
| Shared contracts | `packages/contracts/schemas/probeforge.v1.schema.json`, `packages/contracts/src/model.ts`, `src/validate.ts`, `src/validate.test.ts`, `src/index.ts`, `fixtures/v1/*.json` |
| Rust contracts | `crates/contracts/Cargo.toml`, `src/lib.rs`, `src/model.rs`, `src/validation.rs`, `tests/fixtures.rs` |
| Migrations | `migrations/0001_identity.sql`, `0002_sources_jobs.sql`, `0003_plans.sql`, `0004_targets.sql`, `0005_runs.sql`, `0006_approval_scenarios.sql` |
| API database tooling | `services/api/src/bin/migrate.rs`, `services/api/tests/migrations.rs`, `services/api/Cargo.toml` |
| Checks | `scripts/db-check.mjs`, `scripts/check-docs.mjs` |
| Documentation and config | `README.md`, `Agents.md` (document count only), `.env.example`, `docs/README.md`, `docs/IMPLEMENTATION_CHECKLIST.md`, `docs/DECISION_LOG.md`, `docs/CONTRACTS.md`, `docs/DATABASE.md`, `docs/frontend_design.md`, `docs/phases/02_contracts_db_report.md` |

No real `.env`, credentials, scan evidence, or generated `target/`/`node_modules/` content was committed.

## Validation evidence

Commands were run from `D:\Projects\ProbeForge` unless noted. PowerShell can surface native stderr as a non-zero-looking error record; the observed exit codes and test summaries below are the authoritative results.

| # | Command | Actual outcome |
|---|---|---|
| 1 | `bun install` | Resolved and installed the Ajv dependency; lockfile updated (D-019) |
| 2 | `bun install --frozen-lockfile` | `Checked 215 installs across 293 packages (no changes)` - no lockfile drift |
| 3 | `cargo build --locked --workspace` | Finished successfully with both lockfiles; no dependency drift |
| 4 | `cargo test -p probeforge-contracts` | 3 fixture tests passed: manifest parity, bounded parsing, oversized rejection |
| 5 | `bun test` in `packages/contracts` | 45 tests passed, 0 failed, 75 expectations, across 2 files |
| 6 | `bun run test:ts` | 58 tests passed: contracts 45, test-catalog 4, analysis-worker 7, web 2 |
| 7 | `cargo test --workspace` | 19 tests passed: 5 API unit tests, 1 migration test (skipped without `DATABASE_URL`), 5 execution-worker, 5 scheduler, 3 contracts; 0 failed |
| 8 | `cargo fmt --all -- --check` | Exit 0, no diffs |
| 9 | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0, no warnings |
| 10 | `bun run lint` | `Checked 76 files ... No fixes applied`; exit 0 |
| 11 | `bun run typecheck` | All four TypeScript workspaces exited 0 |
| 12 | `bun run db:reset` + `bun run db:migrate` | PostgreSQL 18.6 recreated healthy; migrations applied from empty state |
| 13 | SQL inventory after migration | 21 domain tables in `public`; 6 rows in `_sqlx_migrations` |
| 14 | `bun run check:db` with `DATABASE_URL` and `PROBE_FORGE_REQUIRE_DB_TESTS=1` | 1 migration test passed in 0.62 s; schema created, migrated, seeded, constrained, and dropped |
| 15 | `bun run check:services` | **Not completed:** a pre-existing user-started Vite dev server on `127.0.0.1:5173` caused the runner's fail-closed port guard to exit 1. The existing web endpoint was left running; no ProbeForge service was stopped. Phase 1's recorded 5/5 result remains the last full service check. |
| 16 | `bun run check` | `PASS: repository contract is satisfied` (45/45 files); Rust fmt/clippy clean with 19 tests; Biome clean on 76 files; 4 typechecks and 58 TypeScript tests passed |

**Issues found and fixed during validation:**

- Ajv strict mode rejected `required`/`maxItems` subschemas without local types. The load-profile `oneOf` now uses `strictRequired: false` where required properties are defined by the parent, and `maxItems: 0` branches declare `type: "array"`.
- SQLx 0.9 needs owned `PathBuf` values for `Migrator::new` and `AssertSqlSafe` for runtime SQL strings. The migration runner/test were updated accordingly.
- The first cross-project FK test failed on the target's empty-scope constraint before reaching the FK. The fixture now uses a valid non-empty scope so the intended FK rejection is exercised.
- Biome rejected a control-character regex; it was replaced with a code-point helper in TypeScript validation.
- Clippy found needless borrows and collapsible `if` blocks in the new Rust validator; fixed and verified with `-D warnings`.

**Not run (and why):**

- Any real security scan, load test, model call, or external HTTP target - prohibited during development. Only loopback PostgreSQL, package installation, and local services were contacted.
- API database pool, transactions, queue claims, and database-aware readiness - deferred to Phase 3/4 (D-018).
- Real repository ingestion, target validation, execution adapters, and reporting - later phases.
- Full `check:services` run - the existing Vite dev server occupies the hard-coded web port; the runner correctly refused to start a second instance. The Phase 2 changes add no service runtime behavior, and the Phase 1 5/5 check remains the last complete result.
- CI - no Git remote exists (Q-004, D-011).
- Graceful shutdown under active database/HTTP load - no such workload exists yet.

## Security review

- **Data, not code.** No contract has a command, script, executable, shell, entrypoint, or arbitrary request-program field. Recursive `safeJson` rejects those property names at any depth, case-insensitively in both languages, and caps depth, array/object size, and strings. Structured configuration, stop conditions, and result details are object-only (`safeJsonObject`); request body templates remain bounded free-form JSON.
- **Explicit authority.** The target authorization contract requires `confirmed: true`, a normalized origin, allowed hosts/paths/ports, and hard rate/concurrency/duration/request limits. The database preserves authorization scope and limits immutably; revocation is the only update.
- **Credential handling.** Git URLs must use HTTPS/SSH without embedded passwords; credentials use `secret://` references. Load headers carrying `Authorization`, `Cookie`, `Proxy-Authorization`, or `Set-Cookie` are rejected, and header names must be valid HTTP token characters (no whitespace, colons, or control characters).
- **Bounded input.** Contract payloads are capped at 256 KiB before parsing; the API's existing 1 MiB body limit remains. Reprojection/response behavior is not part of Phase 2.
- **Server authority.** Schema validation is only the first layer. Backend semantic checks and database constraints remain authoritative; model output cannot authorize execution.
- **Tenant boundaries.** Every project-scoped table carries a project FK; the migration test proves a project FK orphan is rejected. Authorization enforcement on API routes is still Phase 3.
- **Immutable evidence.** Approvals, events, evidence, audit records, and authorization scope cannot be rewritten. This protects auditability but means retention changes need an explicit future design.
- **Test isolation.** The migration test never touches `public`; it creates a random schema and drops it. A missing database fails `check:db` rather than silently passing.
- **No weakened controls.** No security assertion was relaxed to make a check pass. The invalid fixtures and database rejection tests are part of the gate.
- **Dependencies.** Added `ajv 8.20.0` and `sqlx 0.9.0` (Postgres/migrate/runtime-tokio, no TLS features). Versions are exact and locked; SQLx is used only for migrations/tests in this phase.

## Remaining issues

1. **No service database pool.** The API, scheduler, and execution worker still expose static health endpoints. Phase 3 adds the first pool, transaction boundaries, and database-aware readiness (D-018).
2. **Contract validators are not wired into HTTP.** No job/plan/run endpoints exist yet; Phase 3/4 will call the shared validators at the API boundary.
3. **Queue behavior is schema-only.** Atomic claims, `FOR UPDATE SKIP LOCKED`, leases, heartbeats, fencing, retries, stale recovery, cancellation, and SSE replay arrive in Phase 4.
4. **Authorization scope is validated as data, not resolved.** DNS rebinding, redirect, and actual-connection checks belong to the target-validation phase; no execution path exists.
5. **No authentication, project APIs, real jobs, scans, load generation, or reports.** Phases 3-13.
6. **Empty test catalog.** Real, version-pinned catalog entries and adapters arrive in Phase 8.
7. **No CI and no Git remote.** Documented local checks are the gate (D-011, Q-004).
8. **Cross-resource association checks are deferred.** The schema enforces foreign-key existence, run shape, and project roots, but it does not yet constrain every composite association (for example, that an approval belongs to the run's exact plan version in the same project). The Phase 4 run-creation transaction and policy layer must validate those associations before insertion; add composite constraints if the operation proves error-prone.
9. **Local-only database verification.** PostgreSQL 18.6 was exercised on this Windows/Docker Desktop machine; production backups, retention, and migration operations remain future work.
10. **Migration test cleanup on failure.** A failed assertion leaves its temporary schema for inspection; it does not affect `public`, but contributors must clean it up or reset the local database.
11. **Full service check was not rerun.** A user-started Vite dev server occupies port 5173. Stop it and rerun `bun run check:services` when a clean live-service check is needed; Phase 2 did not change service startup behavior.
12. **Health contract unchanged.** The Phase 1 health shape remains; a versioned health schema is not required until it changes.

## Gate check

- [x] **Migrations create a complete database from empty state.** `db:reset` + `db:migrate` produced 21 domain tables and 6 recorded migrations; the isolated test applies and re-applies them successfully.
- [x] **Rollback strategy is documented.** `docs/DATABASE.md` defines forward-only migrations, backup/restore recovery, no production `migrate revert`, and explicit destructive local reset.
- [x] **Contract tests pass.** 45 TypeScript contract tests and 3 Rust fixture tests pass against the shared manifest; 58 TypeScript tests and 19 Rust tests pass workspace-wide.
- [x] **Invalid payloads are rejected before persistence.** Both validators reject invalid fixtures, the 256 KiB cap fails before parsing, and database tests reject bad enums, run shapes, duplicate keys, missing severity, and project FK orphans.
- [x] **Schemas contain no executable script fields for the model to populate.** The schema has no command/script fields, and recursive safe-JSON checks reject those keys in free-form configuration/payloads.

**PHASE 2: PASS**

## Next permitted step

**Phase 3 - Authentication, projects, and authorization** may begin only after explicit user authorization. No later phase may start before Phase 3's exit gate passes.
