# Phase 6 report — Static discovery and application model

**Status:** PHASE 6: PASS

**Date:** 2026-10-23

**Scope:** Added a deterministic, read-only static discovery library to the
TypeScript analysis worker, a bounded discovery toolset for Phase 7 Pi wiring,
and an optional operator CLI inventory artifact. No HTTP endpoint, database
migration, repository execution, scanner run, load test, or external target was
added.

## What changed

### Discovery model and orchestration

- `services/analysis-worker/src/discovery/model.ts` — versioned
  `ApplicationInventory`, routes, surfaces, frameworks, unsupported surfaces,
  ambiguities, coverage, and discovery limits.
- `services/analysis-worker/src/discovery/limits.ts` — defaults and fail-closed
  validation for source file/byte/line, route, surface, dependency, unsupported,
  and ambiguity bounds.
- `services/analysis-worker/src/discovery/shared.ts` — source line documents,
  bounded evidence locations, stable fact IDs, path normalization, provenance,
  confidence, and fact builders.
- `services/analysis-worker/src/discovery/discover.ts` — orchestration over the
  Phase 5 `RepositorySession`; framework detection, source selection, parser
  dispatch, dependency surfaces, deduplication, sorting, truncation reporting,
  and untrusted-code invariants.
- `services/analysis-worker/src/discovery/surfaces.ts` — deterministic checks
  for middleware, auth checks, schema validators, configuration patterns,
  outbound requests, and file uploads.
- `services/analysis-worker/src/discovery/tools.ts` — pure bounded query toolset
  (`summary`, route list/get, surface list, unsupported list) with validated
  limits and query fields.
- `services/analysis-worker/src/discovery/index.ts` — public exports.

### Parsers

Added line-oriented, deterministic route parsers for:

- TypeScript/JavaScript: Express, Fastify, Hono, Koa, NestJS, Next.js App Router
  and Pages API, plus generic route calls.
- Python: FastAPI, Starlette, Flask, Django URL patterns, `add_api_route`, and
  Starlette `Route` registrations.
- Rust: Axum route calls, Actix/Rocket method attributes, and route attributes.
- Java: Spring MVC annotations and class-level bases. JAX-RS is explicitly
  reported as unsupported rather than overclaimed.
- Go: Gin/Echo/Fiber calls, `http.Handle`/`HandleFunc`, and method-qualified
  patterns.
- Ruby: Rails route declarations, resource expansion, and Sinatra routes.
- PHP: Laravel route declarations/resource expansion and Symfony route
  attributes.
- C#: ASP.NET Core controller attributes and minimal API `Map*` methods.

### CLI and detection integration

- `services/analysis-worker/src/ingest-cli.ts` adds optional
  `--discovery-output <file>`, which runs discovery against the same pinned
  workspace and writes the bounded inventory beside the snapshot.
- `services/analysis-worker/src/repository/inspect.ts` now detects Hono,
  Starlette, and ASP.NET Core from manifest dependencies.
- `services/analysis-worker/src/repository/ingest-cli.test.ts` verifies the
  snapshot, audit log, and inventory output together.

### Tests

- `services/analysis-worker/src/discovery/discovery.test.ts` contains synthetic
  fixtures for the supported parsers (including Actix, Go `net/http`, Sinatra,
  Symfony, and Starlette), ambiguity handling, unsupported coverage, bounds, tool
  queries, and an ingestion-to-inventory test using the inert Phase 5 fixture.
- Discovery tests assert `executedRepositoryCode: false`, `untrustedContent:
  true`, and that the fixture execution marker never appears.

### Documentation

- Added `docs/DISCOVERY.md`.
- Updated `docs/REPOSITORY_INGESTION.md`, `docs/README.md`,
  `docs/IMPLEMENTATION_CHECKLIST.md`, `docs/DECISION_LOG.md` (D-027), `README.md`,
  and `scripts/check-docs.mjs`.
- No `.env` file or secret material was added.

## Verification

Commands run from the repository root unless noted:

| Command | Result |
|---|---|
| `bun run lint` | PASS — Biome checked 121 files |
| `bun run typecheck` | PASS — contracts, test-catalog, web, and analysis-worker |
| `bun run test:ts` | PASS — analysis-worker 66 tests / 400 assertions; contracts 46 / 76; test-catalog 4 / 6; web 5 |
| `bun test src/discovery` (analysis-worker) | PASS — 11 tests / 79 assertions |
| `bun test src/repository/ingest-cli.test.ts` (analysis-worker) | PASS — 2 tests / 12 assertions |
| `bun run check:docs` | PASS — required files, links, phase coverage, `.env` hygiene, secret scan |
| `bun run check:rust` | PASS — fmt, clippy, workspace tests (no Rust changes) |
| `bun run check:services` | PASS with a command-scoped `DATABASE_URL` — 5/5 local services healthy; no listener processes left behind. Without the URL, the API and scheduler fail closed with `MissingDatabaseUrl`. |
| `bun run check` | PASS — docs, Rust, and TypeScript gates |

### Gate evidence

- **Structured inventory from fixture repositories:** the discovery integration
  test ingests the inert fixture, runs discovery, and observes the FastAPI
  `GET /fixture` route with file/line evidence and framework attribution. The CLI
  test independently observes the same route in `--discovery-output`.
- **Provenance/inference:** all route, surface, framework, unsupported, and
  ambiguity facts carry provenance and confidence. Test coverage includes
  observed Express routes, inferred Next.js file routes, and ambiguous dynamic
  paths.
- **Unsupported/ambiguous routes:** unsupported language, framework, and file
  entries are explicit; dynamic paths, unknown methods, included URLconfs,
  prefixes/groups, and truncated selections are marked ambiguous or covered by
  an ambiguity fact.
- **No repository execution:** the discovery path only calls the Phase 5
  bounded `readFile`; tests assert the execution marker is absent and the
  inventory reports `executedRepositoryCode: false`.

## Known limitations

- Parsers are line-oriented heuristics, not AST or runtime framework analyzers.
  Unusual formatting, multiline declarations, included URLconfs, nested routers,
  and runtime-built paths can be missed or mislabeled.
- JAX-RS and browser/UI frameworks (React, Angular, Vue, Svelte, etc.) are
  reported as unsupported for server-route coverage.
- Inventories are not persisted to PostgreSQL, and discovery is not exposed as
  an HTTP endpoint or wired to Pi yet; D-027 assigns those to later phases.
- The CLI uses fixed default discovery limits; library callers can pass a
  validated `Partial<DiscoveryLimits>`.
- The Phase 5 DNS-rebinding residual risk is unchanged and out of Phase 6 scope.
- No real repository, scanner, load generator, or deployed target was contacted.

## Next phase

**Phase 7 — Pi agent integration and plan generation** is the next permitted
phase, only after explicit authorization. Phase 6 does not register discovery
tools with Pi; it provides the bounded typed toolset that Phase 7 must adapt,
budget, and register.
