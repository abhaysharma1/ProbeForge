# ProbeForge contracts

Phase 2 defines versioned, data-only contracts for job creation, plans, load
scenarios/profiles, target authorization, run requests, results, and job events.
The same fixtures are exercised by the TypeScript and Rust validators.

## Source of truth

| Artifact | Location |
|---|---|
| JSON Schema bundle (v1) | `packages/contracts/schemas/probeforge.v1.schema.json` |
| TypeScript types | `packages/contracts/src/model.ts` |
| TypeScript validators (Ajv) | `packages/contracts/src/validate.ts` |
| Rust types + validators | `crates/contracts/` |
| Shared fixtures + manifest | `packages/contracts/fixtures/v1/` |

The JSON Schema is the normative wire shape. Rust and TypeScript types mirror it;
the shared fixture manifest is the drift guard between the two validators.

## Versioning

- The bundle is v1 (`schemaVersion: "1"`, filename suffix `.v1`).
- Every contract carries a literal `schemaVersion`; unknown values are rejected.
- Additive optional fields may stay in v1. Required-field changes, removals,
  renames, or semantic changes require a v2 bundle and a database
  `schema_version` migration. Do not silently change v1.
- The database stores `schema_version` on plans and load scenarios and rejects
  anything other than `'1'` today.
- Unknown properties are rejected (`additionalProperties: false` in JSON Schema,
  `deny_unknown_fields` in Rust). Clients must send exactly the documented shape.

## Contract families

| Definition | Purpose |
|---|---|
| `jobCreation` | Goal (`vulnerability_testing` / `load_testing`), intensity, display name, HTTPS/SSH Git source, optional secret reference |
| `plan` | Pi output: summary, confidence, planned tests, load scenarios, required inputs, limitations |
| `plannedTest` | Catalog test ID, rationale, risk, expected evidence, safe configuration |
| `loadScenario` / `loadProfile` | Ordered HTTP steps with typed correlation/assertions, secret-backed headers, and a bounded VU/arrival-rate profile |
| `targetAuthorization` | Explicit confirmation, normalized origin/scope, hard limits, validity window |
| `runRequest` | Approved plan version, selected tests/scenarios, target authorization, idempotency key |
| `testResult` | Outcome, severity, confidence, bounded evidence metadata |
| `jobEvent` | Ordered SSE/replay event envelope; includes `job.queued`, `job.claimed`, `job.retry_scheduled`, `job.failed`, and `job.cancelled` |
| `runEvent` | Run-scoped SSE/replay event envelope (`GET /api/runs/{runId}/events`); the run lifecycle, execution, and report subset of the job vocabulary (`run.*`, `test.*`, `finding.recorded`, `report.*`) |

Enum values are closed. In particular, only the two job goals are accepted; an
unknown goal fails schema validation and the database check constraint.

## Validation layers

1. **Transport size:** single contract payloads are capped at **256 KiB**
   (`MAX_CONTRACT_BYTES`). The API's request body middleware is stricter at the
   HTTP edge (1 MiB); callers must still apply the contract cap before parsing.
2. **JSON Schema:** Ajv (2020-12) in TypeScript; `serde` plus semantic checks in
   Rust. Both reject unknown fields, invalid UUIDs, non-URI/unsafe URLs, unknown
   enums, and out-of-range numbers.
3. **Semantic checks:** forbidden credential-bearing load headers, HTTP-token
   header names and control-free values, origin/host and origin/port consistency,
   goal/content consistency, duplicate IDs, nil UUIDs, timestamp ordering,
   template/dependency ordering, extraction/assertion selector shapes, body-size
   limits, and executable-content rejection. Structured configuration, result
   details, and arbitrary safe JSON must be JSON objects (`safeJsonObject`), not
   arrays or scalars. Load stop conditions are a closed typed object rather than
   arbitrary JSON.
4. **Database constraints:** second-line defense for enums, ranges, project
   references, run shape, idempotency keys, and immutable records.

Server-side validation is authoritative. Frontend checks are never a
substitute for it.

## No executable model output

Model output is data, not code:

- There is no `command`, `script`, `executable`, `shell`, `cmd`, `scripts`, or
  `entrypoint` field anywhere in the schema.
- Free-form JSON fields use recursive `safeJson`, which rejects those property
  names at any depth (case-insensitive in both validators) and caps depth,
  arrays, object size, and strings.
- Load requests are structured method/path/header/body steps. Literal
  credential-bearing headers (`Authorization`, `Cookie`, and related) are
  rejected; static credentials use `secretHeaders` values that must be
  `secret://` references. Hop-by-hop and length-managed headers are rejected.
  Steps may use earlier extraction variables as `${name}` in path, literal header
  values, or body strings. `dependsOn` may only name earlier step IDs. Header or
  JSON-pointer extractions create correlation variables, and bounded
  header/JSON assertions replace scripts.
- Load steps require at least one expected status and cannot attach a body to
  GET, HEAD, or OPTIONS. Per-step body templates are capped at 64 KiB. Profiles
  are capped at 100 VUs or 100 arrivals/s, 300 s ramp, 900 s duration, 100,000
  requests, and 100 requests/s. Stop conditions are limited to `errorRate`,
  `p95LatencyMs`, and `maxFailedRequests`.
- A scenario is versioned by its parent plan version. Approval revalidates each
  selected scenario and pins the exact scenario JSON and profile in the immutable
  approval snapshot; there is no independent scenario-edit endpoint.
- Planned tests reference a vetted catalog ID. Configuration is data validated
  again by the backend; it can never define a command or executable path.

## Adding or changing fixtures

1. Add a JSON fixture under `packages/contracts/fixtures/v1/`.
2. Add a case to `manifest.json` with `validator`, `path`, and `valid`.
3. Run `bun test` in `packages/contracts` and
   `cargo test -p probeforge-contracts`.
4. A valid case must pass both validators; an invalid case must fail both. If
   they disagree, fix the schema/types or the validators before proceeding.

## Usage

TypeScript (server-side only):

```ts
import { validateContractJson, validatePlan } from "@probe-forge/contracts/validate";
```

Rust:

```rust
use probeforge_contracts::{Plan, parse_and_validate_json};

let plan: Plan = parse_and_validate_json(raw_bytes)?;
```

Invalid payloads must be rejected before persistence or execution. Validation
errors include paths and rule descriptions, not the offending values, so target
URLs and credential references do not leak through error responses.

## Job API and SSE

`POST /api/jobs?projectId=<uuid>` accepts the unchanged `jobCreation` body and
returns a job DTO. Project scope is a required query parameter (D-023) so the
closed v1 body does not change. `Idempotency-Key` is an optional request header;
an identical retry returns the existing job, while a different payload with the
same key is rejected with `409`.

`GET /api/jobs/{jobId}/events` emits `text/event-stream`. The SSE `id` is the
per-job event sequence; `Last-Event-ID` and `?after=` replay events greater than
the cursor. `?follow=false` returns a finite replay stream for reconnect/tests;
the default follows until a terminal job state. Event data is the `jobEvent`
contract shape and PostgreSQL remains authoritative.

Event types added in Phase 4 are `job.claimed` and `job.retry_scheduled`; the
existing `job.queued`, `job.failed`, and `job.cancelled` remain valid. A shared
Rust/TypeScript fixture (`event.claimed.valid.json`) verifies the new enum value
in both validators.

## Run API and SSE

`GET /api/runs/{runId}/events` emits the run section's own `text/event-stream`
with the same cursor semantics (`Last-Event-ID`, `?after=`, `?follow=false`,
terminal-status close). Event data is the `runEvent` contract shape;
shared Rust/TypeScript fixtures (`run-event.*.json`) verify the envelope and its
restricted vocabulary — a `job.*` type is rejected there even though the parent
job stream still hears about run transitions through dual-written `run.*` events.
`run.queued` joined the shared event vocabulary alongside `run.scheduled` because
the backend has always emitted it when operator-created runs enter the queue.

## Target authorization API

Phase 9 exposes project-scoped target management:

- `GET /api/projects/{projectId}/targets` and `POST /api/projects/{projectId}/targets`
  register and list normalized target origins, environments, allowed hosts,
  paths, and ports.
- `GET /api/targets/{targetId}` returns the target and its authorization history.
- `POST /api/targets/{targetId}/validate` resolves the host and records the
  server-validated address set.
- `GET` and `POST /api/targets/{targetId}/authorizations` list or create
  explicit, time-bounded authorizations with bounded `TargetLimits`.
- `POST /api/target-authorizations/{authorizationId}/revoke` revokes an
  authorization once.

The server denies loopback, private, link-local, metadata, multicast, reserved,
and documentation destinations by default. Explicit private-target mode is
server-configured, still blocks metadata/link-local ranges, and is disabled by
default. Redirects, DNS changes, and request paths are revalidated against scope
at the connection boundary. The v1 `TargetScope`/`TargetLimits` shapes are
unchanged; target registration and validation add server-owned policy.

## Plan review and approval API

The v1 plan payload remains read-only after generation. Phase 8 adds selection and
approval endpoints around it:

- `GET /api/test-catalog` returns the embedded schema 1 / catalog 0.3.0 entries
  with adapter availability.
- `GET /api/jobs/{jobId}/plan` and `GET /api/plans/{planId}` return the plan,
  catalog metadata, selected flags, revision, and approval summary.
- `PATCH /api/plans/{planId}` accepts `expectedRevision`, `selectedTestIds`, and
  `selectedLoadScenarioIds`. It changes only selected flags. Unknown fields,
  duplicate IDs, and IDs from another plan are rejected.
- `POST /api/plans/{planId}/approve` accepts `expectedRevision` and
  `confirmed: true`. It writes an immutable `plan_approvals` snapshot tied to the
  exact plan version and revision.

All routes are authenticated and return `no-store`. Plan routes are project-scoped;
the catalog is not tenant data. Viewers may read but not edit or approve. A stale
revision returns `409 plan_conflict`; a non-current plan returns
`409 plan_superseded`. Approval writes audit events but does not create a run or
contact a target.

## Load scenario generation and validation

Load plans contain ordered, data-only steps. The contract is closed and rejects
unknown fields, loops, scripts, raw credentials, forward dependencies, unknown
template variables, oversized bodies, and profiles above platform maxima. The
full field and limit reference is in [LOAD_SCENARIOS.md](LOAD_SCENARIOS.md).

Pi proposes scenarios through the existing `plan_validate`/`plan_propose` tools;
the server applies the same contract validation before persistence. The
deterministic load fallback selects one conservative read-only route. Approval
revalidates every selected scenario and pins the exact JSON and profile inside
the immutable plan approval snapshot. Scenario versioning is the parent plan
version; there is no independent scenario-edit endpoint.

`validate_load_scenario_scope` in `crates/target-policy` checks every step against
a chosen target authorization without DNS resolution or traffic. Phase 12 must
call it at run creation and percent-encode extracted values before substitution.

## Health contract

The Phase 1 liveness/readiness shape remains in `packages/contracts/src/index.ts`
and is still mirrored in Rust (D-010). The API and scheduler now require
`DATABASE_URL`: their `/readyz` endpoints compare the applied schema against the
migrations compiled into the binary (D-051) and return `503 not_ready` when the
database is unavailable, unreadable, or missing any of those migrations. A
database that is one migration behind is therefore never reported `ready`. The
execution worker and analysis worker remain static until their own DB-backed
phases.
