import { type FormEvent, useState } from "react";
import { ApiError, createJob, type Job, type JobGoal, type JobIntensity } from "./api";

interface ConnectRepositoryProps {
  readonly projectId: string;
  readonly csrfToken: string;
  readonly canWrite: boolean;
  readonly onCreated: (job: Job) => void;
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

export function ConnectRepository({
  projectId,
  csrfToken,
  canWrite,
  onCreated,
  onSessionExpired,
}: ConnectRepositoryProps) {
  const [repositoryUrl, setRepositoryUrl] = useState("");
  const [reference, setReference] = useState("main");
  const [goal, setGoal] = useState<JobGoal>("vulnerability_testing");
  const [intensity, setIntensity] = useState<JobIntensity>("standard");
  const [displayName, setDisplayName] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const validation = repositoryUrl === "" ? null : publicRepoError(repositoryUrl);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const repoError = publicRepoError(repositoryUrl);
    if (repoError) {
      setError(repoError);
      return;
    }
    if (reference.trim() === "") {
      setError("Enter a branch, tag, or commit SHA.");
      return;
    }
    const name = displayName.trim() === "" ? repoNameFromUrl(repositoryUrl) : displayName.trim();
    setSubmitting(true);
    setError(null);
    setNotice(null);
    try {
      const job = await createJob(
        projectId,
        {
          goal,
          intensity,
          displayName: name,
          repositoryUrl: repositoryUrl.trim(),
          reference: reference.trim(),
        },
        csrfToken,
      );
      onCreated(job);
      setRepositoryUrl("");
      setDisplayName("");
      setNotice(`Connected "${job.displayName}". Analysis is queued.`);
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) {
        onSessionExpired();
        return;
      }
      setError(message(cause));
    } finally {
      setSubmitting(false);
    }
  }

  if (!canWrite) {
    return (
      <section
        className="border-b border-pf-border px-5 py-4"
        aria-labelledby="connect-repo-heading"
      >
        <h3 id="connect-repo-heading" className="text-[13px] font-semibold">
          Connect a repository
        </h3>
        <p className="mt-2 text-[11px] leading-4 text-pf-text-muted">
          Your role is read-only. A collaborator, admin, or owner can connect a public repository.
        </p>
      </section>
    );
  }

  return (
    <section className="border-b border-pf-border px-5 py-4" aria-labelledby="connect-repo-heading">
      <h3 id="connect-repo-heading" className="text-[13px] font-semibold">
        Connect a repository
      </h3>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        Public GitHub, GitLab, or Bitbucket link. Analysis is read-only and pinned to one commit; no
        code is executed.
      </p>

      <form onSubmit={handleSubmit} aria-label="Connect repository" className="mt-3 space-y-3">
        <div>
          <label htmlFor="repo-url" className="mb-1.5 block text-[11px] font-medium">
            Repository URL
          </label>
          <input
            id="repo-url"
            type="url"
            inputMode="url"
            required
            value={repositoryUrl}
            onChange={(event) => {
              setRepositoryUrl(event.target.value);
              setError(null);
            }}
            placeholder="https://github.com/owner/repo"
            aria-invalid={validation !== null}
            className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
          />
          {validation ? (
            <p className="mt-1 text-[10px] leading-4 text-pf-error">{validation}</p>
          ) : null}
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label htmlFor="repo-reference" className="mb-1.5 block text-[11px] font-medium">
              Branch, tag, or SHA
            </label>
            <input
              id="repo-reference"
              required
              value={reference}
              onChange={(event) => setReference(event.target.value)}
              placeholder="main"
              className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 font-mono text-[11px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
            />
          </div>
          <div>
            <label htmlFor="repo-intensity" className="mb-1.5 block text-[11px] font-medium">
              Intensity
            </label>
            <select
              id="repo-intensity"
              value={intensity}
              onChange={(event) => setIntensity(event.target.value as JobIntensity)}
              className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
            >
              {INTENSITY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div>
          <label htmlFor="repo-goal" className="mb-1.5 block text-[11px] font-medium">
            Goal
          </label>
          <select
            id="repo-goal"
            value={goal}
            onChange={(event) => setGoal(event.target.value as JobGoal)}
            className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
          >
            {GOAL_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <p className="mt-1 text-[10px] leading-4 text-pf-text-muted">
            {GOAL_OPTIONS.find((option) => option.value === goal)?.hint}
          </p>
        </div>

        <div>
          <label htmlFor="repo-name" className="mb-1.5 block text-[11px] font-medium">
            Name <span className="font-normal text-pf-text-muted">optional</span>
          </label>
          <input
            id="repo-name"
            value={displayName}
            maxLength={120}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Derived from the repository"
            className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
          />
        </div>

        {error ? (
          <p role="alert" className="text-[11px] leading-4 text-pf-error">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p role="status" className="text-[11px] leading-4 text-pf-success">
            {notice}
          </p>
        ) : null}

        <button
          type="submit"
          disabled={submitting || repositoryUrl.trim() === "" || validation !== null}
          className="h-9 w-full rounded-md bg-pf-accent px-3 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
        >
          {submitting ? "Connecting..." : "Connect repository"}
        </button>
      </form>
    </section>
  );
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
