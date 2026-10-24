import { type FormEvent, useCallback, useEffect, useState } from "react";
import {
  ApiError,
  createAuthorization,
  createTarget,
  listAuthorizations,
  listTargets,
  revokeAuthorization,
  type Target,
  type TargetAuthorization,
  type TargetEnvironment,
  type TargetLimits,
  validateTarget,
} from "./api";

interface TargetsPanelProps {
  readonly projectId: string;
  readonly csrfToken: string;
  readonly canWrite: boolean;
  readonly onSessionExpired: () => void;
}

const ENVIRONMENT_LABELS: Record<TargetEnvironment, string> = {
  local: "Local",
  staging: "Staging",
  production: "Production",
};

const VERIFICATION_LABELS: Record<string, string> = {
  pending: "Not validated",
  verified: "Validated",
  failed: "Validation failed",
};

const DEFAULT_LIMITS: TargetLimits = {
  maxRequestsPerSecond: 5,
  maxConcurrency: 2,
  maxDurationSeconds: 60,
  maxRequests: 200,
};

function message(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function splitList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item !== "");
}

function splitPorts(value: string): number[] {
  return splitList(value)
    .map((item) => Number.parseInt(item, 10))
    .filter((port) => Number.isInteger(port) && port > 0 && port <= 65535);
}

function formatDateTime(value: string): string {
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(value),
  );
}

export function TargetsPanel({
  projectId,
  csrfToken,
  canWrite,
  onSessionExpired,
}: TargetsPanelProps) {
  const [targets, setTargets] = useState<Target[]>([]);
  const [authorizations, setAuthorizations] = useState<Record<string, TargetAuthorization[]>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [origin, setOrigin] = useState("https://");
  const [environment, setEnvironment] = useState<TargetEnvironment>("staging");
  const [hosts, setHosts] = useState("");
  const [paths, setPaths] = useState("/");
  const [ports, setPorts] = useState("");
  const [registering, setRegistering] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

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
      const next = await listTargets(projectId);
      const entries = await Promise.all(
        next.map(async (target) => [target.id, await listAuthorizations(target.id)] as const),
      );
      setTargets(next);
      setAuthorizations(Object.fromEntries(entries));
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
    if (origin.trim() === "") {
      setFormError("Enter the target origin.");
      return;
    }
    setRegistering(true);
    setFormError(null);
    try {
      await createTarget(
        projectId,
        {
          origin: origin.trim(),
          environment,
          allowedHosts: splitList(hosts),
          allowedPaths: splitList(paths),
          allowedPorts: splitPorts(ports),
        },
        csrfToken,
      );
      setOrigin("https://");
      setHosts("");
      setPaths("/");
      setPorts("");
      await reload();
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setFormError(text);
      }
    } finally {
      setRegistering(false);
    }
  }

  async function handleValidate(targetId: string) {
    setBusyId(targetId);
    setError(null);
    try {
      await validateTarget(targetId, csrfToken);
      await reload();
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setBusyId(null);
    }
  }

  async function handleAuthorize(target: Target, limits: TargetLimits, hours: number | null) {
    setBusyId(target.id);
    setError(null);
    try {
      const expiresAt =
        hours === null ? null : new Date(Date.now() + hours * 3_600_000).toISOString();
      await createAuthorization(target.id, limits, csrfToken, expiresAt);
      await reload();
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setBusyId(null);
    }
  }

  async function handleRevoke(authorizationId: string) {
    setBusyId(authorizationId);
    setError(null);
    try {
      await revokeAuthorization(authorizationId, csrfToken);
      await reload();
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
    <section className="border-b border-pf-border px-5 py-4" aria-labelledby="targets-heading">
      <div className="flex items-center justify-between gap-3">
        <h3 id="targets-heading" className="text-[13px] font-semibold">
          Targets
        </h3>
        <span className="text-[11px] text-pf-text-muted">
          {targets.length} {targets.length === 1 ? "target" : "targets"}
        </span>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-pf-text-muted">
        A run needs an authorized target. Register a deployed origin, validate it, then authorize a
        bounded scope.
      </p>

      {loading ? (
        <p role="status" className="mt-3 text-[12px] text-pf-text-secondary">
          Loading targets...
        </p>
      ) : error ? (
        <p role="alert" className="mt-3 text-[12px] text-pf-error">
          {error}
        </p>
      ) : targets.length === 0 ? (
        <p className="mt-3 text-[11px] leading-4 text-pf-text-muted">
          No targets configured for this project yet.
        </p>
      ) : (
        <ul className="mt-3 space-y-3">
          {targets.map((target) => (
            <li
              key={target.id}
              className="rounded-md border border-pf-border bg-pf-surface-muted/40 px-3 py-3"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span
                  className="min-w-0 truncate font-mono text-[11px]"
                  title={target.normalizedOrigin}
                >
                  {target.normalizedOrigin}
                </span>
                <span className="rounded-full border border-pf-border bg-pf-surface px-2 py-0.5 text-[10px] font-medium text-pf-text-secondary">
                  {ENVIRONMENT_LABELS[target.environment]} ·{" "}
                  {VERIFICATION_LABELS[target.verificationStatus] ?? target.verificationStatus}
                </span>
              </div>
              <p className="mt-1 text-[10px] text-pf-text-muted">
                Hosts {target.allowedHosts.join(", ") || "—"} · Ports{" "}
                {target.allowedPorts.join(", ") || "—"}
              </p>

              {canWrite ? (
                <button
                  type="button"
                  onClick={() => void handleValidate(target.id)}
                  disabled={busyId === target.id}
                  className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-55"
                >
                  {busyId === target.id ? "Working..." : "Validate destination"}
                </button>
              ) : null}

              <ul className="mt-2 space-y-1">
                {(authorizations[target.id] ?? []).map((authorization) => (
                  <li
                    key={authorization.id}
                    className="flex flex-wrap items-center justify-between gap-2 rounded border border-pf-border bg-pf-surface px-2 py-1.5 text-[10px] text-pf-text-muted"
                  >
                    <span>
                      {authorization.active
                        ? "Active"
                        : authorization.revokedAt
                          ? "Revoked"
                          : "Expired"}{" "}
                      · {authorization.limits.maxRequests} req ·{" "}
                      {authorization.limits.maxRequestsPerSecond}/s ·{" "}
                      {authorization.limits.maxDurationSeconds}s
                      {authorization.expiresAt
                        ? ` · until ${formatDateTime(authorization.expiresAt)}`
                        : ""}
                    </span>
                    {canWrite && authorization.active ? (
                      <button
                        type="button"
                        onClick={() => void handleRevoke(authorization.id)}
                        disabled={busyId === authorization.id}
                        className="rounded px-1.5 py-0.5 text-[10px] font-medium text-pf-error transition-colors hover:bg-pf-surface-hover disabled:opacity-55"
                      >
                        Revoke
                      </button>
                    ) : null}
                  </li>
                ))}
              </ul>

              {canWrite ? (
                <AuthorizeForm
                  target={target}
                  busy={busyId === target.id}
                  onAuthorize={(limits, hours) => void handleAuthorize(target, limits, hours)}
                />
              ) : null}
            </li>
          ))}
        </ul>
      )}

      {canWrite ? (
        <form
          onSubmit={handleRegister}
          aria-label="Register target"
          className="mt-4 border-t border-pf-border pt-4"
        >
          <p className="text-[11px] font-medium text-pf-text-secondary">Register a target</p>
          <div className="mt-2 space-y-2">
            <input
              aria-label="Target origin"
              required
              value={origin}
              onChange={(event) => setOrigin(event.target.value)}
              placeholder="https://staging.example.test"
              className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] placeholder:text-pf-text-muted focus:border-pf-accent focus:outline-none"
            />
            <div className="grid grid-cols-2 gap-2">
              <select
                aria-label="Target environment"
                value={environment}
                onChange={(event) => setEnvironment(event.target.value as TargetEnvironment)}
                className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-2 text-[12px] focus:border-pf-accent focus:outline-none"
              >
                {(Object.keys(ENVIRONMENT_LABELS) as TargetEnvironment[]).map((value) => (
                  <option key={value} value={value}>
                    {ENVIRONMENT_LABELS[value]}
                  </option>
                ))}
              </select>
              <input
                aria-label="Allowed ports"
                value={ports}
                onChange={(event) => setPorts(event.target.value)}
                placeholder="Ports e.g. 443"
                className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] placeholder:text-pf-text-muted focus:border-pf-accent focus:outline-none"
              />
            </div>
            <input
              aria-label="Allowed hosts"
              value={hosts}
              onChange={(event) => setHosts(event.target.value)}
              placeholder="Allowed hosts (comma separated)"
              className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] placeholder:text-pf-text-muted focus:border-pf-accent focus:outline-none"
            />
            <input
              aria-label="Allowed paths"
              value={paths}
              onChange={(event) => setPaths(event.target.value)}
              placeholder="Allowed paths e.g. /"
              className="h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 font-mono text-[11px] placeholder:text-pf-text-muted focus:border-pf-accent focus:outline-none"
            />
          </div>
          {formError ? (
            <p role="alert" className="mt-2 text-[11px] text-pf-error">
              {formError}
            </p>
          ) : null}
          <button
            type="submit"
            disabled={registering || origin.trim() === ""}
            className="mt-2 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[11px] font-medium transition-colors hover:border-pf-text-muted disabled:opacity-55"
          >
            {registering ? "Registering..." : "Register target"}
          </button>
        </form>
      ) : null}
    </section>
  );
}

function AuthorizeForm({
  target,
  busy,
  onAuthorize,
}: {
  readonly target: Target;
  readonly busy: boolean;
  readonly onAuthorize: (limits: TargetLimits, hours: number | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const [limits, setLimits] = useState<TargetLimits>(DEFAULT_LIMITS);
  const [hours, setHours] = useState("24");

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="mt-2 h-8 rounded-md bg-pf-accent px-3 text-[11px] font-medium text-white transition-colors hover:bg-pf-accent-hover"
      >
        Authorize a scope
      </button>
    );
  }

  return (
    <div className="mt-2 rounded-md border border-pf-border bg-pf-surface px-3 py-3">
      <p className="text-[10px] font-medium text-pf-text-muted">
        Authorizing {target.normalizedOrigin}
      </p>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <label className="text-[10px] text-pf-text-muted">
          Max requests
          <input
            type="number"
            min={1}
            value={limits.maxRequests}
            onChange={(event) =>
              setLimits((current) => ({ ...current, maxRequests: Number(event.target.value) }))
            }
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          />
        </label>
        <label className="text-[10px] text-pf-text-muted">
          Max requests/s
          <input
            type="number"
            min={0.1}
            step={0.1}
            value={limits.maxRequestsPerSecond}
            onChange={(event) =>
              setLimits((current) => ({
                ...current,
                maxRequestsPerSecond: Number(event.target.value),
              }))
            }
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          />
        </label>
        <label className="text-[10px] text-pf-text-muted">
          Max concurrency
          <input
            type="number"
            min={1}
            value={limits.maxConcurrency}
            onChange={(event) =>
              setLimits((current) => ({ ...current, maxConcurrency: Number(event.target.value) }))
            }
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          />
        </label>
        <label className="text-[10px] text-pf-text-muted">
          Max duration (s)
          <input
            type="number"
            min={1}
            value={limits.maxDurationSeconds}
            onChange={(event) =>
              setLimits((current) => ({
                ...current,
                maxDurationSeconds: Number(event.target.value),
              }))
            }
            className="mt-0.5 h-8 w-full rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
          />
        </label>
      </div>
      <label className="mt-2 block text-[10px] text-pf-text-muted">
        Expires in hours (blank = no expiry)
        <input
          value={hours}
          onChange={(event) => setHours(event.target.value)}
          className="mt-0.5 h-8 w-24 rounded border border-pf-border-strong bg-pf-surface px-2 text-[11px] focus:border-pf-accent focus:outline-none"
        />
      </label>
      <div className="mt-2 flex items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            const parsed = hours.trim() === "" ? null : Number.parseFloat(hours);
            onAuthorize(
              limits,
              parsed !== null && Number.isFinite(parsed) && parsed > 0 ? parsed : null,
            );
          }}
          className="h-8 rounded-md bg-pf-accent px-3 text-[11px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
        >
          {busy ? "Authorizing..." : "Authorize"}
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
