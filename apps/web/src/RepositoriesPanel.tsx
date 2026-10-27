import { type FormEvent, useCallback, useEffect, useState } from "react";
import {
  ApiError,
  canRemoveRepository,
  createJob,
  type JobGoal,
  type JobIntensity,
  listRepositories,
  type ProjectRole,
  type Repository,
  registerRepository,
  removeRepository,
} from "./api";

interface RepositoriesPanelProps {
  readonly projectId: string;
  readonly csrfToken: string;
  readonly role: ProjectRole;
  readonly onJobsChanged: () => void;
  readonly onSessionExpired: () => void;
}

const GOAL_OPTIONS: ReadonlyArray<{ value: JobGoal; label: string; hint: string }> = [
  {
    value: "vulnerability_testing",
    label: "Vulnerability testing",
    hint: "Inspect the codebase and propose a bounded set of security checks.",
  },
  {
    value: "load_testing",
    label: "Load testing",
    hint: "Propose endpoint-aware load scenarios from the discovered API surface.",
  },
];

const INTENSITY_OPTIONS: ReadonlyArray<{ value: JobIntensity; label: string }> = [
  { value: "low", label: "Low" },
  { value: "standard", label: "Standard" },
  { value: "high", label: "High" },
];

const PUBLIC_HOSTS = [
  "github.com",
  "www.github.com",
  "gitlab.com",
  "www.gitlab.com",
  "bitbucket.org",
  "www.bitbucket.org",
];

/** Public-repository modes only: an https URL on a supported Git host. */
function publicRepoError(raw: string): string | null {
  const value = raw.trim();
  if (value === "") {
    return "Enter a repository URL.";
  }
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    return "That is not a valid URL. Paste a full link such as https://github.com/owner/repo.";
  }
  if (parsed.protocol !== "https:") {
    return "Only public HTTPS repository URLs are supported.";
  }
  if (parsed.username !== "" || parsed.password !== "") {
    return "Remove credentials from the URL; only public repositories are supported.";
  }
  if (!PUBLIC_HOSTS.includes(parsed.hostname.toLowerCase())) {
    return "Only public GitHub, GitLab, and Bitbucket repositories are supported.";
  }
  if (parsed.pathname.replace(/\/+$/, "").split("/").filter(Boolean).length < 2) {
    return "Include the owner and repository, for example https://github.com/owner/repo.";
  }
  return null;
}

function message(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function repoNameFromUrl(raw: string): string {
  try {
    const parsed = new URL(raw.trim());
    const parts = parsed.pathname.replace(/\/+$/, "").split("/").filter(Boolean);
    const repo = parts[parts.length - 1] ?? "repository";
    return repo.replace(/\.git$/i, "").slice(0, 120) || "repository";
  } catch {
    return "repository";
  }
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(new Date(value));
}

export function RepositoriesPanel({
  projectId,
  csrfToken,
  role,
  onJobsChanged,
  onSessionExpired,
}: RepositoriesPanelProps) {
  const canWrite = role !== "viewer";
  const canRemove = canRemoveRepository(role);

  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [repositoryUrl, setRepositoryUrl] = useState("");
  const [reference, setReference] = useState("main");
  const [displayName, setDisplayName] = useState("");
  const [registering, setRegistering] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const validation = repositoryUrl === "" ? null : publicRepoError(repositoryUrl);

  const handleError = useCallback(
    (cause: unknown): string | null => {
      if (cause instanceof ApiError && cause.status === 401) {
        onSessionExpired();
        return null;
      }
      return message(cause);
    },
    [onSessionExpired],
  );

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setRepositories(await listRepositories(projectId));
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setLoading(false);
    }
  }, [projectId, handleError]);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function handleRegister(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const repoError = publicRepoError(repositoryUrl);
    if (repoError) {
      setFormError(repoError);
      return;
    }
    if (reference.trim() === "") {
      setFormError("Enter a branch, tag, or commit SHA.");
      return;
    }
    setRegistering(true);
    setFormError(null);
    setNotice(null);
    try {
      const repository = await registerRepository(
        projectId,
        {
          repositoryUrl: repositoryUrl.trim(),
          reference: reference.trim(),
          ...(displayName.trim() === "" ? {} : { displayName: displayName.trim() }),
        },
        csrfToken,
      );
      setRepositoryUrl("");
      setDisplayName("");
      setReference("main");
      await reload();
      setNotice(
        `Added "${
          repository.displayName ?? repoNameFromUrl(repository.repositoryUrl)
        }". Start an analysis when you are ready.`,
      );
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setFormError(text);
      }
    } finally {
      setRegistering(false);
    }
  }

  async function handleRemove(repository: Repository) {
    setBusyId(repository.id);
    setError(null);
    setNotice(null);
    try {
      await removeRepository(projectId, repository.id, csrfToken);
      setConfirmRemoveId(null);
      await reload();
      setNotice(
        `Removed "${repository.displayName ?? repoNameFromUrl(repository.repositoryUrl)}".`,
      );
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setBusyId(null);
    }
  }

  async function handleStartAnalysis(
    repository: Repository,
    goal: JobGoal,
    intensity: JobIntensity,
  ) {
    setBusyId(repository.id);
    setError(null);
    setNotice(null);
    try {
      const name = repository.displayName ?? repoNameFromUrl(repository.repositoryUrl);
      const job = await createJob(
        projectId,
        {
          goal,
          intensity,
          displayName: name,
          repositoryUrl: repository.repositoryUrl,
          reference: repository.defaultReference,
        },
        csrfToken,
      );
      onJobsChanged();
      setNotice(`Started "${job.displayName}". Analysis is queued.`);
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setBusyId(null);
    }
  }

  return (
    <section
      className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4"
      aria-labelledby="repositories-heading"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 id="repositories-heading" className="text-[13px] font-semibold">
          Repositories
        </h3>
        <span className="text-[11px] text-pf-text-muted">
          {repositories.length} {repositories.length === 1 ? "repository" : "repositories"}
        </span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        Connect one or more public repositories. Analysis is read-only, pinned to one commit, and
        never executes repository code.
      </p>

      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading repositories...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : repositories.length === 0 ? (
        <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
          No repositories connected to this project yet.
        </p>
      ) : (
        <ul className="mt-3 space-y-3">
          {repositories.map((repository) => {
            const name = repository.displayName ?? repoNameFromUrl(repository.repositoryUrl);
            return (
              <li
                key={repository.id}
                className="rounded-md border border-pf-border bg-pf-surface-muted/40 px-3 py-3"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-[12px] font-medium" title={name}>
                    {name}
                  </span>
                  <span className="rounded-full border border-pf-border bg-pf-surface px-2 py-0.5 text-[10px] font-medium text-pf-text-secondary">
                    {repository.defaultReference}
                  </span>
                </div>
                <p className="mt-1 truncate font-mono text-[10px] text-pf-text-muted">
                  {repository.repositoryUrl}
                </p>
                <p className="mt-1 text-[10px] text-pf-text-muted">
                  Added {formatDate(repository.createdAt)} · {repository.jobCount}{" "}
                  {repository.jobCount === 1 ? "analysis" : "analyses"}
                </p>

                {canWrite ? (
                  <AnalyzeForm
                    repository={repository}
                    busy={busyId === repository.id}
                    onStart={(goal, intensity) =>
                      void handleStartAnalysis(repository, goal, intensity)
                    }
                  />
                ) : null}

                {canRemove ? (
                  confirmRemoveId === repository.id ? (
                    <div className="mt-2 flex flex-wrap items-center gap-2">
                      <span className="text-[10px] text-pf-text-muted">
                        Remove this repository? Analysis history is preserved.
                      </span>
                      <button
                        type="button"
                        onClick={() => void handleRemove(repository)}
                        disabled={busyId === repository.id}
                        className="h-7 rounded-md border border-pf-border-strong bg-pf-surface px-2.5 text-[11px] font-medium text-pf-error transition-colors hover:border-pf-error disabled:opacity-55"
                      >
                        {busyId === repository.id ? "Removing..." : "Confirm remove"}
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirmRemoveId(null)}
                        className="h-7 rounded-md px-2 text-[11px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      type="button"
                      onClick={() => {
                        setConfirmRemoveId(repository.id);
                        setNotice(null);
                      }}
                      className="mt-2 h-7 rounded-md border border-pf-border-strong bg-pf-surface px-2.5 text-[11px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted"
                    >
                      Remove
                    </button>
                  )
                ) : null}
              </li>
            );
          })}
        </ul>
      )}

      {notice ? (
        <p role="status" className="mt-3 text-[11px] leading-4 text-pf-success">
          {notice}
        </p>
      ) : null}

      {canWrite ? (
        <form
          onSubmit={handleRegister}
          aria-label="Add repository"
          className="mt-4 border-t border-pf-border pt-4"
        >
          <p className="text-[11px] font-medium text-pf-text-secondary">Add a repository</p>
          <div className="mt-2 space-y-2">
            <div>
              <label htmlFor="repository-url" className="mb-1.5 block text-[11px] font-medium">
                Repository URL
              </label>
              <input
                id="repository-url"
                type="url"
                inputMode="url"
                required
                value={repositoryUrl}
                onChange={(event) => {
                  setRepositoryUrl(event.target.value);
                  setFormError(null);
                }}
                placeholder="https://github.com/owner/repo"
                aria-invalid={validation !== null}
                className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
              />
              {validation ? (
                <p className="mt-1 text-[10px] leading-4 text-pf-error">{validation}</p>
              ) : null}
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label
                  htmlFor="repository-reference"
                  className="mb-1.5 block text-[11px] font-medium"
                >
                  Branch, tag, or SHA
                </label>
                <input
                  id="repository-reference"
                  required
                  value={reference}
                  onChange={(event) => setReference(event.target.value)}
                  placeholder="main"
                  className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 font-mono text-[11px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                />
              </div>
              <div>
                <label htmlFor="repository-name" className="mb-1.5 block text-[11px] font-medium">
                  Name <span className="font-normal text-pf-text-muted">optional</span>
                </label>
                <input
                  id="repository-name"
                  maxLength={120}
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                  placeholder="Derived from the URL"
                  className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                />
              </div>
            </div>
          </div>
          {formError ? (
            <p role="alert" className="mt-2 text-[11px] text-pf-error">
              {formError}
            </p>
          ) : null}
          <button
            type="submit"
            disabled={registering || repositoryUrl.trim() === "" || validation !== null}
            className="mt-2 h-8 rounded-md bg-pf-accent px-3 text-[11px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
          >
            {registering ? "Adding..." : "Add repository"}
          </button>
        </form>
      ) : (
        <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
          Your role is read-only. A collaborator, admin, or owner can connect a repository.
        </p>
      )}
    </section>
  );
}

function AnalyzeForm({
  repository,
  busy,
  onStart,
}: {
  readonly repository: Repository;
  readonly busy: boolean;
  readonly onStart: (goal: JobGoal, intensity: JobIntensity) => void;
}) {
  const [open, setOpen] = useState(false);
  const [goal, setGoal] = useState<JobGoal>("vulnerability_testing");
  const [intensity, setIntensity] = useState<JobIntensity>("standard");

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted"
      >
        Start analysis
      </button>
    );
  }

  return (
    <div className="mt-2 rounded-md border border-pf-border bg-pf-surface px-3 py-3">
      <p className="text-[10px] font-medium text-pf-text-muted">
        Start analysis · {repoNameFromUrl(repository.repositoryUrl)}
      </p>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <label className="text-[10px] text-pf-text-muted">
          Goal
          <select
            value={goal}
            onChange={(event) => setGoal(event.target.value as JobGoal)}
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          >
            {GOAL_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <label className="text-[10px] text-pf-text-muted">
          Intensity
          <select
            value={intensity}
            onChange={(event) => setIntensity(event.target.value as JobIntensity)}
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          >
            {INTENSITY_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="mt-1.5 text-[10px] leading-4 text-pf-text-muted">
        {GOAL_OPTIONS.find((option) => option.value === goal)?.hint}
      </p>
      <div className="mt-2 flex items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => onStart(goal, intensity)}
          className="h-8 rounded-md bg-pf-accent px-3 text-[11px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
        >
          {busy ? "Starting..." : "Start analysis"}
        </button>
        <button
          type="button"
          onClick={() => setOpen(false)}
          className="h-8 rounded-md px-2 text-[11px] font-medium text-pf-text-secondary transition-colors hover:bg-pf-surface-hover hover:text-pf-text"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
