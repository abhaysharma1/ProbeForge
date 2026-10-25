# ProbeForge decision log

Records implementation decisions not fixed by the six normative documents, plus deferred choices and open questions. The phase plan must not be silently reinterpreted; record the interpretation here.

Format per decision: status, context, decision, rationale, alternatives, impact.

## Decisions

### D-001 - Root agent instructions live in `Agents.md`

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 0 requires a root-level `AGENTS.md` or equivalent that references the five documents and the phase-gate rule.
- **Decision:** Keep `Agents.md` as the single root instructions file and add explicit pointers to the implementation checklist, decision log, and phase reports.
- **Rationale:** The file already exists and is authoritative. On case-insensitive filesystems (Windows/macOS) `Agents.md` and `AGENTS.md` resolve to the same file; agent harnesses that look for `AGENTS.md` load it.
- **Alternatives:** Rename to `AGENTS.md` (churn, no behavior change); add a second instructions file (duplication and drift risk).
- **Impact:** One authoritative instruction file. It must be followed alongside documents 01-05.

### D-002 - Phase 0 documents language checks; executable wiring lands in Phase 1

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 0 requires establishing formatter, linter, and test commands; Phase 1 adds the "root command interface for lint, format, test, and local startup". Phase 0 also forbids product feature implementation.
- **Decision:** Document the Rust and TypeScript commands in the README as the contract, and add one dependency-free repository check (`node scripts/check-docs.mjs`) that is runnable today. The executable language tooling, configs, and lockfiles are created in Phase 1.
- **Rationale:** No packages or Rust crates exist yet, so language tooling would be speculative configuration. The docs check keeps the Phase 0 exit gate ("a new contributor can run the documented checks") literally true without introducing a package manager or product code.
- **Alternatives:** Create full lint/test tooling now (premature, starts the Phase 1 skeleton); document only and leave the gate unverifiable (weaker evidence).
- **Impact:** First executed language-level check evidence will appear in the Phase 1 report. This is stated as a limitation in the Phase 0 report.

### D-003 - Runtime and dependency versions are confirmed and pinned in Phase 1

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Document 04 requires confirming current compatible stable versions and pinning them in lockfiles; floating `latest` tags are forbidden.
- **Decision:** Phase 1 confirmed and pinned: Bun 1.4.4 (`packageManager`, `engines`, installed binary), Rust 1.98.1 (`rust-toolchain.toml`, `rust-version = "1.98"`), Node.js 24.15.0 for build tooling only (`.nvmrc`), PostgreSQL 18.6 (`postgres:18.6-alpine`, Compose), exact JavaScript dependency versions, and committed `bun.lock` and `Cargo.lock`.
- **Rationale:** Pinning without lockfiles cannot be enforced. The Phase 0 observations were superseded by exact pins and lockfiles.
- **Impact:** Reproducible installs are verified with `bun install --frozen-lockfile` and `cargo build --locked`. CI adoption is tracked by D-011.

### D-004 - Analysis worker runtime: Bun 1.4.4

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 1)
- **Context:** Document 04 permits Bun or a supported Node.js LTS but requires choosing one and pinning it without mixing runtimes casually. The user selected Bun.
- **Decision:** Run the analysis worker, repository scripts, and all future product TypeScript services on Bun 1.4.4. Vite 8 and Vitest 5 CLIs are build-only tooling executed through Bun package scripts; the Node.js 24.15.0 CLI used by those CLIs is pinned in `.nvmrc` and is not a product runtime.
- **Constraint:** Exactly one product TypeScript runtime. Do not introduce a second service runtime without a recorded decision.
- **Impact:** `bun.lock` is the JavaScript lockfile and includes the pinned Pi SDK (`@earendil-works/pi-agent-core` 1.4.0).

### D-005 - Git repository initialized in Phase 0

- **Date:** 2026-10-23
- **Status:** Accepted (user-approved)
- **Context:** The phase plan requires the implementation checklist to be "committed or otherwise recorded", but the folder was not a Git repository.
- **Decision:** Initialize Git locally and record Phase 0 deliverables in an initial commit. No remote is configured.
- **Rationale:** Committed history gives honest, reviewable phase checkpoints. No secrets or generated artifacts are included.
- **Alternatives:** Leave the checklist uncommitted (weaker audit trail).
- **Impact:** Later phases commit their own checkpoints. Remote hosting remains an open question (Q-004).

### D-006 - Real environment files are never committed

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Global rules require never committing real `.env` files, credentials, tokens, or target access details.
- **Decision:** `.gitignore` ignores `.env` and `.env.*` while explicitly un-ignoring `.env.example`; only placeholder values may appear in examples.
- **Verification:** `scripts/check-docs.mjs` fails if a real `.env` file exists or if the ignore rules are missing.
- **Impact:** Local secrets stay untracked; Phase 1 adds `.env.example` files with placeholders only.

### D-007 - Package manager and monorepo tooling: Bun workspaces

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 1)
- **Context:** Phase 1 creates the monorepo skeleton and root command interface; documents do not mandate a JavaScript package manager or monorepo tool.
- **Decision:** Use Bun 1.4.4 workspaces (`apps/*`, `services/analysis-worker`, `packages/*`) with a single root `package.json` command interface. Rust uses a Cargo workspace. No extra monorepo tool (Turborepo, Nx) is introduced.
- **Rationale:** Bun already provides the chosen runtime, workspace linking (`workspace:*`), deterministic `bun.lock`, and script filtering (`bun run --filter`).
- **Impact:** Root commands cover install, dev, build, lint, format, typecheck, tests, checks, and database lifecycle. See D-009 for the Rust workspace.

### D-008 - Product naming

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Document 04 requires consistent use of the name ProbeForge and forbids claiming domain or trademark availability.
- **Decision:** Use **ProbeForge** in the README, UI, page metadata, user-facing copy, and docs. Make branding configurable where branding is normally configurable. Third-party dependency and package names are not renamed.
- **Impact:** No domain or trademark claim is made anywhere.

### D-009 - Rust workspace with three binary services and a pinned toolchain

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 1 requires the control plane plus scheduler and execution-worker service structure, health endpoints, graceful shutdown, and pinned tool versions.
- **Decision:** One Cargo workspace (`services/api`, `services/scheduler`, `services/execution-worker`), resolver 3, edition 2024, `rust-version = "1.98"`, and a `rust-toolchain.toml` pinning `1.98.1` with clippy and rustfmt. Each service owns a small bootstrap module; no shared support crate is introduced yet.
- **Rationale:** Three tiny services with identical health/graceful-shutdown logic are reviewable now; a shared crate would be premature. Consolidation is justified only if the bootstrap code drifts or grows.
- **Impact:** `cargo fmt`/`clippy`/`test` cover all services. Bootstrap duplication is a known, accepted cost (D-010 notes contract duplication separately).

### D-010 - Phase 1 health contract is static and mirrored in Rust

- **Date:** 2026-10-23
- **Status:** Accepted (temporary)
- **Context:** The phase plan puts shared API/event contracts and schemas in Phase 2, but Phase 1 requires working health checks across TypeScript and Rust services.
- **Decision:** Define `HealthResponse`/`ReadinessResponse` in `packages/contracts`, and serialize the same shape from Rust `serde` structs. Readiness is static (`ready`) because no dependencies exist yet.
- **Rationale:** Typed contracts for the only existing HTTP surface, without inventing the Phase 2 schema system early.
- **Impact:** The shape is duplicated across languages until Phase 2 replaces it with versioned shared contracts. Tests on both sides assert the current shape.

### D-011 - No CI yet; documented local checks are the Phase 1 gate

- **Date:** 2026-10-23
- **Status:** Accepted (temporary)
- **Context:** The Phase 1 exit gate accepts CI or documented local checks; the repository has no remote (Q-004).
- **Decision:** Use `bun run check` and `bun run check:services` as the documented gate and record their real output in the phase report. Add CI when a remote and branch protection exist.
- **Impact:** Checks are reproducible but manual; nothing claims CI coverage that does not exist.

### D-012 - Scheduler and execution-worker are explicitly health-only skeletons

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 1 scope)
- **Context:** Phase 1 requires the service structure, health endpoints, and graceful shutdown, but queue claiming, adapters, and the load runner belong to later phases.
- **Decision:** Ship both services as minimal Axum servers with config parsing, tracing, health endpoints, request limits, and graceful shutdown. They contain no job, adapter, or policy logic, and comments name the owning phases (4, 10, 12). README and the phase report label them as skeletons.
- **Rationale:** Satisfies the structure requirement without empty placeholder folders or functionality that looks production-ready.
- **Impact:** Health parity and process boundaries exist now; real behavior is added only in its owning phase.

### D-013 - Services do not connect to PostgreSQL until Phase 2

- **Date:** 2026-10-23
- **Status:** Accepted (temporary)
- **Context:** Phase 1 adds local PostgreSQL; Phase 2 adds migrations, the pool, and queue indexes.
- **Decision:** Compose runs PostgreSQL and verifies it with a healthcheck, but no service opens a database connection yet. Readiness stays static.
- **Rationale:** Avoids a half-built schema and connection code before the migration contract exists.
- **Impact:** Phase 2 adds migrations and the first database read path. Amended by D-018: Phase 2 adds the migration runner and schema test, while the service pool and database-aware readiness move to Phase 3.

### D-014 - Test catalog scaffold is empty but validated

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 1 lists `packages/test-catalog`; the versioned catalog with real entries belongs to Phase 8.
- **Decision:** Ship `catalog.json` (`schemaVersion: "1"`, no entries) with a strict loader that rejects executable fields (`command`, `executable`, `script`, `shell`), unknown categories/risks, and unstable IDs. Tests cover those rejections.
- **Rationale:** Establishes the "catalog is data, never commands" invariant before any entry exists, without pretending tests are supported.
- **Impact:** Phase 8 adds entries and adapter mappings; the loader's rejection rules must not be relaxed.

### D-015 - Local PostgreSQL Compose choices

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 1 requires reproducible local PostgreSQL with no committed credentials.
- **Decision:** `postgres:18.6-alpine` pinned; publishes `127.0.0.1:${POSTGRES_PORT}` only; requires `POSTGRES_PASSWORD` with no default; named volume mounted at `/var/lib/postgresql` (PostgreSQL 18 image layout); `pg_isready` healthcheck; `no-new-privileges`. Verified with `up -d --wait` and `down -v`.
- **Rationale:** Loopback-only binding, no default secret, and a pinned image keep local development reproducible and contained.
- **Impact:** Contributors create an untracked `.env` from `.env.example`; CI/production database provisioning is out of scope for Phase 1.

### D-016 - Contract check inspects tracked `.env` files, not working-tree presence

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Contributors need a local `.env` for Compose, but a real `.env` must never be committed. The Phase 0 check failed on any working-tree `.env`, which blocked local verification.
- **Decision:** `scripts/check-docs.mjs` uses `git ls-files` and fails only when a real `.env` file is tracked; without Git it falls back to a working-tree check. Local untracked `.env` files are allowed.
- **Rationale:** Enforces the actual invariant (never committed) while keeping local development workable.
- **Impact:** The check still fails closed in a non-Git checkout that contains `.env`.

### D-017 - Canonical frontend design document path

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** The project owner added `docs/06_frontend_design.md` and updated `Agents.md` to require `docs/frontend_design.md` for frontend work. Only the numbered file existed, so the required path was a broken reference.
- **Decision:** Keep `docs/06_frontend_design.md` as the canonical design contract and add `docs/frontend_design.md` as a short pointer to it. The documentation index lists both.
- **Rationale:** Preserves the owner-authored document and the numbered-document convention while making the required path valid. Phase 2 makes no UI changes.
- **Alternatives:** Rename `06_frontend_design.md` (breaks the numbered sequence); duplicate the document (guaranteed drift).
- **Impact:** Any future frontend task must read the canonical file. Update the pointer, not a second copy, if the location changes.

### D-018 - Database pool and database-aware readiness deferred to Phase 3

- **Date:** 2026-10-23
- **Status:** Accepted (amends D-013)
- **Context:** D-013 expected Phase 2 to add a service database pool and DB-dependent readiness. Phase 2's actual scope is contracts, configuration, and migrations; no authenticated DB-backed feature exists yet.
- **Decision:** Phase 2 adds the SQLx migration runner (`services/api/src/bin/migrate.rs`), migration integration tests, and `DATABASE_URL` configuration. The API, scheduler, and execution worker remain health-only and open no database pool. Phase 3 introduces the first pool, transaction boundaries, and database-aware readiness alongside authentication and project access.
- **Rationale:** Avoids a half-wired pool and a readiness endpoint that cannot exercise real authorization. Keeps `bun run check:services` independent of a running database while still proving the schema from empty state.
- **Alternatives:** Add a pool now with static readiness (misleading); make readiness require DB now (breaks the service check without adding a real feature).
- **Impact:** D-013 remains the target architecture but its Phase 2 timing is superseded. `DATABASE_URL` is required for `db:migrate` and `check:db`, not for service startup.

### D-019 - Contract bundle, validators, and cross-language fixtures

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 2 needs shared API/event contracts, plan/load JSON schemas, enum rejection, request limits, and tests. TypeScript and Rust must agree without duplicating behavior by hand.
- **Decision:** `packages/contracts/schemas/probeforge.v1.schema.json` is the normative JSON Schema (2020-12). TypeScript uses Ajv 8.20.0 validators behind the `@probe-forge/contracts/validate` entry point; Rust uses `crates/contracts` with serde plus semantic validation. Both run the same valid/invalid fixture manifest in `packages/contracts/fixtures/v1/`. The contract cap is 256 KiB per payload. Recursive `safeJson` rejects executable keys case-insensitively. The existing health contract is unchanged.
- **Rationale:** One schema prevents drift, and shared fixtures fail both suites on disagreement. Keeping Ajv out of the root entry point avoids shipping a validator to the browser. Data-only contracts make arbitrary commands unrepresentable.
- **Alternatives:** Zod-only (Rust would duplicate rules); JSON Schema-only without semantic checks (goal/content and authorization consistency need code); separate hand-written schemas per language (drift risk).
- **Impact:** New fields require schema, both type sets, both validators if semantic, and fixture coverage. A breaking change requires a v2 bundle and a DB `schema_version` migration.

### D-020 - Forward-only migrations with schema-isolated tests

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 2 defines the core schema and needs a repeatable, non-destructive way to verify it against PostgreSQL.
- **Decision:** Use SQLx 0.9 runtime migrations in ordered `migrations/0001`-`0006` files. Never edit an applied migration; recover with a backup or a new compensating migration. No down migrations are maintained. The integration test creates a random `pf_test_*` schema, sets `search_path`, applies and re-applies migrations, seeds a full fixture chain, tests constraints/immutability, then drops the schema. It never touches `public`. `PROBE_FORGE_REQUIRE_DB_TESTS=1` turns a missing database into a failure.
- **Rationale:** Forward-only migrations are simple to audit and avoid unsafe automatic rollbacks. Schema isolation makes destructive tests safe in a shared local database.
- **Alternatives:** SQLx offline mode (no runtime DB test); a dedicated test database container (more infrastructure); editing migrations in place (unsafe for existing data).
- **Impact:** Production rollback is restore/forward-fix, not `migrate revert`. The local `db:reset` command is explicitly destructive.

### D-021 - Local Argon2id authentication with opaque server-side sessions

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 3 requires maintained authentication, secure sessions, CSRF protection, login rate limiting, logout, session revocation, secure cookies, and no long-lived tokens in browser storage. No OIDC provider or external identity service exists for local development.
- **Decision:** Use local email/password authentication with the maintained RustCrypto `argon2` 0.5.3 crate and Argon2id PHC hashes. Issue 256-bit opaque session tokens, store only SHA-256 digests in `auth_sessions`, and deliver the token in an `HttpOnly`, `SameSite=Strict` cookie that is `Secure` by default. Use a double-submit CSRF cookie/header pair, `Sec-Fetch-Site`/exact-origin checks for unsafe requests, database-backed login rate limits keyed by hashed account/address and address, and operator-only account provisioning. There is no public registration endpoint.
- **Rationale:** Argon2id is the recommended password KDF; opaque server-side sessions allow immediate revocation without a JWT blocklist. HttpOnly cookies plus CSRF protection keep credentials out of JavaScript-accessible storage. Database-backed limits survive process restarts and work across replicas.
- **Alternatives:** OIDC (no provider available for local development); JWT access tokens (harder to revoke, tempting to store in browser storage); in-memory rate limits (lost on restart, not shared).
- **Impact:** `PROBE_FORGE_COOKIE_SECURE` defaults to `true`; local HTTP development must explicitly set it to `false`. A trusted reverse-proxy client-IP design is still required before per-IP limits are meaningful behind a proxy. MFA and password reset are deferred. See `docs/AUTH.md`.

### D-022 - Hard project deletion deferred behind immutable evidence records

- **Date:** 2026-10-23
- **Status:** Accepted (temporary)
- **Context:** Phase 3 lists project management, but Phase 2 deliberately made `audit_log`, `plan_approvals`, `evidence`, and `job_events` append-only with update/delete rejection triggers. A hard `DELETE /api/projects/{id}` cascades into project-scoped tables and is blocked once those records exist.
- **Decision:** Phase 3 exposes project create/read/update, membership management, ownership transfer, and audit viewing. Hard deletion is not implemented. Do not weaken the immutable-record triggers to enable cascading deletes.
- **Rationale:** Audit and evidence integrity outranks a destructive convenience endpoint. Removing history to satisfy a delete request would undermine the product's evidence guarantees.
- **Alternatives:** Soft delete/archive with a retention workflow (needs its own phase design); cascade exceptions for admin deletion (weakens immutable evidence); no project deletion at all (current).
- **Impact:** Projects accumulate until an explicit archive/retention design is approved. The frontend has no delete control, and the integration tests do not assert deletion. Revisit before a user-facing project lifecycle feature is required.

### D-023 - Jobs API uses a projectId query parameter and shared queue crate

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 4 requires `POST /api/jobs` while Phase 2's `jobCreation` contract is closed (`additionalProperties: false`) and contains no `projectId`. The API and scheduler both need the same claim/fencing logic.
- **Decision:** Keep the v1 job creation body unchanged and require `?projectId=<uuid>` on `POST /api/jobs`. Validate the body with the shared contract validator (256 KiB cap, unknown/executable fields rejected). Put atomic claims, leases, heartbeats, fencing, retries, recovery, and due-run promotion in a shared `crates/queue` crate used by both the API and scheduler. Use the `Idempotency-Key` header, not a body field.
- **Rationale:** Avoids a breaking contract change and a second authorization interpretation. One queue implementation prevents API/scheduler drift. A query parameter is explicit and server-validated; it is not trusted for authorization.
- **Alternatives:** Add `projectId` to the job contract (requires a v2 bundle); use `/api/projects/{id}/jobs` only (does not satisfy the phase path); duplicate SQL in each service (drift and fencing bugs).
- **Impact:** The web client and any future worker must include the project ID. `docs/JOBS.md` documents the surface. A future v2 contract may move project scope into the body with a versioned migration.

### D-024 - Scheduler promotes due runs but does not dispatch them yet

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 4 scope)
- **Context:** Phase 4 requires scheduler claim logic for due run records, but execution adapters and dispatchers are Phases 10 and 12. Claiming a run and never processing it would create fake `running` rows and leases.
- **Decision:** The scheduler loop atomically promotes due `scheduled` runs to `queued` and recovers expired job leases. `claim_due_run` is implemented and tested for the future execution dispatcher but is not called by the background loop until a dispatcher exists.
- **Rationale:** Safe, observable progress without pretending execution happened. The same `FOR UPDATE SKIP LOCKED` and lease pattern is already proven by the queue tests.
- **Alternatives:** Claim and immediately release due runs (misleading churn); leave all run logic for Phase 10 (does not satisfy Phase 4); claim and mark running without an executor (fabricated state).
- **Impact:** Scheduler readiness requires the database. Phase 10/12 wires the dispatcher to `claim_due_run`, heartbeat, completion, and failure.

### D-025 - Phase 5 ingestion is a library and operator CLI, not an HTTP endpoint

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 5 scope)
- **Context:** Phase 5 requires safe Git ingestion but does not require an authenticated HTTP ingestion API. Adding an unauthenticated endpoint would bypass the Phase 3/4 authorization model; wiring the worker to the job queue and snapshot tables also needs a dispatcher and claim ownership that Phase 5 does not define.
- **Decision:** Implement ingestion as a typed analysis-worker library and an operator-only CLI. The worker's HTTP server remains health-only. The library returns an in-memory `RepositorySnapshot` plus a bounded `RepositorySession`; the CLI writes snapshot JSON and append-only JSONL audit events. Database persistence and authenticated ingestion requests are deferred.
- **Rationale:** Demonstrates the complete read-only pipeline without inventing an authorization boundary or half-wiring a worker/queue contract. Keeps the network attack surface unchanged until a later phase defines authenticated worker polling.
- **Alternatives:** Unauthenticated `/ingest` endpoint (rejected); worker polling with no dispatcher (fabricates claims); direct DB writes from the CLI (bypasses control-plane policy and tenant checks).
- **Impact:** Phase 6/7 must define how a claimed job supplies the source and receives the snapshot. Until then, ingestion is a local operator workflow, not a product API.

### D-026 - Git transport policy for Phase 5

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Phase 5 must accept supported Git URLs, resolve immutable commits, and protect credentials. The Git CLI can execute hooks, filters, submodule helpers, credential helpers, and remote helpers if its configuration is not constrained.
- **Decision:** Accept HTTPS (default port only) and SSH (default port only), plus `file:` only in explicit fixture mode. Require allowlisted hosts, reject embedded credentials, resolve DNS before clone and deny private/reserved answers, disable HTTP redirects, and keep TLS verification on. Use an isolated `HOME`, empty global/system config, empty hooks path, no credential helper, no inherited Git/SSH-agent variables, protocol allowlisting, submodule recursion off, LFS smudge downloads off, and `shell: false` argument arrays. HTTP basic credentials are injected as an `http.extraHeader`; SSH credentials are unencrypted private keys written only to the temporary workspace. Reject passphrase-protected keys and do not manage known-hosts files in this phase.
- **Rationale:** Least privilege and fail-closed behavior are more important than broad transport convenience. The chosen credential forms are testable without a secret manager and never appear in remote URLs or logs.
- **Alternatives:** Accept arbitrary hosts/schemes (rejected); use the ambient SSH agent (rejected); store plaintext credentials in the URL (rejected); support only public HTTPS (simpler but would not exercise the SSH job contract).
- **Impact:** SSH deployments must inject an unencrypted key and preconfigure host-key trust outside this phase. DNS rebinding remains a residual risk because the Git CLI resolves again; a later egress policy or address-pinned transport should close it. See `docs/REPOSITORY_INGESTION.md`.

### D-027 - Phase 6 discovery is a bounded library and optional CLI inventory

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 6 scope)
- **Context:** Phase 6 must produce a structured application model and Pi-facing discovery tools without creating an unauthenticated repository-analysis endpoint or inventing persistence/authorization contracts ahead of the worker dispatcher. Repository source is untrusted and scanners must remain deterministic and read-only.
- **Decision:** Implement discovery as a typed analysis-worker library that consumes only the existing `RepositorySession` and returns a separate, versioned `ApplicationInventory`. Keep the Phase 5 snapshot unchanged. Add an optional `--discovery-output` operator CLI artifact. Expose a pure, bounded query toolset (`summary`, `routes`, `route`, `surfaces`, `unsupported`) for Phase 7 to adapt to Pi. Treat all source as data, attach provenance/confidence/evidence, mark dynamic or unknown routes ambiguous, and report unsupported languages/frameworks. Do not persist inventories or add an HTTP endpoint in this phase.
- **Rationale:** This satisfies discovery without widening the network or authorization surface, preserves the immutable snapshot boundary, and keeps inventory size and future Pi context bounded. Deterministic parsers are honest about heuristic coverage rather than pretending to execute framework semantics.
- **Alternatives:** Add an HTTP discovery endpoint (rejected until authenticated worker dispatch exists); persist inventories immediately (no job/snapshot ownership contract yet); use an LLM to parse source (nondeterministic and violates the phase's deterministic scanner requirement); expose unrestricted file reads to Pi (rejected; typed query results only).
- **Impact:** Phase 7 must wire the toolset to Pi, define model-facing schemas/token budgets, and decide how inventories accompany claimed jobs. Frame parsers are line-oriented and intentionally incomplete; `docs/DISCOVERY.md` records supported frameworks and limitations. JAX-RS and browser/UI frameworks remain explicitly unsupported.


### D-028 - Phase 7 planning is a bounded library with server-only persistence

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 7 scope)
- **Context:** Phase 7 must integrate Pi, constrain its tools, validate plans, bound cost/time/output, survive model outages, and version plans without creating an unauthenticated planning endpoint or giving Pi database access. No worker dispatcher exists yet.
- **Decision:** Implement plan generation as a typed analysis-worker library. Pi receives only read-only repository/discovery/catalog tools plus `plan_validate` and `plan_propose`; strict v1 schema validation, policy checks, fallback generation, PostgreSQL persistence, version allocation, and audit writes remain server-side stages outside the model's tool set. Use one agent session per model attempt and a separate `probe-forge/deterministic-fallback-v1` session for a fallback draft. Allocate plan versions under a job-row lock, reject saves on terminal sessions, and keep the worker HTTP surface health-only until an authenticated dispatcher is designed.
- **Rationale:** This preserves least privilege and tenant isolation while satisfying plan generation and the Phase 7 gate without inventing queue/API authorization. A server-owned fallback keeps jobs recoverable without pretending the model succeeded. Faux-provider tests prove the integration offline and deterministically.
- **Alternatives:** Give Pi a direct database writer (rejected: bypasses policy and tenant checks); expose an unauthenticated planning endpoint (rejected: no worker auth contract); add a shell or generic HTTP tool (rejected: prompt injection could reach execution); fail the job on model outage (rejected: loses recoverability and fabricates a terminal state); persist raw model transcripts (rejected: secret and prompt-injection exposure).
- **Impact:** Phase 8 must add plan read/update/approval APIs and the authenticated worker claim/dispatch path. The vetted catalog is still empty, so a vulnerability fallback is unavailable until catalog entries exist. Load fallback is limited to sorted literal `GET`/`HEAD` routes and a conservative baseline profile. Plans are drafts, never approvals or run authorization. See `docs/PLANNING.md`.

### D-029 - Phase 8 approval edits only selections and records immutable snapshots

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 8 scope)
- **Context:** Phase 8 must let users review and select catalog tests, detect concurrent edits, and produce an immutable approval snapshot without creating a run or allowing arbitrary plan edits. The test catalog was only a minimal ID/name/risk scaffold, and the API had no plan review surface.
- **Decision:** Expand the embedded catalog to schema 1 / catalog version 0.2.0 with description, prerequisites, tool requirements, target requirements, timeout, evidence expectations, and confirmation requirements. Add read/update/approval routes. PATCH changes only `planned_tests.selected` and `load_scenarios.selected` under an optimistic `test_plans.revision` check; it cannot change names, rationale, risk, configuration, or executable strings. Approval requires an explicit confirmation flag, locks the plan and job, snapshots the exact selected rows and catalog metadata, marks the plan approved, supersedes older proposals, and writes an audit event. Report all execution adapters as unavailable until Phase 10. Add `plan_approvals.plan_revision` and a composite run/approval/plan/version foreign key so a run cannot cite an approval for another plan version.
- **Rationale:** Selection-only edits make arbitrary command injection unrepresentable, while revisioning and row locks provide deterministic conflict handling. The approval snapshot and composite foreign key enforce the no-run-without-approval invariant at the database layer, not only in UI code.
- **Alternatives:** Allow arbitrary plan field edits (rejected: executable/configuration injection); create a run on approval (rejected: target authorization and execution adapters are later phases); mark every catalog adapter available (rejected: no execution adapters exist yet); rely on UI-only approval checks (rejected: server/database enforcement is required).
- **Impact:** Phase 9 must add target registration and authorization; Phase 10 must populate the adapter availability allowlist and implement execution. The UI can review seeded or worker-generated plans but cannot create jobs. The catalog remains server-owned and any new entry requires both TS and Rust validation. See `docs/PLAN_APPROVAL.md`.

### D-030 - Target validation uses pinned DNS answers, explicit private mode, and fixed platform budgets

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 9 scope)
- **Context:** Phase 9 must register deployed targets and authorize scoped tests without relying on prompt instructions. The main risks are SSRF, DNS rebinding, redirect escapes, disallowed infrastructure ports, unbounded requests, and stale authorizations. The phase does not yet execute tests or run a real target.
- **Decision:** Add project-scoped target and authorization APIs. Put target destination, redirect, connection-pinning, and budget policy in the shared `crates/target-policy` crate so API validation and future execution adapters use one implementation. Normalize origins and require explicit host/path/port scope. Resolve destinations through a server resolver, reject any answer in loopback/private/link-local/metadata/multicast/reserved/documentation ranges by default, and store the verified address set. A separate `PROBE_FORGE_ALLOW_PRIVATE_TARGETS=false` flag permits loopback/RFC1918/ULA only in a secured local/private mode; link-local and metadata remain blocked. `GuardedHttpClient` disables proxies, connection reuse, and automatic redirects, pins the validated IPs with `resolve_to_addrs` per request, manually follows at most five redirects with full scope and downgrade checks, re-resolves on each hop, and caps bodies. `RunBudget` enforces rate, concurrency, duration, request, response-size, total-byte, and cancellation limits. Authorization records require explicit confirmation, a verified target, a 24-hour default/7-day maximum expiry, and immutable scope/limits; only one-time revocation is permitted. The v1 target contract shape stays unchanged; verification evidence uses migration `0010`.
- **Rationale:** Server-side destination checks are authoritative even if the model, repository, or user supplies a malicious URL. Pinning validated addresses closes the DNS rebind window between resolution and connection. Conservative default-deny behavior is safer than broad private-network support, while the explicit flag supports local mock fixtures.
- **Alternatives:** Trust DNS only at registration (rejected: rebinding); follow redirects in the HTTP library (rejected: escapes and credential leakage); allow all private ranges by default (rejected: SSRF into platform infrastructure); make response/total budgets user-editable (rejected: platform-owned caps); support ownership verification immediately (deferred: no verification mechanism exists).
- **Impact:** Phase 10 adapters must use `GuardedHttpClient`/`RunBudget` and cannot bypass target policy. The target workflow has no UI yet. Private-target mode requires operator review and is not suitable for an untrusted multi-tenant deployment as-is. Resolved address evidence is stored on the target and is revalidated at authorization and execution time.

### D-031 - Phase 10 starts with two in-process HTTP adapters and no external tools

- **Date:** 2026-10-23
- **Status:** Accepted (Phase 10 scope)
- **Context:** Phase 10 requires a small vetted tool set, pinned versions, typed argument builders, timeouts, output caps, cancellation, target/redirect scope, normalized results, sanitized evidence, and a gate that no arbitrary shell execution is possible. The catalog contains HTTP, TLS, static, dependency, rate-limit, and load entries, but no adapter implementation exists. Most external security tools would add executable discovery, process supervision, output parsing, and shell-injection risk before the run orchestration path exists.
- **Decision:** Ship a shared `crates/execution-adapters` library with two native adapters: `http.headers.missing` and `http.cors.misconfiguration`, both using the Phase 9 `GuardedHttpClient`/`RunBudget` under `http_probe` 0.1.0. Map catalog IDs to fixed adapter specs in a server-owned registry and derive API availability from that registry. Accept only a typed `route`/`path` configuration with unknown fields denied; reject unsafe paths and out-of-scope URLs before network access. Return a common `AdapterResult` with independent execution status and security outcome, plus minimized evidence and no response body/cookies. Do not add any shell, command field, executable path, or external process in this phase. Unsupported catalog IDs fail closed. Run creation and worker dispatch remain absent.
- **Rationale:** Native adapters satisfy the phase's execution and evidence goals with the smallest attack surface and make "no arbitrary shell execution" structural rather than a validation rule. Reusing the Phase 9 policy layer means target, redirect, DNS, size, and cancellation controls cannot drift between registration and execution.
- **Alternatives:** Add ZAP/nuclei/k6 shell adapters now (rejected: unvetted process boundary, no dispatcher/result persistence, and unnecessary risk before Phase 11/12); mark every catalog entry available (rejected: falsely implies execution); expose a generic HTTP method/header/body adapter (rejected: expands scope and credential risk); implement a process runner with no external adapter (deferred until a specific vetted tool is approved).
- **Impact:** Static secret/dependency, TLS, rate-limit, and load catalog entries remain unavailable. Adapter results are in memory only; Phase 13 owns persistence and reports. Phase 11/12 may add declarative load scenarios and a constrained k6 runner; any future external security adapter must use a fixed server-owned executable path, typed argument vector, bounded structured output, process limits, and process-tree termination on timeout/cancellation. See `docs/EXECUTION_ADAPTERS.md`.

### D-032 - Phase 11 load scenarios use typed correlation data and parent-plan versioning

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 11 scope)
- **Context:** Phase 11 requires a declarative load-scenario schema with steps, methods, path templates, secret-backed headers, payload templates, extraction/correlation, assertions, dependencies, safe profiles, and user review. The existing v1 schema had only flat requests, literal headers, a status list, and a broad profile; database profile limits were far above a conservative platform policy, and no target-scope validation existed for scenario paths.
- **Decision:** Evolve the v1 load contract in a backward-compatible way with optional step IDs, `dependsOn`, typed `secretHeaders`, typed header/JSON extraction and assertion rules, and `${name}` correlation placeholders. Keep `requests` as the ordered steps instead of introducing a second list. Require explicit expected statuses, cap body templates at 64 KiB, and tighten profile maxima to 100 VUs/100 arrivals per second, 300 s ramp, 900 s duration, 100,000 requests, and 100 requests/s. Replace arbitrary stop-condition JSON with a closed `errorRate`, `p95LatencyMs`, and `maxFailedRequests` object. Add migration `0011` `NOT VALID` database constraints for defense in depth. Validate every step against a chosen target scope with `validate_load_scenario_scope`, expanding template placeholders only for the non-network scope check. Version scenarios by their parent plan version and pin the exact JSON/profile in the immutable approval snapshot; keep selection-only plan editing. Do not add a load runner, scheduler, or traffic generation in this phase.
- **Rationale:** Typed correlation data makes the common login-token-then-request flow representable without executable scripts. Rejecting forward dependencies and unknown variables makes ordering deterministic. Parent-plan versioning reuses the already-tested immutable approval path instead of adding a second mutable version graph. Strict platform maxima and typed stop conditions remove broad, unbounded profile knobs before a runner exists.
- **Alternatives:** Add a separate v2 scenario schema (rejected: v1 is unshipped and the new fields can be optional/closed); allow arbitrary template expressions or loops (rejected: code-injection and unbounded execution risk); allow raw Authorization headers with a warning (rejected: credentials would enter prompts/plans); add independent scenario version rows now (deferred: no independent editing requirement and plan approvals already provide immutable evidence); rely only on the runner to enforce caps (rejected: generation and approval must fail closed).
- **Impact:** Existing simple scenarios remain valid because the new fields and stop conditions are optional/defaulted, but `expectedStatuses` is now required. Profiles above the new maxima and unknown stop-condition keys are rejected by contract validation and by new database checks for new/updated rows. Phase 12 must call the scope validator, resolve secret references outside prompts/plans, percent-encode extracted values, combine scenario profiles with target authorization limits, and implement cancellation/metrics. See `docs/LOAD_SCENARIOS.md`.

### D-033 - Phase 12 uses a constrained native runner, not model-authored scripts

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 12 scope)
- **Context:** Phase 12 must integrate a load runner through a constrained adapter or a generated vetted scenario representation, without turning model text into executable scripts. k6 is not installed in the development environment and adding an external binary needs a separate, approved process boundary (D-031). The scheduler also needs persisted, timezone-explicit schedules, leases, quotas, fairness, and cancellation, and the runner needs hard caps, metrics buckets, and stop conditions.
- **Decision:** Convert the already-validated, data-only `LoadScenario` into a typed `RunnerSpec` (fixed method/path/header/body templates, closed extraction/assertion rules) and execute it with a native `GuardedHttpClient`-based adapter in `crates/load-runner`. Effective limits are the minimum of the scenario profile, the target authorization, and platform caps, and the network layer re-applies the same caps. Do not invoke k6 or any external process, and do not synthesize scripts. Add `claim_due_run_fair` (advisory-locked global/per-project quotas), run leases/heartbeat/completion/failure, expired-lease recovery, and schedule cancellation. Persist schedules with a normalized timezone and a UTC `next_run_at`, and promote due schedules idempotently per occurrence. Aggregate per-bucket/per-endpoint/status metrics with nearest-rank p50/p95/p99 and upsert them into `run_metrics`.
- **Rationale:** A typed spec plus the existing target-policy client keeps "no arbitrary scripts" and "hard caps cannot be bypassed" structural, while still exercising real HTTP behavior against a local mock. Language-neutral k6 remains a future adapter behind the same spec once a pinned binary and process limits are approved.
- **Alternatives:** Generate k6 JavaScript from model/plan text (rejected: script injection and untestable without the binary); shell out to a locally discovered k6 (rejected: unapproved process boundary and no version pin); skip scheduling and metrics until Phase 13/15 (rejected: the phase requires them); make quotas best-effort without a lock (rejected: concurrent workers could exceed the global/project limits).
- **Impact:** The runner is a library re-exported by the execution worker; no worker dispatch loop is enabled yet, so no live run can start from the API. The scheduler now promotes due schedules, recovers expired schedules/run leases, and fails closed on missing approval or inactive authorization. `schedules.target_authorization_id`, `expires_at`, and `cancelled_at` are added in migration `0012`, and new schedules require an explicit target authorization. See `docs/LOAD_EXECUTION.md`.

### D-034 - Phase 13 stores results independently and pins every generated claim

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 13 scope)
- **Context:** Phase 13 must expose per-test results and a run summary, preserve raw structured observations and sanitized evidence independently of AI summaries, generate traceable recommendations, distinguish findings from checks and observed metrics from hypotheses, version reports, support rerun comparison, redact secrets, and export JSON plus a readable format. An AI claim must never appear without a stored reference or an explicit hypothesis label, and report generation must not destroy results.
- **Decision:** Add migration `0013`: `test_results.finding_status` and `test_results.observation`; `recommendations` with a `recommendations_traceability` check requiring a `result_id`, `evidence_id`, or `run_metric_id` unless `kind = 'hypothesis'`; `reports` and immutable `report_versions`. Serve `GET /api/projects/{id}/runs`, `GET /api/runs/{id}`, `GET/POST /api/runs/{id}/report`, `GET /api/runs/{id}/report/export`, and `GET /api/runs/{id}/compare` from `services/api/src/results.rs`. Report generation reads results, builds a redacted JSON document whose claims carry references or a hypothesis label, validates a 256 KiB cap before writing, and stores a new immutable version in its own transaction. An oversize document marks the report `failed` without touching results. Redaction runs recursively over sensitive keys and credential markers (Bearer/Basic/Authorization/Cookie/JWT).
- **Rationale:** Keeping reports separate from results makes generation failure non-destructive by construction, and a database-level traceability constraint prevents any writer from storing an unsupported claim. Deterministic system recommendations avoid invoking a model while still exercising the full traceability and versioning path; a model may later write hypotheses through the same constrained shape.
- **Alternatives:** Store generated summaries on `test_results` (rejected: couples model output to raw evidence and risks overwriting observations); allow untraceable recommendations (rejected: the core gate forbids it); generate reports synchronously in the request without versioning (rejected: loses history and rerun comparison); ship a web view without the API and tests (rejected: no server authority). A typed web view was then added on top of the tested API.
- **Impact:** Phase 14 can build retention and structured logging on these tables; Phase 15 owns the end-to-end load/result flow. Reports are read-only snapshots; regenerating always creates a new version. The web app renders the run summary, per-test results, evidence, recommendations, report claims, and comparison from a project's **Runs** list. The execution worker still does not produce results automatically. See `docs/RESULTS_REPORTS.md`.

### D-035 - Shared observability crate owns structured logs, correlation IDs, and redaction

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 14 scope)
- **Context:** Phase 14 requires structured logs with correlation IDs and redaction across the API, scheduler, and workers, and must never emit secrets in logs. Redaction already existed privately inside `services/api/src/results.rs`; duplicating a second implementation for logs would let the two drift.
- **Decision:** Add `crates/observability` with one `init_tracing(service)` entry point (text by default, `PROBE_FORGE_LOG_FORMAT=json` for line-delimited JSON), a validated `RequestId` type, the redaction functions, and a `Redacted` display wrapper. Move the single redaction implementation out of `results.rs` and import it. Add an HTTP `request_id` middleware that honors a bounded printable `X-Request-Id` or generates one, inserts it into request extensions, carries it on a `http_request` tracing span, and echoes the response header.
- **Rationale:** One implementation cannot drift; the middleware makes every API log line correlatable and the response header lets an operator tie a client-visible failure to server logs without exposing internals.
- **Alternatives:** Keep redaction in the API and copy it into workers (rejected: drift and duplicated tests); log an unbounded client-supplied request ID (rejected: log-injection risk); add a metrics/observability crate that owns HTTP concerns for the workers too (rejected: the workers do not serve authenticated HTTP).
- **Impact:** The API, scheduler, and execution worker all initialize tracing through the crate. Phase 15 can add job/run provider spans on the same correlation basis.

### D-036 - Quotas and rate limits are enforced server-side with database-backed state

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 14 scope)
- **Context:** Phase 14 requires per-project quotas, concurrency limits, request budgets, fair scheduling, and rate limits for the API and expensive operations, all enforced server-side rather than in the UI.
- **Decision:** Add migration `0014` with `project_quotas` (per-project `max_active_runs`, optional `max_requests_per_window`, `window_seconds`) and `api_rate_limits` (SHA-256-digested key, fixed window). Extend `claim_due_run_fair` to read the per-project active-run cap via `COALESCE(project_quotas, default)` and to refuse a claim whose project has exhausted its request budget for the window (recorded `run_metrics` plus the candidate profile's reserved maximum). Expose `GET|PUT /api/projects/{id}/quota` (owners/admins may write; any member may read) with an audit record. Add an authenticated per-user API rate-limit middleware and a stricter "expensive" bucket for job creation and report generation. `AuthConfig` carries both limits and `Config::from_env` reads them.
- **Rationale:** Quotas and budgets must not depend on client cooperation, and the advisory-locked claim already serializes concurrent workers, so the same transaction can safely evaluate the budget. Digesting rate-limit keys avoids storing identities in the clear.
- **Alternatives:** In-memory-only rate limits (rejected: not durable or multi-instance safe); enforce quotas only in the execution worker (rejected: the claim path is the authoritative gate); allow any member to change quotas (rejected: quotas bound server resources).
- **Impact:** A missing `project_quotas` row keeps the previous default behavior, so existing projects are unaffected. Phase 15 wires the worker dispatch loop to the same claim function.

### D-037 - Retention is a single sanctioned, opt-in purge path

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 14 scope)
- **Context:** Phase 14 requires retention for repositories, evidence, reports, and events. Evidence, report versions, job events, approvals, and audit rows are protected by the `reject_immutable_row` trigger; deleting them would appear to weaken a security control.
- **Decision:** Replace `reject_immutable_row` so it still rejects every UPDATE and every DELETE, except a DELETE inside a transaction that opts in with `SET LOCAL probeforge.retention_purge = 'on'`. The only code that sets that flag is `probe_forge_queue::run_retention`, which runs from the scheduler in bounded batches and logs counts. It purges evidence past `retention_until` that is not referenced by a recommendation, reports older than the report window (cascading their versions), old job events for terminal jobs, expired sessions, and stale rate-limit/login counters. Repository working directories remain cleaned by the ingestion worker (Phase 5); the DB retention concerns stored artifacts.
- **Rationale:** A transaction-local opt-in keeps the default tamper-resistance unchanged and makes the exception explicit, auditable, and testable. Excluding recommendation-referenced evidence avoids violating the Phase 13 traceability constraint.
- **Alternatives:** Drop the immutability triggers (rejected: weakens a control globally); never delete immutable rows (rejected: fails the retention requirement); mark rows deleted in a new column (rejected: more complex and leaves data resident).
- **Impact:** A regression test asserts ordinary deletes still fail. Retention intervals are configuration (`PROBE_FORGE_RETENTION_*`). Policy may be tuned without a migration.

### D-038 - Backup/restore is scripted with a primary-database guard and a tested roundtrip

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 14 scope)
- **Context:** Phase 14 requires a documented and tested PostgreSQL backup and restore procedure.
- **Decision:** Add `scripts/db-backup.mjs` with `backup`, `restore`, and `check` modes that use `pg_dump`/`psql` inside the local Compose container. `restore` refuses to target the database named by `DATABASE_URL` unless `--force` is passed. `check` performs a real dump and restore into a throwaway scratch database, verifies the migration count matches, and drops the scratch database, never modifying the source.
- **Rationale:** A real roundtrip is stronger evidence than a documented procedure alone, and the scratch-database approach is safe for a developer machine.
- **Alternatives:** Document only without a test (rejected: gate requires tested); restore in place (rejected: destructive); require a host `pg_dump` (rejected: it may be absent, while the container always has it).
- **Impact:** `bun run db:backup`, `db:restore`, and `db:backup:check` are available; `backups/` is gitignored. See `docs/OPERATIONS.md`.

### D-039 - Runs are materialized from an approved plan plus an active authorization

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 15, in progress)
- **Context:** Phase 13 consumed `test_runs`, but nothing created them: approving a plan records an immutable selection only, and `test_runs` requires a non-null target authorization and a run type that matches the selection shape. Phase 15 must start work without inventing an unapproved execution path.
- **Decision:** Add `POST /api/plans/{plan_id}/runs` (body `{ targetAuthorizationId }`) that, for an approved plan, verifies the authorization belongs to the project and is active, then creates one `queued` run per approved planned test (vulnerability) or load scenario. Creation is idempotent per `(approval, selection)` via the run idempotency key, and each new run appends a `run.queued` job event and a `run.queued` audit row. Add `POST /api/runs/{run_id}/cancel` for pre-execution states; a leased running run is refused so its worker stops it and reports the outcome.
- **Rationale:** Keeping selection (approval) separate from execution (run + authorization) preserves the Phase 8 gate that no run exists without approval, and reusing the existing idempotency key avoids duplicate work on retry. Runs stay `queued` until the execution worker claims them, which keeps the security-sensitive network path gated behind the queue.
- **Alternatives:** Create runs at approval time (rejected: no target authorization exists yet and it would start work without an explicit execution choice); let the frontend talk to the queue (rejected: server-owned authority); cancel running runs directly (rejected: would race the lease owner).
- **Impact:** The API can now produce queued runs, but the execution worker does not yet claim them and the analysis worker does not yet turn a queued job into a plan, so Phase 15's exit gate is not met. Both remain the next work items.

### D-040 - Execution dispatch is an operator binary, not an unauthenticated loop

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 15)
- **Context:** Phase 15 must run both workflows end to end, but the execution worker had no database access or dispatch. Giving the health-only HTTP service an automatic claim loop would let queued runs start without an operator present.
- **Decision:** Add `dispatch_once(pool, config)` to `services/execution-worker` and expose it through the operator `dispatch` binary (`--once` drains the queue and exits). It claims via `claim_due_run_fair`, loads the authorization scope/limits and target environment, and executes vulnerability runs with the vetted adapters (persisting `test_results` and evidence) or load runs with the constrained runner (persisting `run_metrics`). Transient target failures retry with backoff; scope violations fail closed. The HTTP service stays health-only.
- **Rationale:** The project already treats ingestion as an operator CLI (Phase 5). Keeping dispatch operator-driven preserves "no work runs without a human," keeps the security-sensitive network path behind an explicit action, and stays testable by calling the same function from the acceptance test.
- **Alternatives:** Add a background dispatch loop to the health service (rejected: implicit execution and no clear stop); expose an HTTP dispatch endpoint (rejected: new authenticated surface and blast radius); run execution inside the API (rejected: couples the control plane to network execution).
- **Impact:** Continuous operation needs an external supervisor to run the binary. Results map to the Phase 13 shapes (`finding_status`/`severity`/`observation`); evidence is redacted and hashed. See `docs/RELEASE_READINESS.md`.

### D-041 - The analysis job loop is operator-invoked and falls back deterministically

- **Date:** 2026-10-24
- **Status:** Accepted (Phase 15)
- **Context:** Phase 15 needs the vulnerability workflow to start from an inert fixture repository. The analysis worker had ingestion, discovery, planning, and a plan store, but no job claim loop.
- **Decision:** Add `processJobOnce(connectionString, options)` (`src/job-loop.ts`) and the `bun run plan` CLI. It claims one queued job with `FOR UPDATE SKIP LOCKED`, ingests the pinned repository read-only with explicit local-fixture opt-in, runs deterministic discovery, and generates a draft plan. With no configured model it uses the existing deterministic fallback planner, then moves the job to `awaiting_approval`; retryable outcomes return it to the queue with backoff.
- **Rationale:** Reusing the tested discovery and fallback planner keeps the loop small and never contacts a model or a target, matching the repository's no-execution invariant. Operator invocation mirrors ingestion.
- **Alternatives:** Add a background loop to the worker HTTP service (rejected: implicit work); require a configured model (rejected: fallback exists and is sufficient for acceptance); generate the plan inside the API (rejected: planning is the worker's job).
- **Impact:** The full chain (job → plan → approval → runs → dispatch → report) is exercised by the Phase 15 acceptance tests. Continuous operation needs an external supervisor.

### D-042 - Analysis and execution loops are opt-in env flags

- **Date:** 2026-10-24
- **Status:** Accepted
- **Context:** D-040 and D-041 made analysis and execution operator CLIs. That left the product headless: a repository connected from the browser produced a job that never became a plan, and a queued run never executed, so the UI looked broken.
- **Decision:** Keep the operator binaries but add opt-in background loops: `PROBE_FORGE_ANALYSIS_JOB_LOOP` runs `processJobOnce` on an interval inside the analysis worker, and `PROBE_FORGE_EXECUTION_DISPATCH_LOOP` runs `dispatch_once` on an interval inside the execution worker. Both default to off in `.env.example` and are enabled in the local `.env`. `processJobOnce` now requeues or fails a job when ingestion or planning throws, instead of leaving it leased.
- **Rationale:** A self-driving local app is what makes the workflow usable, while an explicit flag keeps tests and CI deterministic and preserves "no work without a deliberate action" for production.
- **Alternatives:** Make the loops always-on (rejected: implicit execution in tests/CI); keep CLIs only (rejected: headless product); add HTTP trigger endpoints (rejected: long-running requests and a new authenticated surface).
- **Impact:** `bun run dev` yields a working connect → analyze → approve → target → run → report flow. Continuous non-local operation still wants an external supervisor. See `docs/OPERATIONS.md`.

### D-043 - One logging contract across libraries, services, and the analysis worker

- **Date:** 2026-10-24
- **Status:** Accepted
- **Context:** Phase 14 added `crates/observability`, but logging was uneven: the API, scheduler, and execution worker emitted some events, while `crates/queue`, `crates/target-policy`, `crates/execution-adapters`, and `crates/load-runner` emitted none, the analysis worker had a single ad-hoc `console.log` in `index.ts`, and there was no shared level/field/event contract.
- **Decision:** Adopt one logging contract (documented in `crates/observability` and `docs/OPERATIONS.md`): levels `error`/`warn`/`info`/`debug`; every line carries `service` and a stable `event` name plus the identifiers in scope (`request_id`, `project_id`, `job_id`, `run_id`, `worker_id`) and `error_code` on failures; values are redacted before emission. Library crates emit `tracing` events but never initialize a subscriber. The analysis worker gets a dependency-free `src/logging/logger.ts` that mirrors the Rust redaction and is configured by `PROBE_FORGE_LOG_LEVEL`/`PROBE_FORGE_LOG_FORMAT`.
- **Rationale:** Correlated, greppable events are what make the incident runbook usable. Libraries that emit but do not initialize let a service own its subscriber while the reusable crates stay testable and side-effect-free. A hand-written worker logger avoids a new dependency (AGENTS.md) and keeps redaction parity with Rust.
- **Alternatives:** Add a third-party JS logger (e.g. pino) (rejected: new dependency and duplicated redaction); instrument library well using a global exporter (rejected: libraries must not own global state); log full request/response bodies for detail (rejected: secret and evidence leakage).
- **Impact:** Rust crates take a `tracing` dependency (already a workspace pin). The level of detail is bounded: hot paths log at `debug`, `info` is reserved for transitions and outcomes. No control flow depends on logging. See `docs/phases/14a_logging_hardening_report.md`.

### D-043 - Platform admin is settings-only; secrets are encrypted at rest

- **Date:** 2026-10-24
- **Status:** Accepted
- **Context:** Pi configuration and its API key were env-only, with no UI and no DB, and there is no platform-admin concept. The product needed an admin dashboard to configure all platform settings, including a provider API key.
- **Decision:** Add migration `0015`: `platform_admins` (grants access to `/api/admin/*` only, no cross-tenant access) and a singleton `platform_settings` row holding planning provider/model/limits, an encrypted provider API key, API rate limits, retention windows, and project defaults. Provider keys are stored only as AES-256-GCM ciphertext (`v1.<nonce>.<ct>`) with the master key from `PROBE_FORGE_SETTINGS_KEY`; the API is write-only and returns only a hint. Seed a separate `admin@probeforge.local` via `seed-dev`. The analysis worker decrypts the key (mirroring the same envelope in TS) and passes it into the Pi `streamSimple` call; the request middleware reads a cached copy of the rate limits; the scheduler reads retention; the execution worker reads the default concurrency cap. Env remains the fallback, then the deterministic planner.
- **Rationale:** Settings-only keeps the admin from becoming a cross-tenant superuser. Encrypting at rest with a key held only in the environment avoids storing plaintext secrets in the database while allowing dashboard configuration. Passing the key explicitly to `streamSimple` (confirmed supported) avoids mutating process env.
- **Alternatives:** Cross-tenant superuser (rejected: large isolation change, not requested); env-only key (rejected: no dashboard configuration); plaintext key column (rejected); a new secret store (rejected: scope/dependency).
- **Impact:** A master key is required to store a provider key (fails closed otherwise); the worker needs the same key to decrypt. Rate limits apply live; retention and default concurrency are read by their services. The frontend gains an **Admin** view visible only to platform admins. See `docs/OPERATIONS.md`.

### D-044 - Planners can target an admin-configured OpenAI-compatible provider

- **Date:** 2026-10-24
- **Status:** Accepted
- **Context:** D-043 (platform admin) let an admin set the planning provider/model, but the worker only resolved providers from the pinned Pi SDK catalog (`builtinModels().getModel`). Nebius AI Token Factory and other OpenAI-compatible endpoints are not in that catalog, so configuring them silently produced a fallback plan.
- **Decision:** Add migration `0016`, extending `platform_settings` with `planning_base_url`, `planning_context_window`, and `planning_max_output_tokens`. When a base URL is set, the worker registers a custom `openai-completions` provider via `createProvider` on the collection and it takes precedence over a built-in provider of the same id; otherwise the pinned catalog path is unchanged. The base URL must be HTTPS and must not resolve to (or literally be) a loopback, private, link-local, reserved, or cloud-metadata host, enforced at save time (`admin.rs`) and again at run time (`agent/model.ts`, reusing `repository/net.ts`). `openai-completions` is the only wire protocol exposed. The model metadata fields are optional (defaults 32768/4096) and the output budget must fit inside the context window.
- **Rationale:** Registering a custom provider is the SDK's documented extension point and keeps one credential path (the encrypted, write-only key already passed explicitly to `streamSimple`). Two-layer base-URL validation gives immediate dashboard feedback and defense in depth against an out-of-band database edit. Blocking internal hosts follows the project's existing SSRF stance even though the destination is admin-controlled.
- **Alternatives:** Add a new built-in provider for each vendor (rejected: bundled catalog, not admin-configurable); accept arbitrary URLs with no filtering (rejected: SSRF surface); expose all Pi API types (rejected: per-API compatibility caveats and larger surface, not yet needed); DNS resolution at save time (rejected: still bypassable by rebinding; the worker check remains best-effort).
- **Impact:** `platform_settings` gains three columns and constraints; the admin view gains Base URL, Context window, and Max output tokens fields; the worker runs a custom provider when a base URL is set. Tool calling is required for the target model. Known limitation: DNS-rebinding is only mitigated best-effort because the Pi SDK performs the fetch. See `docs/phases/planning_custom_provider_report.md`.

### D-045 - Analysis ingestion fetches complete blobs and forbids lazy fetching

- **Date:** 2026-10-24
- **Status:** Accepted
- **Context:** Phase 5 ingestion fetched the pinned commit with `--filter=blob:none`, then ran `git ls-tree -r -l` to read blob sizes for the `maxFiles`/`maxTotalBytes` gate before checkout. In a blobs-less partial clone those sizes are not available locally, so `ls-tree -l` performed lazy on-demand blob fetches over the network for every entry and exceeded `gitTimeoutMs`, failing the job with `git_timeout` after exhausting its retries.
- **Decision:** Fetch the pinned commit completely (drop `--filter=blob:none`) and set `GIT_NO_LAZY_FETCH=1` in the isolated git environment so any missing object fails fast instead of reaching the network. `ls-tree -l` and its pre-checkout size gate are retained.
- **Rationale:** The worker checks out and reads every file, so a partial clone saved nothing; it only replaced one bulk pack download with many slow on-demand fetches. The fetch remains `--depth=1 --no-tags` against the resolved commit, so history is still bounded.
- **Alternatives:** Keep the blobs-less clone and enumerate with `ls-tree -r -z` (no `-l`) to gate file count/depth before download, then fetch blobs in a second pass (rejected: git has no sizes-only fetch and backfilling blobs into a partial clone is git-version-sensitive; the byte gate still could not precede the download). Raise `gitTimeoutMs` (rejected: hides the root cause).
- **Impact:** The `maxTotalBytes` check now runs after the blob download rather than before it; content is still never read or processed before the checks, `--depth=1` bounds history, and the workspace is ephemeral. This changes the Phase 5 "validate before ingesting" posture for download bytes only, and is recorded here for that reason. `RepositoryIngestError` also gains a non-sensitive `operation` label (the git subcommand) so failures such as `git_timeout` name the command that failed.

### D-046 - Repositories are a project-scoped registry removed by archiving

- **Date:** 2026-10-24
- **Status:** Accepted (post-phase enhancement)
- **Context:** `git_sources` was only ever written as a side effect of job creation, so a project could not list or remove a repository, and the UI's "Connect a repository" form silently created a job. The user asked for one or more repositories attached to a project with the option to delete them. `jobs.git_source_id` is `ON DELETE RESTRICT`, and `job_events`, `plan_approvals`, and `evidence` reject deletes, so hard-deleting a repository with history is impossible without destroying immutable records.
- **Decision:** Treat `git_sources` as the registry (migration `0017` adds optional `display_name` and `archived_at`). Add `GET`/`POST /api/projects/{id}/repositories` and `DELETE /api/projects/{id}/repositories/{repositoryId}`. Removal archives (`archived_at = now()`), hides the row from the active list, blocks new analysis, and is idempotent; it is refused with `409 repository_busy` while a `queued`/`analyzing`/`planning` job exists. Registering or creating a job for the same URL clears `archived_at`. Registration and analysis are separate operations: registration requires a non-viewer, removal requires owner/admin, and analysis reuses the unchanged `POST /api/jobs` contract.
- **Rationale:** Archiving keeps immutable plans, approvals, runs, evidence, and events referenced while giving the user a working "Remove" action. Decoupling registration from analysis makes multi-repository projects coherent without adding a new job-creation contract.
- **Alternatives:** Hard delete only when unused (rejected: the button fails for every analyzed repository); archive-when-used + hard-delete-when-unused (rejected: inconsistent semantics); a new `repositories` table (rejected: duplicates `git_sources` and the existing snapshot FK); repo-scoped job creation endpoint (rejected: `POST /api/jobs` already upserts the same row, no contract change needed).
- **Impact:** New migration and API surface; the project view gains a Repositories panel and `ConnectRepository.tsx` is replaced by `RepositoriesPanel.tsx`. `scripts/check-docs.mjs` required files updated. True erasure would need a dedicated retention path; none is added. See `docs/REPOSITORIES.md` and `docs/phases/repositories_rerun_report.md`.

### D-047 - Terminal jobs are requeued by an explicit operator re-run

- **Date:** 2026-10-24
- **Status:** Accepted (post-phase enhancement)
- **Context:** Failed jobs were terminal with no re-run path, so an operator had to create a new job to retry. Automatic retries are bounded and attempt-scoped, and `jobs_attempt_range` caps `attempt` at `max_attempts`, so a re-run must reset the budget deliberately.
- **Decision:** Add `POST /api/jobs/{jobId}/rerun` for `failed`, `cancelled`, and `expired` jobs. In one row-locked transaction it resets `status='queued'`, `attempt=0`, `not_before=now()`, clears worker/lease/heartbeat/last-error/commit/plan fields, appends the new `job.requeued` event (added to the v1 event enum, TS/Rust types, and shared fixtures), and writes the `job.rerun_requested` audit row. Non-terminal jobs return `409 job_not_requirable`; viewers `403`; non-members `404`. The transition lives in the API alongside cancellation rather than in `crates/queue`, keeping the status update, event, and audit in one transaction.
- **Rationale:** Re-running is an operator-initiated control-plane action like cancellation, not a worker-driven queue transition, so colocating it with `cancel_job` preserves atomicity and matches existing precedent. Clearing the pinned commit and plan forces a clean re-analysis without touching retained history.
- **Alternatives:** Reuse the existing automatic retry/backoff path (rejected: coupled to attempt exhaustion and does not restore the budget or emit an operator-visible event); allow only `failed` (rejected: the user chose failed + cancelled + expired); put the transition in `crates/queue` (rejected: would split the audit write from the state change).
- **Impact:** Additive v1 event type (`job.requeued`); no migration. The web jobs list gains a "Re-run" action on terminal jobs. See `docs/JOBS.md` and `docs/phases/repositories_rerun_report.md`.

## Open questions

| ID | Question | Needed by | Notes |
|---|---|---|---|
| Q-002 | License and ownership terms | Before any public distribution | No license file exists; do not publish until decided. |
| Q-003 | Private-network target mode (explicitly supported private targets) | Phase 9 | Default-deny remains; a server-only `PROBE_FORGE_ALLOW_PRIVATE_TARGETS` flag exists for secured local/private environments (D-030), but a product approval flow and multi-tenant isolation review are still required. |
| Q-004 | Git remote / hosting, branch protection, and CI | Before collaborative work | Local repository only; CI adoption tracked by D-011. |
