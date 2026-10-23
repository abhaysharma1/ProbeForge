# ProbeForge

ProbeForge is an AI-orchestrated testing platform with exactly two job goals:

| Goal | What the user gets |
|---|---|
| `vulnerability_testing` | Repository analysis, proposed security tests from a vetted catalog, explicit plan approval, explicit target authorization, evidence-based results |
| `load_testing` | Repository analysis, endpoint-aware load scenarios, approved profile and schedule, isolated load execution, aggregate metrics |

The AI agent (Pi) inspects a pinned commit of the user's repository and proposes plans. Deterministic backend policy authorizes every action. PostgreSQL is the source of truth and the initial durable queue. Nothing is tested because a model suggested it: user approval and target authorization are mandatory.

## Status

**Phase 7 — Pi agent integration and plan generation: complete.** See the [phase report](docs/phases/07_planning_report.md).

- Next permitted phase: **Phase 8 — Test catalog and plan approval UI** (starts only after explicit authorization).
- Phase tasks and exit gates: [docs/IMPLEMENTATION_CHECKLIST.md](docs/IMPLEMENTATION_CHECKLIST.md)
- Decisions, interpretations, and open questions: [docs/DECISION_LOG.md](docs/DECISION_LOG.md)
- Contract details: [docs/CONTRACTS.md](docs/CONTRACTS.md) · Database details: [docs/DATABASE.md](docs/DATABASE.md) · Authentication and RBAC: [docs/AUTH.md](docs/AUTH.md) · Jobs and queue: [docs/JOBS.md](docs/JOBS.md) · Read-only ingestion: [docs/REPOSITORY_INGESTION.md](docs/REPOSITORY_INGESTION.md) · Static discovery: [docs/DISCOVERY.md](docs/DISCOVERY.md) · Pi planning: [docs/PLANNING.md](docs/PLANNING.md)
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
    +-- load execution worker                   k6 behind hard caps, isolated from the API
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
crates/queue/              Shared PostgreSQL job/run queue, leases, fencing, retries, recovery
packages/contracts/        JSON Schema, TS contracts, Ajv validators, shared fixtures
packages/test-catalog/     Versioned, non-executable test catalog scaffold
migrations/                Forward-only PostgreSQL migrations (0001-0007)
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
bun run db:migrate              # apply the schema through migration 0007
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
| `bun run check:db` | Migration, auth/RBAC, job API/SSE, and queue lease integration tests (requires `DATABASE_URL`) |
| `bun run check:services` | Starts all five services, verifies health endpoints, stops them |
| `bun run build` | Rust workspace build plus the production web build |
| `bun run format` | Biome format (TS/JSON) and `cargo fmt` |

`check:db` creates random temporary schemas and never touches `public`. It fails if `DATABASE_URL` is missing instead of silently skipping. `check:services` requires a prior `bun run build:rust`, a reachable migrated database, `DATABASE_URL`, and free loopback ports; the API and scheduler readiness endpoints now verify database state. Neither contacts external targets. CI is not configured yet; the documented local checks are the gate until a Git remote exists (decision D-011).

## Phase 5-7 scope and current limitations

- Read-only Git ingestion is implemented as an analysis-worker library and operator CLI (`bun run --cwd services/analysis-worker ingest -- ...`). It is not an HTTP endpoint and is not wired to the job queue or PostgreSQL yet.
- Supported remote transports are HTTPS with HTTP basic credentials and SSH with an unencrypted private-key secret reference. `file:` is accepted only through explicit local-fixture mode. Hosts, ports, paths, refs, and DNS answers are validated; redirects are disabled.
- The worker resolves branches/tags to a commit SHA, fetches only that commit into a unique workspace, enforces process/output/file/byte/depth/workspace limits, records symlinks and submodules without following them, and removes the workspace on success, failure, and cancellation.
- Package manifests and lockfiles are parsed as inert data. Script names may be inventoried, but script bodies are never executed. No install, build, test, application startup, migration, or arbitrary project script runs.
- Snapshot metadata and JSONL audit events can be produced, but snapshot/audit persistence to PostgreSQL and authenticated queue integration remain deferred.
- Authenticated `POST /api/jobs?projectId=...` still creates queued vulnerability-testing or load-testing jobs from the versioned `jobCreation` contract. Owner, admin, and collaborator roles may create/cancel; viewers are read-only. A concrete source reference is required, so no unresolvable job is enqueued. `Idempotency-Key` returns the existing job for an identical retry and returns `409` when reused for a different payload.
- The shared `crates/queue` uses `FOR UPDATE SKIP LOCKED`, commits claims before processing, and enforces worker leases, heartbeats, fencing, bounded exponential retries, and stale-lease recovery.
- Ordered `job_events` drive SSE. `Last-Event-ID`/`after` replay is implemented, with `follow=false` for a finite replay stream and safe `stream.error` events on database failure.
- The scheduler owns a PostgreSQL pool, reports database-aware readiness, promotes due scheduled runs to queued, and recovers expired job leases. Due-run claim logic is implemented and tested for the future execution dispatcher.
- Pi planning is implemented as an analysis-worker library (`services/analysis-worker/src/agent/`) with a constrained read-only/proposal tool registry, strict v1 validation, deterministic fallback, and a narrow PostgreSQL plan writer. It consumes the Phase 5 snapshot and Phase 6 inventory, but it is not yet wired to the job queue or exposed by an HTTP endpoint. The worker HTTP surface remains health-only.
- The planner can persist draft `test_plans`, `planned_tests`, `agent_sessions`, and audit records when a caller supplies a `PostgresPlanStore`. There is no authenticated planning API, plan review UI, approval endpoint, target authorization, execution, or reporting yet (Phases 8-13).
- The vetted catalog is still empty, so a vulnerability-testing fallback cannot be produced until Phase 8 adds catalog entries. Load fallback is limited to sorted literal `GET`/`HEAD` routes and a conservative baseline profile.
- The web app still shows the Phase 3 project workspace; job creation/status and plan review UI work is deferred to Phase 8+.
- Versioned v1 contracts now include `job.claimed` and `job.retry_scheduled` events; catalog tests remain empty but validated.
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
