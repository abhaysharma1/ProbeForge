# 14a — Logging hardening report

**Status:** Complete.
**Scope:** Extend "proper" structured logging to every ProbeForge library and service under
one contract, without changing control flow or adding dependencies. Recorded as a Phase 14
hardening item (see D-043); no new numbered phase was opened.

## What changed

### One logging contract

Documented in `crates/observability` and `docs/OPERATIONS.md`:

- Levels: `error` (needs operator attention), `warn` (recoverable failure, security denial,
  lease loss), `info` (lifecycle/outcome transition), `debug` (per-step diagnosis). Hot paths
  stay at `debug`.
- Every line carries `service` and a stable `event` name plus the identifiers in scope
  (`request_id`, `project_id`, `job_id`, `run_id`, `worker_id`) and `error_code` on failures.
- Values are redacted before emission. Library crates emit `tracing` events but never
  initialize a subscriber; only service binaries call `init_tracing`.

### Rust libraries (previously emitted nothing)

- `crates/queue`: claims, heartbeats (warn on lost lease), completions, retries/failures,
  lease recovery, schedule create/promote/expire/cancel, quota updates, and retention purge.
- `crates/target-policy`: destination/scope/redirect/port denials (warn), DNS resolution and
  guarded-request completion (debug), budget cancellation (debug).
- `crates/execution-adapters`: adapter start/finish with status and outcome; unsupported
  catalog IDs fail closed with a warn.
- `crates/load-runner`: spec build, run start/finish, and metric persistence.

Each crate takes `tracing` (already a workspace pin). No version drift.

### Rust services

- `services/api`: mutations and denials across jobs, runs, plans, targets, quotas, projects,
  reports, admin/platform settings, and audit writes; auth decisions (login success/failure,
  logout, session revoke, CSRF rejection, login rate limit); API rate-limit rejection. The
  `http_request` span now carries `service`, `method`, and `path`.
- `services/execution-worker` / `services/scheduler`: added `event` names, `worker_id`, and
  correlation fields; per-stage scheduler messages moved to `debug` to avoid duplicating the
  queue crate's `info` events.

### Analysis worker (dependency-free)

- New `services/analysis-worker/src/logging/logger.ts`: levels, JSON/text output, child
  loggers with bound fields, and redaction (sensitive keys + `Bearer`/`Basic`/`Authorization:`/
  `Cookie:` markers + JWT-shaped tokens) mirroring `crates/observability`. Configured by
  `PROBE_FORGE_LOG_LEVEL` (default `info`) and `PROBE_FORGE_LOG_FORMAT` (default `json`),
  validated in `config.ts`.
- Wired through `index.ts`, `job-cli.ts`, `job-loop.ts`, and `ingest-cli.ts` (claim, discovery,
  plan, outcome, retry/failure, snapshot created).
- Logs counts, identifiers, and paths only — never repository file contents or secrets.

## Files

- `crates/observability/src/lib.rs` — contract documentation.
- `crates/{queue,target-policy,execution-adapters,load-runner}/` — events + `tracing` dependency.
- `services/api/src/{jobs,runs,plans,targets,quotas,projects,results,audit,http_middleware,admin}.rs`,
  `services/api/src/auth/mod.rs`.
- `services/{scheduler,execution-worker}/src/`.
- `services/analysis-worker/src/logging/logger.ts` (+ `logger.test.ts`), `config.ts`,
  `config.test.ts`, `index.ts`, `job-cli.ts`, `job-loop.ts`, `ingest-cli.ts`.
- `docs/OPERATIONS.md`, `docs/DECISION_LOG.md` (D-043), `docs/IMPLEMENTATION_CHECKLIST.md`.

## Checks run

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test -p probe-forge-observability -p probe-forge-queue -p probe-forge-target-policy -p probe-forge-execution-adapters -p probe-forge-load-runner --lib` | 31 passed, 0 failed |
| `bun run --cwd services/analysis-worker typecheck` | pass |
| `bun test` (analysis worker) | 125 passed, 10 skipped, 0 failed (includes 17 logger/config tests) |
| `bunx biome check` (changed worker files) | pass |
| `bun run check:docs` | pass |

## Security invariants verified

- No secret, token, cookie, request/response body, or repository file content is emitted;
  redaction runs before emission and has tests on both sides.
- Libraries never initialize a subscriber and never change behavior based on logging.
- Model/repository input cannot alter log policy.

## Known limitations

- Frontend logging was intentionally out of scope (no log collector in the browser).
- No external log shipping, alerting, or sampling; `/metricsz` and structured logs remain the
  interfaces.
- The worker's deeper library modules (discovery parsers, repository git/inspect internals)
  log at the job-loop boundary rather than per-file, to avoid noise and unbounded detail.

## Next phase

None opened. This hardening item is complete; the numbered phase sequence remains 0–15.
