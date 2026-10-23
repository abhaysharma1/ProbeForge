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

**Status:** Not started

**Tasks**

- [ ] Implement maintained authentication (established OIDC provider or maintained library; Argon2id if local passwords are supported)
- [ ] Implement secure session handling, CSRF protection where applicable, login rate limits, logout, and session revocation
- [ ] Implement projects and project membership roles
- [ ] Enforce backend authorization for every project-scoped endpoint
- [ ] Implement audit events for sensitive changes
- [ ] Use secure cookies and keep long-lived access tokens out of browser storage
- [ ] Keep secrets out of frontend bundles and logs
- [ ] Do not expose PostgreSQL to the browser

**Exit gate**

- [ ] Authentication and project RBAC have automated tests
- [ ] Cross-project reads and writes are denied
- [ ] Session invalidation and logout work
- [ ] No unauthenticated endpoint can create or approve jobs

## Phase 4 — Job creation and PostgreSQL queue

**Status:** Not started

**Tasks**

- [ ] Implement `POST /api/jobs` for the two supported goals
- [ ] Validate Git source configuration and required fields
- [ ] If the source reference is unspecified, require it before queueing or persist a draft; never enqueue unresolvable work
- [ ] Add idempotency support for job creation where useful
- [ ] Implement atomic queue claims with `FOR UPDATE SKIP LOCKED`
- [ ] Implement worker IDs, leases, heartbeat updates, retry counts, bounded backoff, stale-job recovery, and cancellation state
- [ ] Commit the claim transaction before processing
- [ ] Guard state updates with lease-owner/fencing checks
- [ ] Add `job_events` and SSE with reconnect/replay
- [ ] Implement scheduler claim logic for due run records

**Exit gate**

- [ ] Concurrent worker integration tests show a job is not simultaneously claimed by two live workers
- [ ] Expired leases can be recovered
- [ ] A worker cannot complete a job after losing its lease
- [ ] SSE reconnects can replay events
- [ ] All tests use local fixtures and do not invoke real scanners

## Phase 5 — Read-only Git ingestion

**Status:** Not started

**Tasks**

- [ ] Implement supported Git URL validation and repository access credentials through secret references
- [ ] Resolve branch/tag references to immutable commit SHA
- [ ] Clone a pinned commit into a unique temporary workspace using safe argument handling
- [ ] Handle timeouts, disk limits, file-count limits, symlinks, submodules, and cleanup
- [ ] Avoid leaking credentials in Git remote strings or logs
- [ ] Inspect directory structure and source files with bounded reads
- [ ] Read package manifests and lockfiles as data only
- [ ] Explicitly prohibit package installation, build, tests, application startup, and arbitrary project scripts
- [ ] Add an audit trail for snapshot creation and cleanup
- [ ] Build inert fixture repositories covering supported languages and malformed/oversized files

**Exit gate**

- [ ] Tests prove repository content is never executed
- [ ] The worker can analyze a local inert fixture at a pinned commit
- [ ] Invalid URL schemes and unsafe paths are rejected
- [ ] Workspace cleanup is tested on success, failure, and cancellation
- [ ] No live repository or external target security test is run

## Phase 6 — Static discovery and application model

**Status:** Not started

**Tasks**

- [ ] Implement deterministic scanners/parsers for supported languages and frameworks incrementally
- [ ] Extract routes, HTTP methods, middleware, schema validators, auth checks, configuration patterns, dependencies, outbound requests, and file uploads where possible
- [ ] Store findings with file paths, line ranges, and confidence
- [ ] Track unsupported languages/frameworks explicitly instead of inventing coverage
- [ ] Provide discovery tools to Pi with structured, size-bounded results
- [ ] Treat repository content as untrusted input and isolate it from system/tool instructions
- [ ] Add tests using synthetic repositories and expected route inventories

**Exit gate**

- [ ] The system produces a structured application inventory from fixture repositories
- [ ] Each discovered fact includes provenance or is marked as inferred
- [ ] Unsupported or ambiguous routes are identified as such
- [ ] No repository code is executed

## Phase 7 — Pi agent integration and plan generation

**Status:** Not started

**Tasks**

- [ ] Integrate the Pi SDK in the analysis worker
- [ ] Define a constrained tool registry: repository inspection, route discovery, schema/auth inspection, catalog listing, plan proposal, plan validation, and plan persistence
- [ ] Do not give Pi unrestricted shell access or direct database write credentials
- [ ] Supply job goal, intensity, repository snapshot, catalog entries, and policy constraints
- [ ] Validate output with a strict versioned schema
- [ ] Bound tokens, iterations, wall time, tool output size, and retries
- [ ] Record model/provider metadata without storing secrets
- [ ] Add fallback behavior for malformed or unavailable model output
- [ ] Make plan generation idempotent or version-aware
- [ ] Include explicit user input requests for missing information

**Exit gate**

- [ ] A mocked/model-fixture integration test creates a valid plan
- [ ] Invalid or malicious tool arguments are rejected
- [ ] Model outages leave the job recoverable
- [ ] Repository prompt injection cannot grant new tools or alter server-side policy
- [ ] No proposed tests are executed in this phase

## Phase 8 — Test catalog and plan approval UI

**Status:** Not started

**Tasks**

- [ ] Build a versioned catalog with stable test IDs and schemas
- [ ] Include name, category, description, risk, prerequisites, tool requirements, target requirements, timeout, evidence expectations, and confirmation requirements
- [ ] Implement `GET` and `PATCH` plan APIs
- [ ] Implement plan versioning and immutable approval snapshots
- [ ] Build UI to display recommendations, rationale, prerequisites, risks, unavailable tools, and confidence
- [ ] Allow users to select/deselect supported tests and review configuration
- [ ] Do not allow edits to arbitrary shell strings
- [ ] Audit who approved which test set and when

**Exit gate**

- [ ] User can review and select tests from a generated plan
- [ ] Approval produces an immutable versioned snapshot
- [ ] Concurrent plan edits are detected and resolved safely
- [ ] No run can be created without approval

## Phase 9 — Target validation and authorization

**Status:** Not started

**Tasks**

- [ ] Build target registration and normalized origin handling
- [ ] Capture authorization confirmation, scope, allowed paths/hosts, environment, and limits
- [ ] Implement URL and DNS/IP validation, redirect validation, and destination checks at connection time
- [ ] Block unsafe internal and metadata destinations by default
- [ ] Implement separate policy for any explicitly supported private-network targets
- [ ] Add per-run rate, concurrency, duration, request-count, response-size, and total-data budgets
- [ ] Add an authorization expiry and revalidation strategy
- [ ] Add cancellation and audit records

**Exit gate**

- [ ] Unit/integration tests cover invalid schemes, redirect escapes, private/link-local addresses, metadata ranges, DNS changes, disallowed ports, and scope escapes
- [ ] Target validation does not rely solely on the AI prompt
- [ ] No tests are sent to a real target

## Phase 10 — Security execution adapters

**Status:** Not started

**Tasks**

- [ ] Choose a small, vetted first set of tools with structured output
- [ ] Pin tool versions and maintain catalog-to-adapter mappings
- [ ] Run tools using fixed executable paths and typed argument builders, never string concatenation
- [ ] Apply timeouts, output caps, process limits, and cancellation that kills child processes
- [ ] Restrict target and redirect scope at the runner/network layer
- [ ] Normalize results into a common result schema
- [ ] Separate execution status from security outcome
- [ ] Store sanitized evidence and redact secrets
- [ ] Use mock adapters and safe local fixtures for tests

**Exit gate**

- [ ] Each adapter has schema, timeout, cancellation, and output-normalization tests
- [ ] Unsupported test IDs fail closed
- [ ] No arbitrary shell execution is possible through agent input
- [ ] Real-target scans remain disabled during development

## Phase 11 — Load scenario generation and validation

**Status:** Not started

**Tasks**

- [ ] Define a declarative scenario schema: steps, HTTP methods, path templates, headers from secret references, payload templates, extraction/correlation rules, assertions, and dependencies
- [ ] Pi proposes scenarios from discovered API routes and schemas
- [ ] Validate that every endpoint belongs to the approved target scope
- [ ] Support authentication through secret references; never put raw credentials in prompts or plans
- [ ] Detect dangerous loops, unbounded iteration, oversized bodies, and unsupported script expressions
- [ ] Implement user review and versioning for scenarios
- [ ] Define safe profiles with hard platform maxima
- [ ] Add deterministic fixture-based tests for scenario validation

**Exit gate**

- [ ] A scenario can be generated and validated without sending traffic
- [ ] Invalid paths, unbounded loops, arbitrary code, and excessive profiles are rejected
- [ ] User approval pins scenario version and load profile
- [ ] No load test is run against a live deployment

## Phase 12 — Load runner and scheduler

**Status:** Not started

**Tasks**

- [ ] Integrate k6 through a constrained adapter or generated vetted scenario representation
- [ ] Use fixed templates and validated parameters; do not generate arbitrary executable scripts from model text
- [ ] Persist schedules with explicit time zone semantics and a UTC execution timestamp
- [ ] Add worker leases, concurrency quotas, per-project fairness, and schedule cancellation
- [ ] Enforce maximum VUs or arrival rate, duration, ramp-up, request count, and response sizes
- [ ] Persist aggregate time-series buckets and endpoint summaries
- [ ] Capture latency percentiles, throughput, errors, timeouts, and dropped iterations where supported
- [ ] Stop when the deadline, budget, or cancellation condition is reached

**Exit gate**

- [ ] Scheduler tests verify due, future, cancelled, and expired schedules
- [ ] Load runner tests use a local mock server only
- [ ] Metrics calculations are validated against known fixture data
- [ ] Hard caps cannot be bypassed by a scenario or model output

## Phase 13 — Results, AI recommendations, and reports

**Status:** Not started

**Tasks**

- [ ] Build per-test result pages and a run summary
- [ ] Preserve raw structured observations and sanitized evidence independently of AI summaries
- [ ] Generate recommendations referencing result/evidence IDs
- [ ] Distinguish confirmed findings, suspected findings, passed checks, skipped checks, inconclusive results, and execution errors
- [ ] For load testing, distinguish observed metrics from hypotheses about root causes
- [ ] Add report versioning and rerun comparisons
- [ ] Redact credentials, tokens, cookies, and sensitive response data
- [ ] Add export to JSON and a readable report format

**Exit gate**

- [ ] Every AI claim can be traced to stored evidence or is explicitly labeled as a hypothesis
- [ ] Report generation failure does not destroy test results
- [ ] Secrets do not appear in reports or logs
- [ ] Run comparison shows profile, target, scenario version, and timestamps

## Phase 14 — Observability, quotas, and operational hardening

**Status:** Not started

**Tasks**

- [ ] Add structured logs with correlation IDs and redaction
- [ ] Add worker health, queue depth, lease-expiry, failure-rate, and execution-duration metrics
- [ ] Add per-project quotas, concurrency limits, request budgets, and fair scheduling
- [ ] Add retention policies for repositories, evidence, reports, and events
- [ ] Add dependency/version inventory and security update guidance
- [ ] Add backups and a restore procedure for PostgreSQL
- [ ] Add rate limits for API and expensive AI operations
- [ ] Add graceful shutdown and cancellation behavior across services
- [ ] Add operational runbooks and incident handling

**Exit gate**

- [ ] Operators can diagnose stuck jobs and failed workers
- [ ] Quotas are enforced server-side
- [ ] Restore and retention procedures are documented and tested
- [ ] No secrets are emitted in logs

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
