# Security execution adapters

Phase 10 adds a small, server-owned adapter registry for vetted security checks.
The first set is intentionally narrow: two in-process HTTP observations. No
arbitrary scanner, executable path, shell command, or model-authored script is
accepted.

> **Status:** adapter library complete; run orchestration is not wired. Phase 10
> does not create runs, dispatch jobs, persist results, or contact a real target.
> The only network traffic in tests goes to a loopback mock server.

## Implemented adapters

| Catalog ID | Adapter | Version | Timeout | Required configuration |
|---|---|---|---|---|
| `http.headers.missing` | `http_probe` | `0.1.0` | 30 s | `route` or `path` |
| `http.cors.misconfiguration` | `http_probe` | `0.1.0` | 30 s | `route` or `path` |

Both use `probe_forge_target_policy::GuardedHttpClient` and `RunBudget`. The
adapter registry lives in `crates/execution-adapters`.

The catalog entries for `static.secrets.exposure`,
`static.dependencies.known-vulnerable`, `network.tls.baseline`,
`http.auth.rate-limit`, and load scenarios remain **unavailable**. Calling an
unsupported catalog ID returns an `unsupported` execution status and performs
no network request.

The API computes catalog availability from this registry, not from model output,
repository text, or a tool-requirement string alone. Two catalog entries can
share a tool requirement while only one is implemented; the catalog ID mapping
is authoritative.

## Typed configuration

The configuration object is deserialized with unknown fields denied:

```json
{ "route": "/api/items" }
```

or:

```json
{ "path": "/api/items" }
```

Rules:

- exactly one of `route` or `path` must be present;
- the value must be a safe absolute path;
- query strings, fragments, backslashes, encoded/plain traversal, double slashes,
  control characters, and overlong paths are rejected;
- the resulting URL is checked against the authorized scheme, host, port, and
  path prefix before any request is made;
- no header map, method, body, command, executable, or environment variable can
  be supplied by the plan or model.

The CORS adapter sends only a fixed, harmless `Origin:
https://probe.invalid`. The guarded client refuses credential and hop-by-hop
request headers (`authorization`, `cookie`, `proxy-authorization`, `set-cookie`,
`host`, `content-length`, `transfer-encoding`, `connection`), caps request
headers, and rejects control characters.

## Normalized result

Every adapter returns the same shape:

```json
{
  "catalogTestId": "http.headers.missing",
  "adapterName": "http_probe",
  "adapterVersion": "0.1.0",
  "executionStatus": "completed",
  "securityOutcome": "finding",
  "severity": "low",
  "confidence": 0.9,
  "summary": "Missing baseline security headers: referrer-policy.",
  "durationMs": 12,
  "evidence": [
    {
      "kind": "http_response_headers",
      "summary": "Observed security headers for the authorized route.",
      "attributes": {
        "status": 200,
        "finalPath": "/api/items",
        "scheme": "http",
        "missingHeaders": ["referrer-policy"],
        "presentHeaders": ["content-security-policy", "x-content-type-options"]
      }
    }
  ],
  "errorCode": null
}
```

### Execution status

| Value | Meaning |
|---|---|
| `completed` | The adapter produced an observation, including an inconclusive security result. |
| `failed` | Configuration, scope, network, size, or request-budget failure prevented a valid observation. |
| `timed_out` | The adapter or run duration limit expired. |
| `cancelled` | Cancellation stopped the work. |
| `unsupported` | No vetted adapter exists for the catalog ID; nothing ran. |

### Security outcome

| Value | Meaning |
|---|---|
| `passed` | The observation found no issue under the adapter's rule. |
| `finding` | The observation matched the adapter's issue rule. |
| `inconclusive` | The observation is absent or unreliable, for example a 5xx response or an execution failure. |
| `not_run` | The adapter was not executed (unsupported ID). |

Execution status and security outcome are deliberately independent. A timeout
or server error must never be reported as a pass or a confirmed finding.

## Safety invariants

- **No shell or arbitrary process.** The first adapter set executes in-process
  Rust. There is no `sh -c`, command string, executable path, argument
  concatenation, or child process in the adapter crate. Unknown configuration
  fields such as `command` are rejected before any request.
- **Server-owned registry.** Catalog IDs map to fixed adapter specs. Model and
  repository text cannot register a new adapter, version, timeout, or tool.
- **Network policy.** Target and redirect scope, DNS answer pinning, private/
  metadata blocking, redirect downgrade rejection, and response-size/total-byte
  limits are enforced by `GuardedHttpClient`, not by adapter code.
- **Budgets.** `RunBudget` enforces request rate, concurrency, duration, request
  count, per-response bytes, total bytes, and cancellation. The adapter adds a
  30-second deadline around its own work.
- **Output caps.** The guarded client enforces the platform response (2 MiB) and
  total-data (16 MiB) ceilings before or while reading a body. Adapters never
  return a response body.
- **Cancellation.** The run budget cancellation token is checked while waiting
  for the request, while reading chunks, and before additional redirects. A
  cancelled adapter returns `cancelled`, not a misleading security outcome.
- **Evidence minimization.** Evidence contains selected header names or the
  observed CORS origin/credential flags, the status, and the final path. It does
  not contain bodies, cookies, authorization headers, arbitrary response
  headers, resolved internal addresses, or raw network errors.
- **Server errors are inconclusive.** A 5xx response is reported as
  `inconclusive`, even if a header happens to be missing.

Because this phase selects no external tool, "fixed executable paths", "process
limits", and "kill the child process group" are not applicable to the shipped
adapters. If a future external adapter is added, it must use a fixed,
server-owned executable path and typed argument vector (never a shell string),
apply OS process limits, capture bounded structured output, and kill the process
tree on timeout or cancellation. It must not expose those controls to the model.

## Test coverage

The adapter tests use a loopback mock server only and cover:

- registry uniqueness, catalog IDs, adapter name and version;
- typed schema rejection for both adapters (missing route, unknown field,
  traversal route) with zero requests sent;
- unsupported ID fail-closed behavior with zero requests sent;
- missing and present security-header normalization;
- reflected-origin CORS with credentials, wildcard-origin CORS with credentials,
  and wildcard-origin CORS without credentials;
- 5xx inconclusive behavior;
- per-adapter timeout and cancellation;
- response-size cap enforcement;
- sanitized evidence: response bodies and `Set-Cookie` values never appear in
  the serialized result.

Target-policy tests additionally cover redirect/scope escapes, DNS changes,
private/metadata blocking, budgets, and credential-header rejection.

## Wiring and deferred work

The API imports the registry only to report catalog availability. There is no
run-creation endpoint, dispatcher, worker claim path, result table write, UI, or
report in Phase 10. Those are later phases:

- Phase 11: load scenario generation and validation;
- Phase 12: load runner and scheduler;
- Phase 13: result/evidence persistence and reports.

Until that wiring exists, an "available" catalog entry means a vetted adapter
exists; it does not mean a run can be started.

## Adding an adapter

1. Add or confirm a catalog entry with fixed metadata and bounded requirements.
2. Define a narrow typed configuration with `deny_unknown_fields`; reject
   commands, paths outside scope, and unbounded values.
3. Add a fixed `AdapterSpec` with catalog ID, adapter name/version, and timeout.
4. Execute only through the guarded client/budget, or through a future vetted
   process runner with fixed paths and typed arguments.
5. Normalize to `AdapterResult`, keeping execution status and security outcome
   separate.
6. Add schema, timeout, cancellation, output-cap, scope, and evidence-sanitizing
   tests for that catalog ID.
7. Keep unsupported catalog IDs closed. Availability follows the registry, not a
   shared tool-requirement string.
