# 2. ProbeForge — Broad Implementation Plan
## System Architecture, Boundaries, Data Flow, and Delivery Strategy

**Product:** ProbeForge  
**Audience:** coding agent and technical reviewer  
**Starting assumption:** no code, infrastructure, schema, or UI exists yet  
**Rule:** implement the phases in document 03 in order and stop at every gate

---

## 1. Architecture at a glance

```text
Browser (React)
    |
    | HTTPS / authenticated API / SSE
    v
Rust Control Plane (Axum + Tokio)
    |-- authentication and project RBAC
    |-- job lifecycle and input validation
    |-- target scope and authorization
    |-- plan approval and run scheduling
    |-- report and event APIs
    |
    v
PostgreSQL
    |-- projects, source configs, jobs
    |-- plans, planned tests, approvals
    |-- targets, authorization records
    |-- runs, results, evidence metadata
    |-- events, audit log, load metrics
    |
    +--> Analysis worker (TypeScript + Pi SDK)
    |       |-- temporary Git workspace
    |       |-- read-only repository inspection
    |       |-- Pi planning tools
    |       +-- validated plan persisted to PostgreSQL
    |
    +--> Scheduler / run dispatcher
            |
            +--> Security execution worker
            |       +--> allowlisted tool adapters
            |
            +--> Load execution worker
                    +--> constrained load runner
```

The initial ProbeForge deployment can run API, analysis worker, scheduler, and execution worker as separate processes or containers on a small number of hosts. Keep logical boundaries in code from the beginning so execution can later move to dedicated hosts without redesigning the product.

## 2. Main subsystems

### 2.1 Frontend
The browser is an operator console, not a trusted authority. It renders job creation, source configuration, generated plans, approval screens, schedules, live events, result details, and reports. It calls the API and never connects to PostgreSQL directly.

### 2.2 Rust control plane
Owns authentication, authorization, validation, durable state transitions, scheduling metadata, quotas, API contracts, audit events, and report retrieval. It must not rely on Pi to enforce security rules. It should not run scans inside HTTP request handlers.

### 2.3 PostgreSQL
The initial durable queue is implemented using job/run rows. Use atomic claim queries with `FOR UPDATE SKIP LOCKED`, short transactions, leases, heartbeats, retries, and stale-job recovery. PostgreSQL is the source of truth. Do not add Redis, Kafka, SQS, or another queue in the MVP unless an explicitly measured requirement demands it.

### 2.4 Analysis worker and Pi
A TypeScript worker uses the Pi SDK. It creates an agent session with a small, typed tool surface. It reads repository files and returns structured observations and test-plan candidates. It does not receive unrestricted shell access, database superuser credentials, or target execution privileges.

### 2.5 Test catalog and adapters
Maintain a versioned catalog of supported test types. A catalog entry defines stable ID, purpose, category, prerequisites, risk, required configuration schema, supported target types, adapter version, timeout, and whether explicit extra confirmation is required. The model chooses catalog IDs and structured arguments, never arbitrary commands.

Examples of tool families to consider later:
- source/static checks: Semgrep, Gitleaks, dependency advisory checks using lockfiles, secret/config checks;
- HTTP and web security checks: ZAP in a controlled configuration, selected Nuclei templates after review, TLS/header checks, narrowly scoped HTTP checks;
- network checks: only explicitly approved, scoped, rate-limited checks;
- load tests: k6 as the initial mature runner.

Do not install or invoke every Kali tool indiscriminately. Pick tools with machine-readable output, stable behavior, manageable risk, clear licensing, and bounded configuration. Maintain a vetted allowlist and pin versions.

### 2.6 Security execution worker
Consumes approved run records, checks scope again, invokes only catalog adapters, captures structured output, enforces time/resource/traffic budgets, and records evidence. It has no need to read arbitrary project secrets. Use a restricted process/container and default-deny network policy where practical. Never mount the host Docker socket or expose cloud metadata.

### 2.7 Load execution worker
Runs a validated scenario against an authorized deployment. It enforces hard maximum concurrency, request rate, duration, total requests, and response-body size. It must stop on cancellation or budget exhaustion. Keep the runner separate from the control plane so a heavy test cannot exhaust API resources.

### 2.8 Reporting
Results are persisted before AI summarization. AI reports reference stored result IDs and evidence. If summarization fails, raw results remain available and the run can be marked `completed_with_report_error` or use a separately tracked report status; do not lose the test results.

## 3. Core domain model

Use UUID identifiers. Use foreign keys and tenant/project scoping in queries.

- `users`: account identity and status.
- `sessions` or identity-provider linkage: secure authenticated sessions.
- `projects`: owner and project metadata.
- `project_members`: user, role, and project permissions.
- `git_sources`: provider, normalized URL, secret reference, default ref; never store plaintext access tokens.
- `jobs`: goal, source snapshot, intensity, state, creator, retry/lease fields, current plan version.
- `repository_snapshots`: resolved commit SHA, detected languages/frameworks, manifest inventory, analysis metadata.
- `test_plans`: version, schema version, agent/model metadata, summary, approval state.
- `planned_tests`: plan ID, catalog test ID, arguments, rationale, prerequisites, risk and selection status.
- `targets`: normalized origin, environment, allowed scope, verification status.
- `target_authorizations`: who approved what, when, for which scope, and under which limits.
- `test_runs`: run type, approved plan/scenario version, target, schedule, status, limits, worker lease.
- `test_results`: test ID, outcome, severity, confidence, timestamps and structured result.
- `evidence`: references to sanitized evidence artifacts, hashes, MIME type, size and retention.
- `job_events`: ordered lifecycle events for SSE.
- `audit_log`: security-sensitive actions and actor.
- `run_metrics`: aggregated load metrics by time bucket and endpoint/scenario.
- `agent_sessions` or `agent_traces`: minimal trace metadata, with secrets and untrusted full-content logs redacted.

Use migrations from the first phase that needs a database. Do not create tables opportunistically at runtime.

## 4. Job lifecycle

Recommended analysis states:
`draft -> queued -> analyzing -> planning -> awaiting_approval`

Recommended run states:
`draft -> awaiting_target -> scheduled -> queued -> running -> analyzing_results -> completed`

Terminal alternatives:
`failed`, `cancelled`, `expired`, `completed_with_report_error`.

Keep analysis status separate from run status. A job may have a successful plan and multiple runs. State transitions must be explicit and validated in the service layer. Every transition should write an event/audit entry in the same transaction where possible.

## 5. Queue behavior

1. Insert job and initial event in a single transaction.
2. Workers poll for eligible rows or use `LISTEN/NOTIFY` as a wake-up hint with polling fallback.
3. Claim one row atomically using `FOR UPDATE SKIP LOCKED`.
4. Assign `worker_id`, increment attempt count, set `lease_expires_at` and `heartbeat_at`.
5. Commit before any Git, model, or tool work.
6. Renew the lease periodically.
7. Persist progress in bounded events.
8. Complete/fail only if the worker still owns the current lease/fencing token.
9. Reclaim expired leases and apply bounded exponential backoff for retryable errors.
10. Make result writes idempotent. Duplicate execution is possible after crashes, so avoid duplicate findings and use run/test identifiers as idempotency keys.

Do not use database row locks as a substitute for leases during long-running work.

## 6. Agent contract

The model receives:
- job goal and intensity;
- immutable repository snapshot metadata;
- structured discovery observations;
- catalog entries and their schemas;
- explicit limitations and policy constraints.

It returns:
- concise plan summary;
- structured test candidates or load scenarios;
- rationale and evidence/source references;
- prerequisites and required user inputs;
- risk classification;
- expected result and evidence;
- confidence and limitations.

Validate model output with a versioned JSON schema. Reject unknown tool IDs, unknown fields, invalid arguments, oversized output, or out-of-scope targets. Do not permit model output to mutate user approval or authorization records.

## 7. Repository inspection rules

- Pin a commit SHA before analysis.
- Clone into a unique temporary directory with disk quotas and timeouts.
- Use a Git library or fixed Git executable arguments; never concatenate a user URL into a shell command.
- Support HTTPS and SSH only through explicit policies; protect credentials and avoid credential leakage in logs.
- Do not install packages, run build scripts, run tests, start the app, or execute repository scripts.
- Ignore or carefully handle symlinks and paths that escape the workspace.
- Limit file count, total bytes, individual file size, nesting depth, and binary-file processing.
- Exclude `.git` internals and common generated/binary directories from agent context where appropriate.
- Treat all repository content as untrusted data. Prompt injection in comments/readmes must not change the tool policy.

## 8. Target execution safety

- Require an authorization record and explicit selected tests/profile.
- Normalize target URLs and validate schemes, ports, hostnames, IP ranges and redirects.
- Resolve and validate destination IPs at connection time to reduce DNS rebinding risk.
- Block loopback, private, link-local, multicast, reserved and metadata destinations by default.
- Restrict redirect destinations and cross-origin requests.
- Set hard timeouts, request/response byte limits, concurrency/rate/duration budgets, and kill-on-cancel behavior.
- Do not follow links discovered by scanners outside the approved scope.
- Default to non-destructive checks. High-impact tests need explicit separate approval.
- Redact credentials, cookies, personal data, and secrets from logs and reports.

## 9. Detailed results and event stream

Use SSE initially for dashboard live updates. Each event has sequence number, event type, timestamp, and structured data. Clients reconnect with the last event ID and receive missing events. Do not use SSE as the source of truth; PostgreSQL remains authoritative.

Event examples:
- `job.queued`
- `repository.clone.started`
- `repository.snapshot.ready`
- `agent.discovery.progress`
- `plan.proposed`
- `plan.approved`
- `target.validated`
- `run.scheduled`
- `test.started`
- `test.progress`
- `test.completed`
- `finding.recorded`
- `run.cancelled`
- `report.completed`

Do not emit one event per HTTP request during a high-volume load test. Persist aggregated metrics on intervals and final summaries.

## 10. Non-functional requirements

- Secure defaults and auditable authorization.
- Clear errors and actionable logs with correlation IDs.
- Idempotent API operations where retrying is expected.
- Bounded retries, output sizes, run durations, and resource consumption.
- Migrations and seed data reproducible from a clean checkout.
- Automated unit, integration, and contract tests.
- No secrets in source control, logs, screenshots, fixtures, or generated docs.
- Local development should not require cloud infrastructure.
- Tool execution must be disabled or mocked in tests unless a test explicitly targets a safe fixture.
