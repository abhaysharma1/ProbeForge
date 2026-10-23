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

## Open questions

| ID | Question | Needed by | Notes |
|---|---|---|---|
| Q-002 | License and ownership terms | Before any public distribution | No license file exists; do not publish until decided. |
| Q-003 | Private-network target mode (explicitly supported private targets) | Phase 9 | Default-deny remains; a separate secured mode requires explicit product and security review. |
| Q-004 | Git remote / hosting, branch protection, and CI | Before collaborative work | Local repository only; CI adoption tracked by D-011. |
