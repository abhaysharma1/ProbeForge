# Results, recommendations, and versioned reports

Phase 13 exposes run results and generates traceable, versioned, redacted
reports without coupling generated summaries to raw evidence. Results and
evidence are durable; recommendations and reports are derived and can be
regenerated. No model is called: system recommendations are deterministic, and
the same constrained shapes can accept a model later.

> **Status:** results, recommendations, reports, comparison, redaction, and
> export are implemented as authenticated JSON APIs with tests, and the web app
> renders a run summary, per-test results, evidence metadata, recommendations,
> and report claims (project detail → Runs → View results).

## Data model

Migration `0013_results_reports.sql` adds:

- `test_results.finding_status` (`confirmed`, `suspected`, `not_applicable`) and
  `test_results.observation` — the raw structured observation, kept separate
  from any generated summary. A failed result must be classified as a
  confirmed or suspected finding for new/updated rows.
- `recommendations` — a claim with `kind` (`observed`, `hypothesis`,
  `recommendation`), `severity`, `confidence`, and optional references to a
  `result_id`, `evidence_id`, or `run_metric_id`. The
  `recommendations_traceability` constraint rejects any non-hypothesis claim
  without a stored reference.
- `reports` — one row per run and report type (`security`/`load`) with status
  (`draft`/`final`/`failed`), current version, and last error code.
- `report_versions` — immutable, hash-addressed JSON snapshots with a version
  number, summary, and generator.

## API

All routes require authentication and project membership; non-members receive
`404` so run existence is not disclosed.

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/projects/{projectId}/runs` | Run list with status, plan version, timestamps, outcome counts, and report status |
| GET | `/api/runs/{runId}` | Run summary, per-test results with evidence metadata, recommendations, metric summary, and report metadata |
| GET | `/api/runs/{runId}/report` | Report row, version history, and the latest content |
| POST | `/api/runs/{runId}/report` | Generate a new immutable report version (viewer forbidden) |
| GET | `/api/runs/{runId}/report/export?format=json\|markdown` | Export the latest version as JSON or readable Markdown |
| GET | `/api/runs/{runId}/compare?otherRunId=...` | Compare two runs |

## Traceability

Every entry in a report's `claims` array is either:

- `claimKind: "observed"` with a non-empty `references` set of `resultIds`,
  `evidenceIds`, or `metricIds`; or
- `claimKind: "hypothesis"` with empty references and a rationale where
  relevant.

Security findings cite the result and its evidence. Load observations cite the
`run_metrics` bucket that produced them. Root-cause statements (for example,
"latency may reflect target capacity") are stored as hypotheses, never as
observed facts. Findings (`outcome = failed`) are separated from other checks
(passed, skipped, inconclusive, error, cancelled).

## Redaction and failure safety

- `redact_json` recursively replaces sensitive keys (`authorization`, `cookie`,
  `token`, `password`, `secret`, `api_key`, and similar) and credential markers
  (`Bearer …`, `Basic …`, `Authorization:`/`Cookie:` values, JWT-shaped tokens)
  with `[redacted]` in report content and run-detail observations.
- Report generation validates a 256 KiB content cap before writing. If the
  content is too large it marks the report `failed` and returns `422`; the stored
  results, evidence, and metrics are untouched. Reports live in their own table,
  so generation can never overwrite results.
- Report versions are immutable and hash-addressed, so a rerun or comparison has
  stable provenance.

## Comparison

`compare` returns both sides' run type, plan version, scenario ID/name, load
profile, target origin, timestamps, outcome counts, and metric summary, plus
request/error/latency/throughput deltas.

## Web view

`apps/web/src/RunReport.tsx` renders the run summary, per-test results,
evidence metadata, recommendations, observed metrics, versioned report claims,
and a run comparison. The project detail lists runs under **Runs** and opens the
view with **View results**. Loading, empty, error, permission (viewer cannot
generate), and no-report states are explicit.

## Limitations

- The execution worker does not yet produce `test_results`, `evidence`, or
  `run_metrics` automatically; results are seeded by tests and will be written by
  the Phase 15 end-to-end path.
- Recommendations are deterministic. A model-backed generator is not wired;
  when added it must write through the same traceable, redacted shapes.
- `reports.status` is `failed` on an oversize failure; the version history keeps
  only successful versions.
