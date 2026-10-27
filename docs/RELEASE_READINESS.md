# Release readiness

A checklist an authorized operator can follow before a segmented, manually
approved staging smoke test. The automated end-to-end acceptance runs against
inert fixtures and a local mock only; it never contacts a real target.

## What is supported

- Two job goals: `vulnerability_testing` and `load_testing`.
- Read-only repository ingestion (pinned commit), deterministic static discovery,
  and deterministic plan generation. An optional model planner can be configured
  but is not required; the fallback planner is always available.
- Plan review, selection, and immutable versioned approval.
- Target registration, server-owned URL/DNS/IP policy, and time-bounded
  authorization.
- Run materialization from an approved plan plus an active authorization.
- Execution through vetted adapters (`http.headers.missing`,
  `http.cors.misconfiguration`, `http.auth.rate-limit`, `http.cookies.insecure`,
  `http.info-disclosure`, `network.tls.baseline`, `static.secrets.exposure`) and
  the constrained native load runner.
- Results, evidence metadata, traceable recommendations, versioned reports,
  rerun comparison, JSON/Markdown export, and a web run view.
- Observability, per-project quotas, API rate limits, retention, and backup.

## What is not supported

- No security scanners or arbitrary commands/scripts; only the vetted catalog
  adapters execute.
- No k6 or external load tool; load generation is the native constrained runner.
- No secret manager; secret references are validated but not resolved to values.
  Credentialed repositories cannot be materialized by the execution worker for
  static adapters and are reported as unavailable instead of scanned.
- Static adapters require a `git` binary in the execution worker environment and
  re-materialize the job's pinned commit into a temporary workspace.
- No autonomous execution: the analysis job loop and the execution dispatch are
  operator-invoked binaries, not background daemons, so a human (or an external
  supervisor) drives each stage.
- No external metrics/alerts backend; `/metricsz` and structured logs are the
  interfaces.

## Prerequisites

- Rust 1.98.1 and Bun 1.4.4 (see `rust-toolchain.toml`, `package.json`).
- PostgreSQL 18.6 via `bun run db:up` (or an equivalent instance).
- `git` and `docker` on PATH (git for ingestion, docker for the local database
  and backup tooling).

## Automated checks (must all pass)

```bash
bun install --frozen-lockfile
cargo build --locked
bun run check                                   # docs, fmt, clippy, unit tests, lint, typecheck
DATABASE_URL=... bun run db:migrate             # apply migrations through the latest
DATABASE_URL=... bun run check:db               # DB integration + end-to-end acceptance
bun run check:services                          # all five services start and report healthy
DATABASE_URL=... bun run db:backup:check        # backup/restore roundtrip
```

## End-to-end workflow runbook (fixtures and a local mock only)

1. Start services and migrate: `bun run db:up`, `bun run db:migrate`.
2. Provision an operator account: `bun run user:create -- you@example.test "You"`.
3. Create a project and a job pointing at an **inert fixture** repository.
4. Produce a plan: `bun run --cwd services/analysis-worker plan`. The job moves
   to `awaiting_approval` with a deterministic draft plan.
5. Review and approve the plan (web UI or `POST /api/plans/{id}/approve`).
6. Register and authorize a target; for a local mock, keep the authorization
   scope exact and enable `PROBE_FORGE_ALLOW_PRIVATE_TARGETS` only locally.
7. Create runs: `POST /api/plans/{id}/runs` with the authorization id.
8. Execute queued runs: `cargo run -p probe-forge-execution-worker --bin dispatch -- --once`
   (or run the built `dispatch` binary). Vulnerability runs write results and
   evidence; load runs write metrics.
9. Generate a report: `POST /api/runs/{id}/report`, then read it in the web app.
10. Stop services. Confirm ports are released.

## Security invariants to re-check

- Repository content is never executed; ingestion is read-only and pinned.
- Every request passes target-scope, redirect, DNS, and budget policy below the
  dispatcher; out-of-scope routes are rejected before any request.
- No run exists without an immutable approval, and no execution without an
  active authorization.
- Model output never becomes an executable command; only catalog IDs are used.
- Secrets are redacted before logs, reports, and exports.

## Manual staging smoke test (separate, explicitly approved)

Only after every check above passes may an authorized operator run a bounded
smoke test against a staging target they own, with an approved scope, an approved
test selection, and explicit rate/duration caps. This is never part of the
automated suite.

## Known limitations

- Dispatch and analysis are operator-invoked, not background workers; no
  supervisor, scheduler, or queue-drain daemon runs them automatically.
- A cancelled or failed load dispatch is retried only for transient target
  errors; scope violations fail closed without retry.
- Retention deletes but does not archive immutable rows.
- The recommendation generator is deterministic; a model-backed generator is not
  wired.
