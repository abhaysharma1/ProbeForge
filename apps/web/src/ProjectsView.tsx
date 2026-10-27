import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  ApiError,
  type AuthUser,
  createProject,
  listProjects,
  type Project,
} from "./api";
import { ProjectPage } from "./ProjectPage";
import {
  navigate,
  projectPath,
  projectsPath,
  RouteLink,
  useHashRoute,
} from "./routing";

interface ProjectsViewProps {
  readonly user: AuthUser;
  readonly csrfToken: string;
  readonly onSignOut: () => Promise<void>;
  readonly onSessionExpired: () => void;
  readonly onOpenAdmin?: (() => void) | undefined;
}

const ROLE_LABELS: Record<Project["role"], string> = {
  owner: "Owner",
  admin: "Admin",
  collaborator: "Collaborator",
  viewer: "Viewer",
};

const dateFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
});

function formatDate(value: string): string {
  return dateFormatter.format(new Date(value));
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
  const route = useHashRoute();
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const [signingOut, setSigningOut] = useState(false);

  const projectId = route.kind === "projects" ? null : route.projectId;
  const project = useMemo(
    () =>
      projectId === null
        ? null
        : (projects.find((item) => item.id === projectId) ?? null),
    [projects, projectId],
  );
  const projectUnavailable =
    projectId !== null && !loading && !loadError && project === null;

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

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setCreating(true);
    setCreateError(null);
    try {
      const created = await createProject(name, slug, csrfToken);
      setProjects((current) => [created, ...current]);
      setName("");
      setSlug("");
      setShowCreate(false);
      navigate(projectPath(created.id));
    } catch (error) {
      const message = handleError(error);
      if (message) {
        setCreateError(message);
      }
    } finally {
      setCreating(false);
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

  const heading =
    route.kind === "projects"
      ? "Projects"
      : route.kind === "project"
        ? (project?.name ?? "Project")
        : route.kind === "job"
          ? "Plan review"
          : "Run results";

  return (
    <div className="min-h-screen bg-pf-background text-pf-text ">
      <div className=" flex min-h-screen max-w-[1500px]">
        <div className="flex">
          <aside className=" w-[236px] shrink-0 flex-col border-r border-pf-border bg-pf-surface/60 px-4 py-5 lg:flex h-screen sticky top-0">
            <div className="flex items-center gap-2.5 px-2">
              <span
                aria-hidden="true"
                className="grid size-8 place-items-center rounded-md bg-pf-accent text-[12px] font-semibold tracking-[0.08em] text-white"
              >
                PF
              </span>
              <span className="text-[14px] font-semibold tracking-[-0.01em]">
                ProbeForge
              </span>
            </div>

            <nav aria-label="Primary" className="mt-8">
              <div
                aria-current="page"
                className="flex items-center gap-2 rounded-md bg-pf-accent-soft px-2.5 py-2 text-[13px] font-medium text-pf-accent"
              >
                <span
                  aria-hidden="true"
                  className="size-1.5 rounded-full bg-pf-accent"
                />
                Projects
              </div>
              {onOpenAdmin ? (
                <button
                  type="button"
                  onClick={onOpenAdmin}
                  className="mt-1 flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-[13px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
                >
                  <span
                    aria-hidden="true"
                    className="size-1.5 rounded-full bg-pf-border-strong"
                  />
                  Platform admin
                </button>
              ) : null}
            </nav>

            <div className="mt-auto space-y-4">
              <div className="rounded-lg border border-pf-border bg-pf-surface px-3.5 py-3">
                <p className="text-[11px] font-medium text-pf-text-muted">
                  Access model
                </p>
                <p className="mt-1.5 text-[12px] leading-5 text-pf-text-secondary">
                  Every project action is authorized on the server. The
                  interface only hides controls you cannot use.
                </p>
              </div>

              <div className="border-t border-pf-border pt-4">
                <p
                  className="truncate px-1 text-[12px] font-medium"
                  title={user.displayName}
                >
                  {user.displayName}
                </p>
                <p
                  className="truncate px-1 text-[11px] text-pf-text-muted"
                  title={user.email}
                >
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
        </div>

        <main className="min-w-0 flex-1">
          <header className="border-b border-pf-border bg-pf-surface/50">
            <div className="flex flex-wrap items-end justify-between gap-4 px-6 py-6 lg:px-9 lg:py-7">
              <div className="min-w-0">
                <div className="mb-4 flex items-center gap-2.5 lg:hidden">
                  <span
                    aria-hidden="true"
                    className="grid size-7 place-items-center rounded-md bg-pf-accent text-[11px] font-semibold tracking-[0.08em] text-white"
                  >
                    PF
                  </span>
                  <span className="text-[14px] font-semibold">ProbeForge</span>
                </div>
                {route.kind !== "projects" ? (
                  <RouteLink
                    to={projectsPath()}
                    className="mb-3 inline-flex items-center gap-1.5 rounded text-[12px] font-medium text-pf-text-secondary transition-colors hover:text-pf-text focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-pf-accent"
                  >
                    <span aria-hidden="true">←</span> All projects
                  </RouteLink>
                ) : null}
                <p className="font-mono text-[10px] font-medium tracking-[0.14em] text-pf-text-muted">
                  {route.kind === "projects" ? "WORKSPACE" : "PROJECT"}
                </p>
                <h1 className="mt-1 text-[28px] font-semibold leading-tight tracking-[-0.025em]">
                  {heading}
                </h1>
                {route.kind === "projects" ? (
                  <p className="mt-1.5 max-w-xl text-[13px] text-pf-text-secondary">
                    Each project holds repository analysis, proposed plans,
                    approvals, and evidence for one codebase or deployed
                    application.
                  </p>
                ) : route.kind === "project" ? (
                  project ? (
                    <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-pf-text-secondary">
                      <span className="font-mono text-[11px] text-pf-text-muted">
                        {project.slug}
                      </span>
                      <span aria-hidden="true" className="text-pf-text-muted">
                        ·
                      </span>
                      <span>{ROLE_LABELS[project.role]}</span>
                      <span aria-hidden="true" className="text-pf-text-muted">
                        ·
                      </span>
                      <span>
                        {project.memberCount}{" "}
                        {project.memberCount === 1 ? "member" : "members"}
                      </span>
                      <span aria-hidden="true" className="text-pf-text-muted">
                        ·
                      </span>
                      <span>Created {formatDate(project.createdAt)}</span>
                    </p>
                  ) : (
                    <p className="mt-1.5 text-[13px] text-pf-text-secondary">
                      {loading
                        ? "Loading project..."
                        : "Project details are unavailable."}
                    </p>
                  )
                ) : route.kind === "job" ? (
                  <p className="mt-1.5 max-w-xl text-[13px] text-pf-text-secondary">
                    Review the proposed tests, prerequisites, risks, and
                    configuration before recording an immutable approval.
                  </p>
                ) : (
                  <p className="mt-1.5 max-w-xl text-[13px] text-pf-text-secondary">
                    Inspect stored results, evidence, observed metrics, and
                    versioned report claims for this run.
                  </p>
                )}
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
            {route.kind === "projects" ? (
              <section className="mx-auto w-full max-w-5xl">
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
                          Slug{" "}
                          <span className="font-normal text-pf-text-muted">
                            optional
                          </span>
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
                      The slug is a stable identifier. Leave it blank to derive
                      one from the name.
                    </p>
                    {createError ? (
                      <p
                        role="alert"
                        className="mt-3 text-[12px] text-pf-error"
                      >
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
                    <h3 className="text-[15px] font-semibold">
                      Create your first project
                    </h3>
                    <p className="mx-auto mt-1.5 max-w-sm text-[12px] leading-5 text-pf-text-secondary">
                      A project groups the repository, plans, approvals,
                      targets, and evidence for one application.
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
                    {projects.map((item) => (
                      <li
                        key={item.id}
                        className="border-b border-pf-border last:border-b-0"
                      >
                        <RouteLink
                          to={projectPath(item.id)}
                          className="flex w-full flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3.5 text-left transition-colors hover:bg-pf-surface-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-pf-accent"
                        >
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-[13px] font-medium">
                              {item.name}
                            </span>
                            <span className="mt-0.5 block truncate font-mono text-[11px] text-pf-text-muted">
                              {item.slug}
                            </span>
                          </span>
                          <span className="inline-flex items-center rounded-full border border-pf-border bg-pf-surface px-2 py-0.5 text-[11px] font-medium text-pf-text-secondary">
                            {ROLE_LABELS[item.role]}
                          </span>
                          <span className="w-20 text-right text-[11px] text-pf-text-muted">
                            {item.memberCount}{" "}
                            {item.memberCount === 1 ? "member" : "members"}
                          </span>
                          <span className="hidden w-24 text-right text-[11px] text-pf-text-muted sm:block">
                            {formatDate(item.createdAt)}
                          </span>
                          <span className="shrink-0 text-[11px] font-medium text-pf-accent">
                            Open project →
                          </span>
                        </RouteLink>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            ) : loading ? (
              <div className="mx-auto w-full max-w-5xl">
                <div
                  role="status"
                  className="rounded-lg border border-pf-border bg-pf-surface px-4 py-10 text-center text-[13px] text-pf-text-secondary"
                >
                  Loading project...
                </div>
              </div>
            ) : loadError ? (
              <div className="mx-auto w-full max-w-5xl rounded-lg border border-[#e8c9c4] bg-[#fbf1ef] px-4 py-5">
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
            ) : projectUnavailable || project === null ? (
              <section className="mx-auto w-full max-w-5xl rounded-lg border border-pf-border bg-pf-surface px-6 py-10 text-center">
                <p className="text-[14px] font-medium">
                  This project is not available.
                </p>
                <p className="mx-auto mt-1.5 max-w-md text-[12px] leading-5 text-pf-text-secondary">
                  It may have been removed, or your account may no longer have
                  access. Ask a project owner to confirm your membership.
                </p>
                <RouteLink
                  to={projectsPath()}
                  className="mt-4 inline-flex h-8 items-center rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted hover:text-pf-text"
                >
                  Back to projects
                </RouteLink>
              </section>
            ) : (
              <ProjectPage
                key={project.id}
                project={project}
                route={route}
                csrfToken={csrfToken}
                onProjectsChanged={refreshProjects}
                onSessionExpired={onSessionExpired}
              />
            )}
          </div>
        </main>
      </div>
    </div>
  );
}
