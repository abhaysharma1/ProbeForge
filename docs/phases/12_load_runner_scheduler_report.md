# Phase 12 report — Load runner and scheduler

**Status:** PHASE 12: PASS

**Date:** 2026-10-24

**Scope:** Added a constrained native load runner, hard caps, per-bucket/per-
endpoint metrics, persisted schedules with timezone/UTC semantics, worker
leases, global/per-project concurrency quotas, fair claiming, schedule
cancellation, and expiry recovery. No external process is invoked and no real
target is contacted.

## What changed

### Constrained runner (`crates/load-runner`)

- `build_runner_spec` validates the Phase 11 scenario against the approved target
  scope (`validate_load_scenario_scope`) and compiles it into a typed, data-only
  `RunnerSpec`. No script, expression, or executable is produced from model text.
- `resolve_limits` takes the minimum of the scenario profile, the target
  authorization, and platform caps (`LoadCaps`). `RunBudget` re-applies rate,
  concurrency, duration, request, response-size, and total-byte limits at the
  network boundary.
- `run_load` supports virtual-user and arrival-rate modes, ramp-up, a shared
  deadline, request/cancellation budgets, and closed stop conditions
  (`maxFailedRequests`, `errorRate`, `p95LatencyMs`).
- Steps run in `dependsOn` order. Extracted values are substituted into paths
  with strict percent-encoding, so a token cannot add a path segment; headers and
  body strings substitute raw with control-character rejection. Required secrets
  resolve through a `SecretResolver` before traffic starts; `NoSecretResolver`
  fails closed.
- `GuardedHttpClient::load_request` is the only client path that permits resolved
  credential headers; the safe `get`/`head`/`get_with_headers` path still rejects
  credentials. Request bodies are capped at 64 KiB.

### Metrics

- `aggregate_metrics` groups samples by bucket, endpoint (step ID), and status,
  and computes nearest-rank p50/p95/p99 latency, throughput, errors, timeouts,
  response bytes, and dropped iterations, plus a run summary.
- `persist_metrics` upserts buckets into `run_metrics` on
  `(run_id, bucket_start, endpoint, status_code)` with size/shape validation.
- `TargetRequestError::RequestTimeout` distinguishes request timeouts from
  generic network errors.

### Scheduler and queue (`crates/queue`, `services/scheduler`, migration `0012`)

- `migrations/0012_load_execution.sql` adds `target_authorization_id`,
  `expires_at`, and `cancelled_at` to `schedules`, constrains recurrence to
  `once`/`hourly`/`daily`/`weekly`, and adds due/expiry indexes.
- `create_schedule` requires an approved plan version, a selected scenario whose
  profile matches, and an active target authorization; it normalizes the
  timezone and stores `next_run_at` as UTC.
- `promote_due_schedules` creates one idempotent run per occurrence, advances the
  next UTC occurrence in the schedule's timezone, and disables schedules whose
  plan/authorization is no longer eligible.
- `recover_expired_schedules` disables expired or ineligible schedules.
- `cancel_schedule` disables the schedule and cancels its `scheduled`/`queued`
  runs; a `running` run is left to its lease owner.
- `claim_due_run_fair` uses a transaction advisory lock plus global and
  per-project active-run quotas and prefers the least-loaded project;
  `heartbeat_run`, `complete_run`, `fail_run`, and
  `recover_expired_run_leases` are fenced by worker ID and lease.
- The scheduler loop now expires schedules, promotes due schedules and runs, and
  recovers expired run/job leases on each tick.
- The execution worker re-exports `probe_forge_load_runner` as `load_runner`; its
  HTTP surface stays health-only.

### Documentation

- New `docs/LOAD_EXECUTION.md`.
- Decision D-033.
- Updated the checklist and `docs/LOAD_SCENARIOS.md`.

## Exit gate evidence

- **Scheduler tests verify due, future, cancelled, and expired schedules.**
  - `crates/queue/tests/queue.rs::scheduler_persists_and_handles_due_future_cancelled_and_expired_schedules`
    creates a due schedule (promoted to a queued run), a future schedule (not
    promoted), a cancelled schedule (pending run cancelled), and an expired
    schedule (disabled by `recover_expired_schedules`, no run created).
- **Load runner tests use a local mock server only.**
  - `crates/load-runner/src/executor.rs` tests bind a loopback mock HTTP server,
    run a correlated POST-then-GET flow, cancel a run, and exercise a request
    timeout. No external target or DNS is contacted.
- **Metrics calculations are validated against known fixture data.**
  - `metrics::tests::metrics_percentiles_and_buckets_match_known_fixture` asserts
    exact p50/p95/p99, bucket counts, error counts, and throughput for a fixed
    latency set; `aggregate_run_metrics_are_persisted_and_upserted` asserts the
    `run_metrics` upsert.
- **Hard caps cannot be bypassed by a scenario or model output.**
  - `policy::tests` prove effective limits are the minimum of profile, target,
    and platform caps and that platform caps clamp a generous target; the
    network layer re-applies the same caps and the `0011`/`0012` constraints are a
    third layer.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL` for database and service checks.

| Command | Result |
|---|---|
| `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check` | PASS — docs 80/80 required files, 14 linked docs, 16 phase headings; `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, workspace tests (load-runner 9 passed; target-policy 27; execution-adapters 9; queue DB suites skipped without `DATABASE_URL`); Biome 148 files; TS typecheck; contracts 55, test-catalog 6, analysis-worker 113 pass / 7 skip / 586 assertions, web 9. |
| `DATABASE_URL=... CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check:db` | PASS — api/auth/jobs/plans/targets (5), jobs (2), migrations (1), plan approval (6), targets (5), queue/scheduler (8). |
| `bun run db:migrate` | PASS — migration `0012_load_execution` applied after `0011`. |
| `bun run check:services` | PASS — 5/5 healthy; ports 8080, 8081, 8082, 8090, and 5173 closed afterward. |
| `git diff --check` | PASS — no whitespace errors. |

## Failures observed and resolved

- The first queue schedule test failed on `column s.load_profile_id does not
  exist`; the scenario table column is `profile_id`. Fixed in `create_schedule`.
- The first fair-claim test reused one job and one target per project, which
  violated the unique plan `(job_id, version)` and target-origin constraints.
  The test setup now creates one reusable target/authorization per schema and a
  fresh job per run.
- The first metrics persistence test read Postgres `sum(bigint)` as `i64` and
  failed with a `NUMERIC` decode error. The assertions now cast the sums to
  `bigint`; the upsert test passes.
- No final `bun run check`, `check:db`, or `check:services` failure remained.

## Known limitations

- No authenticated dispatcher or run endpoint starts live traffic; the execution
  worker is health-only and a claimed run is not executed by the service. The
  end-to-end load workflow is Phase 15.
- k6 is not installed and is not invoked. The "generated vetted scenario
  representation" path is used: a data-only `RunnerSpec` executed by a native
  adapter. A future k6 adapter must consume the same spec behind a pinned
  process boundary.
- Schedules are created through the queue library, not an API/UI; there is no
  schedule editor or run control surface yet.
- Secret references resolve only through an injected resolver; no production
  secret store or runtime header injection exists.
- The arrival-rate mode approximates the requested rate and still obeys the
  request/second budget.
- Migration `0012` constraints are `NOT VALID` for historical rows, as intended.
- No load test was run against a live deployment.

## Next permitted phase

**Phase 13 — Results, AI recommendations, and reports.** Not started. It
requires explicit authorization after this report is reviewed.
