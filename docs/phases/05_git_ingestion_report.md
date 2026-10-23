# Phase 5 — Read-only Git ingestion report

**Status: PHASE 5: PASS**

**Date:** 2026-10-23

## Scope delivered

Phase 5 adds a typed, read-only ingestion library and operator CLI to
`services/analysis-worker`:

- Git URL validation for HTTPS, SSH, and explicit local-fixture `file:` URLs;
- host allowlisting, default-port enforcement, embedded-credential rejection,
  DNS resolution checks, and private/reserved/metadata address denial;
- branch, tag, and full-SHA resolution to an immutable commit;
- pinned fetch/checkout in a unique temporary workspace with fixed `git`
  argument arrays, isolated Git configuration, disabled hooks, disabled
  submodule recursion, disabled LFS smudging, fsck, timeouts, and output caps;
- bounded tree inspection, file reads, workspace size, file count, byte totals,
  depth, inventory, and dependency limits;
- inert manifest/lockfile parsing with explicit parse errors and no script body
  execution;
- language/framework inventory and a metadata-only snapshot;
- JSONL or in-memory audit events for clone start, resolution, snapshot ready,
  failure, cleanup, and cleanup failure;
- guaranteed workspace cleanup on success, failure, and cancellation.

The worker HTTP surface remains health-only. No ingestion endpoint, database
persistence, scanner, load generator, external target, or model call was added.

## Files added or changed

**Added**

- `services/analysis-worker/src/repository/`
  - `audit.ts`
  - `errors.ts`
  - `git-url.ts`
  - `git.ts`
  - `index.ts`
  - `ingest.ts`
  - `inspect.ts`
  - `limits.ts`
  - `net.ts`
  - `process.ts`
  - `secrets.ts`
  - `text.ts`
  - `tree.ts`
  - `workspace.ts`
  - `testing.ts` (test-only fixture helpers)
  - unit/integration tests for the modules above
- `services/analysis-worker/fixtures/inert-repo/` — TypeScript, Python, Rust,
  Go, Java, Ruby, PHP, C#, manifest, lockfile, malformed-manifest, Makefile, and
  install/build/test/start script fixtures
- `services/analysis-worker/src/ingest-cli.ts` — operator CLI
- `docs/REPOSITORY_INGESTION.md`
- `docs/phases/05_git_ingestion_report.md` (this report)

**Changed**

- `services/analysis-worker/src/config.ts` — ingestion limits, host allowlist,
  fixture mode, audit path
- `services/analysis-worker/src/config.test.ts`
- `services/analysis-worker/package.json` — `ingest` script
- `biome.json` — fixture directory excluded from linting
- `.env.example` — Phase 5 settings and credential-reference guidance
- `README.md`, `docs/README.md`, `docs/JOBS.md`,
  `docs/IMPLEMENTATION_CHECKLIST.md`, `docs/DECISION_LOG.md`
- `scripts/check-docs.mjs` — require and link the new ingestion/phase docs

## Verification evidence

### TypeScript checks

Command:

```bash
bun run check:ts
```

Observed result: PASS.

- Biome: 105 files checked, no fixes applied.
- Type checks: contracts, test-catalog, web, and analysis-worker all exited 0.
- Tests: 55 analysis-worker tests passed across 12 files (317 assertions);
  contracts 46, test-catalog 4, web 5.

### Documentation contract

Command:

```bash
bun run check:docs
```

Observed result after adding the Phase 5 documents: PASS.

### Targeted analysis-worker checks

Commands:

```bash
bun run --cwd services/analysis-worker typecheck
bun run --cwd services/analysis-worker test
```

Observed result: PASS; 55 tests, 0 failures.

### Runtime CLI smoke test

A temporary local fixture repository was initialized with a pinned commit and
analyzed through the real CLI:

```bash
bun services/analysis-worker/src/ingest-cli.ts \
  --fixture <temp>/fixture \
  --ref main \
  --audit-log <temp>/audit.jsonl \
  --output <temp>/snapshot.json
```

Observed result:

- exit code `0`;
- snapshot `commitSha` matched `git rev-parse HEAD`
  (`cda74d054f300ee1d97071543305bd0353a212cf`);
- `analysisMetadata.fileCount = 23`, `executedRepositoryCode = false`;
- detected languages included TypeScript, Python, Rust, Go, Java, C#, Ruby, PHP,
  JavaScript, JSON, TOML, XML, Markdown, Shell, and Make;
- audit events were written in order: `repository.clone.started`,
  `repository.reference.resolved`, `repository.snapshot.ready`,
  `repository.workspace.cleaned`;
- the `fixture-executed.txt` marker did not exist after ingestion.

### Gate mapping

| Exit-gate item | Evidence |
|---|---|
| Repository content is never executed | Fixture install/build/test/start scripts and `scripts/pwn.js` marker; ingestion tests and smoke test assert the marker is absent. No install/build/test/start command is invoked. |
| Analyze a local inert fixture at a pinned commit | Fixture integration tests resolve `main`, tags, exact refs, and full SHAs; the runtime CLI smoke test used a pinned commit. |
| Invalid schemes and unsafe paths rejected | Unit tests cover `http:`, `git:`, credential-bearing URLs, unknown/private hosts, non-default ports, queries/fragments, encoded traversal, backslashes, `.git` segments, symlink prefixes, gitlink prefixes, and deep paths. |
| Cleanup on success, failure, cancellation | Workspace tests cover success, thrown failure, cancellation, and read-only files; ingestion tests cover reference failure and post-creation cancellation. |
| No live repository or external target | All clone tests use `file:` local fixtures; DNS tests use injected resolvers; no network target, scanner, or load generator is contacted. |

## Security notes

- No repository-provided code, script, hook, submodule, filter, or LFS content is
  executed or downloaded as part of inspection.
- The remote URL never contains credentials. HTTPS credentials are injected via
  Git configuration environment; SSH keys are written only to the temporary
  workspace. Error text is redacted before it reaches messages or audit data.
- DNS is resolved and checked before clone, but Git resolves again. The host
  allowlist and disabled redirects reduce DNS-rebinding risk without eliminating
  it; a later network-egress policy or address-pinned transport is required for
  a hardened deployment.
- Snapshot metadata is bounded to 60 KiB before persistence is considered; the
  database column limit is 64 KiB.
- No real `.env`, credential, key, target detail, or scan evidence was added.

## Known limitations

- Ingestion is not an HTTP API and is not wired to the PostgreSQL job queue.
  Snapshot and audit persistence are deferred.
- SSH support requires an unencrypted private key and externally managed host-key
  trust; passphrase-protected keys and agent forwarding are not supported.
- Submodule contents and Git LFS objects are intentionally not fetched.
- The DNS check is a preflight check, not a connection-time address pin.
- `bun run check:db` was not rerun because Phase 5 adds no migrations or database
  code.
- `bun run check:services` was not rerun; the known local port conflict from
  Phase 4 is unrelated to this change. The API/scheduler runtime was not changed.
- The full `bun run check` (Rust fmt/clippy/tests plus all TypeScript checks)
  passed after the final code and documentation edits; see the final verification
  section.

## Final verification

Commands:

```bash
bun run check:docs
bun run check:ts
bun run check
```

Observed result: PASS (`exit 0`).

- `check:docs`: 60/60 required files, 8 linked documents, 16 checklist phases,
  no tracked `.env`, secret scan clean.
- `check:rust`: `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, and `cargo test --workspace` passed.
- `check:ts`: Biome checked 105 files; all four package typechecks and tests
  exited 0 (analysis-worker 55 tests, contracts 46, test-catalog 4, web 5).

## Next permitted phase

**Phase 6 — Static discovery and application model**, only after explicit
authorization. Phase 6 must consume the bounded snapshot/session and must not
execute repository code.
