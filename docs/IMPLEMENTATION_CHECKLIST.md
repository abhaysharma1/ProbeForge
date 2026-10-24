# ProbeForge implementation checklist

Tracks the sequential delivery gates defined in [03_phase_by_phase_implementation_plan.md](03_phase_by_phase_implementation_plan.md).

Rules:

- Work on one phase at a time. Do not start phase N+1 until phase N's exit gate passes.
- Tick a task only when the implementation exists and the evidence was actually observed.
- Record non-obvious decisions in [DECISION_LOG.md](DECISION_LOG.md) and each phase outcome in [phases/](phases/).
- Never run real security scans or load tests during development; use inert fixtures and local mock targets.

Legend: `[x]` done with evidence - `[ ]` not done.

## Phase 0 — Repository reconnaissance and implementation contract

**Status:** Done - [report](phases/00_recon_report.md)

**Tasks**

- [x] Inspect all files, tooling, and Git history (repository had no git history; folder was not a Git repository)
- [x] Confirm repository state: documentation only (`Agents.md` + `docs/01`-`05`), no prior code or infrastructure
- [x] Create `README.md`, `.gitignore`, `.editorconfig`, and a documentation index (`docs/README.md`)
- [x] Create a decision log ([DECISION_LOG.md](DECISION_LOG.md))
- [x] Establish formatter, linter, and test commands for the chosen frontend/backend languages (documented contract; executable wiring in Phase 1 per D-002)
- [x] Provide root-level coding-agent instructions (`Agents.md`, equivalent to `AGENTS.md` per D-001)
- [x] Do not begin product feature implementation before the repository contract is written

**Exit gate**

- [x] A new contributor can understand the product and run the documented checks (`node scripts/check-docs.mjs` runs today; language checks documented for Phase 1)
- [x] The implementation checklist is recorded and committed
- [x] No feature is marked complete without implementation

## Phase 1 — Monorepo skeleton and local development

**Status:** Done - [report](phases/01_monorepo_report.md)

**Tasks**

- [x] Create the monorepo structure: `apps/web`, `services/api`, `services/analysis-worker`, `services/execution-worker`, `services/scheduler`, `packages/contracts`, `packages/test-catalog`, `infra/local`, `docs`
- [x] Initialize the React/TypeScript frontend and the Rust/Axum backend
- [x] Initialize the TypeScript analysis worker with the Pi SDK (Bun 1.4.4, per D-004/D-007)
- [x] Add PostgreSQL for local development using a reproducible Compose file
- [x] Add environment variable examples containing placeholders only
- [x] Add health endpoints and graceful shutdown for each service
- [x] Add a root command interface for lint, format, test, and local startup
- [x] Pin supported runtime and tool versions

**Exit gate**

- [x] A clean checkout can install/build the platform's own dependencies (`bun install --frozen-lockfile`, `cargo build --locked`)
- [x] Services start locally and health checks work (`bun run check:services`: 5/5)
- [x] No service contains project-specific credentials (placeholders only; contract check passes)
- [x] CI or documented local checks pass (`bun run check`; no remote yet, see D-011)

## Phase 2 — Contracts, configuration, and database migrations

**Status:** Done - [report](phases/02_contracts_db_report.md)

**Tasks**

- [x] Define shared API and event contracts
- [x] Define schemas for job creation, Git source, intensity, plans, planned tests, target configuration, authorization, test runs, result records, and load profiles
- [x] Define allowed enum values and reject unknown goal values
- [x] Implement PostgreSQL migrations for the core domain model
- [x] Add indexes for queue claiming, leases, project-scoped listings, run schedules, and event ordering
- [x] Define a versioned JSON schema for Pi plan output and load scenarios
- [x] Add fixture data and migration tests
- [x] Add validation for request size, strings, UUIDs, URLs, enums, and numerical limits

**Exit gate**

- [x] Migrations create a complete database from empty state
- [x] Rollback strategy is documented
- [x] Contract tests pass
- [x] Invalid payloads are rejected before persistence
- [x] Schemas contain no executable script fields for the model to populate

## Phase 3 — Authentication, projects, and authorization

**Status:** Complete (see [phase report](phases/03_auth_projects_report.md))

**Tasks**

- [x] Implement maintained authentication (established OIDC provider or maintained library; Argon2id if local passwords are supported)
- [x] Implement secure session handling, CSRF protection where applicable, login rate limits, logout, and session revocation
- [x] Implement projects and project membership roles
- [x] Enforce backend authorization for every project-scoped endpoint
- [x] Implement audit events for sensitive changes
- [x] Use secure cookies and keep long-lived access tokens out of browser storage
- [x] Keep secrets out of frontend bundles and logs
- [x] Do not expose PostgreSQL to the browser

**Exit gate**

- [x] Authentication and project RBAC have automated tests
- [x] Cross-project reads and writes are denied
- [x] Session invalidation and logout work
- [x] No unauthenticated endpoint can create or approve jobs

## Phase 4 — Job creation and PostgreSQL queue

**Status:** Complete (see [phase report](phases/04_jobs_queue_report.md))

**Tasks**

- [x] Implement `POST /api/jobs` for the two supported goals
- [x] Validate Git source configuration and required fields
- [x] If the source reference is unspecified, require it before queueing or persist a draft; never enqueue unresolvable work
- [x] Add idempotency support for job creation where useful
- [x] Implement atomic queue claims with `FOR UPDATE SKIP LOCKED`
- [x] Implement worker IDs, leases, heartbeat updates, retry counts, bounded backoff, stale-job recovery, and cancellation state
- [x] Commit the claim transaction before processing
- [x] Guard state updates with lease-owner/fencing checks
- [x] Add `job_events` and SSE with reconnect/replay
- [x] Implement scheduler claim logic for due run records

**Exit gate**

- [x] Concurrent worker integration tests show a job is not simultaneously claimed by two live workers
- [x] Expired leases can be recovered
- [x] A worker cannot complete a job after losing its lease
- [x] SSE reconnects can replay events
- [x] All tests use local fixtures and do not invoke real scanners

## Phase 5 — Read-only Git ingestion

**Status:** Complete

**Tasks**

- [x] Implement supported Git URL validation and repository access credentials through secret references
- [x] Resolve branch/tag references to immutable commit SHA
- [x] Clone a pinned commit into a unique temporary workspace using safe argument handling
- [x] Handle timeouts, disk limits, file-count limits, symlinks, submodules, and cleanup
- [x] Avoid leaking credentials in Git remote strings or logs
- [x] Inspect directory structure and source files with bounded reads
- [x] Read package manifests and lockfiles as data only
- [x] Explicitly prohibit package installation, build, tests, application startup, and arbitrary project scripts
- [x] Add an audit trail for snapshot creation and cleanup
- [x] Build inert fixture repositories covering supported languages and malformed/oversized files

**Exit gate**

- [x] Tests prove repository content is never executed
- [x] The worker can analyze a local inert fixture at a pinned commit
- [x] Invalid URL schemes and unsafe paths are rejected
- [x] Workspace cleanup is tested on success, failure, and cancellation
- [x] No live repository or external target security test is run

## Phase 6 — Static discovery and application model

**Status:** Complete

**Tasks**

- [x] Implement deterministic scanners/parsers for supported languages and frameworks incrementally
- [x] Extract routes, HTTP methods, middleware, schema validators, auth checks, configuration patterns, dependencies, outbound requests, and file uploads where possible
- [x] Store findings with file paths, line ranges, and confidence
- [x] Track unsupported languages/frameworks explicitly instead of inventing coverage
- [x] Provide discovery tools to Pi with structured, size-bounded results
- [x] Treat repository content as untrusted input and isolate it from system/tool instructions
- [x] Add tests using synthetic repositories and expected route inventories

**Exit gate**

- [x] The system produces a structured application inventory from fixture repositories
- [x] Each discovered fact includes provenance or is marked as inferred
- [x] Unsupported or ambiguous routes are identified as such
- [x] No repository code is executed

## Phase 7 — Pi agent integration and plan generation

**Status:** Complete

**Tasks**

- [x] Integrate the Pi SDK in the analysis worker
- [x] Define a constrained tool registry: repository inspection, route discovery, schema/auth inspection, catalog listing, plan proposal, plan validation, and plan persistence
- [x] Do not give Pi unrestricted shell access or direct database write credentials
- [x] Supply job goal, intensity, repository snapshot, catalog entries, and policy constraints
- [x] Validate output with a strict versioned schema
- [x] Bound tokens, iterations, wall time, tool output size, and retries
- [x] Record model/provider metadata without storing secrets
- [x] Add fallback behavior for malformed or unavailable model output
- [x] Make plan generation idempotent or version-aware
- [x] Include explicit user input requests for missing information

**Exit gate**

- [x] A mocked/model-fixture integration test creates a valid plan
- [x] Invalid or malicious tool arguments are rejected
- [x] Model outages leave the job recoverable
- [x] Repository prompt injection cannot grant new tools or alter server-side policy
- [x] No proposed tests are executed in this phase

**Evidence**

- Library: `services/analysis-worker/src/agent/`
- Integration tests: `bun test src/agent` (43 pass, 6 skipped without a test DB)
- Database integration: `bun test src/agent/postgres-store.integration.test.ts` with a command-scoped `PROBE_FORGE_TEST_DATABASE_URL` (4 pass)
- Full checks: `bun run check` (PASS)
- Report: `docs/phases/07_planning_report.md`; design: `docs/PLANNING.md`

## Phase 8 — Test catalog and plan approval UI

**Status:** Complete

**Tasks**

- [x] Build a versioned catalog with stable test IDs and schemas
- [x] Include name, category, description, risk, prerequisites, tool requirements, target requirements, timeout, evidence expectations, and confirmation requirements
- [x] Implement `GET` and `PATCH` plan APIs
- [x] Implement plan versioning and immutable approval snapshots
- [x] Build UI to display recommendations, rationale, prerequisites, risks, unavailable tools, and confidence
- [x] Allow users to select/deselect supported tests and review configuration
- [x] Do not allow edits to arbitrary shell strings
- [x] Audit who approved which test set and when

**Exit gate**

- [x] User can review and select tests from a generated plan
- [x] Approval produces an immutable versioned snapshot
- [x] Concurrent plan edits are detected and resolved safely
- [x] No run can be created without approval

**Evidence**

- Catalog: `packages/test-catalog/catalog.json` (schema 1, catalog 0.2.0)
- API/domain: `services/api/src/catalog.rs`, `services/api/src/plans.rs`
- UI: `apps/web/src/PlanReview.tsx`, `apps/web/src/ProjectsView.tsx`
- Tests: `cargo test --workspace` with command-scoped `DATABASE_URL`; `bun run check:db`; `bun run check` (analysis worker 111 pass / 7 skip; test-catalog 6; web 8; contracts 46)
- Browser check: Playwright against local API + Vite; desktop 1440×1200 and mobile 390×844; selection/save verified
- Report: `docs/phases/08_plan_approval_report.md`; design: `docs/PLAN_APPROVAL.md`

## Phase 9 — Target validation and authorization

**Status:** Complete

**Tasks**

- [x] Build target registration and normalized origin handling
- [x] Capture authorization confirmation, scope, allowed paths/hosts, environment, and limits
- [x] Implement URL and DNS/IP validation, redirect validation, and destination checks at connection time
- [x] Block unsafe internal and metadata destinations by default
- [x] Implement separate policy for any explicitly supported private-network targets
- [x] Add per-run rate, concurrency, duration, request-count, response-size, and total-data budgets
- [x] Add an authorization expiry and revalidation strategy
- [x] Add cancellation and audit records

**Exit gate**

- [x] Unit/integration tests cover invalid schemes, redirect escapes, private/link-local addresses, metadata ranges, DNS changes, disallowed ports, and scope escapes
- [x] Target validation does not rely solely on the AI prompt
- [x] No tests are sent to a real target

**Evidence**

- Policy/client: `crates/target-policy/src/lib.rs`
- API and persistence: `services/api/src/targets.rs`, `migrations/0010_target_validation.sql`
- Tests: `target_policy` unit tests, 5 `targets_authorization` integration tests, `cargo test --workspace`, `bun run check:db`, `bun run check` (analysis worker 111 pass / 7 skip; contracts 47; test-catalog 6; web 8)
- UI: target management is API-only in this phase; project audit labels include target actions
- Report: `docs/phases/09_target_validation_report.md`; design: `docs/TARGET_VALIDATION.md`

## Phase 10 — Security execution adapters

**Status:** Complete - [report](phases/10_execution_adapters_report.md)

**Tasks**

- [x] Choose a small, vetted first set of tools with structured output - two native HTTP adapters (`http_probe` 0.1.0)
- [x] Pin tool versions and maintain catalog-to-adapter mappings - `crates/execution-adapters` registry
- [x] Run tools using fixed executable paths and typed argument builders, never string concatenation - N/A for the shipped native adapters: they run in-process and execute no external/child process; configuration is typed and unknown fields are rejected. Future external adapters must use fixed paths and typed argument vectors.
- [x] Apply timeouts, output caps, process limits, and cancellation that kills child processes - 30 s adapter timeout, guarded-client response/total-byte caps, run-budget cancellation; process limits and child termination are N/A because no child process exists.
- [x] Restrict target and redirect scope at the runner/network layer - reused `GuardedHttpClient`/`validate_request_url`
- [x] Normalize results into a common result schema - `AdapterResult`
- [x] Separate execution status from security outcome - independent enums
- [x] Store sanitized evidence and redact secrets - sanitized evidence is returned in the result; durable persistence is Phase 13. Bodies, cookies, and arbitrary headers are excluded.
- [x] Use mock adapters and safe local fixtures for tests - loopback mock HTTP server only

**Exit gate**

- [x] Each adapter has schema, timeout, cancellation, and output-normalization tests
- [x] Unsupported test IDs fail closed
- [x] No arbitrary shell execution is possible through agent input - no shell/process path exists; unknown `command` configuration is rejected
- [x] Real-target scans remain disabled during development - no run/dispatch endpoint; tests use loopback only

**Evidence**

- Adapter registry and implementations: `crates/execution-adapters/src/lib.rs`
- Guarded request-header support: `crates/target-policy/src/lib.rs`
- API availability: `services/api/src/catalog.rs`, `services/api/tests/plans_approval.rs`
- Tests: 6 adapter tests, 8 target-policy tests, 27 API unit tests, 5 plan approval integration tests, full `cargo test --workspace`
- UI: plan-review fixture reflects the two available adapters; no execution UI exists
- Report: `docs/phases/10_execution_adapters_report.md`; design: `docs/EXECUTION_ADAPTERS.md`; decision D-031

## Phase 11 — Load scenario generation and validation

**Status:** Complete - [report](phases/11_load_scenarios_report.md)

**Tasks**

- [x] Define a declarative scenario schema: steps, HTTP methods, path templates, headers from secret references, payload templates, extraction/correlation rules, assertions, and dependencies - v1 load contract in Rust/TS/JSON Schema
- [x] Pi proposes scenarios from discovered API routes and schemas - bounded prompt policy and existing discovery tools
- [x] Validate that every endpoint belongs to the approved target scope - `validate_load_scenario_scope` (pure, no traffic); Phase 12 must call it at run creation
- [x] Support authentication through secret references; never put raw credentials in prompts or plans - typed `secretHeaders`, `credentialRef`, and raw-credential rejection
- [x] Detect dangerous loops, unbounded iteration, oversized bodies, and unsupported script expressions - closed schema, body cap, template parser, fixture/unit tests
- [x] Implement user review and versioning for scenarios - plan-review detail, selection only, parent plan version, immutable approval snapshot
- [x] Define safe profiles with hard platform maxima - contract maxima plus migration `0011` constraints
- [x] Add deterministic fixture-based tests for scenario validation - shared Rust/TS fixtures and unit tests

**Exit gate**

- [x] A scenario can be generated and validated without sending traffic
- [x] Invalid paths, unbounded loops, arbitrary code, and excessive profiles are rejected
- [x] User approval pins scenario version and load profile
- [x] No load test is run against a live deployment

**Evidence**

- Contract types/validation: `crates/contracts/src/model.rs`, `crates/contracts/src/validation.rs`
- Shared schema/fixtures: `packages/contracts/schemas/probeforge.v1.schema.json`, `packages/contracts/fixtures/v1/`, `packages/contracts/src/validate.ts`
- Planning and generation: `services/analysis-worker/src/agent/planning-input.ts`, `fallback.ts`, `validate.ts`
- Scope validation: `crates/target-policy/src/lib.rs`
- Approval/snapshot validation: `services/api/src/plans.rs`, `services/api/tests/plans_approval.rs`
- Database safety: `migrations/0011_load_safety.sql`, migration/constraint tests
- UI: `apps/web/src/PlanReview.tsx` (step/correlation/profile review)
- Report: `docs/phases/11_load_scenarios_report.md`; design: `docs/LOAD_SCENARIOS.md`; decision D-032

## Phase 12 — Load runner and scheduler

**Status:** Complete - [report](phases/12_load_runner_scheduler_report.md)

**Tasks**

- [x] Integrate k6 through a constrained adapter or generated vetted scenario representation - generated data-only `RunnerSpec` from the validated scenario, executed by a constrained native adapter; no arbitrary scripts are produced from model text. k6 is not installed and no external binary is invoked.
- [x] Use fixed templates and validated parameters; do not generate arbitrary executable scripts from model text - typed `RunnerStep` templates, closed extraction/assertion rules, percent-encoded path substitution, 64 KiB body cap
- [x] Persist schedules with explicit time zone semantics and a UTC execution timestamp - migration `0012`, `create_schedule`, normalized `UTC`/fixed-offset timezone, `next_run_at` always UTC
- [x] Add worker leases, concurrency quotas, per-project fairness, and schedule cancellation - `claim_due_run_fair` (advisory-locked global/per-project quotas), `heartbeat_run`, `complete_run`, `fail_run`, `recover_expired_run_leases`, `cancel_schedule`
- [x] Enforce maximum VUs or arrival rate, duration, ramp-up, request count, and response sizes - `resolve_limits` takes the minimum of profile, target authorization, and platform caps; `RunBudget` repeats the caps at the network boundary
- [x] Persist aggregate time-series buckets and endpoint summaries - `run_metrics` upsert per bucket/endpoint/status
- [x] Capture latency percentiles, throughput, errors, timeouts, and dropped iterations where supported - `aggregate_metrics` and `RunSummary`
- [x] Stop when the deadline, budget, or cancellation condition is reached - `RunBudget` deadline/request/cancel plus closed stop conditions

**Exit gate**

- [x] Scheduler tests verify due, future, cancelled, and expired schedules - queue/scheduler integration test
- [x] Load runner tests use a local mock server only - loopback mock HTTP server
- [x] Metrics calculations are validated against known fixture data - deterministic percentile/bucket fixture
- [x] Hard caps cannot be bypassed by a scenario or model output - policy minimum-of-all-sources tests and `NOT VALID` database checks

**Evidence**

- Runner/policy/metrics: `crates/load-runner/src/`
- Queue/scheduler persistence: `crates/queue/src/lib.rs`, `crates/queue/tests/queue.rs`
- Scheduler loop: `services/scheduler/src/lib.rs`
- Migration: `migrations/0012_load_execution.sql`
- Target-boundary request path: `crates/target-policy/src/lib.rs`
- Report: `docs/phases/12_load_runner_scheduler_report.md`

## Phase 13 — Results, AI recommendations, and reports

**Status:** Complete - [report](phases/13_results_reports_report.md). Result pages and the run summary are available in the web app (project detail → Runs), backed by the authenticated JSON API.

**Tasks**

- [x] Build per-test result pages and a run summary - `RunReport.tsx` renders the run summary, per-test results, evidence metadata, recommendations, metrics, and report claims; the project detail lists runs and opens the view.
- [x] Preserve raw structured observations and sanitized evidence independently of AI summaries - `test_results.observation` and the immutable `evidence` table are read directly; generated summaries live only in `report_versions`/`recommendations`.
- [x] Generate recommendations referencing result/evidence IDs - deterministic `recommendations` rows cite `result_id`/`evidence_id`/`run_metric_id`, enforced by the `recommendations_traceability` constraint.
- [x] Distinguish confirmed findings, suspected findings, passed checks, skipped checks, inconclusive results, and execution errors - `test_results.finding_status` plus the closed outcome vocabulary; reports split `findings` from `checks`.
- [x] For load testing, distinguish observed metrics from hypotheses about root causes - load observations cite metric bucket IDs; root-cause claims are stored as `kind = 'hypothesis'` with no reference.
- [x] Add report versioning and rerun comparisons - immutable `report_versions` per `reports` row; `GET /api/runs/{id}/compare?otherRunId=`.
- [x] Redact credentials, tokens, cookies, and sensitive response data - recursive key/value redaction applied to all report content and run-detail observations.
- [x] Add export to JSON and a readable report format - `GET /api/runs/{id}/report/export?format=json|markdown`.

**Exit gate**

- [x] Every AI claim can be traced to stored evidence or is explicitly labeled as a hypothesis - report claims carry `references` or `claimKind = "hypothesis"`; DB constraint rejects untraceable non-hypothesis recommendations.
- [x] Report generation failure does not destroy test results - reports are a separate table; oversize generation marks the report `failed` and leaves results intact (integration test).
- [x] Secrets do not appear in reports or logs - report content and run-detail observations are redacted; no content is logged.
- [x] Run comparison shows profile, target, scenario version, and timestamps - comparison sides include target origin, scenario ID/name, plan version, profile, and all timestamps.

**Evidence**

- Results/reports API: `services/api/src/results.rs`
- Web result pages and run summary: `apps/web/src/RunReport.tsx`, `apps/web/src/ProjectsView.tsx`, `apps/web/src/api.ts`
- Migration: `migrations/0013_results_reports.sql`
- Integration tests: `services/api/tests/results_reports.rs`, `apps/web/src/App.test.tsx`
- Report: `docs/phases/13_results_reports_report.md`

## Phase 14 — Observability, quotas, and operational hardening

**Status:** Complete - [report](phases/14_observability_operations_report.md).

**Tasks**

- [x] Add structured logs with correlation IDs and redaction - `crates/observability`; the API middleware attaches and echoes a correlation ID and seeds a `http_request` span.
- [x] Add worker health, queue depth, lease-expiry, failure-rate, and execution-duration metrics - `probe_forge_queue::queue_metrics` and `GET /metricsz` on the scheduler.
- [x] Add per-project quotas, concurrency limits, request budgets, and fair scheduling - `project_quotas`, `claim_due_run_fair` enforcement, `GET|PUT /api/projects/{id}/quota`.
- [x] Add retention policies for repositories, evidence, reports, and events - `run_retention` via the single sanctioned purge path; repository workspaces are cleaned by ingestion.
- [x] Add dependency/version inventory and security update guidance - `docs/OPERATIONS.md`.
- [x] Add backups and a restore procedure for PostgreSQL - `scripts/db-backup.mjs` with a tested scratch-database roundtrip.
- [x] Add rate limits for API and expensive AI operations - authenticated per-user middleware with a stricter expensive bucket.
- [x] Add graceful shutdown and cancellation behavior across services - existing graceful shutdown plus scheduled-retention abort and documented cancellation.
- [x] Add operational runbooks and incident handling - `docs/OPERATIONS.md`.

**Exit gate**

- [x] Operators can diagnose stuck jobs and failed workers - `/metricsz`, lease queries, and the incident runbook.
- [x] Quotas are enforced server-side - claim-path enforcement with integration tests.
- [x] Restore and retention procedures are documented and tested - `db:backup:check` roundtrip and the retention purge immutability-regression test.
- [x] No secrets are emitted in logs - single redaction implementation with tests.

**Evidence**

- Shared crate: `crates/observability/`
- Queue quotas, metrics, retention: `crates/queue/src/lib.rs`
- API middleware/rate limit/quotas: `services/api/src/http_middleware.rs`, `rate_limit.rs`, `quotas.rs`
- Scheduler metrics and retention loop: `services/scheduler/src/lib.rs`
- Migration: `migrations/0014_observability.sql`
- Tests: `services/api/tests/operations.rs`
- Tooling and runbook: `scripts/db-backup.mjs`, `docs/OPERATIONS.md`
- Report: `docs/phases/14_observability_operations_report.md`

## Phase 15 — End-to-end acceptance using local fixtures

**Status:** Not started

**Tasks**

- [ ] Run the full vulnerability workflow against inert fixture repositories and mocked targets
- [ ] Run the full load workflow against a local mock HTTP service only
- [ ] Test user approval, scheduling, cancellation, retries, worker crashes, lease recovery, partial results, and report generation
- [ ] Test cross-project authorization and target-scope rejection
- [ ] Test duplicate requests and idempotency
- [ ] Review all phase exit gates and unresolved limitations
- [ ] Produce a release-readiness checklist

**Exit gate**

- [ ] All automated checks pass
- [ ] Both end-to-end workflows work with safe local fixtures
- [ ] No real security scan or load test was executed as part of acceptance
- [ ] Documentation describes what is and is not supported
- [ ] Only after this gate may an authorized operator perform a separate, manually approved staging smoke test
