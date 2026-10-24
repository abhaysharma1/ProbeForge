# ProbeForge operations

Operator-facing behavior added in Phase 14: structured logging, correlation IDs,
metrics, quotas, rate limits, retention, backups, and incident handling. All
commands are read-only or local unless stated.

> **Status:** structured logs, correlation IDs, metrics, per-project quotas,
> rate limits, retention, and a tested backup/restore roundtrip are implemented
> and tested. The execution worker does not yet run a dispatch loop, so no run
> is executed end to end; that is Phase 15.

## Structured logging and correlation IDs

Every Rust service initializes logging through `probe_forge_observability::init_tracing`;
the analysis worker initializes `services/analysis-worker/src/logging/logger.ts`. Both
implementations share one contract so the control plane and the workers read the same way in
an aggregator:

- **Levels.** `error` = an operation failed and needs operator attention; `warn` = a
  recoverable failure, a security denial, lease loss, or a degraded state; `info` = a
  lifecycle/outcome transition (claim, completion, schedule promotion, service start/stop);
  `debug` = per-step diagnosis. Hot paths stay at `debug` so `info` output is meaningful.
- **Events.** Every line carries a stable `event` name in `snake_case` (for example
  `job.claimed`, `run.completed`, `target.destination_blocked`) plus the identifiers in scope:
  `request_id`, `project_id`, `job_id`, `run_id`, `worker_id`. Failures also carry the stable
  `error_code` persisted to the database.
- **Coverage.** Libraries (`crates/queue`, `crates/target-policy`, `crates/execution-adapters`,
  `crates/load-runner`) emit `tracing` events but never initialize a subscriber; only service
  binaries call `init_tracing`. The API logs mutations and denials (not every read); the
  scheduler, execution worker, and analysis worker log claims, outcomes, retries, and
  lease/recovery events.

Configuration:

| Variable | Service | Meaning |
|---|---|---|
| `RUST_LOG` | Rust services | Filter expression (default `info`) |
| `PROBE_FORGE_LOG_FORMAT` | Rust services | `json` emits line-delimited JSON; anything else is human-readable |
| `PROBE_FORGE_LOG_LEVEL` | analysis worker | `trace`/`debug`/`info`/`warn`/`error` (default `info`) |
| `PROBE_FORGE_LOG_FORMAT` | analysis worker | `json` (default) or `text` |

The API attaches an `http_request` span with `service`, `request_id`, `method`, and `path` to
every request. A caller-supplied `X-Request-Id` is used only when it is bounded printable ASCII;
otherwise one is generated. The chosen ID is echoed on the `X-Request-Id` response header, so a
client-visible failure can be tied to the server log lines for that request.

Secrets are removed before any value is logged, stored in a report, or exported. The single Rust
redaction implementation lives in `crates/observability` and replaces sensitive object keys
(`authorization`, `cookie`, `password`, `token`, ...) and inline credential markers (`Bearer`,
`Basic`, `Authorization:`, `Cookie:`, and JWT-shaped tokens) with `[redacted]`. The analysis
worker mirrors this in `logging/logger.ts`. When changing either, keep the sensitive-key list and
marker handling in sync.

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

## Platform settings (admin dashboard)

A **platform administrator** is a settings-only operator: it can manage global
configuration and nothing cross-tenant. It is seeded by `bun run user:seed`
(`admin@probeforge.local`).

- `GET /api/admin/settings` — read all settings (never the secret).
- `PUT /api/admin/settings` — update planning, rate limits, retention, and
  project defaults; optionally set a new provider API key.
- `DELETE /api/admin/settings/planning/api-key` — clear the stored key.

| Group | Effect |
|---|---|
| Planning (Pi) | The analysis worker reads provider/model/limits per job and passes the decrypted key into the Pi stream call. DB overrides env; with none set, the deterministic fallback planner runs. |
| API rate limits | Applied live to new requests via a cached copy. |
| Retention | The scheduler reads the windows on each retention tick. |
| Project defaults | The execution worker uses the default active-run cap at claim time; per-project `project_quotas` rows override it. |

Provider API keys are stored **only as AES-256-GCM ciphertext** with the master
key from `PROBE_FORGE_SETTINGS_KEY` (base64, 32 bytes). They are write-only:
the API returns only a `••••1234` hint, and no code logs them. Saving a key
without the master key configured fails closed. Values are validated for shape
and range; provider/model are additionally validated against the pinned SDK
catalog when a job runs.

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

## End-to-end workflow

By default the app is self-driving for local use: `PROBE_FORGE_ANALYSIS_JOB_LOOP=true`
makes the analysis worker claim queued jobs and produce a draft plan, and
`PROBE_FORGE_EXECUTION_DISPATCH_LOOP=true` makes the execution worker drain
queued runs. Both are disabled by default in `.env.example` and can be turned off
for CI or one-off runs.

From the web app:

1. Open a project and use **Connect a repository** — paste a public GitHub,
   GitLab, or Bitbucket HTTPS link and pick a branch/tag/SHA. This queues an
   analysis job; the plan appears under **Analysis jobs → Review plan**.
2. Review and approve the plan.
3. Register and authorize a target under **Targets**.
4. In the plan review, choose the authorization under **Queue runs**.
5. Open the run under **Runs** to see results, evidence, and the generated
   report.

The same stages can be driven by the operator binaries:

```bash
bun run --cwd services/analysis-worker plan     # one-shot: process queued jobs
cargo run -p probe-forge-execution-worker --bin dispatch -- --once   # one-shot dispatch
```

`dispatch` enforces target scope, redirect, DNS, and budget policy below the
dispatcher; loopback/private targets stay blocked unless
`PROBE_FORGE_ALLOW_PRIVATE_TARGETS=true` is set for a secured local mock. See
`docs/RELEASE_READINESS.md` for the full runbook.

## Limitations

- No external metrics system is integrated; `/metricsz` and structured logs are
  the interfaces. There is no alerting.
- The analysis and execution loops are opt-in env flags, not supervised
  daemons; for continuous non-local operation run them under an external
  supervisor. Very large repositories can exceed the ingestion git timeout
  (`PROBE_FORGE_ANALYSIS_GIT_TIMEOUT_MS`); raise it or split the repo.
- Rate limits are per process window in a shared table; a multi-process
  deployment shares the table, but clock skew between hosts is not corrected.
- Retention deletes immutable rows only through the opt-in path; it does not
  archive them elsewhere first.
