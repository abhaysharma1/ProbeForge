# ProbeForge documentation index

Normative product, architecture, and delivery documents, plus implementation tracking.

## Normative documents (read in order)

| # | Document | Purpose |
|---|---|---|
| 01 | [What we are developing](01_what_we_are_developing.md) | Product definition, workflows, target authorization and scope, report requirements |
| 02 | [Broad plan](02_broad_plan.md) | Architecture, subsystems, domain model, queue behavior, agent contract, safety rules |
| 03 | [Phase-by-phase implementation plan](03_phase_by_phase_implementation_plan.md) | 15 sequential phases with exit gates - the delivery contract |
| 04 | [Tech stack](04_tech_stack.md) | Chosen stack and engineering standards per layer |
| 05 | [Starting prompt](05_starting_prompt.md) | Agent kickoff instructions and the required phase report format |

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
| [phases/](phases/) | One completion report per phase |

## Phase reports

| Phase | Report |
|---|---|
| 0 | [00_recon_report.md](phases/00_recon_report.md) |

## Conventions

- Agent working rules: [Agents.md](../Agents.md)
- Phase status is reported exactly as `PHASE N: PASS` or `PHASE N: BLOCKED`.
- A phase is complete only with observed evidence; anything untested is listed as a limitation.
- No secrets, real `.env` files, credentials, target access details, or scan evidence are committed.
- Documentation is updated when behavior changes; the README stays accurate after every phase.
