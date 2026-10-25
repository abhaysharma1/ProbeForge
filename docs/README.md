# ProbeForge documentation index

Normative product, architecture, and delivery documents, plus implementation tracking.

## Normative documents (read in order)

| # | Document | Purpose |
|---|---|---|
| 01 | [What we are developing](01_what_we_are_developing.md) | Product definition, workflows, target authorization and scope, report requirements |
| 02 | [Broad plan](02_broad_plan.md) | Architecture, subsystems, domain model, queue behavior, agent contract, safety rules |
| 03 | [Phase-by-phase implementation plan](03_phase_by_phase_implementation_plan.md) | 16 sequential phases (0-15) with exit gates - the delivery contract |
| 04 | [Tech stack](04_tech_stack.md) | Chosen stack and engineering standards per layer |
| 05 | [Starting prompt](05_starting_prompt.md) | Agent kickoff instructions and the required phase report format |
| 06 | [Frontend design](06_frontend_design.md) | Required palette, typography, components, motion, accessibility, and visual quality gates |

## Precedence

1. Security and data integrity.
2. The phase gates in `03_phase_by_phase_implementation_plan.md`.
3. The remaining normative documents.

Do not resolve a genuine conflict silently. Choose the safest minimal interpretation and record it in the [decision log](DECISION_LOG.md).

## Implementation tracking

| Document | Purpose |
|---|---|
| [IMPLEMENTATION_CHECKLIST.md](IMPLEMENTATION_CHECKLIST.md) | Trackable tasks and exit gates for Phases 0-15 |
| [DECISION_LOG.md](DECISION_LOG.md) | Decisions, interpretations, deferred choices, open questions |
| [CONTRACTS.md](CONTRACTS.md) | Versioned API/agent contracts, limits, fixtures, and cross-language validation |
| [DATABASE.md](DATABASE.md) | Schema map, migration policy, local commands, and database invariants |
| [AUTH.md](AUTH.md) | Authentication, sessions, CSRF, project roles, API surface, and security limitations |
| [JOBS.md](JOBS.md) | Job API, queue claims, leases, fencing, retries, events, SSE replay, and scheduler behavior |
| [REPOSITORY_INGESTION.md](REPOSITORY_INGESTION.md) | Read-only Git URL policy, secret references, workspace isolation, bounds, snapshots, audit events, and CLI |
| [REPOSITORIES.md](REPOSITORIES.md) | Project repository registry: registration, archive-based removal, RBAC, and analysis start |
| [DISCOVERY.md](DISCOVERY.md) | Static application inventory, supported parsers, provenance, ambiguity, limits, and the bounded Pi discovery toolset |
| [PLANNING.md](PLANNING.md) | Pi planning, tool registry, plan validation, budgets, fallback, versioning, and persistence |
| [PLAN_APPROVAL.md](PLAN_APPROVAL.md) | Versioned test catalog, plan review APIs, optimistic selection, immutable approvals, and UI behavior |
| [TARGET_VALIDATION.md](TARGET_VALIDATION.md) | Target registration, DNS/IP policy, redirect/scope enforcement, budgets, and authorization records |
| [EXECUTION_ADAPTERS.md](EXECUTION_ADAPTERS.md) | Native security adapters, catalog mappings, typed configuration, normalized results, and evidence minimization |
| [LOAD_SCENARIOS.md](LOAD_SCENARIOS.md) | Declarative load steps, correlation/assertions, platform maxima, scope validation, and approval versioning |
| [LOAD_EXECUTION.md](LOAD_EXECUTION.md) | Constrained native load runner, hard caps, metrics, schedules, leases, quotas, and cancellation |
| [RESULTS_REPORTS.md](RESULTS_REPORTS.md) | Run results, traceable recommendations, versioned reports, redaction, export, and comparison |
| [phases/](phases/) | One completion report per phase |

## Phase reports

| Phase | Report |
|---|---|
| 0 | [00_recon_report.md](phases/00_recon_report.md) |
| 1 | [01_monorepo_report.md](phases/01_monorepo_report.md) |
| 2 | [02_contracts_db_report.md](phases/02_contracts_db_report.md) |
| 3 | [03_auth_projects_report.md](phases/03_auth_projects_report.md) |
| 4 | [04_jobs_queue_report.md](phases/04_jobs_queue_report.md) |
| 5 | [05_git_ingestion_report.md](phases/05_git_ingestion_report.md) |
| 6 | [06_static_discovery_report.md](phases/06_static_discovery_report.md) |
| 7 | [07_planning_report.md](phases/07_planning_report.md) |
| 8 | [08_plan_approval_report.md](phases/08_plan_approval_report.md) |
| 9 | [09_target_validation_report.md](phases/09_target_validation_report.md) |
| 10 | [10_execution_adapters_report.md](phases/10_execution_adapters_report.md) |
| 11 | [11_load_scenarios_report.md](phases/11_load_scenarios_report.md) |
| 12 | [12_load_runner_scheduler_report.md](phases/12_load_runner_scheduler_report.md) |
| 13 | [13_results_reports_report.md](phases/13_results_reports_report.md) |
| Post | [planning_custom_provider_report.md](phases/planning_custom_provider_report.md) |
| Post | [repositories_rerun_report.md](phases/repositories_rerun_report.md) |

`docs/frontend_design.md` is an alias that points to the canonical
[06_frontend_design.md](06_frontend_design.md), because the agent instructions
refer to the shorter path.

## Conventions

- Agent working rules: [Agents.md](../Agents.md)
- Phase status is reported exactly as `PHASE N: PASS` or `PHASE N: BLOCKED`.
- A phase is complete only with observed evidence; anything untested is listed as a limitation.
- No secrets, real `.env` files, credentials, target access details, or scan evidence are committed.
- Documentation is updated when behavior changes; the README stays accurate after every phase.
