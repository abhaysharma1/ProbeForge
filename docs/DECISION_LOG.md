# ProbeForge decision log

Records implementation decisions not fixed by the five normative documents, plus deferred choices and open questions. The phase plan must not be silently reinterpreted; record the interpretation here.

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
- **Decision:** Tool versions observed in the Phase 0 environment (Node.js 24.15.0, npm 11.12.1, Rust 1.98.1, Docker 29.4.0) are observations, not commitments. Phase 1 confirms and pins the supported versions in lockfiles and CI.
- **Rationale:** Pinning without a build or lockfile cannot be enforced; recording observations avoids pretending they are contract pins.
- **Impact:** Phase 1 must produce reproducible installs (lockfiles, pinned images) and state the chosen versions.

### D-004 - Analysis worker runtime (Bun vs Node.js LTS) deferred to Phase 1

- **Date:** 2026-10-23
- **Status:** Deferred
- **Context:** Document 04 permits Bun or a supported Node.js LTS but requires choosing one and pinning it without mixing runtimes casually.
- **Decision:** Make and record the choice in Phase 1 when the worker is initialized.
- **Constraint:** Exactly one runtime is chosen, pinned, and used consistently by all TypeScript services.
- **Impact:** No worker runtime is assumed by the Phase 0 contract.

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

### D-007 - Package manager and monorepo tooling deferred to Phase 1

- **Date:** 2026-10-23
- **Status:** Deferred
- **Context:** Phase 1 creates the monorepo skeleton and root command interface; documents do not mandate a JavaScript package manager or monorepo tool.
- **Decision:** Choose and document the package manager and workspace tooling in Phase 1. The Phase 0 docs check deliberately uses Node.js built-ins only, so it commits to no package manager and installs no dependencies.
- **Impact:** No `package.json`, lockfile, or workspace configuration is created in Phase 0.

### D-008 - Product naming

- **Date:** 2026-10-23
- **Status:** Accepted
- **Context:** Document 04 requires consistent use of the name ProbeForge and forbids claiming domain or trademark availability.
- **Decision:** Use **ProbeForge** in the README, UI, page metadata, user-facing copy, and docs. Make branding configurable where branding is normally configurable. Third-party dependency and package names are not renamed.
- **Impact:** No domain or trademark claim is made anywhere.

## Open questions

| ID | Question | Needed by | Notes |
|---|---|---|---|
| Q-001 | Analysis worker runtime: Bun or Node.js LTS? | Phase 1 | See D-004; choose one and pin it. |
| Q-002 | License and ownership terms | Before any public distribution | No license file exists; do not publish until decided. |
| Q-003 | Private-network target mode (explicitly supported private targets) | Phase 9 | Default-deny remains; a separate secured mode requires explicit product and security review. |
| Q-004 | Git remote / hosting and branch protection | Before collaborative work | Local repository only at Phase 0. |
