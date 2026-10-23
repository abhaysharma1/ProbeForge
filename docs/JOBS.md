# ProbeForge jobs and PostgreSQL queue

Phase 4 implements authenticated job creation, durable queue claims, leases,
heartbeats, fencing, retries, stale-job recovery, ordered job events, SSE replay,
and the scheduler's due-run promotion. PostgreSQL remains the source of truth and
the only queue. No Redis, Kafka, or external broker is introduced.

## Job creation

`POST /api/jobs?projectId=<uuid>` accepts the versioned `jobCreation` contract
and requires an authenticated project member.

- Owner, admin, and collaborator may create and cancel jobs.
- Viewer is read-only and receives `403`.
- A non-member receives `404` so project existence is not disclosed.
- `projectId` is a required query parameter. The web client uses the same-origin
  `/api` path. This keeps the Phase 2 `jobCreation` body closed and unchanged.
- The Git URL must use `https` or `ssh`, must not embed credentials, and must not
  use a username except the `git` SSH username. Credentials use `credentialRef`
  (`secret://...`).
- `git.reference` is required. Phase 4 never enqueues a job without a concrete
  reference; draft jobs with an unresolved reference are deferred until Git
  ingestion can resolve them safely.
- The initial job status is `queued`, with `attempt = 0`, no worker, and no lease.
- `Idempotency-Key` is optional. Repeating the same request with the same key and
  the same project returns the existing job with `200`. Reusing the key for a
  different payload returns `409 idempotency_key_reused`.

Creation transaction:

1. Validate the contract and project role.
2. Upsert the project's `git_sources` row.
3. Insert the `jobs` row.
4. Append the `job.queued` event.
5. Insert the immutable `job.created` audit record.
6. Commit.

If the transaction fails, none of those records exist.

### Job API

| Method | Path | Access |
|---|---|---|
| `POST` | `/api/jobs?projectId=<uuid>` | owner, admin, collaborator |
| `GET` | `/api/jobs/{jobId}` | any project member |
| `GET` | `/api/projects/{projectId}/jobs` | any project member; latest 100 |
| `POST` | `/api/jobs/{jobId}/cancel` | owner, admin, collaborator |
| `GET` | `/api/jobs/{jobId}/events` | any project member |

Job DTOs expose the goal, intensity, status, source reference, attempt,
lease/heartbeat timestamps, worker ID, last error code, and provider/repository
URL. Credential references are never returned.

Cancellation is fenced by the job row lock. It clears the worker and lease,
writes `job.cancelled`, and writes an audit record. Cancelling an already
cancelled job is idempotent; failed or expired jobs return `409 job_terminal`.
A worker heartbeat or completion after cancellation fails the lease check.

## Queue claims, leases, and fencing

The shared `crates/queue` crate is used by the API and scheduler. It is the only
implementation of queue transitions.

### Job claim

`claim_job` runs in one short transaction:

```sql
UPDATE jobs ... FROM (
    SELECT id FROM jobs
    WHERE status = 'queued' AND not_before <= now() AND attempt < max_attempts
    ORDER BY priority DESC, not_before, created_at
    FOR UPDATE SKIP LOCKED
    LIMIT 1
) ...
```

The update sets:

- `status = 'analyzing'`
- `worker_id`
- `attempt = attempt + 1`
- `heartbeat_at = now()`
- `lease_expires_at = now() + lease`
- `last_error_code = NULL`

It appends `job.claimed` in the same transaction and commits before returning.
The worker then performs Git, model, or tool work without holding a database row
lock.

### Heartbeat

`heartbeat_job` updates only when the same `worker_id` still owns an unexpired
lease and the status is `analyzing` or `planning`. It renews
`lease_expires_at`; an expired lease cannot be resurrected by the old worker.

### Completion and failure

- `complete_job` requires the same worker and an unexpired lease. It moves the
  job to `awaiting_approval`, clears the lease, and appends `plan.proposed`.
  Plan persistence is added in the planning phase; the queue only fences the
  state transition.
- `fail_job` also requires ownership. Retryable failures return the job to
  `queued` with bounded exponential backoff: 1, 2, 4, 8, ... seconds, capped at
  300 seconds. The attempt is already incremented at claim time. Exhausted or
  non-retryable failures become `failed`.
- A stale worker that lost its lease receives `LeaseLost` (or `false`) and cannot
  overwrite a newer worker's state.

### Stale-lease recovery

The scheduler runs `recover_expired_job_leases` periodically:

- Jobs with an expired lease and remaining attempts go back to `queued` with
  backoff and a `job.retry_scheduled` event.
- Jobs with exhausted attempts become `failed` with a `job.failed` event.

Recovery uses `FOR UPDATE SKIP LOCKED`, so multiple scheduler instances do not
claim the same stale row.

## Events and SSE

Every transition appends one immutable `job_events` row with a per-job sequence:

- `job.queued`
- `job.claimed`
- `job.retry_scheduled`
- `job.failed`
- `job.cancelled`
- `plan.proposed`
- `run.started`

`GET /api/jobs/{jobId}/events` returns `text/event-stream`. Each event includes:

```text
id: <sequence>
event: <event type>
data: {"schemaVersion":"1","jobId":"...","sequence":N,"type":"...","occurredAt":"...","payload":{...}}
```

Reconnect replay:

- `Last-Event-ID` is the primary cursor and is sent automatically by
  `EventSource` on reconnect.
- `?after=<sequence>` is an explicit fallback.
- `?follow=false` returns the replay and closes the finite SSE stream. The
  default follows until the job reaches a terminal state.
- Replay is exclusive: `after=1` returns sequence 2 onward.
- Event data comes from PostgreSQL; SSE is a transport, not the source of truth.
- Invalid cursors return `400`; unauthenticated requests return `401`;
  non-members receive `404`.
- Database errors during a live stream emit a safe `stream.error` event and
  close the stream rather than exposing internal details.
- SSE routes bypass the normal 30-second API request timeout; a fixed timeout
  would terminate every live stream. Replay (`follow=false`) is finite.

## Scheduler

The scheduler now requires `DATABASE_URL`, owns a small PostgreSQL pool, and
reports database-aware readiness.

The background loop:

1. Promotes due `scheduled` test runs to `queued`.
2. Recovers expired job leases.

`claim_due_run` is implemented and tested for the future execution dispatcher:
it atomically claims a due `scheduled`/`queued` run with the same
`FOR UPDATE SKIP LOCKED` and lease pattern, then appends `run.started` to the
parent job event stream. The scheduler loop does not claim runs yet because no
execution adapter exists; claiming without a dispatcher would leave fake running
rows.

`PROBE_FORGE_SCHEDULER_POLL_SECONDS` controls the loop interval (default 5,
range 1-300).

## Verification

- `crates/queue/tests/queue.rs` proves concurrent claims are disjoint, expired
  leases recover, retries back off and terminate, stale workers cannot complete
  or fail after losing a lease, and due runs promote/claim.
- `services/api/tests/jobs_queue.rs` proves contract validation, project roles,
  idempotency, cancellation, event ordering, and SSE `Last-Event-ID` replay.
- All fixtures are inert repository metadata and local database rows. No Git
  clone, scanner, load generator, or external target is contacted.

## Deferred

- Analysis-worker polling is not wired yet. Phase 5 provides the read-only
  ingestion library and operator CLI; Phase 6/7 add discovery and the Pi planning
  session, and a later integration connects them to claimed jobs.
- Repository snapshots and ingestion audit events are not persisted to
  PostgreSQL yet. See `docs/REPOSITORY_INGESTION.md`.
- No execution dispatcher claims due runs yet (Phases 10 and 12).
- No job UI is added in Phase 4; the API and event stream are ready for the
  later workspace.
- No `LISTEN/NOTIFY` wake-up hint; the scheduler and SSE use short polling.
- No job/event retention job beyond the immutable event records.
