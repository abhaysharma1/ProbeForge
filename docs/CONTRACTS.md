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
| `loadScenario` / `loadProfile` | Structured HTTP requests and bounded VU/arrival-rate profile |
| `targetAuthorization` | Explicit confirmation, normalized origin/scope, hard limits, validity window |
| `runRequest` | Approved plan version, selected tests/scenarios, target authorization, idempotency key |
| `testResult` | Outcome, severity, confidence, bounded evidence metadata |
| `jobEvent` | Ordered SSE/replay event envelope; includes `job.queued`, `job.claimed`, `job.retry_scheduled`, `job.failed`, and `job.cancelled` |

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
   goal/content consistency, duplicate IDs, nil UUIDs, timestamp ordering, and
   executable-content rejection. Structured configuration, stop conditions, and
   result details must be JSON objects (`safeJsonObject`), not arrays or scalars.
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
- Load requests are structured method/path/header/body data. Credential-bearing
  headers (`Authorization`, `Cookie`, and related) are rejected; credentials use
  `secret://` references.
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

- `GET /api/test-catalog` returns the embedded schema 1 / catalog 0.2.0 entries
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

## Health contract

The Phase 1 liveness/readiness shape remains in `packages/contracts/src/index.ts`
and is still mirrored in Rust (D-010). The API and scheduler now require
`DATABASE_URL`: their `/readyz` endpoints verify that migrations have been
recorded and return `503 not_ready` when the database is unavailable or
unmigrated. The execution worker and analysis worker remain static until their
own DB-backed phases.
