# Phase 5a - Ingestion complete-fetch fix report

- **Date:** 2026-10-24
- **Area:** Phase 5 repository ingestion (`services/analysis-worker`), surfaced end to end by the Phase 15 job loop.
- **Decision:** D-045 in `docs/DECISION_LOG.md`.

## Symptom

The analysis worker claimed a job, validated the target, resolved the planning configuration, then failed every attempt with `errorCode=git_timeout`. Each attempt took ~120 s, was rescheduled with a 5 s backoff, and after `max_attempts` the job was marked `failed`.

```
job.processing_exception ... errorCode=git_timeout
  at mapProcessError (git.ts:138)
  at #run (git.ts:370)
  at async listTree
  at async withRepositorySnapshot (ingest.ts:461)
  errorCause=BoundedProcessError: Process exceeded the 120000 ms timeout
```

## Root cause

`GitClient.fetch` (`repository/git.ts`) created a blobs-less partial clone with `--filter=blob:none`. `GitClient.listTree` then ran `git ls-tree -r -l -z`, whose `-l` format needs each blob's **size**. In a partial clone the sizes are not local, so git performed lazy on-demand blob fetches from the promisor remote for every entry. That network work routinely exceeded `gitTimeoutMs` (120000 ms in the environment), producing `git_timeout`.

The worker checks out and reads every file, so the partial clone saved nothing: it replaced one bulk pack download with many slow on-demand fetches. `git ls-remote` (reference resolution) and the initial `git fetch` succeeded because they never need blob objects, which is why the failure was specific to `ls-tree`.

## Change

1. `services/analysis-worker/src/repository/git.ts`
   - Added `fetchArguments(sha)` and use it for the pinned fetch. The fetch is now `git fetch --quiet --depth=1 --no-tags origin <sha>` (no `--filter`).
   - Added `GIT_NO_LAZY_FETCH: "1"` to the isolated git environment so any missing object fails fast instead of reaching the network.
   - `listTree` and `parseTreeInventory` are unchanged; `ls-tree -l` and its pre-checkout `maxFiles`/`maxTotalBytes` gate are retained.
2. `services/analysis-worker/src/repository/errors.ts`
   - `RepositoryIngestError` gained a non-sensitive `operation` label (the git subcommand), so a failure names the command that failed. Arguments and remote URLs are never included.
3. `services/analysis-worker/src/repository/git.ts` / `src/job-loop.ts`
   - `mapProcessError` and `#run` populate `operation`; `describeError` surfaces it in the existing failure logs.
4. Tests: `services/analysis-worker/src/repository/git.test.ts`
   - `fetchArguments` includes `--depth=1`/`--no-tags` and no `--filter`.
   - `GIT_NO_LAZY_FETCH` is set in the git environment.
   - Against the inert local fixture: after `fetch`, the clone is not partial (`remote.origin.partialclonefilter` unset) and `git ls-tree -r -l -z` exits 0 under `GIT_NO_LAZY_FETCH=1`, proving every blob is present locally.

## Evidence (commands and results)

- `bun test` (services/analysis-worker): **134 pass, 10 skip, 0 fail** (144 tests, 24 files).
- `bun test src/repository/git.test.ts`: **5 pass, 0 fail**.
- `bun run typecheck` (services/analysis-worker, `tsc --noEmit`): passed.
- `biome check` on the four changed files: passed.
- `bun scripts/check-docs.mjs`: see below.

## Limitations

- `maxTotalBytes` is now enforced **after** the blobs are downloaded, not before. Content is still never read or processed before the checks, history is bounded by `--depth=1`, and the workspace is ephemeral; the change to the download gate is recorded in D-045.
- The regression test uses a `file://` fixture, so it exercises the blob-presence invariant rather than remote network timing.
- `createGitEnvironment` still builds a minimal environment and drops `HTTP(S)_PROXY`/`NO_PROXY`. Egress currently works, so this is not part of the fix, but it is a latent gap if the deployment network ever requires a proxy.

## Phase gate

This is a bug fix inside the Phase 5 ingestion path. **PHASE 5: PASS** for this defect (unit/integration tests and type/lint checks pass; the end-to-end re-run on the staging target is an operator step and is not claimed here).
