import type { JobEvent, RunEvent } from "@probe-forge/contracts";
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ApiError,
  type AuditEvent,
  type AuthUser,
  addMember,
  assignableRoles,
  canManageMembers,
  canViewAudit,
  createProject,
  type Job,
  listAudit,
  listJobs,
  listMembers,
  listProjects,
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
import { TargetsPanel } from "./TargetsPanel";
import { useJobEvents, useRunEvents } from "./useJobEvents";

interface ProjectsViewProps {
  readonly user: AuthUser;
  readonly csrfToken: string;
  readonly onSignOut: () => Promise<void>;
  readonly onSessionExpired: () => void;
  readonly onOpenAdmin?: (() => void) | undefined;
}

const ROLE_LABELS: Record<ProjectRole, string> = {
  owner: "Owner",
  admin: "Admin",
  collaborator: "Collaborator",
  viewer: "Viewer",
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

const dateFormatter = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const dateTimeFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

function formatDate(value: string): string {
  return dateFormatter.format(new Date(value));
}

function formatDateTime(value: string): string {
  return dateTimeFormatter.format(new Date(value));
}

function errorMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

export function ProjectsView({
  user,
  csrfToken,
  onSignOut,
  onSessionExpired,
  onOpenAdmin,
}: ProjectsViewProps) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const detailGeneration = useRef(0);

  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const [members, setMembers] = useState<ProjectMember[]>([]);
  const [membersLoading, setMembersLoading] = useState(false);
  const [membersError, setMembersError] = useState<string | null>(null);
  const [audit, setAudit] = useState<AuditEvent[] | null>(null);
  const [auditLoading, setAuditLoading] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);

  const [jobs, setJobs] = useState<Job[]>([]);
  const [jobsLoading, setJobsLoading] = useState(false);
  const [jobsError, setJobsError] = useState<string | null>(null);
  const [selectedJob, setSelectedJob] = useState<Job | null>(null);
  const [rerunningId, setRerunningId] = useState<string | null>(null);
  const [jobActionError, setJobActionError] = useState<string | null>(null);
  const [jobStreamError, setJobStreamError] = useState<string | null>(null);
  const [runStreamError, setRunStreamError] = useState<string | null>(null);

  const [runs, setRuns] = useState<RunSummaryEntry[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  const [memberEmail, setMemberEmail] = useState("");
  const [memberRole, setMemberRole] = useState<ProjectRole>("viewer");
  const [addingMember, setAddingMember] = useState(false);
  const [memberError, setMemberError] = useState<string | null>(null);
  const [memberNotice, setMemberNotice] = useState<string | null>(null);

  const [signingOut, setSigningOut] = useState(false);

  const selected = useMemo(
    () => projects.find((project) => project.id === selectedId) ?? null,
    [projects, selectedId],
  );
  const selectedProjectId = selected?.id ?? null;
  const selectedRole = selected?.role ?? null;
  const roles = selected ? assignableRoles(selected.role) : [];
  const effectiveMemberRole = roles.includes(memberRole) ? memberRole : (roles[0] ?? "viewer");
  const memberManagement = selected ? canManageMembers(selected.role) : false;

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

  const refreshProjects = useCallback(async () => {
    try {
      const next = await listProjects();
      setProjects(next);
      setLoadError(null);
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setLoadError(message);
      }
    }
  }, [handleError]);

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

  // Live job progress. While a selected project has jobs the worker can still
  // move, subscribe to each job's SSE stream and re-read the job list whenever
  // an event implies a status the UI does not already show. PostgreSQL remains
  // authoritative; the stream is only a prompt to refresh.
  const jobsRef = useRef<Job[]>([]);
  useEffect(() => {
    jobsRef.current = jobs;
  }, [jobs]);

  const activeJobIds = useMemo(
    () => jobs.filter((job) => isLiveJob(job.status)).map((job) => job.id),
    [jobs],
  );

  const refreshJobs = useCallback(async () => {
    const projectId = selectedProjectId;
    if (!projectId) {
      return;
    }
    const generation = detailGeneration.current;
    try {
      const next = await listJobs(projectId);
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
  }, [selectedProjectId, handleError]);

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
    enabled: selectedProjectId !== null && selectedRole !== null,
    onEvent: handleJobEvent,
    onStreamError: handleJobStreamError,
  });

  // Live run progress. While the selected project has runs the worker can still
  // move, subscribe to each run's own SSE stream and re-read the run list
  // whenever a lifecycle event implies a status the UI does not already show,
  // or when a report lands for one of the runs. PostgreSQL remains
  // authoritative; the stream is only a prompt to refresh.
  const runsRef = useRef<RunSummaryEntry[]>([]);
  useEffect(() => {
    runsRef.current = runs;
  }, [runs]);

  const activeRunIds = useMemo(
    () => runs.filter((run) => isLiveRun(run.status)).map((run) => run.id),
    [runs],
  );

  const refreshRuns = useCallback(async () => {
    const projectId = selectedProjectId;
    if (!projectId) {
      return;
    }
    const generation = detailGeneration.current;
    try {
      const next = await listRuns(projectId);
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
  }, [selectedProjectId, handleError]);

  // Coalesce bursts of events into at most one in-flight refresh.
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
    enabled: selectedProjectId !== null && selectedRole !== null,
    onEvent: handleRunEvent,
    onStreamError: handleRunStreamError,
  });

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      try {
        const next = await listProjects();
        if (!cancelled) {
          setProjects(next);
          setLoadError(null);
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        const message = errorMessage(error);
        if (error instanceof ApiError && error.status === 401) {
          onSessionExpired();
          return;
        }
        setLoadError(message);
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [onSessionExpired]);

  useEffect(() => {
    setSelectedJob(null);
    setSelectedRunId(null);
    setJobActionError(null);
    setJobStreamError(null);
    setRunStreamError(null);
    setRerunningId(null);
    if (!selectedProjectId || !selectedRole) {
      detailGeneration.current += 1;
      setMembers([]);
      setMembersLoading(false);
      setMembersError(null);
      setAudit(null);
      setAuditLoading(false);
      setAuditError(null);
      setJobs([]);
      setJobsLoading(false);
      setJobsError(null);
      setRuns([]);
      setRunsLoading(false);
      setRunsError(null);
      return;
    }
    void loadDetails(selectedProjectId, selectedRole);
  }, [selectedProjectId, selectedRole, loadDetails]);

  useEffect(
    () => () => {
      detailGeneration.current += 1;
    },
    [],
  );

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setCreating(true);
    setCreateError(null);
    try {
      const project = await createProject(name, slug, csrfToken);
      setProjects((current) => [project, ...current]);
      setSelectedId(project.id);
      setName("");
      setSlug("");
      setShowCreate(false);
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setCreateError(message);
      }
    } finally {
      setCreating(false);
    }
  }

  async function handleAddMember(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selected) {
      return;
    }
    setAddingMember(true);
    setMemberError(null);
    setMemberNotice(null);
    try {
      const member = await addMember(selected.id, memberEmail, effectiveMemberRole, csrfToken);
      const notice = `${member.displayName} was added as ${ROLE_LABELS[member.role].toLowerCase()}.`;
      setMemberEmail("");
      await loadDetails(selected.id, selected.role);
      await refreshProjects();
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
      if (selected) {
        await loadDetails(selected.id, selected.role);
      }
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setJobActionError(message);
      }
    } finally {
      setRerunningId(null);
    }
  }

  async function handleSignOut() {
    setSigningOut(true);
    try {
      await onSignOut();
    } finally {
      setSigningOut(false);
    }
  }

  return (
    <div className="min-h-screen bg-pf-background text-pf-text">
      <div className="mx-auto flex min-h-screen max-w-[1500px]">
        <aside className="hidden w-[236px] shrink-0 flex-col border-r border-pf-border bg-pf-surface/60 px-4 py-5 lg:flex">
          <div className="flex items-center gap-2.5 px-2">
            <span
              aria-hidden="true"
              className="grid size-8 place-items-center rounded-md bg-pf-accent text-[12px] font-semibold tracking-[0.08em] text-white"
            >
              PF
            </span>
            <span className="text-[14px] font-semibold tracking-[-0.01em]">ProbeForge</span>
          </div>

          <nav aria-label="Primary" className="mt-8">
            <div
              aria-current="page"
              className="flex items-center gap-2 rounded-md bg-pf-accent-soft px-2.5 py-2 text-[13px] font-medium text-pf-accent"
            >
              <span aria-hidden="true" className="size-1.5 rounded-full bg-pf-accent" />
              Projects
            </div>
            {onOpenAdmin ? (
              <button
                type="button"
                onClick={onOpenAdmin}
                className="mt-1 flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-[13px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
              >
                <span aria-hidden="true" className="size-1.5 rounded-full bg-pf-border-strong" />
                Platform admin
              </button>
            ) : null}
          </nav>

          <div className="mt-auto space-y-4">
            <div className="rounded-lg border border-pf-border bg-pf-surface px-3.5 py-3">
              <p className="text-[11px] font-medium text-pf-text-muted">Access model</p>
              <p className="mt-1.5 text-[12px] leading-5 text-pf-text-secondary">
                Every project action is authorized on the server. The interface only hides controls
                you cannot use.
              </p>
            </div>

            <div className="border-t border-pf-border pt-4">
              <p className="truncate px-1 text-[12px] font-medium" title={user.displayName}>
                {user.displayName}
              </p>
              <p className="truncate px-1 text-[11px] text-pf-text-muted" title={user.email}>
                {user.email}
              </p>
              <button
                type="button"
                onClick={handleSignOut}
                disabled={signingOut}
                className="mt-2 rounded px-1 py-1 text-[12px] font-medium text-pf-text-secondary transition-colors hover:text-pf-accent disabled:opacity-50"
              >
                {signingOut ? "Signing out..." : "Sign out"}
              </button>
            </div>
          </div>
        </aside>

        <main className="min-w-0 flex-1">
          <header className="border-b border-pf-border bg-pf-surface/50">
            <div className="flex flex-wrap items-end justify-between gap-4 px-6 py-6 lg:px-9 lg:py-7">
              <div>
                <div className="mb-4 flex items-center gap-2.5 lg:hidden">
                  <span
                    aria-hidden="true"
                    className="grid size-7 place-items-center rounded-md bg-pf-accent text-[11px] font-semibold tracking-[0.08em] text-white"
                  >
                    PF
                  </span>
                  <span className="text-[14px] font-semibold">ProbeForge</span>
                </div>
                <p className="font-mono text-[10px] font-medium tracking-[0.14em] text-pf-text-muted">
                  WORKSPACE
                </p>
                <h1 className="mt-1 text-[28px] font-semibold leading-tight tracking-[-0.025em]">
                  {selectedRunId ? "Run results" : selectedJob ? "Plan review" : "Projects"}
                </h1>
                <p className="mt-1.5 max-w-xl text-[13px] text-pf-text-secondary">
                  {selectedRunId
                    ? "Inspect stored results, evidence, observed metrics, and versioned report claims for this run."
                    : selectedJob
                      ? "Review the proposed tests, prerequisites, risks, and configuration before recording an immutable approval."
                      : "Each project holds repository analysis, proposed plans, approvals, and evidence for one codebase or deployed application."}
                </p>
              </div>

              <div className="flex items-center gap-4">
                <div className="hidden text-right sm:block">
                  <p className="text-[12px] font-medium">{user.displayName}</p>
                  <p className="text-[11px] text-pf-text-muted">{user.email}</p>
                </div>
                <button
                  type="button"
                  onClick={handleSignOut}
                  disabled={signingOut}
                  className="h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted hover:text-pf-text disabled:opacity-50 lg:hidden"
                >
                  {signingOut ? "Signing out..." : "Sign out"}
                </button>
              </div>
            </div>
          </header>

          <div className="px-6 py-7 lg:px-9 lg:py-8">
            {selectedRunId && selectedRole ? (
              <RunReport
                runId={selectedRunId}
                projectId={selectedProjectId ?? ""}
                canWrite={selectedRole !== "viewer"}
                csrfToken={csrfToken}
                onBack={() => setSelectedRunId(null)}
                onSessionExpired={onSessionExpired}
              />
            ) : selectedJob ? (
              <PlanReview
                job={selectedJob}
                canEdit={selectedRole !== null && selectedRole !== "viewer"}
                csrfToken={csrfToken}
                onBack={() => setSelectedJob(null)}
                onSessionExpired={onSessionExpired}
              />
            ) : (
              <section className="grid gap-6 xl:grid-cols-[minmax(0,1.45fr)_minmax(350px,0.8fr)] xl:items-start">
                <div className="min-w-0">
                  <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
                    <div>
                      <h2 className="text-[18px] font-semibold tracking-[-0.01em]">
                        Your projects
                      </h2>
                      <p className="mt-1 text-[12px] text-pf-text-secondary">
                        {projects.length === 0
                          ? "No projects yet."
                          : `${projects.length} ${projects.length === 1 ? "project" : "projects"} you can access.`}
                      </p>
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        setShowCreate((current) => !current);
                        setCreateError(null);
                      }}
                      className="inline-flex h-9 items-center rounded-md bg-pf-accent px-3.5 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover"
                    >
                      {showCreate ? "Cancel" : "New project"}
                    </button>
                  </div>

                  {showCreate ? (
                    <form
                      onSubmit={handleCreate}
                      aria-label="Create project"
                      className="mb-4 rounded-lg border border-pf-border bg-pf-surface p-4"
                    >
                      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,0.7fr)]">
                        <div>
                          <label
                            htmlFor="project-name"
                            className="mb-1.5 block text-[12px] font-medium"
                          >
                            Name
                          </label>
                          <input
                            id="project-name"
                            value={name}
                            onChange={(event) => setName(event.target.value)}
                            required
                            maxLength={120}
                            placeholder="Payments API"
                            className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[13px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                          />
                        </div>
                        <div>
                          <label
                            htmlFor="project-slug"
                            className="mb-1.5 block text-[12px] font-medium"
                          >
                            Slug <span className="font-normal text-pf-text-muted">optional</span>
                          </label>
                          <input
                            id="project-slug"
                            value={slug}
                            onChange={(event) => setSlug(event.target.value)}
                            pattern="[a-z0-9][a-z0-9-]{1,62}"
                            placeholder="payments-api"
                            className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 font-mono text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                          />
                        </div>
                      </div>
                      <p className="mt-2 text-[11px] text-pf-text-muted">
                        The slug is a stable identifier. Leave it blank to derive one from the name.
                      </p>
                      {createError ? (
                        <p role="alert" className="mt-3 text-[12px] text-pf-error">
                          {createError}
                        </p>
                      ) : null}
                      <div className="mt-4 flex items-center gap-2">
                        <button
                          type="submit"
                          disabled={creating || name.trim() === ""}
                          className="h-9 rounded-md bg-pf-accent px-3.5 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
                        >
                          {creating ? "Creating..." : "Create project"}
                        </button>
                        <button
                          type="button"
                          onClick={() => setShowCreate(false)}
                          className="h-9 rounded-md px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
                        >
                          Cancel
                        </button>
                      </div>
                    </form>
                  ) : null}

                  {loading ? (
                    <div
                      role="status"
                      className="rounded-lg border border-pf-border bg-pf-surface px-4 py-10 text-center text-[13px] text-pf-text-secondary"
                    >
                      Loading projects...
                    </div>
                  ) : loadError ? (
                    <div className="rounded-lg border border-[#e8c9c4] bg-[#fbf1ef] px-4 py-5">
                      <p role="alert" className="text-[13px] text-pf-error">
                        {loadError}
                      </p>
                      <button
                        type="button"
                        onClick={() => {
                          setLoading(true);
                          void refreshProjects().finally(() => setLoading(false));
                        }}
                        className="mt-3 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
                      >
                        Retry
                      </button>
                    </div>
                  ) : projects.length === 0 ? (
                    <div className="rounded-lg border border-dashed border-pf-border-strong bg-pf-surface px-6 py-12 text-center">
                      <h3 className="text-[15px] font-semibold">Create your first project</h3>
                      <p className="mx-auto mt-1.5 max-w-sm text-[12px] leading-5 text-pf-text-secondary">
                        A project groups the repository, plans, approvals, targets, and evidence for
                        one application.
                      </p>
                      <button
                        type="button"
                        onClick={() => setShowCreate(true)}
                        className="mt-4 h-9 rounded-md bg-pf-accent px-3.5 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover"
                      >
                        New project
                      </button>
                    </div>
                  ) : (
                    <ul className="overflow-hidden rounded-lg border border-pf-border bg-pf-surface">
                      {projects.map((project) => {
                        const active = project.id === selectedId;
                        return (
                          <li
                            key={project.id}
                            className="border-b border-pf-border last:border-b-0"
                          >
                            <button
                              type="button"
                              aria-pressed={active}
                              onClick={() => {
                                setSelectedId(project.id);
                                setSelectedJob(null);
                                setSelectedRunId(null);
                              }}
                              className={`flex w-full flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3.5 text-left transition-colors ${
                                active ? "bg-pf-accent-soft/65" : "hover:bg-pf-surface-hover"
                              }`}
                            >
                              <span className="min-w-0 flex-1">
                                <span className="block truncate text-[13px] font-medium">
                                  {project.name}
                                </span>
                                <span className="mt-0.5 block truncate font-mono text-[11px] text-pf-text-muted">
                                  {project.slug}
                                </span>
                              </span>
                              <span className="inline-flex items-center rounded-full border border-pf-border bg-pf-surface px-2 py-0.5 text-[11px] font-medium text-pf-text-secondary">
                                {ROLE_LABELS[project.role]}
                              </span>
                              <span className="w-20 text-right text-[11px] text-pf-text-muted">
                                {project.memberCount}{" "}
                                {project.memberCount === 1 ? "member" : "members"}
                              </span>
                              <span className="hidden w-24 text-right text-[11px] text-pf-text-muted sm:block">
                                {formatDate(project.createdAt)}
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </div>

                <div id="project-detail" className="min-w-0 xl:sticky xl:top-8">
                  {!selected ? (
                    <div className="rounded-lg border border-pf-border bg-pf-surface px-5 py-10 text-center">
                      <p className="text-[13px] font-medium">Select a project</p>
                      <p className="mx-auto mt-1.5 max-w-[240px] text-[12px] leading-5 text-pf-text-secondary">
                        Choose a project to review its members and recent security audit events.
                      </p>
                    </div>
                  ) : (
                    <div className="rounded-lg border border-pf-border bg-pf-surface">
                      <div className="border-b border-pf-border px-5 py-4">
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <h2 className="truncate text-[18px] font-semibold tracking-[-0.015em]">
                              {selected.name}
                            </h2>
                            <p className="mt-0.5 truncate font-mono text-[11px] text-pf-text-muted">
                              {selected.slug}
                            </p>
                          </div>
                          <span className="shrink-0 rounded-full border border-pf-border bg-pf-surface-muted px-2 py-0.5 text-[11px] font-medium text-pf-text-secondary">
                            {ROLE_LABELS[selected.role]}
                          </span>
                        </div>
                        <dl className="mt-4 grid grid-cols-3 gap-3 border-t border-pf-border pt-3">
                          <div>
                            <dt className="text-[10px] font-medium text-pf-text-muted">
                              Your role
                            </dt>
                            <dd className="mt-0.5 text-[12px]">{ROLE_LABELS[selected.role]}</dd>
                          </div>
                          <div>
                            <dt className="text-[10px] font-medium text-pf-text-muted">Members</dt>
                            <dd className="mt-0.5 text-[12px]">{selected.memberCount}</dd>
                          </div>
                          <div>
                            <dt className="text-[10px] font-medium text-pf-text-muted">Created</dt>
                            <dd className="mt-0.5 text-[12px]">{formatDate(selected.createdAt)}</dd>
                          </div>
                        </dl>
                      </div>

                      <RepositoriesPanel
                        projectId={selected.id}
                        csrfToken={csrfToken}
                        role={selected.role}
                        onJobsChanged={() => {
                          void loadDetails(selected.id, selected.role);
                        }}
                        onSessionExpired={onSessionExpired}
                      />

                      <section
                        className="border-b border-pf-border px-5 py-4"
                        aria-labelledby="jobs-heading"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <h3 id="jobs-heading" className="text-[13px] font-semibold">
                            Analysis jobs
                          </h3>
                          <span className="flex items-center gap-2 text-[11px] text-pf-text-muted">
                            {activeJobIds.length > 0 && !jobStreamError ? (
                              <span className="inline-flex items-center gap-1.5">
                                <span
                                  aria-hidden="true"
                                  className="size-1.5 rounded-full bg-pf-accent"
                                />
                                Live updates
                              </span>
                            ) : null}
                            <span>
                              {jobs.length} {jobs.length === 1 ? "job" : "jobs"}
                            </span>
                          </span>
                        </div>
                        {jobsLoading ? (
                          <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
                            Loading jobs...
                          </p>
                        ) : jobsError ? (
                          <p role="alert" className="mt-3 text-[12px] text-pf-error">
                            {jobsError}
                          </p>
                        ) : jobs.length === 0 ? (
                          <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
                            No analysis jobs yet. A plan review appears when a job has a proposed
                            plan.
                          </p>
                        ) : (
                          <ul className="mt-3 divide-y divide-pf-border">
                            {jobs.map((job) => {
                              const hasPlan = job.currentPlanId !== null;
                              const canRerun =
                                job.status === "failed" ||
                                job.status === "cancelled" ||
                                job.status === "expired";
                              return (
                                <li
                                  key={job.id}
                                  className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5"
                                >
                                  <button
                                    type="button"
                                    disabled={!hasPlan}
                                    onClick={() => {
                                      setSelectedRunId(null);
                                      setSelectedJob(job);
                                    }}
                                    className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 text-left transition-colors hover:bg-pf-surface-hover disabled:cursor-default disabled:hover:bg-transparent"
                                  >
                                    <span className="min-w-0 flex-1">
                                      <span className="block truncate text-[12px] font-medium">
                                        {job.displayName}
                                      </span>
                                      <span className="mt-0.5 block text-[10px] text-pf-text-muted">
                                        {JOB_GOAL_LABELS[job.goal]} ·{" "}
                                        {JOB_STATUS_LABELS[job.status]}
                                      </span>
                                    </span>
                                    <span
                                      className={`shrink-0 text-[10px] font-medium ${
                                        hasPlan ? "text-pf-accent" : "text-pf-text-muted"
                                      }`}
                                    >
                                      {hasPlan ? "Review plan →" : "No plan yet"}
                                    </span>
                                  </button>
                                  {canRerun && selected.role !== "viewer" ? (
                                    <button
                                      type="button"
                                      onClick={() => void handleRerunJob(job)}
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
                        {jobActionError ? (
                          <p role="alert" className="mt-2 text-[11px] text-pf-error">
                            {jobActionError}
                          </p>
                        ) : null}
                        {jobStreamError ? (
                          <p role="status" className="mt-2 text-[11px] text-pf-warning">
                            {jobStreamError}
                          </p>
                        ) : null}
                      </section>

                      <section
                        className="border-b border-pf-border px-5 py-4"
                        aria-labelledby="runs-heading"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <h3 id="runs-heading" className="text-[13px] font-semibold">
                            Runs
                          </h3>
                          <span className="flex items-center gap-2 text-[11px] text-pf-text-muted">
                            {activeRunIds.length > 0 && !runStreamError ? (
                              <span className="inline-flex items-center gap-1.5">
                                <span
                                  aria-hidden="true"
                                  className="size-1.5 rounded-full bg-pf-accent"
                                />
                                Live updates
                              </span>
                            ) : null}
                            <span>
                              {runs.length} {runs.length === 1 ? "run" : "runs"}
                            </span>
                          </span>
                        </div>
                        {runsLoading ? (
                          <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
                            Loading runs...
                          </p>
                        ) : runsError ? (
                          <p role="alert" className="mt-3 text-[12px] text-pf-error">
                            {runsError}
                          </p>
                        ) : runs.length === 0 ? (
                          <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
                            No runs yet. A run appears after an approved plan is scheduled or
                            executed.
                          </p>
                        ) : (
                          <ul className="mt-3 divide-y divide-pf-border">
                            {runs.map((run) => (
                              <li key={run.id}>
                                <button
                                  type="button"
                                  onClick={() => {
                                    setSelectedJob(null);
                                    setSelectedRunId(run.id);
                                  }}
                                  className="flex w-full flex-wrap items-center gap-x-3 gap-y-1 py-2.5 text-left transition-colors hover:bg-pf-surface-hover"
                                >
                                  <span className="min-w-0 flex-1">
                                    <span className="block truncate text-[12px] font-medium">
                                      {RUN_TYPE_LABELS[run.runType] ?? run.runType}
                                    </span>
                                    <span className="mt-0.5 block text-[10px] text-pf-text-muted">
                                      {RUN_STATUS_LABELS[run.status] ?? run.status} · plan v
                                      {run.planVersion} · {formatDateTime(run.createdAt)}
                                    </span>
                                  </span>
                                  <span className="shrink-0 text-[10px] font-medium text-pf-accent">
                                    View results →
                                  </span>
                                </button>
                              </li>
                            ))}
                          </ul>
                        )}
                        {runStreamError ? (
                          <p role="status" className="mt-2 text-[11px] text-pf-warning">
                            {runStreamError}
                          </p>
                        ) : null}
                      </section>

                      <TargetsPanel
                        projectId={selected.id}
                        csrfToken={csrfToken}
                        canWrite={selected.role !== "viewer"}
                        onSessionExpired={onSessionExpired}
                      />

                      <section
                        className="border-b border-pf-border px-5 py-4"
                        aria-labelledby="members-heading"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <h3 id="members-heading" className="text-[13px] font-semibold">
                            Members
                          </h3>
                          <span className="text-[11px] text-pf-text-muted">
                            {members.length} {members.length === 1 ? "person" : "people"}
                          </span>
                        </div>

                        {membersLoading ? (
                          <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
                            Loading members...
                          </p>
                        ) : membersError ? (
                          <p role="alert" className="mt-3 text-[12px] text-pf-error">
                            {membersError}
                          </p>
                        ) : (
                          <ul className="mt-3 divide-y divide-pf-border">
                            {members.map((member) => (
                              <li
                                key={member.userId}
                                className="flex items-center justify-between gap-3 py-2.5"
                              >
                                <div className="min-w-0">
                                  <p className="truncate text-[12px] font-medium">
                                    {member.displayName}
                                  </p>
                                  <p className="truncate text-[11px] text-pf-text-muted">
                                    {member.email}
                                  </p>
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
                            onSubmit={handleAddMember}
                            aria-label="Add project member"
                            className="mt-4 border-t border-pf-border pt-4"
                          >
                            <p className="text-[11px] font-medium text-pf-text-secondary">
                              Add an existing account
                            </p>
                            <div className="mt-2 grid gap-2 sm:grid-cols-[minmax(0,1fr)_112px]">
                              <label className="sr-only" htmlFor="member-email">
                                Member email
                              </label>
                              <input
                                id="member-email"
                                type="email"
                                required
                                value={memberEmail}
                                onChange={(event) => setMemberEmail(event.target.value)}
                                placeholder="person@example.com"
                                className="h-9 min-w-0 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                              />
                              <label className="sr-only" htmlFor="member-role">
                                Member role
                              </label>
                              <select
                                id="member-role"
                                value={effectiveMemberRole}
                                onChange={(event) =>
                                  setMemberRole(event.target.value as ProjectRole)
                                }
                                className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
                              >
                                {roles.map((role) => (
                                  <option key={role} value={role}>
                                    {ROLE_LABELS[role]}
                                  </option>
                                ))}
                              </select>
                            </div>
                            <button
                              type="submit"
                              disabled={addingMember || memberEmail.trim() === ""}
                              className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-55"
                            >
                              {addingMember ? "Adding..." : "Add member"}
                            </button>
                            {memberError ? (
                              <p role="alert" className="mt-2 text-[11px] text-pf-error">
                                {memberError}
                              </p>
                            ) : null}
                            {memberNotice ? (
                              <p
                                role="status"
                                className="mt-2 text-[11px] leading-4 text-pf-success"
                              >
                                {memberNotice}
                              </p>
                            ) : null}
                          </form>
                        ) : (
                          <p className="mt-3 border-t border-pf-border pt-3 text-[11px] leading-4 text-pf-text-muted">
                            {selected.role === "viewer"
                              ? "Viewers can read project data. Member management is restricted."
                              : "Collaborators can work with project content. Member management is restricted."}
                          </p>
                        )}
                      </section>

                      {canViewAudit(selected.role) ? (
                        <section className="px-5 py-4" aria-labelledby="audit-heading">
                          <div className="flex items-center justify-between gap-3">
                            <h3 id="audit-heading" className="text-[13px] font-semibold">
                              Audit trail
                            </h3>
                            <span className="text-[11px] text-pf-text-muted">Latest 50</span>
                          </div>
                          {auditLoading ? (
                            <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
                              Loading audit events...
                            </p>
                          ) : auditError ? (
                            <p role="alert" className="mt-3 text-[12px] text-pf-error">
                              {auditError}
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
                                          <span className="font-mono">
                                            {event.actorId.slice(0, 8)}
                                          </span>
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
                        </section>
                      ) : null}
                    </div>
                  )}
                </div>
              </section>
            )}
          </div>
        </main>
      </div>
    </div>
  );
}
