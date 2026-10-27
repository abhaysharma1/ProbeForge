# Phase 17a — Run events migration guard

Follow-up to [Phase 17](17_run_section_live_reporting_report.md). Fixes the
deployment gap that made an approved plan's run creation fail with
`relation "run_events" does not exist`, and closes the readiness hole that let
the environment report `ready` while its schema was a migration behind.

Date: 2026-10-27

## Reported symptom

```
INFO  plan.approved  plan_id=d019796a… approval_id=00c34c42… job_id=f2fe2226…
ERROR POST /api/plans/d019796a…/runs  database operation failed
      error=error returned from database: relation "run_events" does not exist
ERROR response failed classification=Status code: 500 Internal Server Error latency=28 ms
ERROR POST /api/plans/d019796a…/runs  (retry 4 s later) — same failure, 24 ms
```

## Root cause

The environment's `_sqlx_migrations` table stopped at `0017`
(`17|repositories|t`), so `migrations/0018_run_events.sql` had never been
applied. Nothing in the API applies migrations at startup — the only callers of
the SQLx `Migrator` are `services/api/src/bin/migrate.rs` and test harnesses —
so a deployment that skips `bun run db:migrate` keeps running against the older
schema.

`POST /api/plans/{plan_id}/runs` reaches the database as follows:
`create_runs` (`services/api/src/runs.rs:61`) inserts each selected test's
`test_runs` row, then calls `append_run_and_job_event` (`crates/queue/src/lib.rs:261`)
→ `append_run_event` → `SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events`
(`crates/queue/src/lib.rs:234`), which fails with `undefined_table` and surfaces
through `ApiError::database` (`services/api/src/error.rs:74`) as a generic
`500 internal_error`.

Because the insert and the event append share one transaction and `create_runs`
returns the error before `transaction.commit()` (`runs.rs:243`), the failure
rolled back cleanly: no orphaned `test_runs` rows, no sequence gaps, and the
`run:{approval_id}:{planned_test_id}` idempotency keys were not consumed. The
plan remained `approved`, so no re-approval was required.

## Immediate remediation

```bash
bun run db:migrate
```

Observed before: 17 successful migrations, `to_regclass('public.run_events')` → null.
Observed after: 18 successful migrations, `run_events` plus
`run_events_run_sequence_idx` present.

## Fix: readiness gates on the migration head

Both readiness handlers previously asked only
`EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success LIMIT 1)` — true for any
schema at or after `0001` — so a replica with a stale schema advertised itself as
ready and failed later, after a user had already approved a plan.
`docs/CONTRACTS.md` already claimed `/readyz` verified migrations.

`services/api/src/lib.rs` and `services/scheduler/src/lib.rs` now:

- embed the migrations at compile time — `static MIGRATOR: Migrator =
  sqlx::migrate!("../../migrations")` — so the expected set is exactly what the
  binary was built from and the runtime image needs no `MIGRATIONS_DIR`;
- compute `pending_migrations(applied)` as the embedded versions absent from
  `SELECT version FROM _sqlx_migrations WHERE success`;
- return `503 not_ready` unless that set is empty, logging
  `db.migrations_pending` with version numbers only (never the connection
  string), and failing closed with `db.migration_state_unavailable` when the
  migration table cannot be read.

Migrations are still applied as a separate step (D-051); nothing auto-migrates
on startup, matching the deployment policy in `docs/DATABASE.md`.

The identical guard is deliberately duplicated in the two services rather than
extracted into a new crate; D-051 records that trade-off.

## Files changed

| File | Change |
|---|---|
| `services/api/src/lib.rs` | Embedded `MIGRATOR`, `pending_migrations`, `schema_is_current`; `/readyz` uses them; unit test for the pending set |
| `services/scheduler/src/lib.rs` | Same guard and unit test; `/readyz` withholds readiness on a pending migration |
| `services/api/src/bin/migrate.rs` | Applies the embedded migration set instead of reading a runtime directory (`MIGRATIONS_DIR` retired) |
| `services/api/tests/auth_projects.rs` | Regression test: deleting the newest migration row from an isolated schema yields `503 not_ready` |
| `migrations/0018_run_events.sql` | Unchanged — applied to the local database |
| `docs/DATABASE.md` | Migration range through `0018`, `run_events` schema-map row, readiness note in the deployment policy |
| `docs/CONTRACTS.md` | Health contract now describes the migration-head comparison |
| `docs/DECISION_LOG.md` | D-051 |
| `docs/IMPLEMENTATION_CHECKLIST.md` | Migration-head readiness entry |
| `scripts/check-docs.mjs` | `0017` and `0018` added to the required-file contract |
| `README.md` | Migration range through `0018` |

## Verification

| Check | Command | Result |
|---|---|---|
| Migration state (before) | `psql … "_sqlx_migrations"` in the local container | 17 rows, newest `0017`; `run_events` absent |
| Migration applied | `bun run db:migrate` | `database migrations applied` |
| Migration state (after) | `psql … "_sqlx_migrations"` | 18 rows; `run_events` and its index present |
| Migrator idempotency (embedded set) | `bun run db:migrate` (second run) | PASS — `migrations=18`, still 18 applied, no re-execution |
| Format | `cargo fmt --all -- --check` | PASS (exit 0) |
| Type check (bins + tests) | `cargo check -p probe-forge-api -p probe-forge-scheduler --all-targets` | PASS (exit 0) |
| Lint | `cargo clippy -p probe-forge-api -p probe-forge-scheduler --all-targets -- -D warnings` | PASS — exit 0, no warnings |
| Scheduler unit tests | `cargo test -p probe-forge-scheduler --lib` | PASS — 7/7, including `pending_migrations_reports_versions_missing_from_the_database` |
| API lib unit tests | `cargo test -p probe-forge-api --lib` | PASS — 49/49, including the new pending-set test and the existing fail-closed readiness test |
| Readiness integration tests | `cargo test -p probe-forge-api --test auth_projects` | PASS — 6/6, including `readiness_fails_when_a_migration_is_pending` |
| Repository contract | `bun scripts/check-docs.mjs` | PASS — 117 required files, links, phases, env hygiene, secret scan |

The Rust tests were run against the local PostgreSQL container with `DATABASE_URL`
loaded from `.env` and `PROBE_FORGE_REQUIRE_DB_TESTS` unset, so a missing database
would have skipped them; the 6/6 integration result confirms they executed. The
integration suite was built in a separate `CARGO_TARGET_DIR` because the running
dev stack holds `target/debug/probe-forge-api.exe`; that directory was removed
afterwards, so no verification artifacts remain.

## Known limitations

- The running local dev stack (`probe-forge-api`, `probe-forge-scheduler`) was
  started before this change. The database fix applies immediately, but the
  readiness behaviour only takes effect after those processes restart; they were
  left running so the operator's session was not interrupted.
- Readiness now compares applied versions only. It does not detect a
  checksum-altered migration (SQLx validates that when the migrator runs) or a
  schema modified manually after migration.
- The analysis worker's `/readyz` remains static (it has no database access) and
  the execution worker's `/readyz` remains static pending its own DB-backed
  phase, so neither participates in the migration-head gate.
- `docs/DATABASE.md` still has no rows for `0012`–`0016`; that pre-existing gap
  was left untouched to keep this fix scoped.
