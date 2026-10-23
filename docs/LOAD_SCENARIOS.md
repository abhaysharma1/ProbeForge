# Load scenario generation and validation

Phase 11 defines a declarative, bounded load-scenario contract and server-side
validation. Pi can propose correlated HTTP steps from discovered routes, but it
cannot create executable scripts, loops, or raw credentials. No load test is run
in this phase.

> **Status:** scenario generation, validation, review, and approval pinning are
> implemented. The k6 runner, scheduling, metrics, and cancellation belong to
> Phase 12.

## Declarative contract

A scenario is an ordered list of data-only `requests` (steps). Each step supports:

| Field | Purpose |
|---|---|
| `id` | Stable lowercase step ID used by `dependsOn` |
| `method` | Closed enum: `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS` |
| `path` | Same-origin absolute path; may contain `${variable}` placeholders from earlier steps |
| `headers` | Literal non-credential headers; values may use earlier `${variable}` placeholders |
| `secretHeaders` | Header name to `secret://...` reference; the only way to supply Authorization/Cookie credentials |
| `dependsOn` | Earlier step IDs only; no self, forward, duplicate, or cyclic references |
| `extract` | Header or JSON-pointer extraction rules that create correlation variables |
| `assertions` | Header or JSON-pointer `equals`, `contains`, or `exists` checks |
| `bodyTemplate` | Bounded JSON payload template; strings may use earlier `${variable}` placeholders |
| `expectedStatuses` | One or more required status assertions |

Scenario-level fields remain `name`, `description`, `requests`, `profile`, and an
optional `credentialRef` for a runner-level secret.

Example:

```json
{
  "name": "Correlated session flow",
  "description": "A bounded two-step fixture.",
  "requests": [
    {
      "id": "session",
      "method": "POST",
      "path": "/api/session",
      "secretHeaders": {
        "Authorization": "secret://fixtures/service-token"
      },
      "extract": [
        { "name": "token", "source": "header", "selector": "x-session-token" }
      ],
      "bodyTemplate": { "username": "fixture-user" },
      "expectedStatuses": [200]
    },
    {
      "id": "profile",
      "method": "GET",
      "path": "/api/profile/${token}",
      "dependsOn": ["session"],
      "headers": { "X-Session": "${token}" },
      "assertions": [
        {
          "source": "json_body",
          "selector": "/status",
          "operator": "equals",
          "expected": "ok"
        }
      ],
      "expectedStatuses": [200]
    }
  ],
  "profile": {
    "virtualUsers": 5,
    "rampUpSeconds": 10,
    "durationSeconds": 60,
    "maxRequests": 1000,
    "maxRequestsPerSecond": 5,
    "stopConditions": { "errorRate": 0.05 }
  }
}
```

## Validation rules

The Rust contract crate and TypeScript contract validator apply the same rules;
the shared fixtures are exercised by both. Validation fails closed on:

- unknown fields anywhere in a scenario, step, extraction, assertion, profile, or
  stop-condition object;
- invalid paths, protocol-relative paths, backslashes, traversal, or malformed
  template placeholders;
- a `${name}` reference that is not produced by an earlier step;
- dependencies that are missing, duplicated, self-referential, or point forward;
- duplicate step IDs or extraction variable names;
- literal Authorization/Cookie headers and hop-by-hop/length-managed headers;
- raw values in `secretHeaders` (only `secret://` references are accepted);
- unsupported operators, selector shapes, or assertion expected values;
- GET/HEAD/OPTIONS steps with a body template;
- missing expected statuses or statuses outside 100-599;
- per-step bodies over **64 KiB**;
- unknown or out-of-range stop conditions;
- loop, repeat, command, script, executable, or shell fields.

There is no loop or iteration expression in the schema. `expectedStatuses`,
extraction rules, assertions, and the bounded profile are the only repetition or
control primitives, and the runner controls their execution.

### Platform maxima

| Field | Maximum |
|---|---|
| `virtualUsers` | 100 |
| `arrivalRatePerSecond` | 100 |
| `rampUpSeconds` | 300 |
| `durationSeconds` | 900 |
| `maxRequests` | 100,000 |
| `maxRequestsPerSecond` | 100 |
| Steps per scenario | 50 |
| Extractions/assertions/dependencies per step | 20 each |
| Expected statuses per step | 20 |
| Body template per step | 64 KiB |

Closed stop conditions:

- `errorRate`: 0 to 1;
- `p95LatencyMs`: 1 to 60,000;
- `maxFailedRequests`: 1 to 100,000.

Migration `0011_load_safety.sql` adds second-line database constraints. They are
`NOT VALID` so historical rows are not rewritten during migration, but every new
or updated profile is checked.

## Target scope validation

`probe_forge_target_policy::validate_load_scenario_scope` checks every step
against an already-approved target scope without resolving DNS or sending
traffic. It:

1. validates the scope shape;
2. expands template placeholders to a harmless literal for the scope check;
3. validates scheme, host, port, path prefix, traversal, and separator escapes;
4. rejects a body on GET/HEAD/OPTIONS.

Phase 12 must call this function when a run is created with a chosen target
authorization, and must percent-encode extracted values before runtime
substitution so an extracted value cannot add path segments.

## Generation

Pi receives the bounded discovery tools and a load-planning policy that requires
ordered discovered routes, explicit expected statuses, `secret://` references,
earlier-step correlation, bounded assertions, and platform-capped profiles. The
server still validates every proposal; the model cannot change limits.

The deterministic fallback emits one conservative read-only scenario for the
first sorted literal `GET`/`HEAD` route, with a step ID and expected statuses. It
never contacts a target.

## Review, versioning, and approval

The plan-review UI shows every step, dependency, secret-backed header name,
extraction, assertion, body-template presence, bounded profile, and stop
condition. Non-GET/HEAD/OPTIONS methods are marked as state-changing. Selection
remains the only editable field.

A scenario has no independent mutable version. It is versioned by its parent
`test_plans.version`. Approval:

- revalidates each selected scenario against the current contract;
- requires the stored row name to match the scenario name;
- writes the exact scenario JSON, profile, and plan version into the immutable
  `plan_approvals.approval_snapshot`;
- records the approved scenario IDs and audit event.

This means changing a scenario requires a new plan version and a new approval;
the previous approval remains historical evidence. No load traffic is sent by
approval.

## Tests

- Shared Rust/TypeScript fixtures accept a correlated scenario and reject unknown
  dependencies, script-expression templates, loop fields, raw secret headers,
  unknown stop conditions, and profiles above platform maxima.
- Rust contract tests cover oversized bodies, platform maxima, loops, traversal,
  forward/unknown dependencies, same-step correlation references, and raw secret
  references.
- `target_policy` tests cover in-scope templated paths, out-of-scope paths,
  invalid expressions, GET bodies, and production HTTP.
- API tests approve an advanced correlated scenario and assert that the immutable
  snapshot contains its dependency and stop conditions.
- Database tests reject over-limit profiles and open stop-condition objects.
- UI tests cover the existing review/approval flow; the load card now exposes the
  step and correlation details.

No test sends a request to a target or starts a load generator.

## Deferred to Phase 12

- k6 or equivalent constrained runner;
- schedules, leases, concurrency quotas, and per-project fairness;
- execution-time secret resolution and extracted-value encoding;
- request/response caps at the runner, metrics buckets, cancellation, and
  aggregate results;
- runtime enforcement of target authorization limits combined with scenario
  profiles.
