# Phase 18 - Dedicated project pages and hash navigation

**Date:** 2026-10-27
**Status:** Complete (post-phase enhancement)
**Decision:** [D-052](../DECISION_LOG.md)

## Goal

Clicking a project previously opened a right-hand detail panel beside the list.
The panel stacked repositories, jobs, runs, targets, members, and audit events in
one narrow, non-addressable column. This change gives every project its own page
with the full workspace width, a tab per working surface, and URLs that survive a
refresh, a shared link, or the browser's Back button.

## What changed

- **Route grammar** (`apps/web/src/routing.tsx`, new): fragment routes
  `#/projects`, `#/projects/{id}[{tab}]`, `#/projects/{id}/jobs/{jobId}`, and
  `#/projects/{id}/runs/{runId}`. Unknown tabs fall back to Repositories; an
  unknown first segment falls back to the projects list. Includes
  `useHashRoute`, `navigate`, and a `RouteLink` anchor that keeps real `href`
  semantics while navigating in-page.
- **Project page** (`apps/web/src/ProjectPage.tsx`, new): moved project data
  loading (members, jobs, runs, audit) and the live job/run SSE subscriptions out
  of `ProjectsView` without changing their semantics (PostgreSQL stays
  authoritative, streams only prompt coalesced refetches). Tabs:
  Repositories (default), Analysis jobs, Runs, Targets, Members, Activity.
  Activity is hidden unless the role can view audit. Plan review and run report
  render in place of the tabs and return to the Jobs or Runs tab.
- **Projects list** (`apps/web/src/ProjectsView.tsx`): removed the detail column
  and selection state. Rows are links to the project page. The shell header now
  shows the route: Projects, the project name plus slug/role/members/created, or
  Plan review / Run results. Create-project navigates to the new project. Sign-out
  resets the fragment to `#/projects`.
- **Panels as cards**: `RepositoriesPanel` and `TargetsPanel` render standalone
  bordered cards instead of `border-b` sections built for the old panel stack.
- **Tests**: existing flows now open a project page and select tabs. New coverage
  for hash-addressable tabs, restoring the active tab on reopen, the neutral
  unauthorized-project state, returning to the list, and route parsing/paths
  (`apps/web/src/routing.test.ts`).

## Security and authorization

- No API, schema, authorization, or policy change.
- The URL carries no authorization. Project resolution is by id against
  `GET /api/projects`, which is already membership-scoped; an id the account
  cannot access renders a neutral unavailable state and reveals no project data.
- A direct link to the Activity tab without audit permission renders a
  permission notice instead of loading audit data.
- Read-only roles keep read access; write controls are still hidden by role and
  every request is still authorized by the server.

## Verification

Commands run from the repository root:

| Command | Result |
|---|---|
| `bun run --cwd apps/web typecheck` | Passed (`tsc --noEmit`, no output) |
| `bun run --cwd apps/web test` | Passed, 33 tests in 2 files |
| `bun run --cwd apps/web build` | Passed (`vite build`, 28 modules) |
| `bunx biome check apps/web/src` | Passed, no findings |
| `bun scripts/check-docs.mjs` | PASS (117/117 required files, 20 documents linked, 16 phase headings) |
| `bun run lint` | Passed (172 files checked) |

## Known limitations

- **No manual browser verification.** The change was verified with jsdom tests,
  type checking, and a production build. The local API/database stack was not
  running during this change, so the visual result, focus behaviour, and Back
  button were not exercised in a real browser.
- Inactive tabs unmount, so an in-progress form (repository or target entry) is
  discarded when switching tabs. This is standard tab behaviour and deliberate;
  the alternative keeps every panel mounted and fetching at once.
- Hash routes are not sent to the server, so link previews and analytics cannot
  see the project id. This is the trade-off that avoids a production SPA fallback
  rule.
- A tab count is hidden while its list is first loading, then shown once loaded.

## Next

No gate is blocked. The next frontend work can build on the route module; adding a
future screen means extending the route union and parsing rather than introducing
a router.
