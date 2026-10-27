import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ApiError,
  type ComparisonSide,
  compareRuns,
  generateRunReport,
  getRun,
  listRuns,
  type RecommendationView,
  type RunComparison,
  type RunDetail,
  type RunEvent,
  type RunSummaryEntry,
  runReportExportPath,
  type TestResultView,
} from "./api";
import { isLiveRun, runStatusForEvent } from "./jobEvents";
import { useRunEvents } from "./useJobEvents";

interface RunReportProps {
  readonly runId: string;
  readonly projectId: string;
  readonly canWrite: boolean;
  readonly csrfToken: string;
  readonly onBack: () => void;
  readonly onSessionExpired: () => void;
}

const DATE_TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

const OUTCOME_LABELS: Record<string, string> = {
  passed: "Passed",
  failed: "Failed",
  skipped: "Skipped",
  inconclusive: "Inconclusive",
  error: "Execution error",
  cancelled: "Cancelled",
};

const FINDING_LABELS: Record<string, string> = {
  confirmed: "Confirmed finding",
  suspected: "Suspected finding",
  not_applicable: "Not applicable",
};

const KIND_LABELS: Record<string, string> = {
  observed: "Observed",
  hypothesis: "Hypothesis",
  recommendation: "Recommendation",
};

const RUN_TYPE_LABELS: Record<string, string> = {
  vulnerability_testing: "Vulnerability testing",
  load_testing: "Load testing",
};

function formatDateTime(value: string | null): string {
  return value ? DATE_TIME_FORMATTER.format(new Date(value)) : "—";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KiB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function formatPercent(value: number | null): string {
  return value === null ? "—" : `${Math.round(value * 100)}%`;
}

function message(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function claimsOf(
  content: Record<string, unknown> | null | undefined,
): Array<Record<string, unknown>> {
  const claims = content?.claims;
  return Array.isArray(claims) ? (claims as Array<Record<string, unknown>>) : [];
}

function referenceCount(references: unknown): number {
  if (typeof references !== "object" || references === null) {
    return 0;
  }
  const record = references as Record<string, unknown>;
  let total = 0;
  for (const key of ["resultIds", "evidenceIds", "metricIds"]) {
    const value = record[key];
    if (Array.isArray(value)) {
      total += value.length;
    }
  }
  return total;
}

function CountStat({ label, value }: { readonly label: string; readonly value: number }) {
  return (
    <div>
      <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">{label}</dt>
      <dd className="mt-0.5 text-[15px] font-semibold tabular-nums">{value}</dd>
    </div>
  );
}

function OutcomeLine({ result }: { readonly result: TestResultView }) {
  const outcome = OUTCOME_LABELS[result.outcome] ?? result.outcome;
  const finding = result.findingStatus ? FINDING_LABELS[result.findingStatus] : null;
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-pf-text-muted">
      <span className="font-medium text-pf-text-secondary">{outcome}</span>
      {finding ? <span aria-hidden="true">·</span> : null}
      {finding ? <span>{finding}</span> : null}
      {result.severity ? (
        <>
          <span aria-hidden="true">·</span>
          <span>Severity {result.severity}</span>
        </>
      ) : null}
      <span aria-hidden="true">·</span>
      <span>Confidence {formatPercent(result.confidence)}</span>
    </div>
  );
}

function ComparisonColumn({ side }: { readonly side: ComparisonSide }) {
  return (
    <div className="min-w-0">
      <p className="truncate text-[12px] font-medium" title={side.name || side.runId}>
        {side.name || side.scenarioName || RUN_TYPE_LABELS[side.runType] || side.runType}
      </p>
      <p className="mt-0.5 text-[10px] text-pf-text-muted">
        Plan v{side.planVersion} · {side.profile?.virtualUsers ?? "—"} VUs ·{" "}
        {side.profile?.durationSeconds ?? "—"}s
      </p>
      <p className="mt-0.5 truncate font-mono text-[10px] text-pf-text-muted">
        {side.targetOrigin ?? "target unknown"}
      </p>
      <p className="mt-0.5 text-[10px] text-pf-text-muted">
        {formatDateTime(side.startedAt)} → {formatDateTime(side.finishedAt)}
      </p>
    </div>
  );
}

export function RunReport({
  runId,
  projectId,
  canWrite,
  csrfToken,
  onBack,
  onSessionExpired,
}: RunReportProps) {
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [liveError, setLiveError] = useState(false);
  const [reportError, setReportError] = useState<string | null>(null);
  const [reportBusy, setReportBusy] = useState(false);
  const [siblings, setSiblings] = useState<readonly RunSummaryEntry[]>([]);
  const [compareId, setCompareId] = useState("");
  const [comparison, setComparison] = useState<RunComparison | null>(null);
  const [compareError, setCompareError] = useState<string | null>(null);
  const [compareBusy, setCompareBusy] = useState(false);

  const handleFailure = useCallback(
    (cause: unknown) => {
      if (cause instanceof ApiError && cause.status === 401) {
        onSessionExpired();
        return null;
      }
      return message(cause);
    },
    [onSessionExpired],
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setDetail(null);
    setComparison(null);
    setCompareId("");
    setCompareError(null);

    getRun(runId)
      .then((next) => {
        if (!cancelled) {
          setDetail(next);
        }
      })
      .catch((cause) => {
        if (cancelled) {
          return;
        }
        const text = handleFailure(cause);
        if (text) {
          setError(text);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [runId, handleFailure]);

  useEffect(() => {
    let cancelled = false;
    if (!detail) {
      return;
    }
    listRuns(projectId)
      .then((runs) => {
        if (!cancelled) {
          setSiblings(runs.filter((run) => run.id !== runId));
        }
      })
      .catch(() => {
        // Sibling runs are optional context; comparison simply stays unavailable.
      });
    return () => {
      cancelled = true;
    };
  }, [detail, projectId, runId]);

  // Live reporting for this run. Status-implying events (`run.*`) and report
  // events (`report.*`) trigger a coalesced re-read; the server snapshot stays
  // authoritative and progress-only events do not churn the results view.
  const detailRef = useRef<RunDetail | null>(null);
  useEffect(() => {
    detailRef.current = detail;
  }, [detail]);

  const refreshState = useRef({ inFlight: false, pending: false });
  const requestDetailRefresh = useCallback(() => {
    const state = refreshState.current;
    if (state.inFlight) {
      state.pending = true;
      return;
    }
    state.inFlight = true;
    void getRun(runId)
      .then((next) => {
        setDetail(next);
        setError(null);
      })
      .catch((cause) => {
        if (cause instanceof ApiError && cause.status === 401) {
          onSessionExpired();
          return;
        }
        // Keep showing the last authoritative snapshot; the live notice below
        // reflects stream health rather than a failed load.
      })
      .finally(() => {
        state.inFlight = false;
        if (state.pending) {
          state.pending = false;
          requestDetailRefresh();
        }
      });
  }, [runId, onSessionExpired]);

  const handleRunEvent = useCallback(
    (streamRunId: string, event: RunEvent) => {
      if (streamRunId !== runId) {
        return;
      }
      const nextStatus = runStatusForEvent(event.type);
      const statusChanged =
        nextStatus !== null && (detailRef.current?.run.status ?? null) !== nextStatus;
      if (!statusChanged && event.type !== "report.completed" && event.type !== "report.failed") {
        return;
      }
      setLiveError(false);
      requestDetailRefresh();
    },
    [runId, requestDetailRefresh],
  );

  const live = detail ? isLiveRun(detail.run.status) : false;
  const runIds = useMemo(() => (live ? [runId] : []), [live, runId]);
  useRunEvents({
    runIds,
    enabled: detail !== null,
    onEvent: handleRunEvent,
    onStreamError: () => {
      setLiveError(true);
    },
  });

  const report = detail?.report ?? null;
  const claims = useMemo(() => claimsOf(report?.latest?.content), [report]);

  async function handleGenerate() {
    setReportBusy(true);
    setReportError(null);
    try {
      const next = await generateRunReport(runId, csrfToken);
      setDetail((current) =>
        current
          ? {
              ...current,
              report: next,
              run: { ...current.run, reportStatus: next.status },
            }
          : current,
      );
    } catch (cause) {
      const text = handleFailure(cause);
      if (text) {
        setReportError(text);
      }
    } finally {
      setReportBusy(false);
    }
  }

  async function handleCompare() {
    if (compareId === "") {
      return;
    }
    setCompareBusy(true);
    setCompareError(null);
    try {
      const next = await compareRuns(runId, compareId);
      setComparison(next);
    } catch (cause) {
      const text = handleFailure(cause);
      if (text) {
        setCompareError(text);
      }
    } finally {
      setCompareBusy(false);
    }
  }

  if (loading) {
    return (
      <div
        role="status"
        className="rounded-lg border border-pf-border bg-pf-surface px-4 py-12 text-center text-[13px] text-pf-text-secondary"
      >
        Loading run results...
      </div>
    );
  }

  if (error || !detail) {
    return (
      <div className="rounded-lg border border-pf-border bg-pf-surface px-5 py-6">
        <p role="alert" className="text-[13px] text-pf-error">
          {error ?? "This run could not be loaded."}
        </p>
        <button
          type="button"
          onClick={onBack}
          className="mt-4 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
        >
          Back to project
        </button>
      </div>
    );
  }

  const { run, reportType, target, scenario, results, recommendations, metrics } = detail;
  const counts = run.resultCounts;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <button
            type="button"
            onClick={onBack}
            className="mb-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:text-pf-accent"
          >
            ← Back to project
          </button>
          <p className="font-mono text-[10px] font-medium tracking-[0.14em] text-pf-text-muted">
            RUN RESULTS
          </p>
          <h2 className="mt-1 text-[22px] font-semibold leading-tight tracking-[-0.02em]">
            {RUN_TYPE_LABELS[run.runType] ?? run.runType}
          </h2>
          <p className="mt-1 text-[12px] font-medium text-pf-text-muted">
            {run.name ? `${run.name} · plan v${run.planVersion}` : `plan v${run.planVersion}`}
          </p>
          <p className="mt-1.5 max-w-2xl text-[13px] text-pf-text-secondary">
            Stored results and evidence are authoritative. Generated summaries and recommendations
            are separate and every claim cites a result, evidence row, or metric bucket, or is
            labeled a hypothesis.
          </p>
        </div>
        <span className="flex items-center gap-2">
          {live && !liveError ? (
            <span className="inline-flex items-center gap-1.5 rounded-full border border-pf-border bg-pf-surface px-2.5 py-1 text-[11px] font-medium text-pf-text-muted">
              <span aria-hidden="true" className="size-1.5 rounded-full bg-pf-accent" />
              Live updates
            </span>
          ) : null}
          <span className="rounded-full border border-pf-border bg-pf-surface px-2.5 py-1 text-[11px] font-medium text-pf-text-secondary">
            {run.status}
          </span>
        </span>
      </div>
      {live && liveError ? (
        <div
          role="status"
          className="flex flex-wrap items-center gap-3 rounded-lg border border-pf-border bg-pf-surface px-4 py-2.5 text-[11px] text-pf-warning"
        >
          <span>Live updates paused. Refresh to check the current status.</span>
          <button
            type="button"
            onClick={requestDetailRefresh}
            className="h-7 rounded-md border border-pf-border-strong bg-pf-surface px-2.5 text-[10px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted"
          >
            Refresh
          </button>
        </div>
      ) : null}

      <section
        aria-labelledby="run-summary-heading"
        className="rounded-lg border border-pf-border bg-pf-surface"
      >
        <div className="border-b border-pf-border px-5 py-4">
          <h3 id="run-summary-heading" className="text-[13px] font-semibold">
            Run summary
          </h3>
          <dl className="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-4">
            <div>
              <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                PLAN VERSION
              </dt>
              <dd className="mt-0.5 text-[13px] tabular-nums">v{run.planVersion}</dd>
            </div>
            <div>
              <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                TARGET
              </dt>
              <dd className="mt-0.5 truncate font-mono text-[12px]" title={target.origin ?? ""}>
                {target.origin ?? "—"}
              </dd>
            </div>
            <div>
              <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                STARTED
              </dt>
              <dd className="mt-0.5 text-[12px]">{formatDateTime(run.startedAt)}</dd>
            </div>
            <div>
              <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                FINISHED
              </dt>
              <dd className="mt-0.5 text-[12px]">{formatDateTime(run.finishedAt)}</dd>
            </div>
          </dl>
          {scenario ? (
            <p className="mt-4 border-t border-pf-border pt-3 text-[11px] text-pf-text-secondary">
              Scenario <span className="font-medium">{scenario.name}</span> ·{" "}
              {scenario.virtualUsers !== null
                ? `${scenario.virtualUsers} virtual users`
                : `${scenario.arrivalRatePerSecond ?? "—"} arrivals/s`}{" "}
              · ramp {scenario.rampUpSeconds}s · duration {scenario.durationSeconds}s · max{" "}
              {scenario.maxRequests} requests at {scenario.maxRequestsPerSecond}/s
            </p>
          ) : null}
        </div>

        <div className="px-5 py-4">
          <dl className="grid grid-cols-3 gap-4 sm:grid-cols-7">
            <CountStat label="PASSED" value={counts.passed} />
            <CountStat label="FAILED" value={counts.failed} />
            <CountStat label="SKIPPED" value={counts.skipped} />
            <CountStat label="INCONCLUSIVE" value={counts.inconclusive} />
            <CountStat label="ERRORS" value={counts.error} />
            <CountStat label="CONFIRMED" value={counts.confirmed} />
            <CountStat label="SUSPECTED" value={counts.suspected} />
          </dl>
          {reportType === "load" ? (
            <dl className="mt-5 grid grid-cols-2 gap-4 border-t border-pf-border pt-4 sm:grid-cols-4">
              <CountStat label="REQUESTS" value={metrics.requestCount} />
              <CountStat label="ERROR RESPONSES" value={metrics.errorCount} />
              <div>
                <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                  P95 LATENCY (MS)
                </dt>
                <dd className="mt-0.5 text-[15px] font-semibold tabular-nums">
                  {metrics.latencyP95Ms.toFixed(0)}
                </dd>
              </div>
              <div>
                <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                  THROUGHPUT (RPS)
                </dt>
                <dd className="mt-0.5 text-[15px] font-semibold tabular-nums">
                  {metrics.throughputRps.toFixed(2)}
                </dd>
              </div>
            </dl>
          ) : null}
        </div>
      </section>

      <section
        aria-labelledby="report-heading"
        className="rounded-lg border border-pf-border bg-pf-surface"
      >
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-pf-border px-5 py-4">
          <h3 id="report-heading" className="text-[13px] font-semibold">
            Report
          </h3>
          <div className="flex items-center gap-2">
            {report?.latest ? (
              <>
                <a
                  href={runReportExportPath(runId, "json")}
                  className="h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium leading-8 text-pf-text-secondary transition-colors hover:border-pf-text-muted hover:text-pf-text"
                >
                  Export JSON
                </a>
                <a
                  href={runReportExportPath(runId, "markdown")}
                  className="h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium leading-8 text-pf-text-secondary transition-colors hover:border-pf-text-muted hover:text-pf-text"
                >
                  Export Markdown
                </a>
              </>
            ) : null}
            {canWrite ? (
              <button
                type="button"
                onClick={handleGenerate}
                disabled={reportBusy}
                className="h-8 rounded-md bg-pf-accent px-3 text-[11px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
              >
                {reportBusy ? "Generating..." : report ? "Generate new version" : "Generate report"}
              </button>
            ) : null}
          </div>
        </div>
        <div className="px-5 py-4">
          {reportError ? (
            <p role="alert" className="mb-3 text-[12px] text-pf-error">
              {reportError}
            </p>
          ) : null}
          {!report ? (
            <p className="text-[12px] text-pf-text-secondary">
              No report has been generated for this run. Reports are versioned snapshots and never
              replace stored results.
            </p>
          ) : (
            <>
              <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
                <div>
                  <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                    STATUS
                  </dt>
                  <dd className="mt-0.5 text-[13px] font-medium">{report.status}</dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                    CURRENT VERSION
                  </dt>
                  <dd className="mt-0.5 text-[13px] tabular-nums">{report.currentVersion}</dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                    GENERATED
                  </dt>
                  <dd className="mt-0.5 text-[12px]">{formatDateTime(report.updatedAt)}</dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                    VERSIONS
                  </dt>
                  <dd className="mt-0.5 text-[13px] tabular-nums">{report.versions.length}</dd>
                </div>
              </dl>

              {report.lastErrorCode ? (
                <p role="alert" className="mt-3 text-[12px] text-pf-error">
                  The last generation attempt failed ({report.lastErrorCode}); stored results were
                  preserved.
                </p>
              ) : null}

              {report.latest ? (
                <p className="mt-4 break-all border-t border-pf-border pt-3 font-mono text-[10px] text-pf-text-muted">
                  Hash {report.latest.contentHash}
                </p>
              ) : null}

              {claims.length > 0 ? (
                <div className="mt-4">
                  <h4 className="text-[12px] font-semibold">Claims</h4>
                  <ul className="mt-2 space-y-2">
                    {claims.map((claim, index) => {
                      const kind = String(claim.claimKind ?? "observed");
                      return (
                        <li
                          // biome-ignore lint/suspicious/noArrayIndexKey: claims are a generated snapshot without stable ids
                          key={index}
                          className="rounded-md border border-pf-border bg-pf-background px-3 py-2.5"
                        >
                          <p className="text-[10px] font-medium tracking-[0.06em] text-pf-text-muted">
                            {kind === "hypothesis" ? "HYPOTHESIS" : "OBSERVED"}
                          </p>
                          <p className="mt-1 text-[12px] leading-5 text-pf-text-secondary">
                            {String(claim.text ?? "")}
                          </p>
                          <p className="mt-1 text-[10px] text-pf-text-muted">
                            {kind === "hypothesis"
                              ? "Not traceable to a stored result; labeled a hypothesis."
                              : `${referenceCount(claim.references)} stored reference(s).`}
                          </p>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ) : null}
            </>
          )}
        </div>
      </section>

      <section
        aria-labelledby="results-heading"
        className="rounded-lg border border-pf-border bg-pf-surface"
      >
        <div className="flex items-center justify-between gap-3 border-b border-pf-border px-5 py-4">
          <h3 id="results-heading" className="text-[13px] font-semibold">
            Per-test results
          </h3>
          <span className="text-[11px] text-pf-text-muted">
            {results.length} {results.length === 1 ? "result" : "results"}
          </span>
        </div>
        {results.length === 0 ? (
          <p className="px-5 py-4 text-[12px] text-pf-text-muted">
            This run has no stored results yet.
          </p>
        ) : (
          <ul className="divide-y divide-pf-border">
            {results.map((result) => (
              <li key={result.id} className="px-5 py-4">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate font-mono text-[11px] text-pf-text-muted">
                      {result.catalogTestId}
                    </p>
                    <p className="mt-1 text-[13px] font-medium">{result.summary}</p>
                  </div>
                  <span className="shrink-0 rounded-full border border-pf-border bg-pf-surface-muted px-2 py-0.5 text-[11px] font-medium text-pf-text-secondary">
                    {OUTCOME_LABELS[result.outcome] ?? result.outcome}
                  </span>
                </div>
                <div className="mt-1.5">
                  <OutcomeLine result={result} />
                </div>
                <p className="mt-2 text-[11px] text-pf-text-muted">
                  {result.toolName ?? "unknown tool"}
                  {result.toolVersion ? ` ${result.toolVersion}` : ""} · {result.durationMs} ms ·{" "}
                  {formatDateTime(result.createdAt)}
                </p>
                {result.evidence.length > 0 ? (
                  <ul className="mt-2 space-y-1">
                    {result.evidence.map((item) => (
                      <li
                        key={item.id}
                        className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded border border-pf-border bg-pf-background px-2.5 py-1.5 text-[10px] text-pf-text-muted"
                      >
                        <span className="font-medium text-pf-text-secondary">{item.mimeType}</span>
                        <span>{formatBytes(item.sizeBytes)}</span>
                        <span className="font-mono">{item.sha256.slice(0, 12)}…</span>
                        <span>{item.redacted ? "redacted" : "unredacted"}</span>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section
        aria-labelledby="recommendations-heading"
        className="rounded-lg border border-pf-border bg-pf-surface"
      >
        <div className="flex items-center justify-between gap-3 border-b border-pf-border px-5 py-4">
          <h3 id="recommendations-heading" className="text-[13px] font-semibold">
            Recommendations
          </h3>
          <span className="text-[11px] text-pf-text-muted">
            {recommendations.length} {recommendations.length === 1 ? "item" : "items"}
          </span>
        </div>
        {recommendations.length === 0 ? (
          <p className="px-5 py-4 text-[12px] text-pf-text-muted">
            No recommendations have been generated. Generate a report to derive them from stored
            results.
          </p>
        ) : (
          <ul className="divide-y divide-pf-border">
            {recommendations.map((recommendation: RecommendationView) => (
              <li key={recommendation.id} className="px-5 py-4">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="rounded-full border border-pf-border bg-pf-surface-muted px-2 py-0.5 text-[10px] font-medium tracking-[0.04em] text-pf-text-secondary">
                    {(KIND_LABELS[recommendation.kind] ?? recommendation.kind).toUpperCase()}
                  </span>
                  <span className="text-[10px] text-pf-text-muted">
                    severity {recommendation.severity} · confidence{" "}
                    {formatPercent(recommendation.confidence)}
                  </span>
                </div>
                <p className="mt-1.5 text-[13px] font-medium">{recommendation.title}</p>
                <p className="mt-1 text-[12px] leading-5 text-pf-text-secondary">
                  {recommendation.body}
                </p>
                {recommendation.rationale ? (
                  <p className="mt-1 text-[11px] text-pf-text-muted">{recommendation.rationale}</p>
                ) : null}
                <p className="mt-1.5 font-mono text-[10px] text-pf-text-muted">
                  {recommendation.resultId
                    ? `result ${recommendation.resultId.slice(0, 8)}`
                    : recommendation.evidenceId
                      ? `evidence ${recommendation.evidenceId.slice(0, 8)}`
                      : recommendation.runMetricId
                        ? `metric ${recommendation.runMetricId.slice(0, 8)}`
                        : "no stored reference (hypothesis)"}
                </p>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section
        aria-labelledby="compare-heading"
        className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4"
      >
        <h3 id="compare-heading" className="text-[13px] font-semibold">
          Compare with another run
        </h3>
        <p className="mt-1 text-[12px] text-pf-text-secondary">
          Compare profile, target, scenario version, timestamps, and metric deltas.
        </p>
        <div className="mt-3 flex flex-wrap items-end gap-2">
          <div>
            <label htmlFor="compare-run" className="mb-1.5 block text-[11px] font-medium">
              Run
            </label>
            <select
              id="compare-run"
              value={compareId}
              onChange={(event) => setCompareId(event.target.value)}
              className="h-9 min-w-[220px] rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
            >
              <option value="">Select a run</option>
              {siblings.map((run) => (
                <option key={run.id} value={run.id}>
                  {run.name} · v{run.planVersion} · {formatDateTime(run.createdAt)}
                </option>
              ))}
            </select>
          </div>
          <button
            type="button"
            onClick={handleCompare}
            disabled={compareBusy || compareId === ""}
            className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-55"
          >
            {compareBusy ? "Comparing..." : "Compare"}
          </button>
        </div>
        {siblings.length === 0 ? (
          <p className="mt-2 text-[11px] text-pf-text-muted">
            This is the only run in the project so far.
          </p>
        ) : null}
        {compareError ? (
          <p role="alert" className="mt-3 text-[12px] text-pf-error">
            {compareError}
          </p>
        ) : null}
        {comparison ? (
          <div className="mt-4 overflow-x-auto border-t border-pf-border pt-4">
            <div className="grid min-w-[520px] grid-cols-2 gap-4">
              <ComparisonColumn side={comparison.left} />
              <ComparisonColumn side={comparison.right} />
            </div>
            <table className="mt-4 w-full min-w-[520px] border-collapse text-[12px]">
              <caption className="sr-only">Metric comparison between the two runs</caption>
              <thead>
                <tr className="text-left text-[10px] tracking-[0.06em] text-pf-text-muted">
                  <th scope="col" className="border-b border-pf-border py-2 font-medium">
                    METRIC
                  </th>
                  <th scope="col" className="border-b border-pf-border py-2 font-medium">
                    LEFT
                  </th>
                  <th scope="col" className="border-b border-pf-border py-2 font-medium">
                    RIGHT
                  </th>
                  <th scope="col" className="border-b border-pf-border py-2 font-medium">
                    DELTA
                  </th>
                </tr>
              </thead>
              <tbody className="tabular-nums">
                {[
                  [
                    "Requests",
                    comparison.left.metrics.requestCount,
                    comparison.right.metrics.requestCount,
                    comparison.delta.requestCount,
                  ],
                  [
                    "Errors",
                    comparison.left.metrics.errorCount,
                    comparison.right.metrics.errorCount,
                    comparison.delta.errorCount,
                  ],
                  [
                    "P95 latency (ms)",
                    comparison.left.metrics.latencyP95Ms.toFixed(0),
                    comparison.right.metrics.latencyP95Ms.toFixed(0),
                    comparison.delta.latencyP95Ms.toFixed(0),
                  ],
                  [
                    "Throughput (rps)",
                    comparison.left.metrics.throughputRps.toFixed(2),
                    comparison.right.metrics.throughputRps.toFixed(2),
                    comparison.delta.throughputRps.toFixed(2),
                  ],
                ].map(([label, left, right, delta]) => (
                  <tr key={String(label)} className="border-b border-pf-border last:border-b-0">
                    <th scope="row" className="py-2 text-left font-medium text-pf-text-secondary">
                      {label}
                    </th>
                    <td className="py-2">{left}</td>
                    <td className="py-2">{right}</td>
                    <td className="py-2">{delta}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </section>
    </div>
  );
}
