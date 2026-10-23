# Pi planning and bounded plan generation

Phase 7 integrates the pinned Pi SDK (`@earendil-works/pi-agent-core` 1.4.0
and `@earendil-works/pi-ai` 1.4.0) inside the TypeScript analysis worker. The
result is a library that turns an untrusted repository snapshot, a bounded
static-discovery inventory, a vetted test catalog, and server-owned job policy
into a draft ProbeForge v1 plan.

A generated plan is advisory. It is not approval, target authorization, or
execution permission. Phase 7 never contacts a target and never executes a
proposed test.

## Pipeline

1. **Trusted input validation.** `createPlanningContext` validates job and
   project UUIDs, goal and intensity values, the display name, the snapshot /
   inventory commit match, inventory provenance flags, catalog entry IDs, and
   duplicate catalog IDs.
2. **Summary-only prompt.** `createPlanningPrompt` sends job metadata, discovery
   counts and coverage, and a bounded catalog summary. Raw repository text does
   not enter the prompt; file text is available only through bounded tool
   results.
3. **Bounded Pi run.** One `PlanningRuntime` is built from the pinned SDK
   catalog with `temperature: 0`, a clamped response-token limit, and the
   provider credential resolved inside Pi. The orchestrator never receives a
   provider API key.
4. **Typed tools.** The model can only call the read-only inspection, discovery,
   catalog, and plan validation/proposal tools listed below.
5. **Server validation.** `validateCandidatePlan` applies the versioned v1 JSON
   Schema and additional server policy. Invalid proposals are not persisted.
6. **Fallback.** If the model is unavailable, exceeds a budget, emits malformed
   output, or is not configured, the server can build a deterministic,
   low-confidence draft from vetted catalog entries and literal discovered
   routes.
7. **Persistence and audit.** A narrow `PostgresPlanStore` writes the agent
   session, draft plan, planned tests, job pointer, session metadata, and audit
   event in one transaction. The agent has no database client or credential.

## Model-facing tool registry

| Tool | Access | Bounds |
|---|---|---|
| `repository_list_files` | Snapshot file metadata | Prefix/language filters, max 100 results |
| `repository_read_file` | One UTF-8 text file | Snapshot membership, max 8 KiB by default |
| `discovery_get_summary` | Discovery counts and coverage | No source text |
| `discovery_list_routes` | Route facts | Max 100 results |
| `discovery_get_route` | One route by stable ID | Not found fails closed |
| `discovery_list_surfaces` | Surface facts | Max 100 results |
| `discovery_list_unsupported` | Unsupported coverage | Max 100 results |
| `inspection_list_auth_schema` | Auth, schema, middleware facts | Max 100 combined results |
| `catalog_list_tests` | Vetted catalog entries | Max 100 results |
| `catalog_get_test` | One vetted entry by ID | Unknown IDs fail closed |
| `plan_validate` | Schema/policy validation only | Does not persist or submit |
| `plan_propose` | Submit one draft plan | Exactly once; valid plans terminate |

There is no shell, process execution, filesystem write, arbitrary HTTP request,
database, secret-read, or execution-adapter tool.

Tool arguments are schema-validated, capped at 16 KiB, and scanned for
forbidden nested keys such as `command`, `script`, `executable`, `shell`,
`__proto__`, and `constructor`. Tool results are capped per call and in total.
Unknown tools and calls after a valid proposal are blocked.

## Server-side plan policy

The contract schema is authoritative for structure. Additional policy rejects:

- a plan goal that differs from the job goal;
- vulnerability plans containing load scenarios or load plans containing
  vulnerability tests (also rejected structurally by the schema);
- unknown or duplicate catalog IDs;
- a test name that does not match its vetted catalog entry;
- a risk below the catalog risk, or a high-risk test without explicit
  confirmation;
- load catalog entries used as vulnerability tests;
- load request paths that are protocol-relative (`//...`) or contain backslashes;
- evidence or rationale references outside the snapshot;
- credential-like material such as private-key blocks, bearer tokens, cloud
  keys, or `password=` / `client_secret=` values;
- plans above `maxPlanBytes`;
- too many planned tests or load scenarios.

The server then adds non-removable `requiredInputs`, including explicit target
authorization, incomplete-route or unsupported-coverage warnings, a missing
catalog prompt, and load-path review. It also adds a Phase 7 limitation stating
that no target was contacted and no test was executed. Required inputs and
limitations stay inside the schema's 20-item limit.

## Budgets

`DEFAULT_PLANNING_LIMITS` is server-owned and configurable through environment
variables in `services/analysis-worker/src/config.ts`.

| Limit | Default |
|---|---:|
| Model calls per attempt | 6 |
| Tool calls per attempt | 24 |
| Agent turns | 8 |
| Retries | 1 |
| Response tokens | 4,096 |
| Wall time | 120,000 ms |
| Prompt | 64 KiB |
| Tool output per call | 32 KiB |
| Total tool output | 128 KiB |
| Plan payload | 256 KiB |
| Repository read | 8 KiB |
| Prompt catalog entries | 200 |
| Planned tests | 20 |
| Load scenarios | 5 |

`validatePlanningLimits` rejects inconsistent or unbounded values. Provider and
model values must be configured together and must match the pinned SDK catalog.

## Fallback behavior

Fallback is deterministic and uses only server-owned data:

- **Vulnerability fallback:** up to three non-load catalog entries, sorted by
  risk then stable ID. It is marked low confidence (`0.2`) and high-risk entries
  still require explicit confirmation.
- **Load fallback:** one conservative profile (2/5/10 virtual users for
  low/standard/high) against the first sorted literal `GET` or `HEAD` route. It
  never contacts the target.

Fallback reasons are stable codes: `model_not_configured`, `model_not_found`,
`model_unavailable`, `model_timeout`, `model_invalid_output`,
`planning_limit_exceeded`, `prompt_too_large`, and `unexpected`.

A model outage does not mark the job failed. The result is either a persisted
fallback draft or a retryable failure when no safe fallback exists or
persistence fails. Cancellation returns without creating a plan.

## Persistence and versioning

`PostgresPlanStore` uses a short transaction that:

1. locks the agent session and verifies project/job ownership and `running`
   state;
2. locks the job row;
3. allocates `MAX(version) + 1` for the job;
4. rejects terminal jobs (`cancelled`, `failed`, `expired`);
5. inserts the `test_plans` row with `status = 'proposed'` and the v1 plan
   payload;
6. inserts `planned_tests` rows (unselected by default);
7. updates `jobs.current_plan_id`;
8. marks the session `completed` with bounded trace metadata;
9. appends `plan.proposed` or `plan.fallback_proposed` to `audit_log`.

The job-row lock serializes concurrent attempts, so two workers cannot allocate
the same version. A terminal session cannot save a second plan, and terminal
jobs are rejected. Model/provider metadata, counts, elapsed time, failures, and
tool statistics are stored; prompt text, repository text, raw provider errors,
and credentials are not.

Versioning prevents overwrites but is not exactly-once delivery. If a transaction
commits while its response is lost, a retry creates a new version; callers should
reconcile using the job, session, and version records.

`InMemoryPlanStore` mirrors the state and version rules for deterministic tests.

## Configuration

```text
PROBE_FORGE_PLANNING_PROVIDER
PROBE_FORGE_PLANNING_MODEL
PROBE_FORGE_PLANNING_MAX_MODEL_CALLS
PROBE_FORGE_PLANNING_MAX_TOOL_CALLS
PROBE_FORGE_PLANNING_MAX_TURNS
PROBE_FORGE_PLANNING_MAX_RETRIES
PROBE_FORGE_PLANNING_MAX_RESPONSE_TOKENS
PROBE_FORGE_PLANNING_MAX_WALL_TIME_MS
PROBE_FORGE_PLANNING_MAX_PROMPT_BYTES
PROBE_FORGE_PLANNING_MAX_TOOL_OUTPUT_BYTES
PROBE_FORGE_PLANNING_MAX_TOTAL_TOOL_OUTPUT_BYTES
PROBE_FORGE_PLANNING_MAX_PLAN_BYTES
PROBE_FORGE_PLANNING_MAX_REPOSITORY_READ_BYTES
PROBE_FORGE_PLANNING_MAX_CATALOG_ENTRIES
PROBE_FORGE_PLANNING_MAX_PLANNED_TESTS
PROBE_FORGE_PLANNING_MAX_LOAD_SCENARIOS
```

Set provider and model together or leave both unset. Provider API keys are
resolved by the Pi SDK from the provider environment and must never be placed in
`.env.example`, prompts, logs, traces, plans, or source control.

The Phase 7 worker HTTP server remains health-only. The planning library is
ready for the authenticated claim/dispatch integration; it is not an
unauthenticated planning endpoint.

## Prompt-injection and secret handling

Repository text, paths, comments, discovery facts, catalog names, and tool output
are treated as untrusted data. The system prompt and server policy remain
separate from that data. The model cannot widen tool availability or budgets,
persist a plan, alter the catalog, approve a plan, authorize a target, or execute
anything. A repository instruction that tries to do so has no corresponding tool
and no server path.

## Verification

See `docs/phases/07_planning_report.md` for the Phase 7 gate evidence, commands,
and limitations. The integration tests use the Pi faux provider and inert
fixtures; no live model credentials or network targets are required.
