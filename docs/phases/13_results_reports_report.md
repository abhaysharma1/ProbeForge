# Phase 13 report — Results, AI recommendations, and reports

**Status:** PHASE 13: PASS (server-side)

**Date:** 2026-10-24

**Scope:** Added durable run results, raw structured observations, traceable
recommendations, immutable versioned reports, rerun comparison, recursive
redaction, and JSON/Markdown export as authenticated JSON APIs with database
tests. No model is called and no report content is logged. A dedicated web
result page is deferred.

## What changed

### Schema (migration `0013_results_reports.sql`)

- `test_results.finding_status` and `test_results.observation`, so raw
  structured observations live beside results rather than in generated text.
- `recommendations` with `kind` (`observed`/`hypothesis`/`recommendation`) and a
  `recommendations_traceability` constraint requiring a `result_id`,
  `evidence_id`, or `run_metric_id` unless the row is a hypothesis.
- `reports` (one per run and type) and immutable `report_versions`
  (hash-addressed JSON snapshots).

### API (`services/api/src/results.rs`)

- `GET /api/projects/{projectId}/runs`, `GET /api/runs/{runId}` — run summary,
  per-test results with evidence metadata, recommendations, metric summary, and
  report metadata, scoped to project membership.
- `POST /api/runs/{runId}/report` — deterministic generation of a redacted JSON
  document whose claims carry references or a hypothesis label, stored as a new
  immutable version. Viewers are forbidden.
- `GET /api/runs/{runId}/report`, `GET /api/runs/{runId}/report/export?format=json|markdown`.
- `GET /api/runs/{runId}/compare?otherRunId=...` — profile, target, scenario
  version, timestamps, outcome counts, and metric deltas.
- Recursive redaction of sensitive keys and credential markers in report content
  and run-detail observations.

### Documentation

- New `docs/RESULTS_REPORTS.md`; decision D-034; checklist and index updates.

## Exit gate evidence

- **Every AI claim can be traced to stored evidence or is explicitly labeled a
  hypothesis.** Report claims carry `references` (`resultIds`/`evidenceIds`/
  `metricIds`) or `claimKind = "hypothesis"`; `recommendations_traceability`
  rejects untraceable non-hypothesis rows. The integration test
  `report_generation_is_versioned_traceable_and_redacted` walks every claim.
- **Report generation failure does not destroy test results.**
  `report_generation_failure_preserves_results` seeds an oversize report, gets
  `422`, and asserts all 40 stored results remain and the report is `failed`.
- **Secrets do not appear in reports or logs.** `redaction_removes_credentials_from_nested_content`
  and the report tests assert the seeded secret never appears in report content,
  run detail, or Markdown export.
- **Run comparison shows profile, target, scenario version, and timestamps.**
  `run_comparison_shows_profile_target_scenario_and_timestamps` asserts scenario
  names, profile VUs, target origin, plan version, and start time on both sides.

## Verification

Commands were run from `D:\Projects\ProbeForge` with a command-scoped local
`DATABASE_URL`.

| Command | Result |
|---|---|
| `cargo test -p probe-forge-api --lib results` | PASS — 5 unit tests (redaction, traceability, Markdown, report type, hash). |
| `DATABASE_URL=... cargo test -p probe-forge-api --test results_reports` | PASS — 4 integration tests. |
| `bun run db:migrate` | PASS — migration `0013_results_reports` applied after `0012`. |
| `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 bun run check` | PASS — docs 80/80, 14 linked docs, 16 phase headings; fmt; clippy `-D warnings`; workspace tests (api lib 32, results_reports skipped without DB); Biome 148 files; typecheck; contracts 55, test-catalog 6, analysis-worker 113 pass / 7 skip / 586 assertions, web 9. |
| `DATABASE_URL=... bun run check:db` | PASS — api/auth/jobs/plans/targets (5), jobs (2), migrations (1), plan approval (6), results/reports (4), targets (5), queue/scheduler (8). |
| `bun run check:services` | PASS — 5/5 healthy; ports 8080, 8081, 8082, 8090, and 5173 closed afterward. |
| `git diff --check` | PASS — no whitespace errors. |

Failures observed and resolved during verification:

- The first report-content test found no `[redacted]` marker because the report
  embedded findings/checks but not the raw observation. The report now includes
  the redacted `observation` and the fixture seeds a credential there.
- Clippy rejected `assert_eq!(..., true)` in the integration test; changed to
  `assert!`.
- Docker Desktop was not running at the start of verification; it was started
  and the Postgres container was healthy before migration and DB checks.

## Known limitations

- No web result page or run summary view yet; the data is available through the
  authenticated JSON API. A dedicated UI is deferred until it can meet the
  frontend design quality gate.
- The execution worker does not yet write `test_results`, `evidence`, or
  `run_metrics` automatically; results are seeded by tests and will be produced
  by the Phase 15 end-to-end path.
- Recommendations are deterministic system output; no model-backed generator is
  wired. A future generator must write through the same traceable, redacted
  shapes.
- Report content embeds a bounded, redacted observation but not the full
  `details` blob; the raw details remain in `test_results` and are redacted when
  returned through the run detail API.
- No real target or scan results are involved; all fixtures are inert.

## Next permitted phase

**Phase 14 — Observability, quotas, and operational hardening.** Not started. It
requires explicit authorization after this report is reviewed.
