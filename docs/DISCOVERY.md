# ProbeForge static discovery and application model

Phase 6 adds a deterministic, read-only discovery layer to the TypeScript
analysis worker. It consumes the `RepositorySession` produced by Phase 5 and
returns a bounded `ApplicationInventory`. It does not add an HTTP endpoint, does
not persist to PostgreSQL yet, and never executes repository content.

## Scope and non-goals

The discovery layer can:

- inspect only source text files that Phase 5 already inventoried as bounded
  UTF-8 text;
- detect supported languages and frameworks from the snapshot and conservative
  source markers;
- extract routes, HTTP methods, handlers, middleware, authentication checks,
  schema validators, configuration patterns, dependencies, outbound requests,
  and file uploads where a deterministic pattern is available;
- attach a file path, line range, provenance, and confidence to every fact;
- mark dynamic or statically unknown routes as ambiguous instead of inventing a
  concrete path or method;
- report unsupported languages, frameworks, and oversized files explicitly;
- enforce file, byte, line, route, surface, dependency, and ambiguity limits;
- expose a small query-only toolset suitable for wiring into Pi in Phase 7;
- append a bounded inventory to the operator CLI workflow when
  `--discovery-output` is supplied.

Non-goals in Phase 6:

- no framework execution, import resolution, dynamic evaluation, build, test, or
  application start;
- no full AST/data-flow analysis, runtime reflection, or framework
  configuration execution;
- no browser/UI route discovery (React, Angular, Vue, Svelte, and similar
  frameworks are reported as unsupported for server-route coverage);
- no persistence of inventories to `repository_snapshots` or another table;
- no Pi model call or tool registration yet. The toolset is a typed library
  boundary for Phase 7;
- no scanner, load generator, network request, or target contact.

## Application inventory

`ApplicationInventory` is versioned (`schemaVersion: "1"`) and contains:

| Field | Meaning |
|---|---|
| `commitSha` | The immutable Phase 5 snapshot commit |
| `frameworks` | Detected frameworks with supported/unsupported status and evidence |
| `routes` | Method, path, path kind, handler, ambiguity, notes, evidence |
| `surfaces` | Middleware, auth checks, validators, configuration, dependencies, outbound requests, uploads |
| `unsupported` | Explicit language, framework, or file coverage gaps |
| `ambiguities` | Route, framework, and coverage caveats |
| `coverage` | Examined/skipped counts, truncation flags, parsers, unsupported lists |
| `limits` | Limits that actually applied |
| `untrustedContent` | Always `true` |
| `executedRepositoryCode` | Always `false` |

Every route and surface has `provenance` (`observed` or `inferred`) and
`confidence` (`high`, `medium`, or `low`). File-derived Next.js routes are
inferred. Rails/Laravel resource routes are inferred. Parser-matched decorators
and registration calls are observed. A route with a runtime expression, an
unresolved prefix, or an unknown HTTP method has `ambiguous: true` and an
explanatory note.

Evidence contains only a path and start/end line. The inventory intentionally
does not retain source snippets, secret values, full URLs with query strings, or
absolute workspace paths. Outbound-request details retain only a protocol, host,
and path when a safe literal URL is available.

## Supported source parsers

The following parsers are implemented in Phase 6. They are line-oriented
heuristics, not full framework compilers.

| Language | Frameworks/patterns covered |
|---|---|
| TypeScript / JavaScript | Express, Fastify, Hono, Koa route calls; NestJS controller/method decorators; Next.js App Router route files and Pages API methods; generic `router.get`-style calls |
| Python | FastAPI/Starlette decorators, Flask decorators, Django `path`/`re_path` patterns, `add_api_route`, Starlette `Route` |
| Rust | Axum `.route` calls, Actix/Rocket method attributes, route attributes |
| Java | Spring MVC mapping annotations, including class-level bases; JAX-RS is explicitly unsupported |
| Go | Gin/Echo/Fiber route calls, `http.Handle`/`HandleFunc`, Go 1.22 method-qualified patterns |
| Ruby | Rails route declarations and resource expansions, Sinatra route declarations |
| PHP | Laravel `Route::` declarations and resource expansions, Symfony route attributes |
| C# | ASP.NET Core controller attributes, minimal API `Map*` registrations |

Languages with no parser (for example Kotlin, Scala, Swift, C, and C++) are
reported in `unsupported`; they are not quietly treated as covered. Frontend
frameworks are reported as unsupported for server-route discovery.

Known limitations are deliberate:

- route groups, included URLconfs, middleware prefixes, and nested routers are
  usually not expanded; the inventory records an ambiguity where recognized;
- decorators with the path on a later line may be missed;
- a route path or method assembled at runtime is not resolved;
- regex scanners can miss unusual formatting and can match commented examples;
- handlers are best-effort identifiers and may be `null`;
- line ranges generally cover the route registration/decorator, not the full
  handler body;
- schema validators are reported by name/location, not by reconstructed schema;
- dependencies come from Phase 5 manifest inventories, not from lockfile
  resolution.

## Limits

Defaults are defined in `services/analysis-worker/src/discovery/limits.ts`:

| Limit | Default |
|---|---|
| Source files | 250 |
| Total source bytes | 8 MiB |
| One source file | 512 KiB |
| Lines per file | 10,000 |
| Routes | 500 |
| Surfaces | 1,000 |
| Facts per file | 100 |
| Dependencies | 500 |
| Unsupported entries | 200 |
| Ambiguities | 200 |

The CLI uses these defaults. Library callers can pass a validated
`Partial<DiscoveryLimits>`; invalid values fail closed. Truncation is always
reported through `coverage` and, where useful, an ambiguity fact. Empty or
partial results remain visible rather than being converted into false coverage.

## Discovery toolset for Pi

`createDiscoveryToolset(inventory)` returns bounded query methods:

- `getSummary()` - framework names, route counts, ambiguous-route count, surface
  counts, coverage, and limits;
- `listRoutes({ method?, pathPrefix?, framework?, limit? })`;
- `getRoute(id)`;
- `listSurfaces({ kind?, limit? })`;
- `listUnsupported(limit?)`.

Limits default to 50 and are capped at 100. Queries validate method, surface
kind, prefix/framework length, control characters, and IDs. Every list result
reports `total` and `truncated`. The toolset is pure and reads only the already
bounded inventory; it cannot read arbitrary files, invoke parsers, fetch URLs, or
execute commands. Phase 7 remains responsible for adapters, Pi registration,
token budgets, and model-facing schemas.

## CLI usage

```text
bun run ingest -- --fixture <directory> --ref main \
  --audit-log <audit.jsonl> \
  --output <snapshot.json> \
  --discovery-output <inventory.json>
```

`--discovery-output` is optional. Without it, the CLI behaves as in Phase 5.
The inventory shares the same pinned commit and workspace lifetime as the
snapshot; discovery never extends the workspace or starts another Git process.
Snapshots and inventories are still operator artifacts, not product API
responses.

## Untrusted content

Repository text is data. The discovery layer does not interpret source comments,
README files, test names, or route strings as instructions. It does not pass
source text to a model in Phase 6. Any later tool exposure must keep inventory
fields typed and bounded, preserve provenance, label ambiguous facts, and treat
repository content as lower-priority evidence.

## Verification

- `services/analysis-worker/src/discovery/discovery.test.ts` covers synthetic
  Express, FastAPI, Flask, Django, Axum, Actix, Spring, Gin, Go `net/http`,
  Rails, Sinatra, Laravel, Symfony, ASP.NET, Next.js, NestJS, Fastify, Hono, Koa,
  and Starlette fixtures; unsupported coverage; limits; tool queries; and an ingestion-to-inventory fixture test.
- `services/analysis-worker/src/repository/ingest-cli.test.ts` verifies the
  optional CLI inventory output.
- Tests assert that the inert fixture's execution marker never appears and that
  the inventory reports `executedRepositoryCode: false`.
