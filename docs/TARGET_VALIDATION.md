# Target validation and authorization

Phase 9 adds target registration, server-owned URL/DNS/IP policy, redirect and
scope enforcement, per-run request budgets, and durable target authorization
records. A target URL supplied by a user or model is never proof of permission.

No target test is executed in this phase. The guarded HTTP client exists for
future adapters and is tested only against an explicitly enabled local mock
fixture.

## Target registration

Authenticated project owners, admins, and collaborators can register targets.
Viewers may read target state.

`POST /api/projects/{projectId}/targets` accepts:

```json
{
  "origin": "https://staging.example.test",
  "environment": "staging",
  "allowedHosts": ["staging.example.test"],
  "allowedPaths": ["/api"],
  "allowedPorts": [443]
}
```

The server:

- parses and normalizes the origin (lowercase host, canonical port, no
  credentials, no path/query/fragment);
- accepts `http` or `https`, but requires HTTPS for production targets;
- rejects wildcard/invalid hosts, sensitive ports, unsafe path prefixes, and
  duplicate normalized origins in the project;
- rejects private/link-local/metadata literal IPs unless the separate
  private-network policy is enabled;
- derives missing host/port entries from the origin, but requires at least one
  explicit path prefix;
- stores the target as `pending` until destination validation succeeds.

Targets and authorizations are project-scoped. An outsider receives a not-found
response; a viewer can read but cannot create, validate, authorize, or revoke.

## Destination validation

`POST /api/targets/{targetId}/validate` resolves the target host at the
authorization's port and records the checked time and resolved addresses.

Default policy blocks loopback, RFC1918/ULA, link-local, CGNAT, multicast,
reserved, documentation, unspecified, IPv4-mapped private addresses, and known
cloud metadata addresses, including:

- `169.254.169.254` and `169.254.170.2`;
- `100.100.100.200`;
- `192.0.0.192`;
- `168.63.129.16`;
- `fd00:ec2::254` and link-local metadata aliases.

The explicit private-target mode (`PROBE_FORGE_ALLOW_PRIVATE_TARGETS=true`)
allows loopback and RFC1918/ULA destinations for local or private environments.
It still blocks link-local, metadata, multicast, reserved, unspecified, and
documentation ranges. It is disabled by default and should only be enabled in a
secured local/mock environment.

A resolution is failed closed if any returned address is disallowed, if the
answer is empty, or if DNS fails. The returned addresses are sorted and stored
with the target.

## Request and redirect enforcement

`TargetGuard` is the only path through which a future adapter may reach a target.
`validate_request_url` checks every request against the exact authorized scheme,
host, port, and path prefix. It rejects credentials, fragments, control
characters, encoded traversal, backslashes, double slashes, out-of-scope paths,
and disallowed ports.

Redirects are not followed by the HTTP library. `GuardedHttpClient`:

1. resolves the destination and pins the validated IP set for that request;
2. disables proxies, connection reuse, and automatic redirects;
3. follows at most five redirects;
4. validates every redirect against scheme/host/port/path scope;
5. rejects HTTPS-to-HTTP downgrades;
6. re-resolves and revalidates on every hop, so a DNS change cannot silently
   move traffic outside policy;
7. caps the response body and total data budget.

The client uses HTTPS certificate verification and does not expose the resolved
IP in user-facing errors.

## Authorization

`POST /api/targets/{targetId}/authorizations` requires a verified target,
`authorizationMethod: "explicit_confirmation"`, `confirmed: true`, and bounded
`TargetLimits`. Ownership verification is not implemented in this phase and is
rejected.

The authorization stores the target's normalized scope, the caller's limits,
confirmation time, valid-from time, and an expiry. The default expiry is 24 hours;
the maximum is 7 days. Expired or revoked records are reported inactive and must
not be used for a run.

`POST /api/target-authorizations/{authorizationId}/revoke` sets `revoked_at`
once. Updates that would change scope, limits, or the original confirmation are
rejected by the existing immutable authorization trigger.

## Per-run budgets

`RunBudget` enforces:

- maximum requests per second (rate spacing);
- maximum concurrency (semaphore);
- maximum duration (deadline and timeout);
- maximum request count;
- maximum response size (2 MiB platform default);
- maximum total bytes (16 MiB platform default);
- cancellation, which interrupts rate waits, connection requests, and body reads.

Authorization limits supply the first four values. Response and total-data
ceilings are server-owned platform limits and cannot be widened by a model,
scenario, or client.

## Audit

The API writes immutable audit records for:

- `target.created`;
- `target.validated`;
- `target.validation_failed`;
- `target.authorization_created`;
- `target.authorization_revoked`.

Audit metadata contains normalized origins, environment, address counts, and
error codes. It does not contain request bodies, response bodies, cookies, or
credentials.

## API summary

| Method | Path | Permission |
|---|---|---|
| `GET` | `/api/projects/{projectId}/targets` | Any project member |
| `POST` | `/api/projects/{projectId}/targets` | Owner, admin, collaborator |
| `GET` | `/api/targets/{targetId}` | Any project member |
| `POST` | `/api/targets/{targetId}/validate` | Owner, admin, collaborator |
| `GET` | `/api/targets/{targetId}/authorizations` | Any project member |
| `POST` | `/api/targets/{targetId}/authorizations` | Owner, admin, collaborator |
| `POST` | `/api/target-authorizations/{authorizationId}/revoke` | Owner, admin, collaborator |

## Verification

See `docs/phases/09_target_validation_report.md`. Tests use literal inert
addresses, injected DNS answers, and one explicit local mock HTTP server. No
test points at a public target.

## Deferred

Target management UI, ownership verification, target verification tokens, and
the actual execution adapter integration are later work. Phase 10 must use the
`TargetGuard`, `GuardedHttpClient`, and `RunBudget` paths rather than a raw HTTP
client.
