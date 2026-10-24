import type { Risk } from "@probe-forge/contracts";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ApiError,
  approvePlan,
  createRuns,
  getJobPlan,
  type Job,
  type JobPlan,
  listAuthorizations,
  listTargets,
  type PlanLoadScenario,
  type PlannedTest,
  type TargetAuthorization,
  updatePlanSelection,
} from "./api";

interface PlanReviewProps {
  readonly job: Job;
  readonly canEdit: boolean;
  readonly csrfToken: string;
  readonly onBack: () => void;
  readonly onSessionExpired: () => void;
}

const dateTimeFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

const RISK_LABELS: Record<Risk, string> = {
  low: "Low risk",
  medium: "Medium risk",
  high: "High risk",
  critical: "Critical risk",
};

const RISK_STYLES: Record<Risk, string> = {
  low: "border-[#cfe0d6] bg-[#eef6f1] text-pf-success",
  medium: "border-[#ead9bd] bg-[#fbf5e9] text-pf-warning",
  high: "border-[#e8c9c4] bg-[#fbf1ef] text-pf-error",
  critical: "border-[#e2b8b3] bg-[#f9e9e6] text-pf-error",
};

const GOAL_LABELS: Record<Job["goal"], string> = {
  vulnerability_testing: "Vulnerability testing",
  load_testing: "Load testing",
};

const STATUS_LABELS: Record<JobPlan["status"], string> = {
  draft: "Draft",
  proposed: "Proposed",
  approved: "Approved",
  superseded: "Superseded",
  rejected: "Rejected",
};

const JOB_STATUS_LABELS: Record<Job["status"], string> = {
  draft: "Draft",
  queued: "Queued",
  analyzing: "Analyzing",
  planning: "Planning",
  awaiting_approval: "Awaiting approval",
  failed: "Failed",
  cancelled: "Cancelled",
  expired: "Expired",
};

function formatDateTime(value: string): string {
  return dateTimeFormatter.format(new Date(value));
}

function errorMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function sameSelection(left: readonly string[], right: ReadonlySet<string>): boolean {
  if (left.length !== right.size) return false;
  return left.every((value) => right.has(value));
}

function isConflict(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.code === "plan_conflict" ||
      error.code === "plan_superseded" ||
      error.code === "plan_not_editable")
  );
}

function RiskBadge({ risk }: { readonly risk: Risk }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 text-[10px] font-medium ${RISK_STYLES[risk]}`}
    >
      {RISK_LABELS[risk]}
    </span>
  );
}

function StringList({
  items,
  empty,
}: {
  readonly items: readonly string[];
  readonly empty: string;
}) {
  if (items.length === 0) {
    return <p className="text-[12px] text-pf-text-muted">{empty}</p>;
  }
  return (
    <ul className="space-y-1">
      {items.map((item) => (
        <li key={item} className="flex gap-2 text-[12px] leading-5 text-pf-text-secondary">
          <span
            aria-hidden="true"
            className="mt-2 size-1 shrink-0 rounded-full bg-pf-border-strong"
          />
          <span className="min-w-0 break-words">{item}</span>
        </li>
      ))}
    </ul>
  );
}

function TestCard({
  test,
  editable,
  checked,
  onChange,
}: {
  readonly test: PlannedTest;
  readonly editable: boolean;
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
}) {
  const inputId = `plan-test-${test.id}`;
  const catalog = test.catalog;
  return (
    <li
      className={`border-b border-pf-border px-4 py-4 last:border-b-0 ${checked ? "bg-pf-accent-soft/30" : ""}`}
    >
      <div className="flex items-start gap-3">
        <input
          id={inputId}
          type="checkbox"
          checked={checked}
          disabled={!editable}
          onChange={(event) => onChange(event.target.checked)}
          className="mt-1 size-4 shrink-0 rounded border-pf-border-strong text-pf-accent accent-pf-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-pf-accent disabled:opacity-50"
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <label htmlFor={inputId} className="min-w-0 text-[13px] font-medium">
              {test.name}
            </label>
            <RiskBadge risk={test.risk} />
            {test.requiresExplicitConfirmation ? (
              <span className="rounded-full border border-pf-border-strong px-2 py-0.5 text-[10px] font-medium text-pf-text-secondary">
                Explicit confirmation
              </span>
            ) : null}
          </div>
          <p className="mt-1 font-mono text-[10px] text-pf-text-muted">{test.catalogTestId}</p>
          <p className="mt-2 text-[12px] leading-5 text-pf-text-secondary">{test.rationale}</p>

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <div>
              <p className="text-[10px] font-medium text-pf-text-muted">Prerequisites</p>
              <div className="mt-1">
                <StringList items={test.prerequisites} empty="No prerequisites listed." />
              </div>
            </div>
            <div>
              <p className="text-[10px] font-medium text-pf-text-muted">Target requirements</p>
              <div className="mt-1">
                <StringList
                  items={test.targetRequirements}
                  empty="No target requirements listed."
                />
              </div>
            </div>
          </div>

          <div className="mt-3">
            <p className="text-[10px] font-medium text-pf-text-muted">Expected evidence</p>
            <p className="mt-1 text-[12px] leading-5 text-pf-text-secondary">
              {test.expectedEvidence}
            </p>
          </div>

          {catalog ? (
            <div className="mt-3 rounded-md border border-pf-border bg-pf-surface-muted/60 px-3 py-2.5">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-[11px] font-medium">{catalog.name}</span>
                <span className="font-mono text-[10px] text-pf-text-muted">
                  {catalog.category} · {catalog.timeoutSeconds}s
                </span>
                {catalog.available ? (
                  <span className="rounded-full border border-[#cfe0d6] bg-[#eef6f1] px-2 py-0.5 text-[10px] font-medium text-pf-success">
                    Adapter available
                  </span>
                ) : (
                  <span className="rounded-full border border-[#ead9bd] bg-[#fbf5e9] px-2 py-0.5 text-[10px] font-medium text-pf-warning">
                    Adapter unavailable
                  </span>
                )}
              </div>
              <p className="mt-1.5 text-[11px] leading-4 text-pf-text-secondary">
                {catalog.description}
              </p>
              {!catalog.available && catalog.availabilityNote ? (
                <p className="mt-1.5 text-[11px] leading-4 text-pf-warning">
                  {catalog.availabilityNote}
                </p>
              ) : null}
              <div className="mt-2 grid gap-2 sm:grid-cols-2">
                <div>
                  <p className="text-[10px] font-medium text-pf-text-muted">Tool requirements</p>
                  <p className="mt-0.5 break-all font-mono text-[10px] text-pf-text-secondary">
                    {catalog.toolRequirements.join(", ") || "None listed"}
                  </p>
                </div>
                <div>
                  <p className="text-[10px] font-medium text-pf-text-muted">
                    Evidence expectations
                  </p>
                  <p className="mt-0.5 text-[10px] leading-4 text-pf-text-secondary">
                    {catalog.evidenceExpectations.join(" · ")}
                  </p>
                </div>
              </div>
            </div>
          ) : (
            <p className="mt-3 rounded-md border border-[#e8c9c4] bg-[#fbf1ef] px-3 py-2 text-[11px] text-pf-error">
              This catalog entry is no longer available. Approval will fail closed until the catalog
              is updated.
            </p>
          )}

          <details className="mt-2">
            <summary className="cursor-pointer text-[11px] font-medium text-pf-text-secondary">
              Review configuration
            </summary>
            <pre className="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md border border-pf-border bg-pf-surface-muted px-3 py-2 font-mono text-[10px] leading-4 text-pf-text-secondary">
              {JSON.stringify(test.configuration, null, 2)}
            </pre>
            {test.rationaleReferences.length > 0 ? (
              <p className="mt-1.5 break-all font-mono text-[10px] text-pf-text-muted">
                References: {test.rationaleReferences.join(", ")}
              </p>
            ) : null}
          </details>
        </div>
      </div>
    </li>
  );
}

function LoadScenarioCard({
  scenario,
  planVersion,
  editable,
  checked,
  onChange,
}: {
  readonly scenario: PlanLoadScenario;
  readonly planVersion: number;
  readonly editable: boolean;
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
}) {
  const inputId = `plan-scenario-${scenario.id}`;
  const profile = scenario.profile;
  const sizing =
    profile.virtualUsers === undefined
      ? `${profile.arrivalRatePerSecond ?? 0} arrivals/s`
      : `${profile.virtualUsers} virtual users`;
  const stopConditions = profile.stopConditions ?? {};
  const stopConditionText = [
    stopConditions.errorRate === undefined ? null : `error rate ${stopConditions.errorRate}`,
    stopConditions.p95LatencyMs === undefined ? null : `p95 ${stopConditions.p95LatencyMs}ms`,
    stopConditions.maxFailedRequests === undefined
      ? null
      : `failed requests ${stopConditions.maxFailedRequests}`,
  ]
    .filter((value) => value !== null)
    .join(" · ");
  return (
    <li
      className={`border-b border-pf-border px-4 py-4 last:border-b-0 ${checked ? "bg-pf-accent-soft/30" : ""}`}
    >
      <div className="flex items-start gap-3">
        <input
          id={inputId}
          type="checkbox"
          checked={checked}
          disabled={!editable}
          onChange={(event) => onChange(event.target.checked)}
          className="mt-1 size-4 shrink-0 rounded border-pf-border-strong text-pf-accent accent-pf-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-pf-accent disabled:opacity-50"
        />
        <div className="min-w-0 flex-1">
          <label htmlFor={inputId} className="text-[13px] font-medium">
            {scenario.name}
          </label>
          <p className="mt-1 text-[12px] leading-5 text-pf-text-secondary">
            {scenario.description}
          </p>
          <p className="mt-1 text-[10px] text-pf-text-muted">
            Approval pins this scenario to plan v{planVersion}. No load traffic is sent by approval.
          </p>
          <ul className="mt-2 space-y-2">
            {scenario.requests.map((request, index) => (
              <li
                key={`${scenario.id}-${request.id ?? index}-${request.method}-${request.path}`}
                className="rounded border border-pf-border bg-pf-surface-muted/40 px-2 py-1.5"
              >
                <div className="flex flex-wrap items-center gap-2 font-mono text-[11px]">
                  {request.id ? <span className="text-pf-text-muted">{request.id}</span> : null}
                  <span className="rounded border border-pf-border bg-pf-surface px-1.5 py-0.5 font-medium">
                    {request.method}
                  </span>
                  <span className="break-all">{request.path}</span>
                  {request.method !== "GET" &&
                  request.method !== "HEAD" &&
                  request.method !== "OPTIONS" ? (
                    <span className="text-pf-warning">state-changing method</span>
                  ) : null}
                  {request.expectedStatuses.length > 0 ? (
                    <span className="text-pf-text-muted">
                      expected {request.expectedStatuses.join(", ")}
                    </span>
                  ) : null}
                  {request.dependsOn && request.dependsOn.length > 0 ? (
                    <span className="text-pf-text-muted">after {request.dependsOn.join(", ")}</span>
                  ) : null}
                </div>
                {Object.keys(request.secretHeaders ?? {}).length > 0 ? (
                  <p className="mt-1 text-[10px] text-pf-text-muted">
                    Secret-backed headers: {Object.keys(request.secretHeaders ?? {}).join(", ")}
                  </p>
                ) : null}
                {request.extract && request.extract.length > 0 ? (
                  <p className="mt-1 text-[10px] text-pf-text-muted">
                    Extracts:{" "}
                    {request.extract
                      .map(
                        (extraction) =>
                          `${extraction.name} from ${extraction.source}:${extraction.selector}`,
                      )
                      .join(" · ")}
                  </p>
                ) : null}
                {request.assertions && request.assertions.length > 0 ? (
                  <p className="mt-1 text-[10px] text-pf-text-muted">
                    Asserts:{" "}
                    {request.assertions
                      .map(
                        (assertion) =>
                          `${assertion.source}:${assertion.selector} ${assertion.operator}`,
                      )
                      .join(" · ")}
                  </p>
                ) : null}
                {request.bodyTemplate !== undefined ? (
                  <p className="mt-1 text-[10px] text-pf-text-muted">Body template present</p>
                ) : null}
              </li>
            ))}
          </ul>
          <dl className="mt-3 grid grid-cols-2 gap-2 border-t border-pf-border pt-2 text-[11px] sm:grid-cols-3">
            <div>
              <dt className="text-[10px] text-pf-text-muted">Sizing</dt>
              <dd className="mt-0.5">{sizing}</dd>
            </div>
            <div>
              <dt className="text-[10px] text-pf-text-muted">Ramp</dt>
              <dd className="mt-0.5">{profile.rampUpSeconds}s</dd>
            </div>
            <div>
              <dt className="text-[10px] text-pf-text-muted">Duration</dt>
              <dd className="mt-0.5">{profile.durationSeconds}s</dd>
            </div>
            <div>
              <dt className="text-[10px] text-pf-text-muted">Max requests</dt>
              <dd className="mt-0.5">{profile.maxRequests}</dd>
            </div>
            <div>
              <dt className="text-[10px] text-pf-text-muted">Request rate cap</dt>
              <dd className="mt-0.5">{profile.maxRequestsPerSecond}/s</dd>
            </div>
            <div>
              <dt className="text-[10px] text-pf-text-muted">Credential reference</dt>
              <dd className="mt-0.5 break-all font-mono text-[10px]">
                {scenario.credentialRef ?? "None"}
              </dd>
            </div>
            <div className="sm:col-span-3">
              <dt className="text-[10px] text-pf-text-muted">Stop conditions</dt>
              <dd className="mt-0.5">{stopConditionText || "No stop conditions"}</dd>
            </div>
          </dl>
        </div>
      </div>
    </li>
  );
}

export function PlanReview({ job, canEdit, csrfToken, onBack, onSessionExpired }: PlanReviewProps) {
  const [plan, setPlan] = useState<JobPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedTestIds, setSelectedTestIds] = useState<string[]>([]);
  const [selectedScenarioIds, setSelectedScenarioIds] = useState<string[]>([]);
  const [confirmed, setConfirmed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [approving, setApproving] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const [authorizations, setAuthorizations] = useState<TargetAuthorization[]>([]);
  const [authorizationId, setAuthorizationId] = useState("");
  const [queuing, setQueuing] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [runNotice, setRunNotice] = useState<string | null>(null);

  const handleError = useCallback(
    (error: unknown): string | null => {
      if (error instanceof ApiError && error.status === 401) {
        onSessionExpired();
        return null;
      }
      return errorMessage(error);
    },
    [onSessionExpired],
  );

  useEffect(() => {
    void reloadKey;
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setActionError(null);
    setConflict(false);
    setNotice(null);
    setConfirmed(false);
    getJobPlan(job.id)
      .then((next) => {
        if (cancelled) return;
        setPlan(next);
        setSelectedTestIds(
          next.plannedTests.filter((test) => test.selected).map((test) => test.id),
        );
        setSelectedScenarioIds(
          next.loadScenarios.filter((scenario) => scenario.selected).map((scenario) => scenario.id),
        );
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        const message = handleError(error);
        if (message) setLoadError(message);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [job.id, reloadKey, handleError]);

  const editable =
    canEdit &&
    plan !== null &&
    (plan.status === "proposed" || plan.status === "draft") &&
    plan.approval === null;

  const approvedPlanId = plan?.approval !== null && plan !== null ? plan.id : null;
  const approvedProjectId = plan !== null && plan.approval !== null ? plan.projectId : null;

  useEffect(() => {
    if (approvedPlanId === null || approvedProjectId === null) {
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const targets = await listTargets(approvedProjectId);
        const lists = await Promise.all(targets.map((target) => listAuthorizations(target.id)));
        const active = lists.flat().filter((authorization) => authorization.active);
        if (!cancelled) {
          setAuthorizations(active);
          setAuthorizationId(active[0]?.id ?? "");
        }
      } catch (error) {
        if (!cancelled && error instanceof ApiError && error.status === 401) {
          onSessionExpired();
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [approvedPlanId, approvedProjectId, onSessionExpired]);
  const savedTestIds = useMemo(
    () => plan?.plannedTests.filter((test) => test.selected).map((test) => test.id) ?? [],
    [plan],
  );
  const savedScenarioIds = useMemo(
    () =>
      plan?.loadScenarios.filter((scenario) => scenario.selected).map((scenario) => scenario.id) ??
      [],
    [plan],
  );
  const dirty =
    !sameSelection(savedTestIds, new Set(selectedTestIds)) ||
    !sameSelection(savedScenarioIds, new Set(selectedScenarioIds));
  const selectionCount = selectedTestIds.length + selectedScenarioIds.length;

  function toggleTest(testId: string, checked: boolean) {
    setSelectedTestIds((current) =>
      checked ? [...new Set([...current, testId])] : current.filter((id) => id !== testId),
    );
    setConfirmed(false);
    setNotice(null);
    setActionError(null);
    setConflict(false);
  }

  function toggleScenario(scenarioId: string, checked: boolean) {
    setSelectedScenarioIds((current) =>
      checked ? [...new Set([...current, scenarioId])] : current.filter((id) => id !== scenarioId),
    );
    setConfirmed(false);
    setNotice(null);
    setActionError(null);
    setConflict(false);
  }

  async function handleSave() {
    if (!plan) return;
    setSaving(true);
    setActionError(null);
    setConflict(false);
    setNotice(null);
    try {
      const updated = await updatePlanSelection(
        plan.id,
        plan.revision,
        selectedTestIds,
        selectedScenarioIds,
        csrfToken,
      );
      setPlan(updated);
      setSelectedTestIds(
        updated.plannedTests.filter((test) => test.selected).map((test) => test.id),
      );
      setSelectedScenarioIds(
        updated.loadScenarios
          .filter((scenario) => scenario.selected)
          .map((scenario) => scenario.id),
      );
      setConfirmed(false);
      setNotice(`Selection saved at revision ${updated.revision}.`);
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setActionError(message);
        setConflict(isConflict(error));
      }
    } finally {
      setSaving(false);
    }
  }

  async function handleApprove() {
    if (!plan || dirty || selectionCount === 0 || !confirmed) return;
    setApproving(true);
    setActionError(null);
    setConflict(false);
    setNotice(null);
    try {
      const updated = await approvePlan(plan.id, plan.revision, csrfToken);
      setPlan(updated);
      setConfirmed(false);
      setNotice("Approval recorded. The selected test set is now an immutable snapshot.");
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setActionError(message);
        setConflict(isConflict(error));
      }
    } finally {
      setApproving(false);
    }
  }

  async function handleQueueRuns() {
    if (!plan || authorizationId === "") return;
    setQueuing(true);
    setRunError(null);
    setRunNotice(null);
    try {
      const runs = await createRuns(plan.id, authorizationId, csrfToken);
      setRunNotice(
        `Queued ${runs.length} ${runs.length === 1 ? "run" : "runs"}. Open them from the project's Runs list.`,
      );
    } catch (error) {
      const text = handleError(error);
      if (text) {
        setRunError(text);
      }
    } finally {
      setQueuing(false);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <button
            type="button"
            onClick={onBack}
            className="inline-flex h-8 items-center rounded-md px-2 text-[12px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
          >
            ← Back to project
          </button>
          <p className="mt-4 font-mono text-[10px] font-medium tracking-[0.14em] text-pf-text-muted">
            PLAN REVIEW
          </p>
          <h2 className="mt-1 break-words text-[24px] font-semibold leading-tight tracking-[-0.02em]">
            {job.displayName}
          </h2>
          <p className="mt-1.5 text-[12px] text-pf-text-secondary">
            {GOAL_LABELS[job.goal]} · Review every proposed test and its constraints before
            approving a snapshot.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="rounded-full border border-pf-border bg-pf-surface px-2.5 py-1 text-[11px] font-medium text-pf-text-secondary">
            Job {JOB_STATUS_LABELS[job.status]}
          </span>
          {plan ? (
            <span className="rounded-full border border-pf-border-strong bg-pf-surface-muted px-2.5 py-1 text-[11px] font-medium">
              {STATUS_LABELS[plan.status]} · v{plan.version} · revision {plan.revision}
            </span>
          ) : null}
        </div>
      </div>

      {loading ? (
        <div
          role="status"
          className="rounded-lg border border-pf-border bg-pf-surface px-4 py-12 text-center text-[13px] text-pf-text-secondary"
        >
          Loading plan...
        </div>
      ) : loadError ? (
        <div className="rounded-lg border border-[#e8c9c4] bg-[#fbf1ef] px-4 py-5">
          <p role="alert" className="text-[13px] text-pf-error">
            {loadError}
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => setReloadKey((key) => key + 1)}
              className="h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
            >
              Retry
            </button>
            <button
              type="button"
              onClick={onBack}
              className="h-8 rounded-md px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
            >
              Back to project
            </button>
          </div>
        </div>
      ) : plan ? (
        <div className="grid gap-6 xl:grid-cols-[minmax(0,1.4fr)_minmax(330px,0.6fr)] xl:items-start">
          <div className="min-w-0 space-y-6">
            <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <h3 className="text-[13px] font-semibold">Plan summary</h3>
                  <p className="mt-1.5 max-w-2xl text-[12px] leading-5 text-pf-text-secondary">
                    {plan.summary}
                  </p>
                </div>
                <span className="rounded-full border border-pf-border bg-pf-surface-muted px-2 py-0.5 text-[10px] font-medium text-pf-text-secondary">
                  {plan.createdByAgent ? "Agent-generated" : "Deterministic fallback"}
                </span>
              </div>
              <dl className="mt-4 grid grid-cols-2 gap-3 border-t border-pf-border pt-3 sm:grid-cols-4">
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Confidence</dt>
                  <dd className="mt-0.5 text-[12px] tabular-nums">
                    {plan.confidence.toFixed(2)} / 1.00
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Catalog</dt>
                  <dd className="mt-0.5 font-mono text-[11px]">{plan.catalogVersion}</dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Provider</dt>
                  <dd className="mt-0.5 break-all font-mono text-[10px]">
                    {plan.agentProvider ?? "none"} / {plan.agentModel ?? "none"}
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Updated</dt>
                  <dd className="mt-0.5 text-[11px]">{formatDateTime(plan.updatedAt)}</dd>
                </div>
              </dl>
              <div className="mt-3">
                <div
                  role="progressbar"
                  aria-label={`Plan confidence ${plan.confidence.toFixed(2)} out of 1.00`}
                  aria-valuemin={0}
                  aria-valuemax={1}
                  aria-valuenow={plan.confidence}
                  className="h-1.5 w-full overflow-hidden rounded-full bg-pf-surface-muted"
                >
                  <div
                    className="h-full rounded-full bg-pf-accent"
                    style={{ width: `${Math.round(plan.confidence * 100)}%` }}
                  />
                </div>
              </div>
            </section>

            <section className="grid gap-4 md:grid-cols-2">
              <div className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
                <h3 className="text-[13px] font-semibold">Required inputs</h3>
                <div className="mt-2">
                  <StringList
                    items={plan.requiredInputs}
                    empty="No additional inputs are recorded."
                  />
                </div>
              </div>
              <div className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
                <h3 className="text-[13px] font-semibold">Limitations</h3>
                <div className="mt-2">
                  <StringList items={plan.limitations} empty="No limitations are recorded." />
                </div>
              </div>
            </section>

            <fieldset className="min-w-0 rounded-lg border border-pf-border bg-pf-surface">
              <legend className="px-4 pt-4 text-[13px] font-semibold">
                Proposed tests{" "}
                <span className="font-normal text-pf-text-muted">({plan.plannedTests.length})</span>
              </legend>
              {plan.plannedTests.length === 0 ? (
                <p className="px-4 py-8 text-center text-[12px] text-pf-text-secondary">
                  This plan does not contain vulnerability tests.
                </p>
              ) : (
                <ul className="mt-2">
                  {plan.plannedTests.map((test) => (
                    <TestCard
                      key={test.id}
                      test={test}
                      editable={editable}
                      checked={selectedTestIds.includes(test.id)}
                      onChange={(checked) => toggleTest(test.id, checked)}
                    />
                  ))}
                </ul>
              )}
            </fieldset>

            <fieldset className="min-w-0 rounded-lg border border-pf-border bg-pf-surface">
              <legend className="px-4 pt-4 text-[13px] font-semibold">
                Load scenarios{" "}
                <span className="font-normal text-pf-text-muted">
                  ({plan.loadScenarios.length})
                </span>
              </legend>
              {plan.loadScenarios.length === 0 ? (
                <p className="px-4 py-8 text-center text-[12px] text-pf-text-secondary">
                  This plan does not contain load scenarios.
                </p>
              ) : (
                <ul className="mt-2">
                  {plan.loadScenarios.map((scenario) => (
                    <LoadScenarioCard
                      key={scenario.id}
                      scenario={scenario}
                      planVersion={plan.version}
                      editable={editable}
                      checked={selectedScenarioIds.includes(scenario.id)}
                      onChange={(checked) => toggleScenario(scenario.id, checked)}
                    />
                  ))}
                </ul>
              )}
            </fieldset>

            {plan.evidenceReferences.length > 0 ? (
              <section className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
                <h3 className="text-[13px] font-semibold">Evidence references</h3>
                <p className="mt-2 break-all font-mono text-[10px] leading-5 text-pf-text-secondary">
                  {plan.evidenceReferences.join(" · ")}
                </p>
              </section>
            ) : null}
          </div>

          <aside className="space-y-4 xl:sticky xl:top-8">
            <section className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-[13px] font-semibold">Review and approval</h3>
                {plan.approval ? (
                  <span className="rounded-full border border-[#cfe0d6] bg-[#eef6f1] px-2 py-0.5 text-[10px] font-medium text-pf-success">
                    Approved
                  </span>
                ) : (
                  <span className="rounded-full border border-pf-border bg-pf-surface-muted px-2 py-0.5 text-[10px] font-medium text-pf-text-secondary">
                    Awaiting decision
                  </span>
                )}
              </div>

              <dl className="mt-3 grid grid-cols-2 gap-3">
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Tests selected</dt>
                  <dd className="mt-0.5 text-[18px] font-medium tabular-nums">
                    {selectedTestIds.length}
                    <span className="text-[11px] font-normal text-pf-text-muted">
                      /{plan.plannedTests.length}
                    </span>
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] font-medium text-pf-text-muted">Scenarios selected</dt>
                  <dd className="mt-0.5 text-[18px] font-medium tabular-nums">
                    {selectedScenarioIds.length}
                    <span className="text-[11px] font-normal text-pf-text-muted">
                      /{plan.loadScenarios.length}
                    </span>
                  </dd>
                </div>
              </dl>

              {plan.approval ? (
                <div className="mt-4 rounded-md border border-pf-border bg-pf-surface-muted/60 px-3 py-3">
                  <p className="text-[11px] font-medium">Immutable approval recorded</p>
                  <p className="mt-1 text-[11px] leading-4 text-pf-text-secondary">
                    Approved by {plan.approval.approvedByName} on{" "}
                    {formatDateTime(plan.approval.approvedAt)} at plan revision{" "}
                    {plan.approval.planRevision}.
                  </p>
                  <p className="mt-1 break-all text-[10px] text-pf-text-muted">
                    Test IDs: {plan.approval.approvedTestIds.join(", ") || "none"}
                    <br />
                    Scenario IDs: {plan.approval.approvedLoadScenarioIds.join(", ") || "none"}
                  </p>
                </div>
              ) : !canEdit ? (
                <p className="mt-4 rounded-md border border-pf-border bg-pf-surface-muted px-3 py-2.5 text-[11px] leading-4 text-pf-text-secondary">
                  You can review this plan, but your project role does not permit changing the
                  selection or approving it.
                </p>
              ) : (
                <>
                  <div className="mt-4 border-t border-pf-border pt-3">
                    <button
                      type="button"
                      onClick={() => void handleSave()}
                      disabled={!editable || !dirty || saving || approving}
                      className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-50"
                    >
                      {saving ? "Saving selection..." : "Save selection"}
                    </button>
                  </div>

                  <label className="mt-4 flex items-start gap-2.5 text-[11px] leading-4 text-pf-text-secondary">
                    <input
                      type="checkbox"
                      checked={confirmed}
                      disabled={!editable || selectionCount === 0 || dirty}
                      onChange={(event) => setConfirmed(event.target.checked)}
                      className="mt-0.5 size-4 shrink-0 rounded border-pf-border-strong text-pf-accent accent-pf-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-pf-accent disabled:opacity-50"
                    />
                    <span>
                      I reviewed the selected tests, their prerequisites, risks, and configuration.
                      I confirm this approval.
                    </span>
                  </label>

                  <button
                    type="button"
                    onClick={() => void handleApprove()}
                    disabled={
                      !editable ||
                      dirty ||
                      selectionCount === 0 ||
                      !confirmed ||
                      approving ||
                      saving
                    }
                    className="mt-3 h-10 w-full rounded-md bg-pf-accent px-3 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-50"
                  >
                    {approving ? "Approving..." : "Approve selected plan"}
                  </button>
                  <p className="mt-2 text-[10px] leading-4 text-pf-text-muted">
                    Approval creates an immutable snapshot. It does not start a target scan or load
                    test in this phase.
                  </p>
                </>
              )}

              {dirty && editable ? (
                <p role="status" className="mt-2 text-[11px] text-pf-warning">
                  Save the selection before approving.
                </p>
              ) : null}
              {actionError ? (
                <div className="mt-3" aria-live="polite">
                  <p role="alert" className="text-[11px] leading-4 text-pf-error">
                    {actionError}
                  </p>
                  {conflict ? (
                    <button
                      type="button"
                      onClick={() => setReloadKey((key) => key + 1)}
                      className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted"
                    >
                      Reload latest plan
                    </button>
                  ) : null}
                </div>
              ) : null}
              {notice ? (
                <p role="status" className="mt-3 text-[11px] leading-4 text-pf-success">
                  {notice}
                </p>
              ) : null}
            </section>

            {plan.approval ? (
              <section className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
                <h3 className="text-[13px] font-semibold">Approval record</h3>
                <p className="mt-2 text-[11px] leading-4 text-pf-text-secondary">
                  {plan.approval.approvedByName} · {plan.approval.approvedByEmail}
                </p>
                <p className="mt-0.5 text-[10px] text-pf-text-muted">
                  {formatDateTime(plan.approval.approvedAt)}
                </p>
                <p className="mt-2 break-all font-mono text-[10px] text-pf-text-muted">
                  Approval {plan.approval.id}
                </p>
                <p className="mt-2 text-[10px] leading-4 text-pf-text-muted">
                  The approval snapshot is immutable and preserves the exact selected tests,
                  scenarios, risks, and catalog version.
                </p>
              </section>
            ) : null}

            {plan.approval ? (
              <section className="rounded-lg border border-pf-border bg-pf-surface px-4 py-4">
                <h3 className="text-[13px] font-semibold">Queue runs</h3>
                <p className="mt-1.5 text-[11px] leading-4 text-pf-text-secondary">
                  Create runs for the approved selection against an active target authorization.
                  Execution sends traffic only to the authorized scope.
                </p>
                {authorizations.length === 0 ? (
                  <p className="mt-3 rounded-md border border-pf-border bg-pf-surface-muted px-3 py-2.5 text-[11px] leading-4 text-pf-text-secondary">
                    No active target authorization. Authorize a target in the project before queuing
                    runs.
                  </p>
                ) : (
                  <div className="mt-3 space-y-2">
                    <label htmlFor="run-authorization" className="block text-[11px] font-medium">
                      Target authorization
                    </label>
                    <select
                      id="run-authorization"
                      value={authorizationId}
                      onChange={(event) => setAuthorizationId(event.target.value)}
                      className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
                    >
                      {authorizations.map((authorization) => (
                        <option key={authorization.id} value={authorization.id}>
                          {authorization.scope.origin} · {authorization.limits.maxRequests} req
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      onClick={() => void handleQueueRuns()}
                      disabled={queuing || !canEdit || authorizationId === ""}
                      className="h-9 w-full rounded-md bg-pf-accent px-3 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-50"
                    >
                      {queuing ? "Queuing runs..." : "Queue runs"}
                    </button>
                    {!canEdit ? (
                      <p className="text-[10px] leading-4 text-pf-text-muted">
                        Your role permits review only; a collaborator or above can queue runs.
                      </p>
                    ) : null}
                  </div>
                )}
                {runError ? (
                  <p role="alert" className="mt-2 text-[11px] leading-4 text-pf-error">
                    {runError}
                  </p>
                ) : null}
                {runNotice ? (
                  <p role="status" className="mt-2 text-[11px] leading-4 text-pf-success">
                    {runNotice}
                  </p>
                ) : null}
              </section>
            ) : null}
          </aside>
        </div>
      ) : (
        <div className="rounded-lg border border-dashed border-pf-border-strong bg-pf-surface px-6 py-12 text-center">
          <h3 className="text-[15px] font-semibold">No plan available</h3>
          <p className="mx-auto mt-1.5 max-w-md text-[12px] leading-5 text-pf-text-secondary">
            This job does not have a proposed plan yet. Return to the project and check again after
            analysis completes.
          </p>
          <button
            type="button"
            onClick={onBack}
            className="mt-4 h-9 rounded-md border border-pf-border-strong bg-pf-surface px-3.5 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
          >
            Back to project
          </button>
        </div>
      )}
    </div>
  );
}
