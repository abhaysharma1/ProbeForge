import type { JobEvent, RunEvent } from "@probe-forge/contracts";
import {
  type FormEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ApiError,
  type AuditEvent,
  addMember,
  assignableRoles,
  canManageMembers,
  canViewAudit,
  type Job,
  listAudit,
  listJobs,
  listMembers,
  listRuns,
  type Project,
  type ProjectMember,
  type ProjectRole,
  type RunSummaryEntry,
  rerunJob,
} from "./api";
import { isLiveJob, isLiveRun, jobStatusForEvent, runStatusForEvent } from "./jobEvents";
import { PlanReview } from "./PlanReview";
import { RepositoriesPanel } from "./RepositoriesPanel";
import { RunReport } from "./RunReport";
import {
  activeTabOf,
  jobPath,
  navigate,
  PROJECT_TABS,
  type ProjectTab,
  projectPath,
  type Route,
  RouteLink,
  runPath,
} from "./routing";
import { TargetsPanel } from "./TargetsPanel";
import { useJobEvents, useRunEvents } from "./useJobEvents";

type ProjectRoute = Extract<Route, { kind: "project" | "job" | "run" }>;

interface ProjectPageProps {
  readonly project: Project;
  readonly route: ProjectRoute;
  readonly csrfToken: string;
  readonly onProjectsChanged: () => Promise<void> | void;
  readonly onSessionExpired: () => void;
}

const ROLE_LABELS: Record<ProjectRole, string> = {
  owner: "Owner",
  admin: "Admin",
  collaborator: "Collaborator",
  viewer: "Viewer",
};

const TAB_LABELS: Record<ProjectTab, string> = {
  repositories: "Repositories",
  jobs: "Analysis jobs",
  runs: "Runs",
  targets: "Targets",
  members: "Members",
  activity: "Activity",
};

const ACTION_LABELS: Record<string, string> = {
  "auth.login_succeeded": "Signed in",
  "auth.logout": "Signed out",
  "auth.logout_all": "Signed out all sessions",
  "auth.sessions_revoked": "Revoked all sessions",
  "auth.session_revoked": "Revoked a session",
  "project.created": "Created project",
  "project.updated": "Updated project",
  "project.member_added": "Added member",
  "project.member_role_changed": "Changed member role",
  "project.member_removed": "Removed member",
  "project.owner_transferred": "Transferred ownership",
  "plan.selection_updated": "Updated plan selection",
  "plan.approved": "Approved plan",
  "target.created": "Registered target",
  "target.validated": "Validated target destination",
  "target.validation_failed": "Rejected target destination",
  "target.authorization_created": "Authorized target",
  "target.authorization_revoked": "Revoked target authorization",
  "repository.registered": "Added repository",
  "repository.removed": "Removed repository",
  "job.rerun_requested": "Re-ran analysis job",
};

const JOB_GOAL_LABELS: Record<Job["goal"], string> = {
  vulnerability_testing: "Vulnerability testing",
  load_testing: "Load testing",
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

const RUN_TYPE_LABELS: Record<RunSummaryEntry["runType"], string> = {
  vulnerability_testing: "Vulnerability testing",
  load_testing: "Load testing",
};

const RUN_STATUS_LABELS: Record<string, string> = {
  draft: "Draft",
  awaiting_target: "Awaiting target",
  scheduled: "Scheduled",
  queued: "Queued",
  running: "Running",
  analyzing_results: "Analyzing results",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  expired: "Expired",
  completed_with_report_error: "Completed with report error",
};

const dateTimeFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

function formatDateTime(value: string): string {
  return dateTimeFormatter.format(new Date(value));
}

function errorMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function SectionCard({ children }: { readonly children: ReactNode }) {
  return (
    <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
      {children}
    </section>
  );
}

function LiveBadge({ visible }: { readonly visible: boolean }) {
  if (!visible) {
    return null;
  }
  return (
    <span className="inline-flex items-center gap-1.5">
      <span aria-hidden="true" className="size-1.5 rounded-full bg-pf-accent" />
      Live updates
    </span>
  );
}

interface JobsSectionProps {
  readonly projectId: string;
  readonly jobs: readonly Job[];
  readonly loading: boolean;
  readonly error: string | null;
  readonly streamError: string | null;
  readonly actionError: string | null;
  readonly rerunningId: string | null;
  readonly canWrite: boolean;
  readonly liveCount: number;
  readonly onRerun: (job: Job) => void;
}

function JobsSection({
  projectId,
  jobs,
  loading,
  error,
  streamError,
  actionError,
  rerunningId,
  canWrite,
  liveCount,
  onRerun,
}: JobsSectionProps) {
  return (
    <SectionCard>
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-[15px] font-semibold tracking-[-0.01em]">Analysis jobs</h2>
        <span className="flex items-center gap-2 text-[11px] text-pf-text-muted">
          <LiveBadge visible={liveCount > 0 && !streamError} />
          <span>
            {jobs.length} {jobs.length === 1 ? "job" : "jobs"}
          </span>
        </span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        Each analysis reads one pinned commit and proposes a plan for review. Repository code is
        never executed.
      </p>
      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading jobs...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : jobs.length === 0 ? (
        <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
          No analysis jobs yet. Start one from the Repositories tab; its plan appears here for
          review.
        </p>
      ) : (
        <ul className="mt-3 divide-y divide-pf-border">
          {jobs.map((job) => {
            const hasPlan = job.currentPlanId !== null;
            const canRerun =
              job.status === "failed" || job.status === "cancelled" || job.status === "expired";
            const summary = (
              <>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] font-medium">{job.displayName}</span>
                  <span className="mt-0.5 block text-[10px] text-pf-text-muted">
                    {JOB_GOAL_LABELS[job.goal]} · {JOB_STATUS_LABELS[job.status]}
                  </span>
                </span>
                <span
                  className={`shrink-0 text-[10px] font-medium ${
                    hasPlan ? "text-pf-accent" : "text-pf-text-muted"
                  }`}
                >
                  {hasPlan ? "Review plan →" : "No plan yet"}
                </span>
              </>
            );
            return (
              <li key={job.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5">
                {hasPlan ? (
                  <RouteLink
                    to={jobPath(projectId, job.id)}
                    className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 rounded-md text-left transition-colors hover:bg-pf-surface-hover px-2"
                  >
                    {summary}
                  </RouteLink>
                ) : (
                  <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1">
                    {summary}
                  </div>
                )}
                {canRerun && canWrite ? (
                  <button
                    type="button"
                    onClick={() => onRerun(job)}
                    disabled={rerunningId === job.id}
                    className="shrink-0 rounded-md border border-pf-border-strong bg-pf-surface px-2.5 py-1 text-[10px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted disabled:opacity-55"
                  >
                    {rerunningId === job.id ? "Re-running..." : "Re-run"}
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      {actionError ? (
        <p role="alert" className="mt-2 text-[11px] text-pf-error">
          {actionError}
        </p>
      ) : null}
      {streamError ? (
        <p role="status" className="mt-2 text-[11px] text-pf-warning">
          {streamError}
        </p>
      ) : null}
    </SectionCard>
  );
}

interface RunsSectionProps {
  readonly projectId: string;
  readonly runs: readonly RunSummaryEntry[];
  readonly loading: boolean;
  readonly error: string | null;
  readonly streamError: string | null;
  readonly liveCount: number;
}

function RunsSection({
  projectId,
  runs,
  loading,
  error,
  streamError,
  liveCount,
}: RunsSectionProps) {
  return (
    <SectionCard>
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-[15px] font-semibold tracking-[-0.01em]">Runs</h2>
        <span className="flex items-center gap-2 text-[11px] text-pf-text-muted">
          <LiveBadge visible={liveCount > 0 && !streamError} />
          <span>
            {runs.length} {runs.length === 1 ? "run" : "runs"}
          </span>
        </span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        A run appears after an approved plan is scheduled or executed. Results and reports stay
        traceable to the approved plan version.
      </p>
      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading runs...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : runs.length === 0 ? (
        <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
          No runs yet. Approve a plan to make it available for execution.
        </p>
      ) : (
        <ul className="mt-3 divide-y divide-pf-border">
          {runs.map((run) => (
            <li key={run.id}>
              <RouteLink
                to={runPath(projectId, run.id)}
                className="flex w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-md py-2.5 text-left transition-colors hover:bg-pf-surface-hover px-3"
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] font-medium">
                    {RUN_TYPE_LABELS[run.runType] ?? run.runType}
                  </span>
                  <span className="mt-0.5 block truncate text-[10px] text-pf-text-muted">
                    {run.name ? run.name : null}
                  </span>
                  <span className="mt-0.5 block text-[10px] text-pf-text-muted">
                    {RUN_STATUS_LABELS[run.status] ?? run.status} · plan v{run.planVersion} ·{" "}
                    {formatDateTime(run.createdAt)}
                  </span>
                </span>
                <span className="shrink-0 text-[10px] font-medium text-pf-accent">
                  View results →
                </span>
              </RouteLink>
            </li>
          ))}
        </ul>
      )}
      {streamError ? (
        <p role="status" className="mt-2 text-[11px] text-pf-warning">
          {streamError}
        </p>
      ) : null}
    </SectionCard>
  );
}

interface MembersSectionProps {
  readonly role: ProjectRole;
  readonly members: readonly ProjectMember[];
  readonly loading: boolean;
  readonly error: string | null;
  readonly roles: readonly ProjectRole[];
  readonly memberManagement: boolean;
  readonly email: string;
  readonly effectiveRole: ProjectRole;
  readonly adding: boolean;
  readonly addError: string | null;
  readonly notice: string | null;
  readonly onEmailChange: (value: string) => void;
  readonly onRoleChange: (role: ProjectRole) => void;
  readonly onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}

function MembersSection({
  role,
  members,
  loading,
  error,
  roles,
  memberManagement,
  email,
  effectiveRole,
  adding,
  addError,
  notice,
  onEmailChange,
  onRoleChange,
  onSubmit,
}: MembersSectionProps) {
  return (
    <SectionCard>
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-[15px] font-semibold tracking-[-0.01em]">Members</h2>
        <span className="text-[11px] text-pf-text-muted">
          {members.length} {members.length === 1 ? "person" : "people"}
        </span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        Project roles are enforced by the server on every action; this page only hides controls your
        role cannot use.
      </p>

      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading members...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : (
        <ul className="mt-3 divide-y divide-pf-border">
          {members.map((member) => (
            <li key={member.userId} className="flex items-center justify-between gap-3 py-2.5">
              <div className="min-w-0">
                <p className="truncate text-[12px] font-medium">{member.displayName}</p>
                <p className="truncate text-[11px] text-pf-text-muted">{member.email}</p>
              </div>
              <span className="shrink-0 text-[11px] text-pf-text-secondary">
                {ROLE_LABELS[member.role]}
              </span>
            </li>
          ))}
        </ul>
      )}

      {memberManagement && roles.length > 0 ? (
        <form
          onSubmit={onSubmit}
          aria-label="Add project member"
          className="mt-4 border-t border-pf-border pt-4"
        >
          <p className="text-[11px] font-medium text-pf-text-secondary">Add an existing account</p>
          <div className="mt-2 grid gap-2 sm:grid-cols-[minmax(0,1fr)_112px]">
            <label className="sr-only" htmlFor="member-email">
              Member email
            </label>
            <input
              id="member-email"
              type="email"
              required
              value={email}
              onChange={(event) => onEmailChange(event.target.value)}
              placeholder="person@example.test"
              className="h-9 min-w-0 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
            />
            <label className="sr-only" htmlFor="member-role">
              Member role
            </label>
            <select
              id="member-role"
              value={effectiveRole}
              onChange={(event) => onRoleChange(event.target.value as ProjectRole)}
              className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
            >
              {roles.map((option) => (
                <option key={option} value={option}>
                  {ROLE_LABELS[option]}
                </option>
              ))}
            </select>
          </div>
          <button
            type="submit"
            disabled={adding || email.trim() === ""}
            className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-55"
          >
            {adding ? "Adding..." : "Add member"}
          </button>
          {addError ? (
            <p role="alert" className="mt-2 text-[11px] text-pf-error">
              {addError}
            </p>
          ) : null}
          {notice ? (
            <p role="status" className="mt-2 text-[11px] leading-4 text-pf-success">
              {notice}
            </p>
          ) : null}
        </form>
      ) : (
        <p className="mt-3 border-t border-pf-border pt-3 text-[11px] leading-4 text-pf-text-muted">
          {role === "viewer"
            ? "Viewers can read project data. Member management is restricted."
            : "Collaborators can work with project content. Member management is restricted."}
        </p>
      )}
    </SectionCard>
  );
}

interface ActivitySectionProps {
  readonly audit: readonly AuditEvent[] | null;
  readonly loading: boolean;
  readonly error: string | null;
}

function ActivitySection({ audit, loading, error }: ActivitySectionProps) {
  return (
    <SectionCard>
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-[15px] font-semibold tracking-[-0.01em]">Audit trail</h2>
        <span className="text-[11px] text-pf-text-muted">Latest 50</span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        Immutable records of project, approval, target, repository, and job actions.
      </p>
      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading audit events...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : audit && audit.length > 0 ? (
        <ol className="mt-3 space-y-3">
          {audit.map((event) => (
            <li key={event.id} className="flex gap-2.5">
              <span
                aria-hidden="true"
                className="mt-1.5 size-1.5 shrink-0 rounded-full bg-pf-border-strong"
              />
              <div className="min-w-0">
                <p className="text-[11px] font-medium">
                  {ACTION_LABELS[event.action] ?? event.action}
                </p>
                <p className="mt-0.5 text-[10px] text-pf-text-muted">
                  {formatDateTime(event.createdAt)}
                  {event.actorId ? (
                    <>
                      {" · "}
                      <span className="font-mono">{event.actorId.slice(0, 8)}</span>
                    </>
                  ) : null}
                </p>
              </div>
            </li>
          ))}
        </ol>
      ) : (
        <p className="mt-3 text-[11px] text-pf-text-muted">
          No audit events have been recorded for this project.
        </p>
      )}
    </SectionCard>
  );
}

function UnavailableNotice({
  title,
  detail,
  backTo,
  backLabel,
}: {
  readonly title: string;
  readonly detail: string;
  readonly backTo: string;
  readonly backLabel: string;
}) {
  return (
    <section className="mx-auto w-full max-w-5xl rounded-lg border border-pf-border bg-pf-surface px-6 py-10 text-center">
      <p className="text-[14px] font-medium">{title}</p>
      <p className="mx-auto mt-1.5 max-w-md text-[12px] leading-5 text-pf-text-secondary">
        {detail}
      </p>
      <RouteLink
        to={backTo}
        className="mt-4 inline-flex h-8 items-center rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted hover:text-pf-text"
      >
        {backLabel}
      </RouteLink>
    </section>
  );
}

export function ProjectPage({
  project,
  route,
  csrfToken,
  onProjectsChanged,
  onSessionExpired,
}: ProjectPageProps) {
  const [members, setMembers] = useState<ProjectMember[]>([]);
  const [membersLoading, setMembersLoading] = useState(false);
  const [membersError, setMembersError] = useState<string | null>(null);
  const [audit, setAudit] = useState<AuditEvent[] | null>(null);
  const [auditLoading, setAuditLoading] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);

  const [jobs, setJobs] = useState<Job[]>([]);
  const [jobsLoading, setJobsLoading] = useState(false);
  const [jobsError, setJobsError] = useState<string | null>(null);
  const [rerunningId, setRerunningId] = useState<string | null>(null);
  const [jobActionError, setJobActionError] = useState<string | null>(null);
  const [jobStreamError, setJobStreamError] = useState<string | null>(null);
  const [runStreamError, setRunStreamError] = useState<string | null>(null);

  const [runs, setRuns] = useState<RunSummaryEntry[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);

  const [memberEmail, setMemberEmail] = useState("");
  const [memberRole, setMemberRole] = useState<ProjectRole>("viewer");
  const [addingMember, setAddingMember] = useState(false);
  const [memberError, setMemberError] = useState<string | null>(null);
  const [memberNotice, setMemberNotice] = useState<string | null>(null);

  const detailGeneration = useRef(0);

  const handleError = useCallback(
    (error: unknown) => {
      if (error instanceof ApiError && error.status === 401) {
        onSessionExpired();
        return null;
      }
      return errorMessage(error);
    },
    [onSessionExpired],
  );

  const loadDetails = useCallback(
    async (projectId: string, role: ProjectRole) => {
      const generation = detailGeneration.current + 1;
      detailGeneration.current = generation;

      setMembersLoading(true);
      setMembersError(null);
      setMemberError(null);
      setMemberNotice(null);

      try {
        const next = await listMembers(projectId);
        if (generation === detailGeneration.current) {
          setMembers(next);
        }
      } catch (error) {
        if (generation !== detailGeneration.current) {
          return;
        }
        const message = handleError(error);
        if (!message) {
          return;
        }
        setMembersError(message);
      } finally {
        if (generation === detailGeneration.current) {
          setMembersLoading(false);
        }
      }

      setJobsLoading(true);
      setJobsError(null);
      try {
        const next = await listJobs(projectId);
        if (generation === detailGeneration.current) {
          setJobs(next);
        }
      } catch (error) {
        if (generation !== detailGeneration.current) {
          return;
        }
        const message = handleError(error);
        if (!message) {
          return;
        }
        setJobsError(message);
      } finally {
        if (generation === detailGeneration.current) {
          setJobsLoading(false);
        }
      }

      setRunsLoading(true);
      setRunsError(null);
      try {
        const next = await listRuns(projectId);
        if (generation === detailGeneration.current) {
          setRuns(next);
        }
      } catch (error) {
        if (generation !== detailGeneration.current) {
          return;
        }
        const message = handleError(error);
        if (!message) {
          return;
        }
        setRunsError(message);
      } finally {
        if (generation === detailGeneration.current) {
          setRunsLoading(false);
        }
      }

      if (canViewAudit(role)) {
        setAuditLoading(true);
        setAuditError(null);
        try {
          const next = await listAudit(projectId);
          if (generation === detailGeneration.current) {
            setAudit(next);
          }
        } catch (error) {
          if (generation !== detailGeneration.current) {
            return;
          }
          const message = handleError(error);
          if (!message) {
            return;
          }
          setAuditError(message);
        } finally {
          if (generation === detailGeneration.current) {
            setAuditLoading(false);
          }
        }
      } else if (generation === detailGeneration.current) {
        setAudit(null);
        setAuditLoading(false);
        setAuditError(null);
      }

      if (generation === detailGeneration.current) {
        setMemberRole((current) => {
          const roles = assignableRoles(role);
          return roles.includes(current) || roles.length === 0 ? current : (roles[0] ?? "viewer");
        });
      }
    },
    [handleError],
  );

  // Live job progress. PostgreSQL remains authoritative; the stream is only a
  // prompt to re-read the project's job list.
  const jobsRef = useRef<Job[]>([]);
  useEffect(() => {
    jobsRef.current = jobs;
  }, [jobs]);

  const activeJobIds = useMemo(
    () => jobs.filter((job) => isLiveJob(job.status)).map((job) => job.id),
    [jobs],
  );

  const refreshJobs = useCallback(async () => {
    const generation = detailGeneration.current;
    try {
      const next = await listJobs(project.id);
      if (generation === detailGeneration.current) {
        setJobs(next);
        setJobsError(null);
      }
    } catch (error) {
      if (generation !== detailGeneration.current) {
        return;
      }
      const message = handleError(error);
      if (message) {
        setJobsError(message);
      }
    }
  }, [project.id, handleError]);

  // Coalesce bursts of events into at most one in-flight refresh.
  const refreshState = useRef({ inFlight: false, pending: false });
  const requestJobsRefresh = useCallback(() => {
    const state = refreshState.current;
    if (state.inFlight) {
      state.pending = true;
      return;
    }
    state.inFlight = true;
    void refreshJobs().finally(() => {
      state.inFlight = false;
      if (state.pending) {
        state.pending = false;
        requestJobsRefresh();
      }
    });
  }, [refreshJobs]);

  const handleJobEvent = useCallback(
    (jobId: string, event: JobEvent) => {
      const nextStatus = jobStatusForEvent(event.type);
      if (nextStatus === null) {
        return;
      }
      const current = jobsRef.current.find((job) => job.id === jobId);
      if (current && current.status === nextStatus) {
        return;
      }
      setJobStreamError(null);
      requestJobsRefresh();
    },
    [requestJobsRefresh],
  );

  const handleJobStreamError = useCallback(() => {
    setJobStreamError("Live updates paused. Reopen the project to refresh status.");
  }, []);

  useJobEvents({
    jobIds: activeJobIds,
    enabled: true,
    onEvent: handleJobEvent,
    onStreamError: handleJobStreamError,
  });

  // Live run progress, with the same server-authoritative refresh model.
  const runsRef = useRef<RunSummaryEntry[]>([]);
  useEffect(() => {
    runsRef.current = runs;
  }, [runs]);

  const activeRunIds = useMemo(
    () => runs.filter((run) => isLiveRun(run.status)).map((run) => run.id),
    [runs],
  );

  const refreshRuns = useCallback(async () => {
    const generation = detailGeneration.current;
    try {
      const next = await listRuns(project.id);
      if (generation === detailGeneration.current) {
        setRuns(next);
        setRunsError(null);
      }
    } catch (error) {
      if (generation !== detailGeneration.current) {
        return;
      }
      const message = handleError(error);
      if (message) {
        setRunsError(message);
      }
    }
  }, [project.id, handleError]);

  const runRefreshState = useRef({ inFlight: false, pending: false });
  const requestRunsRefresh = useCallback(() => {
    const state = runRefreshState.current;
    if (state.inFlight) {
      state.pending = true;
      return;
    }
    state.inFlight = true;
    void refreshRuns().finally(() => {
      state.inFlight = false;
      if (state.pending) {
        state.pending = false;
        requestRunsRefresh();
      }
    });
  }, [refreshRuns]);

  const handleRunEvent = useCallback(
    (runId: string, event: RunEvent) => {
      const nextStatus = runStatusForEvent(event.type);
      if (nextStatus !== null) {
        const current = runsRef.current.find((run) => run.id === runId);
        if (current && current.status === nextStatus) {
          return;
        }
      }
      setRunStreamError(null);
      requestRunsRefresh();
    },
    [requestRunsRefresh],
  );

  const handleRunStreamError = useCallback(() => {
    setRunStreamError("Live run updates paused. Reopen the project to refresh results.");
  }, []);

  useRunEvents({
    runIds: activeRunIds,
    enabled: true,
    onEvent: handleRunEvent,
    onStreamError: handleRunStreamError,
  });

  useEffect(() => {
    void loadDetails(project.id, project.role);
  }, [project.id, project.role, loadDetails]);

  useEffect(
    () => () => {
      detailGeneration.current += 1;
    },
    [],
  );

  const roles = assignableRoles(project.role);
  const effectiveMemberRole = roles.includes(memberRole) ? memberRole : (roles[0] ?? "viewer");
  const memberManagement = canManageMembers(project.role);
  const canWrite = project.role !== "viewer";

  const activeTab = activeTabOf(route) ?? "repositories";

  const tabCounts: Partial<Record<ProjectTab, number>> = {};
  if (!jobsLoading) tabCounts.jobs = jobs.length;
  if (!runsLoading) tabCounts.runs = runs.length;
  if (!membersLoading) tabCounts.members = members.length;

  const visibleTabs = PROJECT_TABS.filter(
    (tab) => tab !== "activity" || canViewAudit(project.role),
  );

  async function handleAddMember(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setAddingMember(true);
    setMemberError(null);
    setMemberNotice(null);
    try {
      const member = await addMember(project.id, memberEmail, effectiveMemberRole, csrfToken);
      const notice = `${member.displayName} was added as ${ROLE_LABELS[member.role].toLowerCase()}.`;
      setMemberEmail("");
      await loadDetails(project.id, project.role);
      await onProjectsChanged();
      setMemberNotice(notice);
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setMemberError(message);
      }
    } finally {
      setAddingMember(false);
    }
  }

  async function handleRerunJob(job: Job) {
    setRerunningId(job.id);
    setJobActionError(null);
    try {
      await rerunJob(job.id, csrfToken);
      await loadDetails(project.id, project.role);
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setJobActionError(message);
      }
    } finally {
      setRerunningId(null);
    }
  }

  if (route.kind === "run") {
    return (
      <div className="mx-auto w-full max-w-5xl">
        <RunReport
          runId={route.runId}
          projectId={project.id}
          canWrite={canWrite}
          csrfToken={csrfToken}
          onBack={() => navigate(projectPath(project.id, "runs"))}
          onSessionExpired={onSessionExpired}
        />
      </div>
    );
  }

  if (route.kind === "job") {
    if (jobsLoading) {
      return (
        <div
          role="status"
          className="mx-auto w-full max-w-5xl rounded-lg border border-pf-border bg-pf-surface px-4 py-10 text-center text-[13px] text-pf-text-secondary"
        >
          Loading plan...
        </div>
      );
    }
    const activeJob = jobs.find((job) => job.id === route.jobId) ?? null;
    if (activeJob === null) {
      return (
        <UnavailableNotice
          title="This analysis job is not available."
          detail={
            jobsError ??
            "It may have been removed, or your role no longer has access to this project."
          }
          backTo={projectPath(project.id, "jobs")}
          backLabel="Back to analysis jobs"
        />
      );
    }
    return (
      <div className="mx-auto w-full max-w-5xl">
        <PlanReview
          job={activeJob}
          canEdit={canWrite}
          csrfToken={csrfToken}
          onBack={() => navigate(projectPath(project.id, "jobs"))}
          onSessionExpired={onSessionExpired}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-5xl">
      <nav aria-label="Project sections" className="border-b border-pf-border">
        <ul className="-mb-px flex gap-1 overflow-x-auto">
          {visibleTabs.map((tab) => {
            const current = tab === activeTab;
            const count = tabCounts[tab];
            return (
              <li key={tab}>
                <RouteLink
                  to={projectPath(project.id, tab)}
                  aria-current={current ? "page" : undefined}
                  className={`inline-flex whitespace-nowrap border-b-2 px-3 py-2.5 text-[12px] font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-pf-accent ${
                    current
                      ? "border-pf-accent text-pf-text"
                      : "border-transparent text-pf-text-secondary hover:border-pf-border-strong hover:text-pf-text"
                  }`}
                >
                  {TAB_LABELS[tab]}
                  {count !== undefined ? (
                    <span aria-hidden="true" className="ml-1.5 text-[10px] text-pf-text-muted">
                      {count}
                    </span>
                  ) : null}
                </RouteLink>
              </li>
            );
          })}
        </ul>
      </nav>

      <div className="mt-6">
        {activeTab === "repositories" ? (
          <RepositoriesPanel
            projectId={project.id}
            csrfToken={csrfToken}
            role={project.role}
            onJobsChanged={() => {
              void loadDetails(project.id, project.role);
            }}
            onSessionExpired={onSessionExpired}
          />
        ) : activeTab === "jobs" ? (
          <JobsSection
            projectId={project.id}
            jobs={jobs}
            loading={jobsLoading}
            error={jobsError}
            streamError={jobStreamError}
            actionError={jobActionError}
            rerunningId={rerunningId}
            canWrite={canWrite}
            liveCount={activeJobIds.length}
            onRerun={(job) => void handleRerunJob(job)}
          />
        ) : activeTab === "runs" ? (
          <RunsSection
            projectId={project.id}
            runs={runs}
            loading={runsLoading}
            error={runsError}
            streamError={runStreamError}
            liveCount={activeRunIds.length}
          />
        ) : activeTab === "targets" ? (
          <TargetsPanel
            projectId={project.id}
            csrfToken={csrfToken}
            canWrite={canWrite}
            onSessionExpired={onSessionExpired}
          />
        ) : activeTab === "members" ? (
          <MembersSection
            role={project.role}
            members={members}
            loading={membersLoading}
            error={membersError}
            roles={roles}
            memberManagement={memberManagement}
            email={memberEmail}
            effectiveRole={effectiveMemberRole}
            adding={addingMember}
            addError={memberError}
            notice={memberNotice}
            onEmailChange={setMemberEmail}
            onRoleChange={setMemberRole}
            onSubmit={handleAddMember}
          />
        ) : canViewAudit(project.role) ? (
          <ActivitySection audit={audit} loading={auditLoading} error={auditError} />
        ) : (
          <UnavailableNotice
            title="Audit access is restricted."
            detail="Only project owners and admins can read the audit trail."
            backTo={projectPath(project.id, "repositories")}
            backLabel="Back to repositories"
          />
        )}
      </div>
    </div>
  );
}
