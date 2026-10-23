# 1. What We Are Developing — ProbeForge
## Product Definition and Functional Requirements

**Audience:** coding agent implementing the product from an empty repository  
**Status:** normative product specification  
**Product name:** ProbeForge  
**Primary goals:** Vulnerability Testing and Load Testing

---

## 1. Product summary

Build **ProbeForge**, a web platform that uses an AI agent to analyze a user's Git repository, discover the application's architecture and API surface, propose appropriate tests, and—only after user review and authorization—run approved tests against a deployed version of that application. The platform provides a highly detailed, evidence-based view of each test, its execution, findings, metrics, and AI-generated recommendations.

The product has exactly two primary job goals exposed to the user:

1. **Vulnerability Testing**
2. **Load Testing**

The user chooses the goal. The AI autonomously decides which repository-inspection steps, test candidates, tools, and analysis steps are relevant. For vulnerability testing, the user reviews and selects the proposed tests before they are run against a deployed target. For load testing, the AI discovers endpoints and request requirements, proposes scenarios, and the user configures and schedules an approved run.

This is not a platform for running the user's source code. Repository analysis is read-only: clone the repository at a pinned commit and inspect files, source, manifests, route definitions, schemas, and configuration. Do not run the project, install dependencies, execute build scripts, run repository tests, or start its server as part of the default workflow.

## 2. Product principles

- **Approval before impact:** repository analysis and planning never trigger target testing automatically.
- **AI plans; deterministic policy enforces:** Pi may propose actions, but server-side policy validates every execution request.
- **Evidence over assertions:** a finding must cite observations and evidence. Label hypotheses and unverified source-level suspicions clearly.
- **Reproducibility:** record commit SHA, plan version, selected tests, tool versions, target configuration, test profile, and timestamps.
- **No arbitrary shell authority:** the model cannot invent shell commands or bypass approved tool adapters.
- **Least privilege:** the dashboard never accesses the database directly. Workers get only the credentials and network access needed for their task.
- **Cost-aware by default:** use a PostgreSQL-backed queue initially. Do not introduce a separate queue service unless measured requirements justify it.
- **Observable lifecycle:** users can see progress, plan decisions, skipped tests, errors, evidence, and final recommendations.
- **Explicit scope:** every active test run is tied to a verified target, authorized scope, budget, and cancellation mechanism.
- **Phase gates:** implementation phases are completed and verified sequentially. Do not start later phases while the current phase's exit criteria fail.

## 3. User personas and permissions

### Project owner
- Creates a project and configures its Git source.
- Provides or authorizes Git access.
- Adds deployed targets and verifies ownership/authorization.
- Creates jobs, reviews plans, approves tests, schedules runs, and views reports.
- Cancels active runs.

### Project collaborator
- Can be granted scoped access to view jobs/reports and, if permitted, create or approve runs.
- Cannot change project ownership or access secrets unless explicitly granted.

### Platform administrator
- Manages platform-level configuration, tool catalog, quotas, worker health, and audit review.
- Must not silently alter an existing run's approved test scope.

Use project-scoped RBAC. Enforce permissions on the backend for every operation; hiding a button is not authorization.

## 4. Primary workflow A: Vulnerability Testing

1. User creates a job and selects `vulnerability_testing`.
2. User selects or configures a Git repository. The repository reference may be a branch/tag/commit; resolve it to an immutable commit SHA before analysis.
3. API validates user permission, source format, repository access configuration, and quotas. If required input is missing, keep the job as a draft or return a clear validation error; do not enqueue incomplete work.
4. Analysis worker clones only the selected commit into a temporary workspace.
5. Worker inspects source statically. It must not install packages or execute project code.
6. Pi uses structured discovery tools to identify frameworks, entry points, routes, methods, request schemas, authentication/authorization logic, file upload paths, database usage patterns, outbound requests, secrets-handling patterns, and likely security surfaces.
7. Pi proposes tests from the platform's supported test catalog. Every proposed test includes a rationale, affected surface, prerequisites, risk level, expected evidence, and whether it can be meaningfully run remotely.
8. Backend validates the proposal against a schema and available tools. Unsupported tests are rejected or marked unavailable; they are never silently mapped to arbitrary commands.
9. User reviews the plan and selects individual test candidates.
10. User chooses a deployed target, allowed host/path scope, environment, and authorization confirmation. Require stronger verification for public targets or higher-impact test categories.
11. Backend revalidates the selected test set, target scope, authorization, rate/time budgets, and plan version.
12. Execution worker runs only selected, allowlisted tests using fixed adapters and validated parameters.
13. Results are stored per test with status, evidence, duration, tool/version, and outcome classification.
14. AI summarizes results and recommends remediation, linking each claim to evidence. It must distinguish confirmed findings, suspected findings, passed checks, skipped checks, inconclusive results, and tool errors.
15. User sees a detailed report and can rerun a saved plan version or create a new plan.

## 5. Primary workflow B: Load Testing

1. User creates a job and selects `load_testing`.
2. Worker performs the same pinned-commit, read-only repository analysis.
3. Pi discovers API routes and methods, request/response types where inferable, validation constraints, authentication mechanisms, dependencies between endpoints, pagination, idempotency considerations, and likely required seed data.
4. Pi generates load scenarios using a constrained scenario schema—not arbitrary code. A scenario describes requests, parameters, payload templates, sequencing, extraction/correlation rules, authentication references, and assertions.
5. Backend validates scenario syntax, endpoint references, request limits, credential references, and scenario safety. User reviews the scenarios and can disable or edit them.
6. User configures the deployed target, authorized scope, schedule/time zone, virtual-user or arrival-rate profile, ramp-up, duration, maximum request budget, and stop conditions.
7. Backend validates the target and configuration. The schedule is stored durably; browser closure does not affect it.
8. Scheduler enqueues the run when due.
9. Load runner executes the approved scenario with enforced caps and cancellation support.
10. Store aggregate metrics and useful per-endpoint metrics. Avoid writing every request as an event row; high-frequency metrics belong in time-series buckets/histograms.
11. AI analyzes results and produces evidence-linked observations and recommendations. Distinguish observed symptoms from inferred root causes.
12. User can compare runs only when their target, scenario version, and profiles are clearly shown.

## 6. Target authorization and scope

A user-provided URL is not sufficient proof of permission. Before running tests, capture a durable authorization record containing:
- project and user IDs;
- target URL and normalized origin;
- allowed hostnames and path scope;
- environment designation (for example, staging or production);
- authorization method/confirmation and timestamp;
- approved test IDs or scenario version;
- permitted time window, intensity, rate, and duration limits;
- who approved the run.

Default-deny destinations resolving to loopback, private, link-local, multicast, reserved, or cloud metadata ranges unless a deliberately configured private-target mode exists and is separately secured. Validate all resolved addresses, redirects, DNS changes, and the final connection destination. Avoid SSRF paths into platform infrastructure. Revalidate at execution time.

Do not provide destructive tests by default. High-impact tests require explicit disclosure and a separate confirmation. Do not run tests outside the approved target scope.

## 7. What the product does not do

- It does not execute or build the source repository during normal discovery.
- It does not install dependencies from user repositories.
- It does not let the AI run unrestricted shell commands.
- It does not automatically scan a deployment immediately after repository analysis.
- It does not guarantee that an application is secure merely because selected checks passed.
- It does not assert a root cause without sufficient evidence.
- It does not perform unbounded load testing.
- It does not expose database credentials to the frontend or the model.
- It does not treat repository contents, API responses, or scanner output as trusted instructions.

## 8. Report quality requirements

Every report must include:
- job ID, goal, repository URL, immutable commit SHA, plan version;
- target identifier and authorized scope, with secrets redacted;
- run ID, schedule, start/end timestamps, environment and effective limits;
- summary of completed, failed, skipped, inconclusive, and errored tests;
- per-test detail and evidence;
- severity and confidence with explanation;
- reproducibility details, including tool versions and sanitized parameters;
- remediation recommendations prioritized by risk and effort;
- explicit limitations and untested surfaces;
- AI-generated content clearly separated from direct tool observations.

The report must never include raw access tokens, passwords, cookies, private keys, or unredacted sensitive response bodies.
