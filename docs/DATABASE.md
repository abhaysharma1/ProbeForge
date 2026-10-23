# ProbeForge database

PostgreSQL 18.6 is the durable source of truth. Phase 2 introduced the schema,
migration runner, and schema-isolated migration test. Phase 3 adds the first API
database pool, authentication/session tables, project RBAC, and database-aware
readiness. The browser never connects to PostgreSQL.

## Local commands

```bash
bun run db:up         # start PostgreSQL 18.6 on 127.0.0.1:5432
bun run db:migrate    # apply forward-only migrations (reads DATABASE_URL)
bun run check:db      # run migration, auth/RBAC, jobs, plan approval, and queue integration tests (reads DATABASE_URL)
bun run db:down       # stop PostgreSQL, keep the volume
bun run db:reset      # destructive: stop, delete the volume, start empty
```

Set `DATABASE_URL` from `.env.example`. `POSTGRES_PASSWORD` and the password in
`DATABASE_URL` must match. Do not commit a real `.env` or a real password.

The migration binary validates only the URL scheme and never logs the connection
string. The test runner sets `PROBE_FORGE_REQUIRE_DB_TESTS=1`, so a missing
`DATABASE_URL` fails the check instead of silently skipping database coverage.

## Migration policy

Migrations live in `migrations/` and are applied by
`cargo run -p probe-forge-api --bin migrate`, which uses SQLx's runtime migrator.

- **Forward-only.** Migrations are ordered (`0001` through `0011` today) and are never
  edited after being applied. Fix a mistake with a new migration.
- **Do not use `sqlx migrate revert` in shared or production environments.** No
  down migrations are maintained. SQLx records applied versions in
  `_sqlx_migrations`.
- **Local recovery:** `bun run db:reset` recreates the local database. This is
  destructive and is intentionally a manual command.
- **Production recovery:** restore the last verified backup, or ship a reviewed
  compensating migration. Document the incident and the data impact first.
- **Reviewability:** migrations must be safe for existing data, state whether
  they lock or rewrite tables, and explain any destructive operation. No manual
  production edits.
- **Deployment:** take a backup before a destructive change, apply migrations as
  a separate step from starting application replicas, and never run migrations
  concurrently from multiple deployment jobs.

The Phase 2 migrations create the schema from empty state and only use guarded
DDL (`CREATE`, `ALTER`, constraints, indexes, and functions). Re-running the
migrator is idempotent because SQLx skips already-applied versions.

## Schema map

| Migration | Tables | Notes |
|---|---|---|
| `0001_identity.sql` | `users`, `projects`, `project_members` | Tenant roots, owner/member roles, case-normalized email, update triggers |
| `0002_sources_jobs.sql` | `git_sources`, `repository_snapshots`, `jobs`, `job_events` | Repository metadata, job state, queue/lease indexes, ordered append-only events |
| `0003_plans.sql` | `test_plans`, `planned_tests`, `plan_approvals` | Versioned plans, catalog test references, immutable approval snapshots |
| `0004_targets.sql` | `targets`, `target_authorizations` | Normalized target origins/scope, immutable authorization scope and limits |
| `0005_runs.sql` | `load_profiles`, `load_scenarios`, `schedules`, `test_runs`, `test_results`, `evidence`, `run_metrics`, `agent_sessions`, `audit_log` | Bounded load profiles, due schedules, run shape/leases, result/evidence/metric records, append-only audit |
| `0006_approval_scenarios.sql` | `plan_approvals` | Adds approved load-scenario IDs and allows either test or scenario selections |
| `0007_auth_sessions.sql` | `users`, `auth_sessions`, `auth_login_attempts` | Argon2id password hash, opaque session-token digests, revocation/expiry indexes, hashed login-rate-limit keys |
| `0008_plan_review.sql` | `test_plans`, `plan_approvals`, `planned_tests` | Plan selection revision, approval revision evidence, rationale references |
| `0009_approval_run_match.sql` | `plan_approvals`, `test_runs` | Composite approval/plan/version foreign key for run integrity |
| `0010_target_validation.sql` | `targets` | Verification timestamp, resolved address evidence, and allowed ports |
| `0011_load_safety.sql` | `load_profiles` | Platform profile maxima and closed stop-condition keys; new/updated rows enforced with `NOT VALID` constraints |

Every project-scoped table carries `project_id` and cascades from `projects`
except where a deliberate restriction preserves approval/authorization history.
Foreign keys and check constraints are the second line of defense behind the
contract validators, not a replacement for them.

## Queue, lease, and ordering indexes

- `jobs_queue_idx` and `test_runs_queue_idx` support due-work selection.
- `jobs_lease_idx` and `test_runs_lease_idx` support stale-lease recovery.
- `jobs_idempotency_key_key` and `test_runs(project_id, idempotency_key)` reject
  duplicate creation.
- `job_events(job_id, sequence)` supports ordered SSE replay.
- `schedules_due_idx` supports scheduler claims.
- Project/status/created indexes support bounded project-scoped listings.

Queue claiming itself is implemented in `crates/queue` and used by the API and
scheduler: atomic `FOR UPDATE SKIP LOCKED` claims, committed-before-work
transactions, leases, heartbeats, worker-ID fencing, bounded exponential
retries, stale-lease recovery, cancellation, and due-run promotion. The schema
constraints above remain the second line of defense.

## Invariants and immutability

- `users.password_hash` is a nullable Argon2id PHC string. Users without a hash
  cannot authenticate; passwords are never stored in plaintext.
- `auth_sessions.token_hash` is exactly 32 bytes (SHA-256). Sessions require
  `revoked_at IS NULL` and `expires_at > now()`; logout sets `revoked_at`.
- `auth_login_attempts` stores only SHA-256 digests of account/address or address
  rate-limit keys, never raw emails or IP addresses.
- Only `vulnerability_testing` and `load_testing` are legal job goals.
- `vulnerability_testing` runs require a planned test and no load scenario;
  `load_testing` runs require a scenario and profile and no planned test.
- Target scopes are normalized and server-validated. `targets.allowed_ports`,
  verification timestamp, and resolved-address evidence were added in migration
  `0010`. Target authorization scope, limits, and confirmation are immutable;
  only `revoked_at` may be set once.
- Plan selection changes increment `test_plans.revision` and require the
  reviewed revision. A `plan_approvals` row records the selected tests or
  scenarios, the reviewed revision, and an immutable snapshot; it cannot be
  updated or deleted. `test_runs.plan_approval_id` is `NOT NULL`, and the
  composite approval/plan/version foreign key prevents citing an approval for a
  different plan version. Updating the scope or limits of a target authorization
  fails; a revocation timestamp is the only permitted update, and authorizations
  cannot be deleted.
- Load profiles are capped by the contract at 100 VUs or 100 arrivals/s, 300 s
  ramp, 900 s duration, 100,000 requests, and 100 requests/s. Migration `0011`
  adds `NOT VALID` database constraints for new/updated rows and closes stop
  conditions to `errorRate`, `p95LatencyMs`, and `maxFailedRequests`. Existing
  historical rows are not rewritten by the migration; validate or remediate them
  before enforcing the constraints on all rows.
- Job events are append-only and ordered per job. The API emits
  `job.queued`, `job.claimed`, `job.retry_scheduled`, `job.failed`,
  `job.cancelled`, `plan.proposed`, and `run.started`; SSE replay uses the
  sequence number as the event ID.
- `jobs.idempotency_key` is unique per project when present. Reusing a key with
  a different request is rejected by the API before a second row is inserted.
- `job_events`, `plan_approvals`, `evidence`, and `audit_log` reject updates and
  deletes with triggers. Retention policies must be designed explicitly rather
  than bypassing these triggers.
- Secret references use the `secret://` form; credentials are not stored in
  repository URLs, load headers, logs, or fixtures.
- JSONB payloads have size ceilings, and the contract's `safeJson` validation
  rejects executable command/script field names before persistence.

## Migration test

`services/api/tests/migrations.rs` runs only against a real PostgreSQL database:

1. Creates a random schema (`pf_test_<pid>_<nanos>`), never `public`.
2. Creates a pool whose connections set `search_path` to that schema.
3. Applies all migrations, then re-applies to prove idempotence.
4. Asserts every expected table, key index, plan-review/target-validation
   column, and approval/run constraint exists.
5. Seeds a complete fixture chain: user â†’ project â†’ Git source â†’ job â†’ snapshot â†’
   plan â†’ planned test â†’ approval â†’ target â†’ authorization â†’ load profile â†’
   scenario â†’ schedule â†’ run â†’ result â†’ evidence â†’ event â†’ metric â†’ agent session
   â†’ audit record.
6. Asserts unknown goals, duplicate idempotency keys, duplicate event sequences,
   mutable approvals/events/authorizations, failed results without severity,
   duplicate result keys, malformed run shapes, and project FK orphans are
   rejected.
7. Drops the temporary schema.

`services/api/tests/auth_projects.rs` and `services/api/tests/jobs_queue.rs`
run against their own isolated schemas. They apply all migrations and exercise
login/logout, session revocation, CSRF/origin rejection, rate limiting, project
RBAC, cross-project denial, audit writes, job creation/idempotency/cancellation,
event ordering, and SSE replay. They never clone a repository, scan a target, or
contact an external host.

`crates/queue/tests/queue.rs` uses its own schema to prove concurrent claims,
lease expiry recovery, bounded retries, fencing, and due-run promotion/claiming.

A failing assertion leaves the temporary schema in place for local inspection.
Drop it manually or reset the local database after investigating.

## Not in Phase 5

- Persistence of repository snapshots or ingestion audit events to PostgreSQL,
  and analysis-worker polling (planned with the queue integration). Phase 5
  produces bounded snapshot/audit output through the operator CLI and library;
  it does not write the control-plane tables.
- The Pi planning session and static discovery (Phases 6-7).
- Approval workflows, per-member approval grants, and target validation
  (Phases 6-10).
- Execution dispatch, scanners, load generation, and reporting (Phases 10-13).
- Hard project deletion/archiving and retention jobs; immutable audit,
  approval, and evidence triggers intentionally block destructive cascades
  (D-022).
- Backups, retention jobs, and production operations (later hardening).
