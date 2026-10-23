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
| [phases/](phases/) | One completion report per phase |

## Phase reports

| Phase | Report |
|---|---|
| 0 | [00_recon_report.md](phases/00_recon_report.md) |
| 1 | [01_monorepo_report.md](phases/01_monorepo_report.md) |
| 2 | [02_contracts_db_report.md](phases/02_contracts_db_report.md) |
| 3 | [03_auth_projects_report.md](phases/03_auth_projects_report.md) |

`docs/frontend_design.md` is an alias that points to the canonical
[06_frontend_design.md](06_frontend_design.md), because the agent instructions
refer to the shorter path.

## Conventions

- Agent working rules: [Agents.md](../Agents.md)
- Phase status is reported exactly as `PHASE N: PASS` or `PHASE N: BLOCKED`.
- A phase is complete only with observed evidence; anything untested is listed as a limitation.
- No secrets, real `.env` files, credentials, target access details, or scan evidence are committed.
- Documentation is updated when behavior changes; the README stays accurate after every phase.
