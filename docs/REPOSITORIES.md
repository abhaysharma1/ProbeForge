# ProbeForge repositories

A project can attach one or more repositories. A repository is project-scoped Git
metadata (`git_sources`) used as the source of read-only analysis. This document
describes the registry API, the archive-based removal model, and how repositories
relate to jobs.

## Scope

- List the repositories attached to a project.
- Register a repository without starting an analysis.
- Remove (archive) a repository.
- Start an analysis for a registered repository.

Repository contents are never executed. Ingestion remains read-only and pinned to
one commit; see [REPOSITORY_INGESTION.md](REPOSITORY_INGESTION.md).

## Source of truth

`git_sources` is the only repository registry. It is project-scoped and unique on
`(project_id, normalized_url)`. Migration `0017_repositories.sql` added:

| Column | Meaning |
|---|---|
| `display_name` | Optional operator-facing name; falls back to a name derived from the URL in the UI. |
| `archived_at` | `NULL` while active. Set when the repository is removed. |

`repository_snapshots` cascade from `git_sources`; `jobs.git_source_id` restricts
deletion (`ON DELETE RESTRICT`) so a repository referenced by a job cannot be
hard-deleted without destroying immutable history.

## API

| Method | Path | Access |
|---|---|---|
| `GET` | `/api/projects/{projectId}/repositories` | any project member; active repositories only, newest first, limit 100 |
| `POST` | `/api/projects/{projectId}/repositories` | owner, admin, collaborator |
| `DELETE` | `/api/projects/{projectId}/repositories/{repositoryId}` | owner, admin |

- A non-member receives `404` so project existence is not disclosed.
- A viewer may list but receives `403` when registering.
- A collaborator may register but receives `403` when removing.

### Registration

`POST` accepts a JSON body (not a versioned contract; it is not model output):

```json
{ "repositoryUrl": "https://github.com/owner/repo", "reference": "main", "displayName": "Optional" }
```

Validation mirrors job creation:

- `repositoryUrl` must be `https` or `ssh`, must not embed credentials, must not
  include a username except the `git` SSH username, and must not include a query
  or fragment. Length is bounded to 8-2048 characters.
- `reference` must match `[A-Za-z0-9._/-]{1,200}`, must not start with `-`, and
  must not contain `..`, matching the database `jobs_source_reference_safe` rule.
- `displayName` is optional and bounded to 120 printable characters.

Registration upserts on `(project_id, normalized_url)`. Re-registering an
archived URL reactivates it (`archived_at` cleared). Registering an already-active
URL updates its default reference and display name.

### Removal

`DELETE` archives the repository: it sets `archived_at`, hides the row from the
active list, and blocks new analysis. It returns `204 No Content`.

- Removing an already-archived repository is idempotent (`204`).
- Removal is refused with `409 repository_busy` while the repository has an active
  analysis (a job in `queued`, `analyzing`, or `planning`).
- The row is **not** deleted. Job history (plans, approvals, runs, evidence) and
  `job_events` are append-only and protected by delete-rejecting triggers, so a
  hard delete would either be rejected or destroy immutable records. Archiving
  preserves that history.

A true erase would require a dedicated retention/purge path; none is introduced
here.

## Jobs and analysis

- Registering a repository does not start analysis.
- "Start analysis" posts the repository's URL and default reference to the
  existing `POST /api/jobs?projectId=...` endpoint (the Phase 2 `jobCreation`
  contract is unchanged). The job upsert reuses the registered `git_sources` row.
- Creating a job for an archived URL reactivates the repository.

## Events and audit

- Job events: `job.requeued` (see [JOBS.md](JOBS.md)).
- Audit actions: `repository.registered`, `repository.removed`,
  `job.rerun_requested`. Audit rows are immutable.

## Frontend

`apps/web/src/RepositoriesPanel.tsx` renders the project's repositories with a
registration form, a per-repository "Start analysis" form (goal and intensity),
and an inline-confirmed "Remove" action shown only to owners and admins. It
follows the restrained, accessible patterns of the other project panels; state
comes from the server, never optimistic.

## Verification

`services/api/tests/repositories.rs` (isolated schema, inert metadata) proves:

- registration, listing, and archive-based removal, including idempotent removal
  and reactivation on re-registration;
- RBAC (viewer, collaborator, admin, owner) and tenant isolation;
- `409 repository_busy` while an analysis is active, cleared on cancellation;
- URL, reference, and name validation.

## Deferred

- No ingestion request endpoint; the operator CLI in
  [REPOSITORY_INGESTION.md](REPOSITORY_INGESTION.md) is still the ingestion entry
  point.
- No true hard delete; archived rows remain for history and future retention
  design.
