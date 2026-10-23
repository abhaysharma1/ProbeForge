# Phase 1 report - Monorepo skeleton and local development

- **Phase:** 1 of 15
- **Date:** 2026-10-23
- **Status:** PHASE 1: PASS
- **Next permitted phase:** Phase 2 - Contracts, environment configuration, and database migrations (after explicit authorization)

## Implemented

1. **Bun workspace and root command interface.** Root `package.json` declares Bun 1.4.4 (`packageManager`, `engines`) and workspaces for `apps/*`, `services/analysis-worker`, and `packages/*`. `bun.lock` is committed. Root commands cover `dev`, `check`, `check:docs`, `check:rust`, `check:ts`, `check:services`, `lint`, `typecheck`, `test:ts`, `format`, `build`, `build:rust`, `build:web`, `db:up`, and `db:down`.
2. **React frontend.** `apps/web` is a React 19.3.1 + TypeScript 7.0.2 + Vite 8.3.5 + Tailwind CSS 4.3.3 app. It renders the two job goals from the shared contract, states the approval/authorization boundary, and has accessible landmarks and headings. Vitest 5 with jsdom and Testing Library covers rendering and the safety copy, with explicit cleanup between tests.
3. **Rust control plane.** `services/api` is an Axum 0.8.9 service with `GET /healthz` and `GET /readyz`, 30-second request timeout, 1 MiB request-body limit, tracing, loopback-only default bind (`127.0.0.1:8080`), and graceful shutdown on Ctrl+C/SIGTERM.
4. **Scheduler and execution-worker skeletons.** `services/scheduler` (port 8081) and `services/execution-worker` (port 8082) use the same minimal health/config/logging/shutdown pattern. They contain no job, adapter, or policy logic and are labeled as skeletons (D-012).
5. **Analysis worker (Bun).** `services/analysis-worker` runs under Bun 1.4.4, binds to `127.0.0.1:8090` by default, serves `/healthz` and `/readyz`, validates configuration, shuts down on SIGINT/SIGTERM, and pins `@earendil-works/pi-agent-core` 1.4.0. A test proves the pinned SDK loads under Bun; no model calls, repository access, or planning tools exist yet (Phase 7).
6. **Shared contracts.** `packages/contracts` exports the two job goals, the four service names, a job-goal guard, and typed health/readiness responses used by the web app and analysis worker. Phase 2 replaces the temporary cross-language mirror with versioned schemas (D-010).
7. **Test catalog scaffold.** `packages/test-catalog` ships `catalog.json` with `schemaVersion: "1"` and no entries, plus a strict loader that rejects executable fields (`command`, `executable`, `script`, `shell`), unknown categories/risks, and unstable IDs. Real entries belong to Phase 8 (D-014).
8. **Local PostgreSQL.** `infra/local/compose.yaml` pins `postgres:18.6-alpine`, publishes only to `127.0.0.1`, requires `POSTGRES_PASSWORD` with no default, uses the PostgreSQL 18 volume layout, includes a `pg_isready` healthcheck, and sets `no-new-privileges` (D-015).
9. **Environment template.** `.env.example` contains loopback addresses and placeholder-only database values. `.env` is ignored; the contract check fails if a real `.env` is tracked (D-016).
10. **Runnable checks.** `scripts/check-docs.mjs` validates required files, doc links, Phase 0-15 coverage, environment hygiene, and high-signal secret patterns. `scripts/services.mjs` starts all five services, polls their health endpoints, and tears them down; it refuses to start when a port is already served.
11. **Pinned toolchain.** Rust 1.98.1 via `rust-toolchain.toml` (clippy + rustfmt, minimal profile), MSRV `rust-version = "1.98"`, edition 2024, resolver 3. Node.js 24.15.0 is pinned in `.nvmrc` as build-only tooling for the Vite/Vitest CLIs (D-004). JavaScript and Rust dependency versions are exact and locked.

## Files changed

| Area | Files |
|---|---|
| Root workspace and tooling | `package.json`, `bun.lock`, `tsconfig.base.json`, `biome.json`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.nvmrc`, `.env.example` |
| Frontend | `apps/web/package.json`, `apps/web/tsconfig.json`, `apps/web/vite.config.ts`, `apps/web/index.html`, `apps/web/src/main.tsx`, `apps/web/src/App.tsx`, `apps/web/src/index.css`, `apps/web/src/test-setup.ts`, `apps/web/src/App.test.tsx` |
| API | `services/api/Cargo.toml`, `services/api/src/lib.rs`, `services/api/src/main.rs` |
| Scheduler | `services/scheduler/Cargo.toml`, `services/scheduler/src/lib.rs`, `services/scheduler/src/main.rs` |
| Execution worker | `services/execution-worker/Cargo.toml`, `services/execution-worker/src/lib.rs`, `services/execution-worker/src/main.rs` |
| Analysis worker | `services/analysis-worker/package.json`, `tsconfig.json`, `src/index.ts`, `src/config.ts`, `src/health.ts`, `src/config.test.ts`, `src/health.test.ts`, `src/pi-sdk.test.ts` |
| Contracts | `packages/contracts/package.json`, `tsconfig.json`, `src/index.ts`, `src/index.test.ts` |
| Test catalog | `packages/test-catalog/package.json`, `tsconfig.json`, `catalog.json`, `src/index.ts`, `src/index.test.ts` |
| Local infrastructure | `infra/local/compose.yaml` |
| Scripts | `scripts/check-docs.mjs`, `scripts/services.mjs` |
| Documentation | `README.md`, `Agents.md` (check command pointer only), `docs/README.md`, `docs/IMPLEMENTATION_CHECKLIST.md`, `docs/DECISION_LOG.md`, this report |

Generated but intentionally not committed: `node_modules/`, `target/`, `apps/web/dist/`. No `.env` was created.

## Validation evidence

Commands were actually run from `D:\Projects\ProbeForge`.

| # | Command | Actual outcome |
|---|---|---|
| 1 | `bun --version` | `1.4.4` (installed as a pinned platform tool via npm; Bun was not previously available) |
| 2 | `bun install` | `205 packages installed [71.39s]`; lockfile saved; 2 lifecycle scripts blocked by Bun's default policy (`@google/genai` no-op preinstall, `protobufjs` postinstall) |
| 3 | `bun install --frozen-lockfile` | `Checked 211 installs across 289 packages (no changes) [56.00ms]` - no lockfile drift |
| 4 | `cargo build --workspace` | Rust 1.98.1 toolchain auto-installed from the pin; workspace compiled successfully |
| 5 | `cargo build --locked --workspace` | `Finished dev profile ... in 3.42s` - no lockfile drift |
| 6 | `cargo fmt --all` then `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0; no warnings. The deprecated `TimeoutLayer::new` was replaced with `TimeoutLayer::with_status_code` in all three services |
| 7 | `cargo test --workspace` | 15 tests passed, 0 failed (5 per service: health, readiness, 404, valid config, invalid config) |
| 8 | `bunx biome migrate --write` then `bunx biome check --write .` then `bun run lint` | Config migrated to the 2.6.2 schema; `Checked 32 files ... No fixes applied` |
| 9 | `bun run typecheck` | All four TypeScript packages exited 0: contracts, test-catalog, web, analysis-worker |
| 10 | `bun run test:ts` | 16 tests passed: contracts 3, test-catalog 4, analysis-worker 7 (including Pi SDK load), web 2 |
| 11 | `bun run build:web` | `17 modules transformed`; `dist` produced (`index.html` 0.52 kB, CSS 11.63 kB, JS 222.35 kB) |
| 12 | `bun run check:services` | `PASS: 5/5 local services healthy` - API 8080, scheduler 8081, execution worker 8082, analysis worker 8090, web 5173, each checked over loopback |
| 13 | `docker compose -f infra/local/compose.yaml config --quiet` + `up -d --wait` + `ps` + `down -v` | Image `postgres:18.6-alpine` pulled; container healthy; named volume removed by `down -v`. Docker Desktop had to be started manually first; the daemon was not running |
| 14 | `bun scripts/check-docs.mjs` (before this report existed) | `FAIL: 1 contract issue(s)` - only the missing `docs/phases/01_monorepo_report.md`, proving the required-file check fails closed |
| 15 | Listener check on ports 8080, 8081, 8082, 8090, 5173, 5432 after teardown | No listeners remained; the service runner's process-tree cleanup worked |
| 16 | `bun run check` (after this report) | `PASS: repository contract is satisfied` (28/28 files); Rust fmt/clippy clean with 15 tests passed; Biome clean on 32 files; 4 typechecks and 16 TypeScript tests passed |

**Issues found and fixed during validation:**

- `bun --cwd apps/web run build` is invalid syntax; corrected to `bun run --cwd apps/web build` after the CLI printed its usage.
- `tower-http` deprecates `TimeoutLayer::new`; replaced to keep `clippy -D warnings` clean.
- React Testing Library did not auto-clean between Vitest tests (globals disabled), causing duplicate matches; added explicit `afterEach(cleanup)`.
- The first Pi SDK import exceeded Bun's 5-second test timeout; test timeout raised to 30 seconds (import now completes in about 1 second warm).
- Biome 2.6 deprecated `rules.recommended`; migrated to `rules.preset` and excluded Markdown from formatting.

**Not run (and why):**

- Any real security scan, load test, model call, or external HTTP target - prohibited during development. The only network traffic was package installation, the Docker image pull, and loopback health checks.
- API database integration - deferred to Phase 2 (D-013).
- Graceful-shutdown drain under load - signal handlers and server shutdown paths are implemented, but the automated check terminates process trees (`taskkill /T /F` on Windows); no drained-connection test exists yet.
- CI - no Git remote exists (Q-004, D-011).

## Security review

- **No project execution:** no inspected repository, project code, dependency, build script, or test was run. The only executed code is ProbeForge's own services, tests, and scripts.
- **No secrets:** `.env.example` contains placeholders only; no `.env` was created or committed. `POSTGRES_PASSWORD` is mandatory with no default. The secret scan found no high-signal patterns. Local verification used a throwaway inline value passed to Compose, never written to a file.
- **No credentials in logs or URLs:** health responses expose only service name, status, and version. No API keys or model-provider credentials are required yet.
- **Least privilege and binding:** every service defaults to loopback; PostgreSQL is published to loopback only. Request timeout and body-size limits are enabled on the Rust services. No service runs as root, listens publicly, or reaches the network in tests.
- **AI boundary:** the Pi SDK is pinned and load-tested only. There are no tools, model calls, prompts, repository readers, or execution paths, so no model output can authorize anything in this phase.
- **Catalog invariant:** the catalog loader rejects executable fields, so a command cannot be introduced as data even before real entries exist.
- **Environment hygiene:** the contract check fails if a real `.env` is tracked and still fails closed without Git.
- **Dependency posture:** versions are exact and locked. Bun blocked two transitive lifecycle scripts by default and they remain blocked; the Pi SDK imports and tests successfully without them. A full license/ownership review remains open (Q-002).
- **No weakened controls:** no security check or assertion was relaxed to make this phase pass; the failures above were fixed in code or configuration.

## Remaining issues

1. **No database wiring.** No service opens a PostgreSQL connection; readiness is static. Phase 2 adds migrations, a pool, and database-aware readiness (D-013).
2. **Skeleton services.** Scheduler and execution worker have no queue, adapter, load-runner, or policy behavior; the analysis worker has no planning session or tools (D-012). They must not be presented as functional testing features.
3. **Empty catalog.** No test definitions exist yet; Phase 8 adds vetted, version-pinned entries and adapter mapping (D-014).
4. **No authentication, authorization, tenant model, job API, SSE, target validation, execution, or reporting.** These are Phases 3-13; all current endpoints are unauthenticated loopback health probes.
5. **No CORS or frontend-to-API call.** The web app is a static shell; cross-origin policy will be designed with the first real API use rather than guessed now.
6. **Node.js tooling pin.** Vite/Vitest CLIs run through Bun package scripts but their CLI entry points require Node.js; 24.15.0 is recorded in `.nvmrc` and README (D-004). The product runtime remains Bun.
7. **Blocked lifecycle scripts.** `protobufjs` and `@google/genai` postinstall scripts are blocked. If a later Pi integration needs generated artifacts from them, trust and necessity must be reviewed explicitly.
8. **Graceful shutdown not load-tested.** Handlers exist, but drain behavior under active connections is unverified.
9. **No CI and no remote.** Documented local checks are the gate until Q-004 is resolved (D-011).
10. **Docker Desktop was not running.** It was started manually for verification; Compose is verified on this Windows machine only.

## Gate check

- [x] **A clean checkout can install/build the platform's own dependencies.** `bun install --frozen-lockfile` reports no changes and `cargo build --locked --workspace` succeeds. This was verified in the working tree with both lockfiles, not by cloning to a second directory.
- [x] **Services start locally and health checks work.** `bun run check:services` reports `PASS: 5/5 local services healthy`; PostgreSQL Compose comes up healthy and is removed cleanly.
- [x] **No service contains project-specific credentials.** `.env.example` uses placeholders; no `.env` exists; the contract check's environment hygiene and secret scan pass.
- [x] **CI or documented local checks pass.** No CI is configured (D-011); `bun run check` runs the documented documentation, Rust, and TypeScript checks and passes, and `bun run check:services` covers live health checks.

## Next permitted step

**Phase 2 - Contracts, environment configuration, and database migrations** may begin only after explicit user authorization. No later phase may start before Phase 2's exit gate passes.
