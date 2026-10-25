# Post-phase enhancement — project repositories and job re-run

**Date:** 2026-10-24
**Status:** Complete
**Decisions:** [D-046](../DECISION_LOG.md), [D-047](../DECISION_LOG.md)

## Summary

Two operator-facing capabilities were added after the numbered phases (0-15):

1. **Project repositories** — a project can attach one or more repositories, list
   them, register one without starting analysis, and remove (archive) one.
2. **Re-run terminal jobs** — a failed, cancelled, or expired job can be requeued
   for a fresh attempt.

Both are tracked as post-phase enhancements (decision log + checklist), not new
phase gates.

## What changed

### Database

- `migrations/0017_repositories.sql`: adds `git_sources.display_name` (optional,
  bounded to 120 characters) and `git_sources.archived_at` (removal marker), plus
  a partial index `git_sources_project_active_idx` for active-list queries. The
  existing `UNIQUE (project_id, normalized_url)` is unchanged.

### API

- `services/api/src/repositories.rs` (new):
  - `GET /api/projects/{projectId}/repositories` — any member; active repositories,
    newest first, limit 100, with a job count.
  - `POST /api/projects/{projectId}/repositories` — owner, admin, collaborator;
    URL/reference validation mirroring job creation; upsert reactivates archived
    URLs; audit `repository.registered`.
  - `DELETE /api/projects/{projectId}/repositories/{repositoryId}` — owner, admin;
    archives (`archived_at = now()`), idempotent, refused with `409 repository_busy`
    while a `queued`/`analyzing`/`planning` job exists; audit `repository.removed`.
- `services/api/src/jobs.rs`:
  - `POST /api/jobs/{jobId}/rerun` — owner, admin, collaborator; requires a
    terminal job; resets attempt/lease/plan state in one row-locked transaction,
    appends `job.requeued`, writes audit `job.rerun_requested`.
  - Job-creation upsert now clears `archived_at` so reconnecting a URL reactivates
    it.
  - `provider_for` is `pub(crate)` for reuse.
- `services/api/src/projects.rs`: `ProjectRole::can_manage_repositories`
  (owner/admin).
- `services/api/src/lib.rs`: mounts `repositories::routes()`.

### Contracts (additive, stays v1)

- Added `job.requeued` to `packages/contracts/schemas/probeforge.v1.schema.json`,
  `packages/contracts/src/model.ts` (`JobEventType`), and
  `crates/contracts/src/model.rs` (`EventType`).
- New shared fixture `packages/contracts/fixtures/v1/event.requeued.valid.json`
  registered in `manifest.json`, validated by both the TypeScript and Rust
  validators.

### Frontend

- `apps/web/src/RepositoriesPanel.tsx` (new): lists repositories, registers one,
  starts analysis per repository (goal/intensity), and removes with inline
  confirmation (owner/admin only). Follows the existing restrained, accessible
  panel patterns.
- `apps/web/src/ConnectRepository.tsx` removed; `ProjectsView.tsx` renders
  `RepositoriesPanel` and shows a "Re-run" action on terminal jobs with an inline
  error surface.
- `apps/web/src/api.ts`: repository DTOs and `listRepositories`,
  `registerRepository`, `removeRepository`, `rerunJob`,
  `canRemoveRepository`.
- `scripts/check-docs.mjs`: required-file and linked-doc lists updated
  (`RepositoriesPanel.tsx`, `docs/REPOSITORIES.md`, `services/api/tests/repositories.rs`).

### Documentation

- New `docs/REPOSITORIES.md`.
- Updated `docs/JOBS.md` (re-run endpoint, `job.requeued`, verification),
  `docs/DATABASE.md` (migration, invariants, tests), `docs/README.md` (index and
  report table), `docs/IMPLEMENTATION_CHECKLIST.md` (post-phase entries),
  `docs/DECISION_LOG.md` (D-046, D-047).

## Checks run and observed results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS (no diff) |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (no warnings) |
| `bun run lint` (`biome check .`) | PASS (164 files, no fixes) |
| `bun run typecheck` (all workspaces) | PASS |
| `bun run test:ts` (contracts, web, analysis-worker, test-catalog) | PASS (web 16, contracts 56, analysis-worker 134 pass / 10 skip, catalog 6) |
| `cargo test -p probeforge-contracts` | PASS (fixtures + validation) |
| `cargo test -p probe-forge-api --test migrations --test jobs_queue --test repositories` (real DB) | PASS (1 + 3 + 4 tests) |
| `cargo test -p probe-forge-queue --test queue` (real DB) | PASS (8 tests) |
| `bun run check:docs` | PASS (115/115 required files, links resolve) |
| `bun run check:db` full run | Rust suites PASS; the TS analysis job-loop integration test timed out once at its 5000 ms limit under build/test load, then PASSED in isolation (3.55 s) — a load-induced flake, not a regression |

Note: the running local dev services lock `target/debug/probe-forge-api.exe`, so
the Rust checks above were run with `CARGO_TARGET_DIR` pointed at a separate
directory. The default `target/` build is unaffected.

## Verification detail (new tests)

- `services/api/tests/repositories.rs`: registration/list/archive, idempotent
  removal, reactivation on re-registration, RBAC (viewer/collaborator/admin/owner),
  tenant isolation, `409 repository_busy` cleared on cancellation, and URL/
  reference/name validation.
- `services/api/tests/jobs_queue.rs` (`rerun_requeues_terminal_jobs_and_audits`):
  `409` on a queued job, `403` for a viewer, `404` for a non-member, successful
  re-run of a failed job (status/attempt/lease/error reset, `job.requeued` event,
  `job.rerun_requested` audit), and re-run of a cancelled job.
- `apps/web/src/App.test.tsx`: add-then-analyze, remove-with-history-preserved,
  re-run a terminal job, and non-public URL rejection.

## Security and data-integrity notes

- Removal is an archive, never a hard delete, so immutable plans, approvals, runs,
  evidence, and `job_events` remain intact (D-022 / D-046).
- Repository URL validation rejects non-`https`/`ssh` schemes, embedded
  credentials, non-`git` SSH usernames, and query/fragment suffixes.
- Reference validation mirrors the database `jobs_source_reference_safe` rule.
- RBAC and tenant isolation are enforced server-side; a non-member gets `404`.
- No repository is cloned or contacted by any test; all fixtures are inert.

## Known limitations

- Archived repositories are hidden from the active list; there is no dedicated
  hard-delete or purge path yet.
- No ingestion-request endpoint; the operator CLI remains the ingestion entry
  point.
- Re-run clears the pinned commit and plan, so a re-run re-analyzes from scratch;
  it does not resume prior partial work.
- `docs/DATABASE.md` schema map still lists only migrations up to `0011` plus the
  new `0017` row; the `0012`-`0016` rows were already absent before this change.

## Next permitted work

None gated. This is a post-phase enhancement; the numbered phases remain complete
and their exit gates unchanged.
