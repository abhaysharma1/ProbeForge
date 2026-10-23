# 4. ProbeForge — Technology Stack and Engineering Standards
## Required Choices, Alternatives, and Implementation Constraints

---

## Product identity

The product name is **ProbeForge**. Use this name consistently in user-facing copy, the README, page titles, app metadata, and documentation. Keep the product name configurable where branding is normally configurable; do not rename third-party dependencies, package names, or internal technical identifiers without a reason. Do not claim domain or trademark availability has been checked.

## 1. Stack summary

| Layer | Default choice | Reason |
|---|---|---|
| Web frontend | React + TypeScript + Vite | Simple SPA and fast iteration |
| UI | Tailwind CSS + shadcn/ui | Reusable, accessible components |
| Client data | TanStack Query | Caching, mutation state and request lifecycle |
| Forms/schema | React Hook Form + Zod | Typed forms and client-side validation; server remains authoritative |
| Charts | Recharts or another maintained chart library | Load metrics and run comparisons |
| Control-plane API | Rust + Axum + Tokio | Efficient concurrent API and strong type safety |
| Database access | SQLx | Explicit SQL, async support and compile-time query checking where configured |
| Database | PostgreSQL | Durable state, relational constraints and transactional queue |
| Migrations | SQLx migrations or a single established migration tool | Versioned, reproducible schema changes |
| Agent runtime | TypeScript + Pi SDK | Native fit for the Pi agent runtime and typed tool integrations |
| Worker runtime | Bun or supported Node.js LTS | Choose one and pin it; do not mix runtimes casually |
| Security runner | Version-pinned allowlisted tools behind adapters | Structured output and bounded execution |
| Load runner | k6 initially | Mature load generation and metrics |
| Live updates | Server-Sent Events (SSE) | One-way progress updates with reconnect support |
| Local services | Docker Compose | Repeatable PostgreSQL and local development |
| Testing | Rust tests, TypeScript tests, API integration tests, browser tests | Verify boundaries at each layer |
| Observability | Structured logs, metrics, health checks | Diagnose jobs and worker failures |

The coding agent must confirm current compatible stable versions during setup and pin them in lockfiles. Avoid floating `latest` tags for production dependencies and tool images.

## 2. Frontend standards

- Use strict TypeScript mode.
- Organize by feature: authentication, projects, jobs, plans, targets, runs, findings, metrics, settings.
- Keep API clients typed from shared contracts where practical.
- Use Zod to improve form feedback but always repeat validation in Rust.
- Treat API content, agent text and scanner output as untrusted display data. Escape rendered text and avoid unsafe HTML rendering.
- Implement loading, empty, error, stale-data, cancellation and permission-denied states.
- Make plan selection and approval explicit. Show the exact version and target being approved.
- Use SSE for live events; recover from disconnects by fetching job/run state and replaying events.
- Use accessible labels, keyboard navigation and clear severity text/icons—not color alone.
- Do not store Git credentials, target credentials or long-lived access tokens in local storage.
- The frontend never receives database credentials and never decides whether a target is safe.

## 3. Rust API standards

- Use Axum routers, typed request/response DTOs, middleware for authentication and authorization, and Tokio for async work.
- Use SQLx with parameterized queries. Never interpolate user input into SQL.
- Separate modules for auth, projects, sources, jobs, plans, targets, runs, results, events, policy, and persistence.
- Keep business rules in service/policy modules rather than handlers.
- Define typed enums for job goals, statuses, risk classes and test outcomes.
- Use structured errors with stable error codes; avoid returning internal stack traces.
- Set request body and timeout limits.
- Implement graceful shutdown and health/readiness endpoints.
- Use transactions for state transition + event/audit writes where consistency requires it.
- Avoid long transactions during Git/model/tool work.
- Use database constraints as a second line of defense, not a replacement for service validation.
- Keep credentials in a secret manager or environment injection mechanism; never commit `.env`.

## 4. PostgreSQL standards

- PostgreSQL is the source of truth for jobs, approvals, schedules, results and reports.
- Use UUID primary keys for major entities and foreign keys for relationships.
- Store flexible agent/tool metadata in JSONB only where appropriate; use normal columns for fields that need constraints, indexes, joins or filtering.
- Version JSON schemas for plans and scenarios.
- Use atomic `FOR UPDATE SKIP LOCKED` claims for competing workers.
- Keep claims short and commit immediately.
- Use leases and heartbeats for long jobs.
- Use fencing/lease ownership checks to prevent stale workers from overwriting newer state.
- Index queue status/priority/creation time, lease expiration, due schedules, project scope and event sequence.
- Use unique idempotency keys for result writes and external operation tracking.
- Avoid high-cardinality per-request event records during load tests; persist time-bucketed aggregates.
- Define retention and backup/restore procedures.
- Do not expose the database to the browser or the target under test.

## 5. Pi agent and tool design

- Use Pi as a planner/orchestrator, not the policy authority.
- Expose tools with strict typed schemas and bounded results.
- Tools should return structured JSON and stable error codes.
- Separate read-only repository tools from test-execution tools.
- Analysis worker sessions must not have access to execution tools.
- Execution tools are available only to the execution worker after server-side approval checks.
- Never expose unrestricted shell, arbitrary executable paths, arbitrary tool names, or model-controlled raw command strings.
- Treat repository content, web responses, headers, and scanner output as untrusted data.
- Bound context, output sizes, tool calls, agent iterations and wall-clock duration.
- Record model/provider/version and relevant trace metadata; redact secrets.
- Have deterministic fallback behavior if Pi returns invalid JSON or the provider fails.
- The AI may recommend a test; only the backend can authorize its execution.

## 6. Repository inspection and Git

- Use a Git library or fixed argument invocation with no shell interpolation.
- Normalize supported source URLs and allow only approved protocols.
- Resolve references to a commit SHA and record it.
- Use short-lived, read-only tokens with repository-scoped permissions where possible.
- Never write credentials into the repository URL or logs.
- Do not run `npm install`, `pnpm install`, `yarn install`, `pip install`, build commands, tests, or repository scripts during source analysis.
- Treat lockfiles and manifests as data.
- Limit file count, file size, total bytes, and scan time.
- Clean workspaces on all exits, including cancellation and panic/error paths.

## 7. Security tool adapters

Each adapter must have:
- stable adapter ID and version;
- supported catalog test IDs;
- typed configuration schema;
- typed result schema;
- fixed executable/image version;
- argument builder that does not use shell interpolation;
- target-scope enforcement;
- timeout, output cap, process/resource limits and cancellation;
- exit-code mapping separated from security outcome;
- secret redaction;
- tests using mocks or local inert fixtures.

Do not equate a tool's nonzero exit code with a vulnerability. Normalize findings, execution errors and inconclusive results separately. Review tool licensing, update cadence and data handling before adding a tool.

## 8. Load testing standards

- Use a mature runner such as k6 initially rather than writing a custom load generator.
- Generate scenarios from a constrained schema or vetted templates.
- Do not execute arbitrary model-generated scripts.
- Store credentials by secret reference and inject them only at execution time.
- Enforce hard caps for VUs/arrival rate, duration, ramp-up, request count, response size, total bytes and concurrent runs.
- Validate the target and redirects at execution time.
- Ensure cancellation terminates the runner and its child processes.
- Store aggregate latency distributions, throughput, status codes, errors, timeouts and dropped iterations when available.
- Label backend-resource explanations as hypotheses unless backend metrics confirm them.
- Do not compare runs without showing target, scenario version, profile and test window.

## 9. Authentication and secret handling

Preferred approach: an established OIDC provider or maintained auth library rather than custom authentication. If local passwords are required, use a modern password hash such as Argon2id. Use secure cookies, session revocation, CSRF protections where applicable, rate limits, and project-level RBAC.

Secrets include Git tokens, API keys, target credentials, cookies, session tokens, database credentials and model-provider keys. Store references in application data; retrieve actual values only in the worker that needs them. Redact secrets before logs, agent context, evidence storage, and report rendering. Rotate and revoke credentials.

## 10. Testing standards

- Unit tests for validators, policy, transitions, schema parsing, redaction and result normalization.
- Database integration tests for migrations, queue claiming, leases, retries and idempotency.
- Contract tests for Pi tool schemas and API DTOs.
- Frontend tests for approval flows, permission states and event reconnection.
- End-to-end tests against local mock repositories and mock HTTP targets.
- Never point automated development tests at public targets.
- Avoid requiring real model-provider credentials in CI; use a deterministic fake Pi/model adapter.
- Add explicit regression tests for prompt injection, SSRF/destination validation, redirects, unbounded scenario loops, cancellation, stale workers and secret leakage.

## 11. Deployment strategy

Begin with local development via Docker Compose and separate process entry points. Production can use containers and a small number of services, but the security execution and load runners should be isolated from the API/control plane by process, network and resource boundaries. Use private database access, least-privilege service identities, TLS, secret injection, backups, health checks, and a restricted egress policy.

Do not require Kubernetes, a service mesh, a dedicated message broker, NAT Gateway, or VPC endpoints for the first implementation. The architecture should not depend on those services. If the platform is deployed to a cloud, explicitly design how workers reach approved targets while preserving outbound restrictions and database isolation.

## 12. Naming and code quality

- Use clear domain terms consistently: `job` for the user's overall workflow, `plan` for proposed tests, `run` for one execution, `planned_test` for one proposed test, `test_result` for one result.
- Keep shared JSON contracts versioned.
- Prefer small modules with explicit dependencies.
- Add comments for security rationale and non-obvious invariants, not comments that repeat code.
- Do not suppress compiler/linter errors without a documented reason.
- Keep the README accurate after every phase.
