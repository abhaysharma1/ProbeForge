# Load execution: constrained runner and scheduler

Phase 12 turns the validated Phase 11 scenario into a bounded, non-scriptable
load run, and adds persisted schedules with leases, quotas, fairness,
cancellation, and aggregate metrics. The runner is a library owned by the
execution worker. No load test is run against a real target in this phase; all
runner tests use a loopback mock HTTP server.

> **Status:** runner, caps, metrics, scheduling, leases, quotas, and
> cancellation are implemented. The execution worker remains health-only; no
> authenticated dispatcher or run endpoint starts live traffic yet, and k6 is
> not wired.

## Why a native adapter instead of k6

Phase 12 permits "a constrained adapter **or** a generated vetted scenario
representation." k6 is not installed, and adding an external binary needs an
approved, pinned process boundary (D-031). ProbeForge therefore:

1. Validates the data-only scenario exactly as in Phase 11.
2. Compiles it into a typed `RunnerSpec` — no JavaScript, no model text, no
   expressions beyond `${name}` substitution from earlier extraction rules.
3. Executes the spec with a native adapter built on the shared
   `GuardedHttpClient`/`RunBudget`.

A future k6 adapter must consume the same `RunnerSpec` behind a fixed,
version-pinned executable with typed parameters, bounded structured output, and
process-tree termination. Model output never becomes a script.

## Hard caps (cannot be bypassed)

`resolve_limits` computes the effective run limits as the **minimum of three
sources**, so neither a scenario nor model output can raise them:

| Limit | Platform cap (`LoadCaps`) | Target authorization | Scenario/profile |
|---|---|---|---|
| Virtual users | 100 | `maxConcurrency` | `virtualUsers` |
| Arrival rate | 100/s | `maxRequestsPerSecond` | `arrivalRatePerSecond` |
| Ramp-up | 300 s | — | `rampUpSeconds` |
| Duration | 900 s | `maxDurationSeconds` | `durationSeconds` |
| Requests | 100,000 | `maxRequests` | `maxRequests` |
| Requests/second | 100/s | `maxRequestsPerSecond` | `maxRequestsPerSecond` |
| Concurrency | 100 | `maxConcurrency` | derived from VUs |
| Response size | 2 MiB | — | — |
| Total bytes | 16 MiB | — | — |

`RunBudget` re-applies rate, concurrency, duration, request, response-size, and
total-byte limits at the network boundary, so a bug in a caller cannot exceed
them. The `0011` database constraints remain a third, `NOT VALID` layer for new
and updated rows.

## Execution model

- `virtualUsers` mode: N workers loop until the shared deadline, request cap,
  cancellation, or a stop condition is reached. Workers start over the ramp
  window.
- `arrivalRatePerSecond` mode: iterations are launched at the requested rate
  behind a concurrency semaphore.
- Each iteration walks steps in `dependsOn` order, substitutes extracted values
  (path values are percent-encoded, so an extracted token cannot escape its path
  segment), resolves `secret://` references through a `SecretResolver`, and
  evaluates expected statuses, assertions, and extractions.
- Missing required secrets fail before traffic starts. The default
  `NoSecretResolver` resolves nothing and fails closed; there is still no
  production secret manager or runtime header injection.
- Stop conditions are closed and typed: `maxFailedRequests`, `errorRate`, and
  `p95LatencyMs`. Deadlines, the request budget, and cancellation stop work.

## Metrics

`aggregate_metrics` groups samples by time bucket, endpoint (step ID), and
status code and computes nearest-rank p50/p95/p99 latency, throughput, error
count, timeout count, response bytes, and dropped iterations. The summary
aggregates the whole run. `persist_metrics` upserts buckets into `run_metrics`
on `(run_id, bucket_start, endpoint, status_code)`; it rejects oversized or
malformed aggregates.

## Schedules

Migration `0012_load_execution.sql` adds `target_authorization_id`,
`expires_at`, and `cancelled_at` to `schedules` and constrains recurrence to a
closed vocabulary (`once`, `hourly`, `daily`, `weekly`). `create_schedule`:

- requires an approved plan version, a selected scenario whose profile matches,
  and an active target authorization for the same project;
- stores the timezone normalized to `UTC` or a fixed `+HH:MM` offset;
- stores `next_run_at` as the authoritative **UTC** execution timestamp;
- rejects invalid start/expiry windows.

`promote_due_schedules` creates at most one idempotent load run per schedule
occurrence (`schedule:<id>:<epoch>`), advances `next_run_at` in the schedule's
timezone, disables `once` schedules, and fails closed (disables the schedule) if
the plan is no longer approved or the authorization is inactive.
`recover_expired_schedules` disables expired or otherwise ineligible schedules.
`cancel_schedule` disables the schedule and cancels its `scheduled`/`queued`
runs; a `running` run is left to its lease owner to stop.

## Leases, quotas, and fairness

- `claim_due_run_fair` claims one due run under a transaction advisory lock and
  enforces global and per-project active-run quotas, preferring the project with
  the fewest active runs. It also refuses runs whose target authorization is
  inactive.
- `heartbeat_run` renews a lease only for the owning worker.
- `complete_run` and `fail_run` are fenced by worker ID and an unexpired lease;
  retryable failures return the run to `queued` with bounded backoff.
- `recover_expired_run_leases` requeues or fails runs whose lease expired.

The scheduler service runs these on each tick in order: expire schedules,
promote due schedules, promote due scheduled runs, recover run leases, recover
job leases.

## Limitations

- No live dispatch loop and no API/UI schedule creation yet; schedules are
  created through the queue library and promoted by the scheduler.
- The execution worker is health-only; a claimed run is not yet executed by the
  service (Phase 15 owns the end-to-end path).
- Secret references are validated and resolved only through an injected
  resolver; no production secret store or runtime injection exists.
- The `arrivalRate` mode approximates the requested rate and still obeys the
  request/second budget.
- Local PostgreSQL migration `0012` is applied; schedule constraints are
  `NOT VALID` for historical rows, as intended.
