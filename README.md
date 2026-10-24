# ProbeForge

ProbeForge is an AI-orchestrated testing platform with exactly two job goals:

| Goal | What the user gets |
|---|---|
| `vulnerability_testing` | Repository analysis, proposed security tests from a vetted catalog, explicit plan approval, explicit target authorization, evidence-based results |
| `load_testing` | Repository analysis, endpoint-aware load scenarios, approved profile and schedule, isolated load execution, aggregate metrics |

The AI agent (Pi) inspects a pinned commit of the user's repository and proposes plans. Deterministic backend policy authorizes every action. PostgreSQL is the source of truth and the initial durable queue. Nothing is tested because a model suggested it: user approval and target authorization are mandatory.

## Status

**Phase 14 - Observability, quotas, and operational hardening: complete.** See the [phase report](docs/phases/14_observability_operations_report.md).

- Next permitted phase: **Phase 15 - End-to-end acceptance using local fixtures** (starts only after explicit authorization).
- Phase tasks and exit gates: [docs/IMPLEMENTATION_CHECKLIST.md](docs/IMPLEMENTATION_CHECKLIST.md)
- Decisions, interpretations, and open questions: [docs/DECISION_LOG.md](docs/DECISION_LOG.md)
- Contract details: [docs/CONTRACTS.md](docs/CONTRACTS.md) · Database details: [docs/DATABASE.md](docs/DATABASE.md) · Authentication and RBAC: [docs/AUTH.md](docs/AUTH.md) · Jobs and queue: [docs/JOBS.md](docs/JOBS.md) · Read-only ingestion: [docs/REPOSITORY_INGESTION.md](docs/REPOSITORY_INGESTION.md) · Static discovery: [docs/DISCOVERY.md](docs/DISCOVERY.md) · Pi planning: [docs/PLANNING.md](docs/PLANNING.md) · Plan approval: [docs/PLAN_APPROVAL.md](docs/PLAN_APPROVAL.md) · Target validation: [docs/TARGET_VALIDATION.md](docs/TARGET_VALIDATION.md) · Execution adapters: [docs/EXECUTION_ADAPTERS.md](docs/EXECUTION_ADAPTERS.md) · Load scenarios: [docs/LOAD_SCENARIOS.md](docs/LOAD_SCENARIOS.md) · Load execution: [docs/LOAD_EXECUTION.md](docs/LOAD_EXECUTION.md) · Results and reports: [docs/RESULTS_REPORTS.md](docs/RESULTS_REPORTS.md) · Operations: [docs/OPERATIONS.md](docs/OPERATIONS.md)
- Phase reports: [docs/phases/](docs/phases/)

## Architecture at a glance

```text
Browser (React SPA)
    | HTTPS: authenticated API + SSE
    v
Rust control plane (Axum + Tokio)
    | authn/authz - validation - job/run state - approvals - scheduling - audit
    v
PostgreSQL  <- durable source of truth and initial job queue (leases, heartbeats)
    |
    +-- analysis worker (TypeScript + Pi SDK)   read-only repository inspection, planning
    +-- scheduler / dispatcher                  due runs, leases, cancellation
    +-- security execution worker               approved catalog adapters only
    +-- load execution worker                   constrained load runner behind hard caps, isolated from the API
```

Boundaries that must not be crossed:

- The browser never connects to PostgreSQL and never holds database credentials.
- The analysis worker never executes repository code, installs dependencies, runs build scripts, runs project tests, or starts the inspected application.
- Pi gets typed, bounded tools - never unrestricted shell access or direct database credentials.
- Execution happens only after persisted approval and target authorization, with hard budgets and cancellation that stops work.
- Security execution and load generation are isolated from the control plane by process and resource boundaries.

## Repository layout

```text
apps/web/                  React 19 + TypeScript + Vite 8 + Tailwind 4 dashboard
services/api/              Rust + Axum control plane (auth, projects/RBAC, jobs/SSE, readiness)
services/analysis-worker/  Bun + TypeScript worker (health, read-only ingestion, discovery, Pi planning)
services/execution-worker/ Rust execution-worker skeleton (health endpoints)
services/scheduler/        Rust scheduler skeleton (health endpoints)
crates/contracts/          Rust contract types, validation, and shared-fixture tests
crates/execution-adapters/ Native, typed security adapters and catalog-to-adapter registry
crates/queue/              Shared PostgreSQL job/run queue, leases, fences, retries, recovery
crates/target-policy/      Shared target URL/DNS/IP, redirect, request-budget, and guarded HTTP policy
packages/contracts/        JSON Schema, TS contracts, Ajv validators, shared fixtures
packages/test-catalog/     Versioned, non-executable test catalog (schema 1, catalog 0.2.0)
migrations/                Forward-only PostgreSQL migrations (0001-0011)
infra/local/               Docker Compose for local PostgreSQL
scripts/                   Repository contract check, database check, and local service runner
docs/                      Normative project documents, contracts/database docs, and phase reports
```

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| [Bun](https://bun.sh) | 1.4.4 | Product TypeScript runtime, workspace installs, repo scripts |
| Rust (rustup) | 1.98.1 | Control plane and worker services; pinned in `rust-toolchain.toml` |
| Docker Engine + Compose v2 | 29.x / v5.x | Local PostgreSQL only |
| Node.js | 24.15.0 (`.nvmrc`) | Build tooling only (Vite/Vitest CLIs) |
| Git | any recent | Version control |

No cloud resources, API keys, or model-provider credentials are required for local development or the documented checks.

## Quick start

```bash
bun install                     # install workspace dependencies (lockfile: bun.lock)
cp .env.example .env            # PowerShell: Copy-Item .env.example .env
# set POSTGRES_PASSWORD in .env to a local-only value
bun run db:up                   # start PostgreSQL 18.6 (loopback only)
# export DATABASE_URL with the same password (see .env.example)
# PowerShell: $env:DATABASE_URL = "postgres://probeforge:<password>@127.0.0.1:5432/probeforge"
bun run db:migrate              # apply the schema through migration 0011
bun run user:create -- owner@example.test "Owner"   # prompts for the password
bun run build:rust              # build the Rust workspace
bun run dev                     # start all five local services (DATABASE_URL required)
```

Local endpoints (loopback only):

| Service | URL |
|---|---|
| Web dashboard | http://127.0.0.1:5173 |
| API liveness / readiness | http://127.0.0.1:8080/healthz · `/readyz` |
| Scheduler | http://127.0.0.1:8081/healthz · `/readyz` |
| Execution worker | http://127.0.0.1:8082/healthz · `/readyz` |
| Analysis worker | http://127.0.0.1:8090/healthz · `/readyz` |
| PostgreSQL | `127.0.0.1:5432` (Compose) |

Stop PostgreSQL with `bun run db:down`.

## Checks

| Command | What it verifies |
|---|---|
| `bun run check` | Documentation contract, Rust fmt/clippy/tests, TypeScript lint/typecheck/tests |
| `bun run check:docs` | Required files, doc links, Phase 0-15 coverage, `.env` hygiene, secret patterns |
| `bun run check:rust` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace` |
| `bun run check:ts` | Biome lint, strict type checks, Bun/Vitest tests for all TS packages |
| `bun run check:db` | Migration, auth/RBAC, job API/SSE, plan approval, target authorization, and queue lease integration tests (requires `DATABASE_URL`) |
| `bun run check:services` | Starts all five services, verifies health endpoints, stops them |
| `bun run build` | Rust workspace build plus the production web build |
| `bun run format` | Biome format (TS/JSON) and `cargo fmt` |

`check:db` creates random temporary schemas and never touches `public`. It fails if `DATABASE_URL` is missing instead of silently skipping. `check:services` requires a prior `bun run build:rust`, a reachable migrated database, `DATABASE_URL`, and free loopback ports; the API and scheduler readiness endpoints now verify database state. Neither contacts external targets. CI is not configured yet; the documented local checks are the gate until a Git remote exists (decision D-011).

## Phase 5-11 scope and current limitations

- Read-only Git ingestion is implemented as an analysis-worker library and operator CLI (`bun run --cwd services/analysis-worker ingest -- ...`). It is not an HTTP endpoint and is not wired to the job queue or PostgreSQL yet.
- Supported remote transports are HTTPS with HTTP basic credentials and SSH with an unencrypted private-key secret reference. `file:` is accepted only through explicit local-fixture mode. Hosts, ports, paths, refs, and DNS answers are validated; redirects are disabled.
- The worker resolves branches/tags to a commit SHA, fetches only that commit into a unique workspace, enforces process/output/file/byte/depth/workspace limits, records symlinks and submodules without following them, and removes the workspace on success, failure, and cancellation.
- Package manifests and lockfiles are parsed as inert data. Script names may be inventoried, but script bodies are never executed. No install, build, test, application startup, migration, or arbitrary project script runs.
- Snapshot metadata and JSONL audit events can be produced, but snapshot/audit persistence to PostgreSQL and authenticated queue integration remain deferred.
- Authenticated `POST /api/jobs?projectId=...` still creates queued vulnerability-testing or load-testing jobs from the versioned `jobCreation` contract. Owner, admin, and collaborator roles may create/cancel; viewers are read-only. A concrete source reference is required, so no unresolvable job is enqueued. `Idempotency-Key` returns the existing job for an identical retry and returns `409` when reused for a different payload.
- The shared `crates/queue` uses `FOR UPDATE SKIP LOCKED`, commits claims before processing, and enforces worker leases, heartbeats, fencing, bounded exponential retries, and stale-lease recovery.
- Ordered `job_events` drive SSE. `Last-Event-ID`/`after` replay is implemented, with `follow=false` for a finite replay stream and safe `stream.error` events on database failure.
- The scheduler owns a PostgreSQL pool, reports database-aware readiness, expires and promotes due schedules, promotes due scheduled runs to queued, and recovers expired run/job leases. Fair due-run claim logic with global/per-project quotas and worker leases is implemented and tested for the future execution dispatcher.
- Pi planning is implemented as an analysis-worker library (`services/analysis-worker/src/agent/`) with a constrained read-only/proposal tool registry, strict v1 validation, deterministic fallback, and a narrow PostgreSQL plan writer. It consumes the Phase 5 snapshot and Phase 6 inventory, and the control plane now exposes authenticated plan review/approval APIs. It is still not wired to an authenticated worker claim/dispatch path; the worker HTTP surface remains health-only.
- Plans can be reviewed, selected, and approved with an immutable snapshot. Targets can be registered, validated against server-owned URL/DNS/IP policy, and authorized for a bounded time window. Run creation and dispatch remain absent; reporting is Phase 13.
- Target management is API-only for now; there is no target registration or authorization UI. The guarded HTTP client and run budget are used by the Phase 10 native adapters and the Phase 12 load runner, but no real scan or load test is executed.
- The catalog contains inert entries (catalog 0.2.0). Only `http.headers.missing` and `http.cors.misconfiguration` have vetted native adapters (`http_probe` 0.1.0); every other entry is reported unavailable. Approval records the selected test set and does not start a run.
- The Phase 10 adapter library (`crates/execution-adapters`) returns normalized, sanitized in-memory results. It is not dispatched by the worker, not persisted, and has no UI. No shell or external executable is used.
- Phase 11 load scenarios are ordered, correlated, data-only steps with secret-backed headers, extraction/assertion rules, explicit statuses, and platform-capped profiles. They can be generated and approved without sending traffic.
- Phase 12 adds a constrained native load runner (`crates/load-runner`) that executes a typed `RunnerSpec` under hard caps with extraction/assertion rules, correlation, metrics, stop conditions, and cancellation. k6 is not installed or invoked; a future adapter must consume the same spec behind a pinned process boundary. The execution worker stays health-only, so no API or UI starts live traffic yet.
- Schedules persist a normalized timezone and a UTC `next_run_at`, are promoted idempotently per occurrence, and require an explicit active target authorization. Schedule creation is a server library operation; there is no schedule API or UI yet.
- Load scenarios are versioned by their parent plan version. Approval revalidates selected scenarios and pins the exact JSON/profile in the immutable snapshot; there is no independent scenario-edit endpoint.
- Target-scope validation for load paths is called by the Phase 12 runner before any traffic; extracted values are percent-encoded before substitution.
- Phase 13 exposes run results, evidence metadata, traceable recommendations, versioned reports, redaction, export, and rerun comparison through the authenticated JSON API (`services/api/src/results.rs`) and the web app (`apps/web/src/RunReport.tsx`, reached from a project's **Runs** list). Generated summaries are separate from raw results, and every report claim cites a stored result/evidence/metric or is labeled a hypothesis. The execution worker does not yet write results automatically.
- Phase 14 adds a shared observability crate (structured logs, correlation IDs, one redaction implementation), an internal `GET /metricsz` queue/worker metrics endpoint, server-enforced per-project quotas and request budgets (`project_quotas`, `GET|PUT /api/projects/{id}/quota`), per-user API and expensive-operation rate limits, a scheduler retention purge through a single opt-in path, and a tested backup/restore roundtrip. Operators use `docs/OPERATIONS.md`. The execution worker is still health-only, so no live run is dispatched yet.
- The web app now shows analysis jobs and a focused plan review, but there is still no job creation UI. Jobs are created through the API and plans must come from the worker/seed data until the authenticated job path is completed.
- Versioned v1 contracts now include `job.claimed` and `job.retry_scheduled` events; catalog entries are populated and validated in TypeScript and Rust.
- All services bind to loopback by default.

## Security invariants

- Repository analysis is read-only and never executes repository content.
- A model plan is a proposal, not authorization. Backend policy validates every execution.
- Only vetted, version-pinned catalog tests and typed adapters may run; no arbitrary shell commands or executable paths.
- Targets are validated by scheme, port, scope, DNS answers, redirects, and actual connection destination; loopback, private, link-local, reserved, and cloud metadata addresses are denied by default.
- Every run has hard time, concurrency, rate, request, response-size, and output budgets; cancellation terminates work.
- Credentials stay in secret references, never in prompts, logs, traces, reports, fixtures, URLs, or source control.
- The frontend never receives database credentials and never decides whether a target is safe.

## Contributing

1. Read the six numbered documents in [docs/](docs/) and [Agents.md](Agents.md) before changing anything.
2. Work only on the current phase; stop at its exit gate and do not start the next phase until it passes.
3. Tick checklist items only when the implementation exists and the evidence was actually observed.
4. Record non-obvious decisions and interpretations in the [decision log](docs/DECISION_LOG.md).
5. Write a phase report in [docs/phases/](docs/phases/) and state `PHASE N: PASS` or `PHASE N: BLOCKED` with real command results.
6. Never weaken a security control to make a check pass. Add a regression test instead.

## Product identity

The product name is **ProbeForge** and is used consistently in the README, UI, page metadata, and user-facing copy. No domain or trademark availability has been checked or is claimed.
