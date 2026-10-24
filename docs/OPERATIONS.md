# ProbeForge operations

Operator-facing behavior added in Phase 14: structured logging, correlation IDs,
metrics, quotas, rate limits, retention, backups, and incident handling. All
commands are read-only or local unless stated.

> **Status:** structured logs, correlation IDs, metrics, per-project quotas,
> rate limits, retention, and a tested backup/restore roundtrip are implemented
> and tested. The execution worker does not yet run a dispatch loop, so no run
> is executed end to end; that is Phase 15.

## Structured logging and correlation IDs

Every Rust service initializes logging through `probe_forge_observability::init_tracing`.

- `RUST_LOG` sets the filter (default `info`).
- `PROBE_FORGE_LOG_FORMAT=json` emits line-delimited JSON for log aggregation;
  any other value keeps the human-readable format.
- The API attaches an `http_request` span with a `request_id` field to every
  request. A caller-supplied `X-Request-Id` is used only when it is bounded
  printable ASCII; otherwise one is generated. The chosen ID is echoed on the
  `X-Request-Id` response header, so a client-visible failure can be tied to the
  server log lines for that request.

Secrets are removed before any value is logged, stored in a report, or exported.
The single redaction implementation lives in `crates/observability` and replaces
sensitive object keys (`authorization`, `cookie`, `password`, `token`, ...) and
inline credential markers (`Bearer`, `Basic`, `Authorization:`, `Cookie:`, and
JWT-shaped tokens) with `[redacted]`.

## Metrics and diagnostics

The scheduler exposes internal, aggregate metrics (no tenant content):

```
GET http://127.0.0.1:8081/metricsz
```

| Field | Meaning |
|---|---|
| `jobsQueued` / `runsQueued` | Work waiting to be claimed |
| `jobsActive` / `runsActive` | Work currently leased |
| `jobsFailed` / `runsFailed` | Terminal failures |
| `jobsLeaseExpired` / `runsLeaseExpired` | Leased rows whose lease has lapsed (stuck workers) |
| `oldestJobQueueAgeSeconds` / `oldestRunQueueAgeSeconds` | Age of the oldest waiting item |
| `jobsFailedRecent` / `runsFailedRecent` | Failures in the last hour (failure rate numerator) |
| `completedRunAvgDurationSeconds` | Mean completed-run duration over the last 24h |

### Diagnosing a stuck job or failed worker

1. Check `runsLeaseExpired` / `jobsLeaseExpired`. Non-zero means a worker
   stopped heartbeating. The scheduler recovers these automatically on its poll
   interval (retry with backoff, then fail).
2. Query the queue directly for stuck leases:
   ```sql
   SELECT id, status, worker_id, lease_expires_at, last_error_code
   FROM jobs
   WHERE status IN ('analyzing', 'planning') AND lease_expires_at <= now()
   ORDER BY lease_expires_at;
   ```
   ```sql
   SELECT id, status, worker_id, lease_expires_at, last_error_code
   FROM test_runs
   WHERE status = 'running' AND lease_expires_at <= now()
   ORDER BY lease_expires_at;
   ```
3. Inspect recent failure reasons:
   ```sql
   SELECT id, last_error_code, attempt, max_attempts, updated_at
   FROM jobs WHERE status = 'failed' ORDER BY updated_at DESC LIMIT 20;
   ```
4. Follow one job's lifecycle events (`job_events` ordered by `sequence`) or the
   SSE stream at `GET /api/jobs/{id}/events`.

## Per-project quotas

Quotas bound server resources and are enforced by the queue claim path, not the
UI. Owners and admins manage them; any member may read them.

```
GET /api/projects/{project_id}/quota
PUT /api/projects/{project_id}/quota
  { "maxActiveRuns": 2, "maxRequestsPerWindow": 1000000, "windowSeconds": 86400 }
```

- `maxActiveRuns` (1–100): concurrent runs for the project. Without a row, the
  scheduler's default per-project cap applies.
- `maxRequestsPerWindow` (optional): when set, a claim is refused if the
  project's requests already recorded in the window plus the candidate
  scenario's profile maximum would exceed the budget.
- `windowSeconds` (60–604800): the budget window.

Changing a quota writes an audit event (`project.quota_updated`).

## Rate limits

The API enforces a fixed 60-second per-user window. Counters are keyed by a
SHA-256 digest, so raw identities are never stored.

- `PROBE_FORGE_API_RATE_LIMIT_PER_MINUTE` (default 240) applies to ordinary API
  calls.
- `PROBE_FORGE_EXPENSIVE_RATE_LIMIT_PER_MINUTE` (default 20) applies to
  expensive operations that trigger model work or report generation
  (`POST /api/jobs`, `POST /api/runs/{id}/report`).

A rejected request returns `429` with `Retry-After`. Login attempts keep their
own separate limit.

## Retention

The scheduler runs a retention purge on `PROBE_FORGE_RETENTION_INTERVAL_SECONDS`
(default hourly), in bounded batches, and logs the counts.

| Data | Rule |
|---|---|
| Evidence | Past `retention_until`, unless a recommendation references it |
| Reports | Older than `PROBE_FORGE_RETENTION_REPORT_DAYS` (default 180), including versions |
| Job events | Older than `PROBE_FORGE_RETENTION_EVENT_DAYS` (default 30) for terminal jobs |
| Sessions | Expired longer than `PROBE_FORGE_RETENTION_COUNTER_DAYS` (default 7) ago |
| Rate-limit / login counters | Idle longer than the counter window |

Evidence, report versions, job events, approvals, and audit rows are protected
by the `reject_immutable_row` trigger: every UPDATE and every DELETE is rejected
**except** a DELETE inside the one transaction that opts in with
`SET LOCAL probeforge.retention_purge = 'on'`, which only
`probe_forge_queue::run_retention` does. Ordinary code paths keep full
tamper-resistance; this is asserted by a regression test. Repository working
directories are cleaned by the ingestion worker, not this purge.

## Backup and restore

Uses `pg_dump`/`psql` inside the local Compose container
(`PROBE_FORGE_DB_CONTAINER`, default `probe-forge-local-postgres-1`).

```bash
bun run db:backup                    # writes backups/probeforge-<timestamp>.sql
bun run db:restore -- --input <file> --into <database>   # refuses the primary DB unless --force
bun run db:backup:check              # dump -> restore into a scratch DB -> verify -> drop scratch
```

`db:backup:check` verifies the migration count matches after restore and never
modifies the source database. Run it after schema changes. `backups/` is
gitignored and must never contain real target data.

### Restore procedure

1. Stop the API, scheduler, and workers so nothing writes during the restore.
2. `bun run db:backup` to capture the current state first.
3. Restore into a fresh database and validate with `db:backup:check`, or restore
   in place with `bun run db:restore -- --input <file> --into probeforge --force`.
4. Run migrations (`bun run db:migrate`) to apply anything newer than the dump.
5. Restart services and confirm `/readyz` on each.

## Dependency inventory and security updates

- Pin and lock: `Cargo.lock` (Rust), `bun.lock` (JavaScript). Install with
  `cargo build --locked` and `bun install --frozen-lockfile`.
- Review outdated packages before updating:
  ```bash
  cargo update --dry-run
  bun outdated
  ```
- Top-level pins live in `Cargo.toml` (`[workspace.dependencies]`) and the
  `package.json` files. Runtime pins are in `rust-toolchain.toml`, `.nvmrc`, and
  `packageManager`.
- Advisories: install a scanner (for example `cargo install cargo-audit`) and run
  `cargo audit` in CI. Prefer patching within a minor line; re-run `bun run
  check` and `bun run check:db` after any update.
- Do not add a dependency without reviewing maintenance, license, and known
  advisories (see `AGENTS.md`).

## Graceful shutdown and cancellation

- Each service installs a Ctrl+C/SIGTERM handler and drains in-flight requests
  (`axum::serve(...).with_graceful_shutdown`). The scheduler also aborts its
  poll and retention tasks on shutdown.
- `POST /api/jobs/{id}/cancel` sets the job terminal and clears its lease; SSE
  streams replay the `job.cancelled` event.
- `POST` schedule cancellation disables the schedule and cancels its not-yet-
  running runs; a running run is left to its lease owner to stop with its own
  budget.
- Cancellation changes state and stops future work; it does not silently mark a
  running workload as stopped without the lease owner acting.

## Incident handling runbook

1. **Stuck work.** Check `/metricsz`, then the lease queries above. If leases are
   expiring, confirm the worker process is running and its clock is sane.
   Recovery is automatic; escalate only if `lease_expired` retries exceed
   `max_attempts`.
2. **Failure spike.** Compare `jobsFailedRecent` / `runsFailedRecent` against
   normal. Read `last_error_code` values and the correlated log lines by
   `request_id`.
3. **Suspected secret exposure.** Redaction runs before logs and reports; if a
   value still leaked, rotate it, then file a fix with a regression test. Never
   paste the value into an issue.
4. **Database pressure.** Check queue age; raise `maxActiveRuns` deliberately or
   reduce intake. Never remove a quota to clear a backlog without approval.
5. **Restore.** Follow the restore procedure above; validate with
   `db:backup:check` before resuming traffic.

## Limitations

- No external metrics system is integrated; `/metricsz` and structured logs are
  the interfaces. There is no alerting.
- The execution worker is health-only; it does not claim or run jobs yet, so
  quotas and leases are enforced and tested at the queue layer but not exercised
  end-to-end.
- Rate limits are per process window in a shared table; a multi-process
  deployment shares the table, but clock skew between hosts is not corrected.
- Retention deletes immutable rows only through the opt-in path; it does not
  archive them elsewhere first.
