# 16 — Security adapter coverage expansion (catalog 0.3.0)

**Date:** 2026-10-27
**Status:** POST-PHASE 15b: PASS for the implemented core adapter set.
**Task-wide status:** BLOCKED on the remaining adapter groups listed under
"Remaining coverage" (dependency/CVE, SAST, IaC, configuration, DAST, API
authorization, GraphQL, and controlled-availability adapters).
**Decision:** [D-049](../DECISION_LOG.md)
**Design:** [EXECUTION_ADAPTERS.md](../EXECUTION_ADAPTERS.md)

## Audit (before this change)

The registry contained two in-process HTTP adapters
(`http.headers.missing`, `http.cors.misconfiguration`). Five catalog entries were
advertised but unavailable:

| Catalog ID | Adapter state |
|---|---|
| `http.auth.rate-limit` | no adapter; config contract unresolved |
| `network.tls.baseline` | no adapter |
| `static.secrets.exposure` | no adapter and no execution-time repository access |
| `static.dependencies.known-vulnerable` | no adapter |
| `load.baseline` | separate load runner, not an adapter |

Two root gaps made static tests impossible, not merely unimplemented:

1. The analysis worker's pinned workspace is temporary and removed before
   approval; `repository_snapshots` stores metadata only.
2. `validate_configuration` required a `route` for every adapter-backed ID, so a
   route-less TLS or static test could not pass run creation.

## Implemented adapters

| Catalog ID | Adapter | Category | Safety | Config | Result |
|---|---|---|---|---|---|
| `network.tls.baseline` | `tls_probe` 0.1.0 | network | passive | none | protocol, cipher, chain, leaf fingerprint, subject/issuer, SAN count, expiry, verification outcome |
| `http.auth.rate-limit` | `http_probe` 0.1.0 | http | bounded active | route | max 5 GETs, 250 ms spacing, status distribution, Retry-After, suspected finding when unthrottled |
| `http.cookies.insecure` | `http_probe` 0.1.0 | http | passive | route | cookie names + Secure/HttpOnly/SameSite flags; values never read |
| `http.info-disclosure` | `http_probe` 0.1.0 | http | passive | route | version banners, debug headers, bounded body marker IDs; body never retained |
| `static.secrets.exposure` | `static_secret_scanner` 0.1.0 | static | passive | none | redacted rule/file/line/confidence/entropy/preview; values never recorded |

Existing adapters (`http.headers.missing`, `http.cors.misconfiguration`) were
retained unchanged in behavior; they now also return CWE/rule/remediation
metadata.

## Coverage matrix (requested categories)

| Requested group | Status | Notes |
|---|---|---|
| Repository — secret scanning | **Implemented** | Native scanner + pinned snapshot materializer. |
| Repository — dependency/CVE | **Missing** | No OSV/advisory client; catalog entry remains unavailable. |
| Repository — SAST | **Missing** | No Semgrep/AST engine; no catalog entry yet. |
| Repository — IaC | **Missing** | No Checkov/Terraform/K8s parser. |
| Repository — configuration | **Missing** | Overlaps secret/IaC; no dedicated adapter. |
| Website — TLS/HTTPS | **Implemented (baseline)** | TLS 1.2/1.3, cert validity/hostname/issuer/expiry. TLS 1.0/1.1-only origins and HTTPS redirect enforcement are not classified. |
| Website — headers | **Implemented** | Baseline CSP/XCTO/Referrer-Policy/HSTS. |
| Website — cookies | **Implemented** | Passive attribute inspection. |
| Website — exposed resources | **Partial** | Directory-listing body marker only; no backup/source-map/debug path probing (active). |
| Website — information disclosure | **Implemented** | Banners, debug headers, trace markers. |
| Website — CORS | **Implemented** | Reflected/wildcard + credentials. |
| Website — open redirects | **Missing** | Active probing not implemented. |
| Website — common web vulns | **Missing** | No ZAP/Nuclei or active injection adapters. |
| Website — DNS/subdomain takeover | **Missing** | No DNS record analysis adapter. |
| API — discovery/spec ingestion | **Missing** | Discovery exists for planning, not as an API security adapter. |
| API — authentication | **Missing** | No test-account or per-run credential mechanism. |
| API — BOLA/IDOR, privilege escalation, mass assignment, data exposure | **Missing** | Requires authorized test accounts and designated data. |
| API — rate limits | **Partial** | Auth GET observation only; no per-account quota model. |
| API — input validation | **Missing** | No malformed-input adapter. |
| API — GraphQL | **Missing** | Not implemented. |
| Availability — request limits | **Partial** | Rate-limit observation exists; no generic quota adapter. |
| Availability — request size / upload / pagination / queue / ReDoS / resource usage | **Missing** | No adapters; would need strict budgets. |
| Load baseline | **Available via load runner** | Not part of the adapter registry. |

## Key implementation points

- **Registry-owned capabilities.** `AdapterSpec` now declares category,
  passivity, configuration contract (`Route`/`Empty`), and snapshot requirement.
  `GET /api/test-catalog` exposes the same metadata; availability is derived from
  the registry.
- **TLS probe.** Authorized HTTPS origin only; `validate_target_authority`,
  server resolver, blocked-address filtering, pinned-address TCP connect,
  verified rustls handshake, bounded observation-only retry on certificate
  failure, x509 metadata, no application data. Evidence excludes IPs and chains.
- **Bounded rate-limit observation.** Five unauthenticated GETs, 250 ms apart.
  HTTP 429 (or 503 with `Retry-After`) is a pass; other 5xx is inconclusive; no
  throttle is a medium, 0.6-confidence suspected finding; budget exhaustion is
  `failed` with partial evidence, never a pass.
- **Cookie/disclosure evidence minimization.** Cookie values and response bodies
  never enter evidence; disclosure records marker identifiers only.
- **Pinned snapshot materializer.** The execution worker queries the job's
  repository URL, credential reference, and pinned SHA; rejects credentials,
  missing commits, and unsupported schemes; runs fixed-argv `git` with hooks and
  prompts disabled, `--depth=1`, no LFS, 60 s/command and 180 s total; verifies
  `HEAD`; hands the workspace to the adapter; removes it on drop (including
  read-only objects). A failure persists one `error` result without failing
  sibling tests.
- **Secret scanner.** `crates/static-scanner`: no regex engine, fixed token
  shapes + Shannon entropy, symlink/binary/extension/directory skips, file/byte/
  finding caps, cooperative cancellation checked per directory entry, file, and
  line, and all matched spans on a line redacted together.
- **Result extensions.** Optional `remediation`, `cweIds`, `cveIds`,
  `affectedAssets`, `ruleId`, `references` are stored in existing
  `test_results.details`; no migration.
- **UI capability badges.** Plan review shows passivity (`Passive` /
  `Bounded active`), `Pinned snapshot`, and `No configuration` from registry
  metadata, alongside the existing availability badge.
- **Tool health.** `GET /readyz` reports `tools.git` and `tools.tlsRoots`;
  missing optional tools report `degraded` without blocking HTTP-only dispatch.
- **Planning contract.** HTTP tests still require a discovered route;
  network/static tests require an empty configuration. Fallback and validation
  enforce both, and run creation revalidates.

## Files

**New**

- `crates/static-scanner/` (`Cargo.toml`, `src/lib.rs`)
- `services/execution-worker/src/snapshot.rs`
- `docs/phases/16_security_adapter_coverage_report.md` (this file)

**Changed (adapter scope)**

- `crates/execution-adapters/{Cargo.toml,src/lib.rs}`
- `crates/target-policy/src/lib.rs` (`validate_target_authority`)
- `services/execution-worker/{Cargo.toml,src/lib.rs,src/dispatch.rs}`
- `services/api/src/catalog.rs`, `services/api/tests/plans_approval.rs`
- `packages/test-catalog/catalog.json` (0.3.0 + two entries)
- `services/analysis-worker/src/agent/{validate.ts,fallback.ts,planning-input.ts}`
  and their tests
- `apps/web/src/api.ts`, `apps/web/src/PlanReview.tsx`
- `docs/{EXECUTION_ADAPTERS.md,DECISION_LOG.md,IMPLEMENTATION_CHECKLIST.md,CONTRACTS.md,PLAN_APPROVAL.md,RELEASE_READINESS.md}`, `README.md`
- `.env.example`

## Verification

| Check | Command | Result |
|---|---|---|
| Rust format | `cargo fmt --all -- --check` | PASS |
| Adapter tests | `cargo test -p probe-forge-execution-adapters` | 15 passed (includes a real TLS handshake against a generated self-signed loopback server) |
| Static scanner tests | `cargo test -p probe-forge-static-scanner` | 8 passed |
| Execution worker tests | `cargo test -p probe-forge-execution-worker` | 9 passed (includes real local Git snapshot materialization, commit mismatch, cleanup, and `/readyz` tool health) |
| Rust lint | `cargo clippy --target-dir target/test-build --workspace --all-targets -- -D warnings` | PASS |
| API catalog unit test | `cargo test -p probe-forge-api --lib catalog` | 3 passed |
| Analysis worker tests | `bun test` (analysis worker) | 141 passed, 0 failed |
| Test catalog tests | `bun test` (packages/test-catalog) | 6 passed, 0 failed |
| Contracts tests | `bun run test:ts` | 59 passed |
| Web tests | `bun run test:ts` | 22 passed |
| TS typecheck | `bun run typecheck` | PASS (all four packages) |
| Biome | `bun run lint` | PASS (169 files) |
| Docs contract | `bun scripts/check-docs.mjs` | Our links resolve and the secret scan is clean, but the command exits FAIL because an unrelated in-progress change links to a missing `phases/17_run_section_live_reporting_report.md`. That file is outside this task's scope and was not created. |
| DB integration tests | `plans_approval` via `run-with-env` | **NOT RUN**: `PoolTimedOut`; Docker daemon unavailable and the local database was not accepting the configured credentials. No result is claimed. |
| Full `cargo test --workspace` | interrupted by environment restart | **NOT RUN to completion**. Targeted suites above passed. |

No real target was contacted. Adapter tests use a loopback mock HTTP server, a
temporary snapshot directory, and a local Git fixture repository.

## Honest limitations

1. **Static tests still require a target authorization.** `test_runs.target_authorization_id`
   is `NOT NULL`, so run creation asks for a target even though the static adapter
   never contacts it. A follow-up migration/API change should allow repository-only
   runs.
2. **Credentialed repositories are unavailable.** `credential_ref` causes a
   structured `repository_credentials_not_configured` error because the execution
   worker has no server-owned secret resolver. Private repositories therefore
   cannot run static tests yet.
3. **TLS 1.0/1.1-only origins** fail as `tls_handshake_failed` instead of a
   protocol-version finding; rustls does not offer those versions.
4. **HTTPS enforcement** (HTTP→HTTPS redirect) is not tested by the TLS adapter.
5. **Rate-limit confidence is 0.6** and read-only; the adapter cannot submit
   credentials or exercise method-specific limits. A no-throttle result is
   classified `suspected`, not confirmed.
6. **Secret scanning is pattern/entropy based**, not a full SAST engine. Fixtures
   and example values can produce findings; every match is a potential exposure.
7. **Information disclosure** does not probe guessed paths (backups, source maps,
   debug endpoints); only a single discovered route is inspected.
8. **An adapter failure is a test-result `error`, but the run still ends
   `completed`.** Existing run-level semantics (D-048/15a) were not changed; the
   result, severity, and report accurately show the error. Changing run status to
   `completed_with_errors` would require a queue/API change and was out of scope.
9. **`git` is required in the execution worker environment** for static tests.
   Missing Git or an empty native root store produces a structured failure and is
   visible on `/readyz`.
10. **The analysis fallback can still propose catalog entries with no adapter**
    (`static.dependencies.known-vulnerable`) because availability is registry-owned
    and is not part of the shared catalog file consumed by the worker. The review
    UI marks it unavailable and execution skips it; it is never reported as a pass.
11. **The remaining adapter groups in the coverage matrix are not implemented.**
    Catalog entries that have no adapter remain accurately marked unavailable; no
    placeholder adapter claims success.

## Deployment changes

- Execution worker image/host must provide `git` (`PROBE_FORGE_GIT_BINARY`,
  default `git`). Static tests only.
- `PROBE_FORGE_ALLOW_LOCAL_SNAPSHOTS=1` is required for `file:` repositories and
  must stay `0` outside isolated development.
- TLS requires native certificate roots in the execution environment; an empty
  store yields `tls_crypto_unavailable`.
- `GET /readyz` on the execution worker now reports `tools.git` and
  `tools.tlsRoots`. A missing optional tool returns `degraded` (HTTP 200) so
  HTTP-only dispatch can continue; coverage is visibly reduced.
- No new external scanner binary, container, queue, database, or object store was
  added. The scanner is an in-process workspace crate.

## Next permitted work

- Decide whether repository-only runs should relax the target-authorization
  requirement (migration + API contract).
- Wire a server-owned execution credential resolver so private repositories can
  be scanned.
- Add OSV-based dependency scanning and a Semgrep-compatible SAST adapter behind
  the documented fixed-path/typed-argv process runner.
- Add passive exposed-resource checks and DNS/subdomain takeover checks.
- Design the authorized-test-account model before any BOLA/IDOR or mass-assignment
  adapter; report "requires configuration" when accounts are absent.
