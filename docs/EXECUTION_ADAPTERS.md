# Security execution adapters

ProbeForge runs only server-owned, vetted security adapters. No arbitrary
scanner, executable path, shell command, or model-authored script is accepted.
The registry maps a stable catalog ID to a fixed adapter, version, category,
configuration contract, timeout, and safety metadata.

> **Status (catalog 0.3.0):** seven adapters are registered and executable.
> `static.dependencies.known-vulnerable` and `load.baseline` remain unavailable
> (no adapter / separate load runner). Static adapters materialize the job's
> pinned commit in the execution worker; credentialed repositories are reported
> as unavailable until a server-owned execution credential resolver exists.

## Implemented adapters

| Catalog ID | Adapter | Category | Passivity | Timeout | Configuration |
|---|---|---|---|---|---|
| `http.headers.missing` | `http_probe` 0.1.0 | http | passive | 30 s | `route` or `path` |
| `http.cors.misconfiguration` | `http_probe` 0.1.0 | http | passive | 30 s | `route` or `path` |
| `http.auth.rate-limit` | `http_probe` 0.1.0 | http | bounded active | 60 s | `route` or `path` |
| `http.cookies.insecure` | `http_probe` 0.1.0 | http | passive | 30 s | `route` or `path` |
| `http.info-disclosure` | `http_probe` 0.1.0 | http | passive | 30 s | `route` or `path` |
| `network.tls.baseline` | `tls_probe` 0.1.0 | network | passive | 30 s | none |
| `static.secrets.exposure` | `static_secret_scanner` 0.1.0 | static | passive | 60 s | none |

Registry metadata (`probe_forge_execution_adapters::registry_metadata`) is the
authoritative source for category, applicable mode, passivity, configuration
contract, snapshot requirement, test-account requirement, safety level, and
required runtime dependencies. The API exposes the same metadata on
`GET /api/test-catalog`; availability is computed from the registry, never from
model output, repository text, or a tool-requirement string.

Unsupported catalog IDs return an `unsupported` execution status and perform no
work. `load.baseline` is executed by the separate load runner, not this registry.

## Typed configuration

Route-backed HTTP adapters deserialize with unknown fields denied:

```json
{ "route": "/api/items" }
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

Network and static adapters accept **no** configuration values (`{}` or `null`).
A plan that supplies `route` or any other key is rejected before a run is queued.
This keeps the static adapter from ever receiving a filesystem path.

The CORS adapter sends only a fixed, harmless `Origin:
https://probe.invalid`. The guarded client refuses credential and hop-by-hop
request headers, caps request headers, and rejects control characters.

## Network adapters

### HTTP observations

All HTTP adapters use `probe_forge_target_policy::GuardedHttpClient` and
`RunBudget`; target/redirect scope, DNS answer pinning, private/metadata
blocking, response-size and total-byte caps, rate, concurrency, duration, and
cancellation are enforced below the adapter.

- **Headers** (`http.headers.missing`): requires CSP, X-Content-Type-Options,
  and Referrer-Policy; adds HSTS for HTTPS origins. Missing headers are a
  low-severity finding (`PF-HTTP-001`, CWE-693).
- **CORS** (`http.cors.misconfiguration`): reflects the probe origin once.
  Wildcard with credentials or reflected origin with credentials is high;
  reflected origin without credentials is medium (`PF-HTTP-002`, CWE-942).
- **Cookies** (`http.cookies.insecure`): reads `Set-Cookie` attributes only.
  Missing `HttpOnly` on session-like cookies is high; other missing
  `Secure`/`HttpOnly`/`SameSite` combinations are medium or low
  (`PF-HTTP-004`, CWE-614/1004/1275). Cookie values are never read into
  evidence. A response that sets no cookies is a pass for this check.
- **Information disclosure** (`http.info-disclosure`): inspects version banners,
  debug headers, directory-listing titles, and a fixed set of stack-trace/file
  path markers in at most 256 KiB of textual body (`PF-HTTP-005`,
  CWE-200/209). Marker identifiers are recorded; body text and paths are not.
  A 5xx response is inspected (that is where traces appear) and is
  inconclusive when no marker is present.
- **Rate limiting** (`http.auth.rate-limit`): sends at most **5** unauthenticated
  `GET` probes with at least **250 ms** between attempts. HTTP 429, or 503 with
  `Retry-After`, is a pass. Other 5xx responses are inconclusive. No throttling
  after the bounded sequence is a medium-confidence **suspected** finding
  (`PF-HTTP-003`, CWE-307/799); the adapter cannot submit credentials or exercise
  method-specific limits. A run budget exhaustion returns `failed` with partial
  evidence, never a pass.

### TLS baseline

`network.tls.baseline` requires an authorized `https` origin and validates the
scheme/host/port authority against `TargetGuard` before connecting. It resolves
the host through the server resolver, discards any address blocked by policy,
and connects to a pinned address (no second DNS answer). The probe:

1. performs a verified rustls handshake using native roots (TLS 1.2 / 1.3);
2. records the negotiated protocol, cipher suite, certificate chain length,
   leaf SHA-256 fingerprint, subject/issuer, SAN count, and expiry;
3. on a certificate verification failure, repeats the handshake with a
   bounded observation-only verifier so validity/hostname/issuer metadata can be
   recorded. That verifier is never used for application traffic;
4. classifies expired/not-yet-valid/hostname-mismatch/revoked certificates as
   high and untrusted/self-signed as medium (`PF-TLS-001`, CWE-295); a
   certificate expiring within 30 days is low (`PF-TLS-003`).

Limitations: rustls does not offer TLS 1.0/1.1, so an origin that only supports
those versions fails as `tls_handshake_failed` rather than producing a
protocol-version finding. HTTPS redirect enforcement is not tested by this
adapter.

## Static adapters and the pinned snapshot

`static.secrets.exposure` scans the exact commit the plan was generated from.
The execution worker:

1. loads the job's `git_sources.normalized_url`, `credential_ref`, and pinned
   `current_commit_sha` scoped to the run's project;
2. rejects missing commits, credentialed repositories, and any URL scheme other
   than `https`/`ssh` (plus `file` only when
   `PROBE_FORGE_ALLOW_LOCAL_SNAPSHOTS=1` for isolated development);
3. creates a fresh temporary workspace and runs a fixed `git` binary with typed
   arguments (`init`, `remote add`, `fetch --depth=1 --no-tags`, detached
   checkout, `rev-parse`) with hooks disabled, prompts disabled, no system
   config, no LFS smudging, and a 60 s per-command / 180 s total budget;
4. verifies `HEAD` equals the pinned SHA, then hands the workspace root to the
   adapter as a `RepositorySnapshot`;
5. removes the workspace when the run finishes, including read-only Git objects.

No shell is constructed, no plan value becomes an argument, and a materialization
failure persists a structured `error` result for that test without failing sibling
test results in the same plan.

The scanner (`crates/static-scanner`) walks only the server-provided root, never
follows symlinks, skips dependency/build directories and binary or oversized
files, and enforces file/byte/finding limits. Matching uses fixed prefix/shape
checks and Shannon entropy — no regular-expression engine, so there is no ReDoS
surface. It reports AWS access keys, GitHub/Slack/Stripe/Google token shapes,
private-key headers, and generic high-entropy assignments to secret-like names
(`PF-SEC-001..006`, `PF-SEC-100`, CWE-798/522).

**Redaction invariant:** a result contains the rule, file path, line/column,
confidence, entropy, value length, and a line preview where every matched span is
replaced by `***`. A matched value is never written to evidence, logs, summaries,
or storage. Findings on the same line are redacted together so a second match
cannot leak through another finding's preview.

The adapter does not distinguish test fixtures from live credentials; every
match is reported as a potential exposure for human review.

## Planning and run-creation contract

- The planning prompt requires `configuration.route` from a literal route
  returned by `discovery_list_routes` for HTTP tests, and an empty configuration
  for network/static tests.
- The deterministic fallback attaches the first sorted literal `GET`/`HEAD` route
  to HTTP tests, `{}` to network/static tests, omits HTTP entries when no literal
  route exists (while keeping route-less network/static tests), and returns no
  plan when nothing executable remains.
- `validateCandidatePlan` rejects route-backed tests whose configuration is
  missing, ambiguous, unsafe, or undiscovered, and rejects any non-empty
  configuration on network/static tests. The rejection code is
  `invalid_adapter_configuration`.
- `POST /api/plans/{planId}/runs` calls
  `probe_forge_execution_adapters::validate_configuration` before inserting any
  run. An unusable configuration returns `422 invalid_adapter_configuration` and
  queues nothing.
- The review UI warns when an available HTTP test has no `route`/`path`; the API
  and worker remain authoritative.

Runs are still created against a target authorization (the schema requires one),
including static tests. The static adapter ignores the target and contacts only
the repository. Requiring a target selection for a repository-only test is a
known UX limitation tracked for a follow-up schema change.

## Normalized result

Every adapter returns the same shape:

```json
{
  "catalogTestId": "http.cookies.insecure",
  "adapterName": "http_probe",
  "adapterVersion": "0.1.0",
  "executionStatus": "completed",
  "securityOutcome": "finding",
  "severity": "high",
  "confidence": 0.9,
  "summary": "1 of 2 observed cookies are missing one or more protective attributes.",
  "durationMs": 12,
  "evidence": [
    {
      "kind": "cookie_security",
      "summary": "Cookie attribute flags observed in Set-Cookie headers. Cookie values are never read or returned.",
      "attributes": {
        "status": 200,
        "finalPath": "/login",
        "scheme": "https",
        "cookiesInspected": 2,
        "cookiesFlagged": 1,
        "cookies": [
          {
            "name": "sessionid",
            "secure": false,
            "httpOnly": false,
            "sameSite": null,
            "sessionLike": true,
            "issues": ["missing_secure", "missing_http_only", "missing_same_site"]
          }
        ]
      }
    }
  ],
  "errorCode": null,
  "remediation": "Set Secure and HttpOnly on sensitive cookies ...",
  "cweIds": ["CWE-614", "CWE-1004", "CWE-1275"],
  "affectedAssets": ["/login"],
  "ruleId": "PF-HTTP-004",
  "references": ["https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie"]
}
```

Extension fields (`remediation`, `cweIds`, `cveIds`, `affectedAssets`, `ruleId`,
`references`) are optional and are stored inside the existing `test_results.details`
JSON payload, so no result-schema migration is required.

### Execution status

| Value | Meaning |
|---|---|
| `completed` | The adapter produced an observation, including an inconclusive security result. |
| `failed` | Configuration, scope, network, snapshot, size, or budget failure prevented a valid observation. |
| `timed_out` | The adapter or run duration limit expired. |
| `cancelled` | Cancellation stopped the work. |
| `unsupported` | No vetted adapter exists for the catalog ID; nothing ran. |

### Security outcome

| Value | Meaning |
|---|---|
| `passed` | The observation found no issue under the adapter's rule. |
| `finding` | The observation matched the adapter's issue rule. |
| `inconclusive` | The observation is absent or unreliable (5xx, truncation, failure). |
| `not_run` | The adapter was not executed (unsupported ID). |

Execution status and security outcome are deliberately independent. A timeout,
server error, or snapshot failure must never be reported as a pass or a confirmed
finding.

## Safety invariants

- **No shell or arbitrary process in adapters.** All adapters execute in-process
  Rust. The only child process is the execution worker's pinned-snapshot `git`
  materializer, which uses a fixed binary and typed argument vector (no shell
  string, no model/plan input) and is not exposed to the model.
- **Server-owned registry.** Catalog IDs map to fixed adapter specs and
  versions. Model and repository text cannot register a new adapter, version,
  timeout, tool, or filesystem path.
- **Network policy.** Target and redirect scope, DNS answer pinning, private/
  metadata blocking, redirect downgrade rejection, and response-size/total-byte
  limits are enforced by `GuardedHttpClient`. TLS resolves and pins an allowed
  address and validates the authority before connecting.
- **Budgets.** `RunBudget` enforces request rate, concurrency, duration, request
  count, per-response bytes, total bytes, and cancellation. The rate-limit
  adapter adds a fixed 5-request/250 ms bound; the TLS probe consumes one request
  permit and enforces its own timeout.
- **Output caps.** The guarded client enforces the platform response (2 MiB) and
  total-data (16 MiB) ceilings before or while reading a body. Adapters never
  return a full response body or full certificate chain.
- **Cancellation.** The budget cancellation token is checked while waiting for
  requests, while reading chunks, before redirects, during resolution, and during
  TLS connects/handshakes. The static scanner checks a cooperative flag per
  directory entry, file, and line; an adapter timeout also trips that flag so a
  bounded blocking scan stops instead of finishing unnecessarily. A cancelled
  adapter returns `cancelled`.
- **Evidence minimization.** Evidence contains selected header names, CORS
  origin/credential flags, cookie names/flags, disclosure marker identifiers,
  TLS metadata, or redacted secret matches. It does not contain bodies, cookie
  values, authorization headers, arbitrary response headers, matched secret
  values, internal addresses, raw network errors, or workspace paths.
- **Server errors are inconclusive.** A 5xx response is `inconclusive` for
  header/CORS/cookie/rate-limit checks. The information-disclosure adapter
  inspects 5xx bodies for trace markers and is `inconclusive` when none appear.
- **Static values are redacted before reporting.** All matched spans on a line
  are masked together; the scanner has unit tests asserting the secret is absent
  from serialized results.

## Dependencies and deployment

| Dependency | Where | Notes |
|---|---|---|
| rustls 0.23 + tokio-rustls 0.26 + rustls-native-certs 0.8 | execution worker / adapters | TLS handshake and native roots. |
| x509-parser 0.18 | adapters | Certificate validity/subject/issuer/SAN metadata. |
| cookie 0.18 | adapters | Strict `Set-Cookie` attribute parsing. |
| git | execution worker host/image | Required only for static adapters. Fixed binary on `PATH` or `PROBE_FORGE_GIT_BINARY`. |
| `probe-forge-static-scanner` | workspace crate | In-process scanner; no external tool. |
| `rcgen` | dev-dependency only | Generates a loopback self-signed certificate for the TLS adapter test. |

Environment:

- `PROBE_FORGE_GIT_BINARY` — fixed server-owned Git binary (default `git`).
- `PROBE_FORGE_ALLOW_LOCAL_SNAPSHOTS` — `1` enables `file:` repositories for
  isolated development only; default `0`.
- Credentialed repositories (`git_sources.credential_ref` set) are rejected with
  `repository_credentials_not_configured` until the execution worker resolves
  secrets through a server-owned provider.

No dependency is assumed to be installed merely because it exists on a developer
machine: a missing `git` or empty native root store produces a structured failure,
not a pass. `GET /readyz` on the execution worker reports `tools.git` and
`tools.tlsRoots`; a missing optional tool reports `degraded` (still HTTP 200) so
HTTP-only dispatch can continue while static/TLS coverage is visibly reduced.

## Test coverage

Adapter tests use a loopback mock server and a temporary snapshot fixture only:

- registry uniqueness, catalog IDs, adapter name/version, metadata, config
  contract per kind;
- typed schema rejection (missing/ambiguous/unsafe route, unknown field,
  non-empty static/network config) with zero requests sent;
- unsupported ID fail-closed behavior with zero requests sent;
- missing and present security headers;
- reflected-origin CORS with credentials, wildcard with credentials, wildcard
  without credentials;
- bounded rate-limit pass/finding/inconclusive paths and the 5-attempt cap;
- insecure/safe/no-cookie attribute normalization with no cookie value leakage;
- stack-trace/banner disclosure normalization with no body/path leakage;
- TLS HTTPS requirement, port scope, DNS failure, cancellation, stable
  certificate-error mapping, and a real TLS handshake against a generated
  self-signed loopback server (protocol/cipher/fingerprint/subject/issuer
  metadata, untrusted-issuer finding);
- static secret scanning through the adapter with a server-owned snapshot, plus
  missing-snapshot failure and redaction assertions;
- 5xx inconclusive behavior, per-adapter timeout, cancellation, and response-size
  caps;
- snapshot materialization against a local pinned repository, commit mismatch,
  URL scheme validation, and workspace cleanup;
- target-policy redirect/scope escapes, DNS changes, private/metadata blocking,
  budgets, and credential-header rejection (unchanged).

## Adding an adapter

1. Add or confirm a catalog entry with fixed metadata and bounded requirements.
2. Choose a configuration contract (`Route` or `Empty`) and reject every other
   shape with `deny_unknown_fields`.
3. Add a fixed `AdapterSpec` with catalog ID, adapter name/version, category,
   passivity, snapshot requirement, contract, and timeout.
4. Execute only through the guarded client/budget, a bounded TLS probe, or the
   server-owned snapshot handle. Never accept a path, command, executable, or
   header map from the plan.
5. Normalize to `AdapterResult`, keeping execution status and security outcome
   separate and populating remediation/CWE/rule metadata where known.
6. Add schema, timeout, cancellation, output-cap, scope, redaction, and
   evidence-sanitizing tests for that catalog ID.
7. Keep unsupported catalog IDs closed. Availability follows the registry.

## Known gaps

- SAST, dependency/CVE scanning, IaC scanning, configuration scanning, DAST
  (ZAP/Nuclei), API authorization (BOLA/IDOR/mass assignment), GraphQL, CORS
  active exploitation, open-redirect probing, and controlled-availability
  (upload/pagination/ReDoS/queue) adapters are not implemented.
- There is no test-account or per-run credential mechanism for API authorization
  adapters.
- TLS 1.0/1.1-only origins and HTTPS redirect enforcement are not classified.
- Static scanning is pattern/entropy based; it is not a full SAST engine and can
  produce false positives on fixtures and test data.
- Static tests still require a run target authorization because the run schema
  requires one; the adapter never contacts that target.
