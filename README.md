# ProbeForge

ProbeForge is an AI-orchestrated testing platform with exactly two job goals:

| Goal | What the user gets |
|---|---|
| `vulnerability_testing` | Repository analysis, proposed security tests from a vetted catalog, explicit plan approval, explicit target authorization, evidence-based results |
| `load_testing` | Repository analysis, endpoint-aware load scenarios, approved profile and schedule, isolated load execution, aggregate metrics |

The AI agent (Pi) inspects a pinned commit of the user's repository and proposes plans. Deterministic backend policy authorizes every action. PostgreSQL is the source of truth and the initial durable queue. Nothing is tested because a model suggested it: user approval and target authorization are mandatory.

## Status

**Phase 0 — repository reconnaissance and implementation contract: complete.**

This repository currently contains documentation and a repository contract only. There is no product code, no build, and no service to run yet.

- Next permitted phase: **Phase 1 — Monorepo skeleton and local development** (starts only after explicit authorization).
- Phase tasks and exit gates: [docs/IMPLEMENTATION_CHECKLIST.md](docs/IMPLEMENTATION_CHECKLIST.md)
- Decisions, interpretations, and open questions: [docs/DECISION_LOG.md](docs/DECISION_LOG.md)
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

## Planned repository layout (Phase 1)

```text
apps/web/                  React + TypeScript + Vite dashboard
services/api/              Rust + Axum control plane
services/analysis-worker/  TypeScript + Pi SDK planning worker
services/execution-worker/ security adapters and constrained load runner
services/scheduler/        durable due-run dispatcher
packages/contracts/        shared versioned API/event/plan contracts
packages/test-catalog/     versioned catalog of supported tests
infra/local/               Docker Compose for PostgreSQL and local support
docs/                      normative project documents and phase reports
```

## Documentation

The five numbered documents in [docs/](docs/) are normative. If they conflict, preserve security and data integrity first, follow the phase gates in [03_phase_by_phase_implementation_plan.md](docs/03_phase_by_phase_implementation_plan.md), and record the interpretation in the [decision log](docs/DECISION_LOG.md) instead of silently choosing a risky reading. See [docs/README.md](docs/README.md) for the documentation index.

## Documented checks

The repository contract check is runnable today and requires only Node.js built-ins (no dependencies installed):

| Area | Command | Status |
|---|---|---|
| Documentation contract | `node scripts/check-docs.mjs` | Available now |
| Rust formatting | `cargo fmt --all -- --check` | Wired in Phase 1 |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` | Wired in Phase 1 |
| Rust tests | `cargo test --workspace` | Wired in Phase 1 |
| TypeScript type check | `<package manager> typecheck` | Wired in Phase 1 |
| TypeScript lint/format | `<package manager> lint` / `format:check` | Wired in Phase 1 |
| TypeScript tests | `<package manager> test` | Wired in Phase 1 |
| Everything | single root entry command | Added in Phase 1 |

Package manager, worker runtime, and dependency versions are confirmed and pinned in Phase 1 (see decisions D-002, D-003, D-004, D-007). Floating `latest` tags are not allowed.

## Local development

- PostgreSQL runs locally through the reproducible Compose file in `infra/local/` (added in Phase 1).
- No cloud resources or paid services are required or used during development.
- Development tests use inert fixture repositories and local mock targets only: no real scans, no real load, no external traffic.
- Installing the platform's own dependencies is allowed; installing dependencies from inspected repositories is never allowed.

## Security invariants

- Repository analysis is read-only and never executes repository content.
- A model plan is a proposal, not authorization. Backend policy validates every execution.
- Only vetted, version-pinned catalog tests and typed adapters may run; no arbitrary shell commands or executable paths.
- Targets are validated by scheme, port, scope, DNS answers, redirects, and actual connection destination; loopback, private, link-local, reserved, and cloud metadata addresses are denied by default.
- Every run has hard time, concurrency, rate, request, response-size, and output budgets; cancellation terminates work.
- Credentials stay in secret references, never in prompts, logs, traces, reports, fixtures, URLs, or source control.
- The frontend never receives database credentials and never decides whether a target is safe.

## Contributing

1. Read the five numbered documents in [docs/](docs/) and [Agents.md](Agents.md) before changing anything.
2. Work only on the current phase; stop at its exit gate and do not start the next phase until it passes.
3. Tick checklist items only when the implementation exists and the evidence was actually observed.
4. Record non-obvious decisions and interpretations in the [decision log](docs/DECISION_LOG.md).
5. Write a phase report in [docs/phases/](docs/phases/) and state `PHASE N: PASS` or `PHASE N: BLOCKED` with real command results.
6. Never weaken a security control to make a check pass. Add a regression test instead.

## Product identity

The product name is **ProbeForge** and is used consistently in the README, UI, page metadata, and user-facing copy. No domain or trademark availability has been checked or is claimed.
