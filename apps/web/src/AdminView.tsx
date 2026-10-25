import { type FormEvent, useCallback, useEffect, useState } from "react";
import {
  type AdminSettings,
  ApiError,
  type AuthUser,
  clearPlanningApiKey,
  getAdminSettings,
  updateAdminSettings,
} from "./api";

interface AdminViewProps {
  readonly user: AuthUser;
  readonly csrfToken: string;
  readonly onBack: () => void;
  readonly onSignOut: () => Promise<void>;
  readonly onSessionExpired: () => void;
}

interface Form {
  provider: string;
  model: string;
  baseUrl: string;
  contextWindow: string;
  maxOutputTokens: string;
  maxModelCalls: string;
  maxTurns: string;
  maxResponseTokens: string;
  maxWallTimeMs: string;
  maxPlannedTests: string;
  maxLoadScenarios: string;
  apiKey: string;
  apiRequestsPerMinute: string;
  expensiveRequestsPerMinute: string;
  retentionEventDays: string;
  retentionReportDays: string;
  retentionCounterDays: string;
  defaultMaxActiveRuns: string;
  defaultMaxRequestsPerWindow: string;
  defaultWindowSeconds: string;
}

const LIMIT_FIELDS: ReadonlyArray<{ key: keyof Form; limit: string; label: string }> = [
  { key: "maxModelCalls", limit: "maxModelCalls", label: "Max model calls" },
  { key: "maxTurns", limit: "maxTurns", label: "Max turns" },
  { key: "maxResponseTokens", limit: "maxResponseTokens", label: "Max response tokens" },
  { key: "maxWallTimeMs", limit: "maxWallTimeMs", label: "Max wall time (ms)" },
  { key: "maxPlannedTests", limit: "maxPlannedTests", label: "Max planned tests" },
  { key: "maxLoadScenarios", limit: "maxLoadScenarios", label: "Max load scenarios" },
];

function numberOr(value: string, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function toForm(settings: AdminSettings): Form {
  const limits = settings.planning.limits;
  return {
    provider: settings.planning.provider ?? "",
    model: settings.planning.model ?? "",
    baseUrl: settings.planning.baseUrl ?? "",
    contextWindow:
      settings.planning.contextWindow === null ? "" : String(settings.planning.contextWindow),
    maxOutputTokens:
      settings.planning.maxOutputTokens === null ? "" : String(settings.planning.maxOutputTokens),
    maxModelCalls: String(limits.maxModelCalls ?? ""),
    maxTurns: String(limits.maxTurns ?? ""),
    maxResponseTokens: String(limits.maxResponseTokens ?? ""),
    maxWallTimeMs: String(limits.maxWallTimeMs ?? ""),
    maxPlannedTests: String(limits.maxPlannedTests ?? ""),
    maxLoadScenarios: String(limits.maxLoadScenarios ?? ""),
    apiKey: "",
    apiRequestsPerMinute: String(settings.rateLimits.apiRequestsPerMinute),
    expensiveRequestsPerMinute: String(settings.rateLimits.expensiveRequestsPerMinute),
    retentionEventDays: String(settings.retention.eventDays),
    retentionReportDays: String(settings.retention.reportDays),
    retentionCounterDays: String(settings.retention.counterDays),
    defaultMaxActiveRuns: String(settings.projectDefaults.maxActiveRuns),
    defaultMaxRequestsPerWindow:
      settings.projectDefaults.maxRequestsPerWindow === null
        ? ""
        : String(settings.projectDefaults.maxRequestsPerWindow),
    defaultWindowSeconds: String(settings.projectDefaults.windowSeconds),
  };
}

function message(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "The request could not be completed. Check the API connection and try again.";
}

function Field({
  id,
  label,
  value,
  onChange,
  type = "number",
  placeholder,
}: {
  readonly id: string;
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly type?: string;
  readonly placeholder?: string;
}) {
  return (
    <label htmlFor={id} className="block text-[11px] text-pf-text-secondary">
      {label}
      <input
        id={id}
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        className="mt-1 h-9 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
      />
    </label>
  );
}

export function AdminView({
  user,
  csrfToken,
  onBack,
  onSignOut,
  onSessionExpired,
}: AdminViewProps) {
  const [settings, setSettings] = useState<AdminSettings | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

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

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await getAdminSettings();
      setSettings(next);
      setForm(toForm(next));
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setLoading(false);
    }
  }, [handleError]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!user.isAdmin) {
    return (
      <main className="grid min-h-screen place-items-center bg-pf-background px-6">
        <div className="w-full max-w-md rounded-lg border border-pf-border bg-pf-surface p-6 text-center">
          <h1 className="text-[18px] font-semibold">Platform administrator access required</h1>
          <p role="alert" className="mt-2 text-[13px] text-pf-text-secondary">
            This account is not a platform administrator.
          </p>
          <button
            type="button"
            onClick={onBack}
            className="mt-4 h-9 rounded-md border border-pf-border-strong bg-pf-surface px-3.5 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
          >
            Back to projects
          </button>
        </div>
      </main>
    );
  }

  function update<K extends keyof Form>(key: K, value: string) {
    setForm((current) => (current === null ? current : { ...current, [key]: value }));
    setNotice(null);
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (form === null || settings === null) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const planningLimits: Record<string, number> = { ...settings.planning.limits };
      for (const field of LIMIT_FIELDS) {
        const raw = form[field.key];
        if (raw.trim() === "") {
          delete planningLimits[field.limit];
        } else {
          planningLimits[field.limit] = numberOr(raw, 0);
        }
      }
      const updated = await updateAdminSettings(
        {
          planningProvider: form.provider.trim() === "" ? null : form.provider.trim(),
          planningModel: form.model.trim() === "" ? null : form.model.trim(),
          planningBaseUrl: form.baseUrl.trim() === "" ? null : form.baseUrl.trim(),
          planningContextWindow:
            form.contextWindow.trim() === "" ? null : numberOr(form.contextWindow, 32768),
          planningMaxOutputTokens:
            form.maxOutputTokens.trim() === "" ? null : numberOr(form.maxOutputTokens, 4096),
          planningLimits,
          ...(form.apiKey.trim() === "" ? {} : { apiKey: form.apiKey.trim() }),
          apiRequestsPerMinute: numberOr(form.apiRequestsPerMinute, 240),
          expensiveRequestsPerMinute: numberOr(form.expensiveRequestsPerMinute, 20),
          retentionEventDays: numberOr(form.retentionEventDays, 30),
          retentionReportDays: numberOr(form.retentionReportDays, 180),
          retentionCounterDays: numberOr(form.retentionCounterDays, 7),
          defaultMaxActiveRuns: numberOr(form.defaultMaxActiveRuns, 2),
          defaultMaxRequestsPerWindow:
            form.defaultMaxRequestsPerWindow.trim() === ""
              ? null
              : numberOr(form.defaultMaxRequestsPerWindow, 0),
          defaultWindowSeconds: numberOr(form.defaultWindowSeconds, 86400),
        },
        csrfToken,
      );
      setSettings(updated);
      setForm(toForm(updated));
      setNotice("Platform settings saved.");
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setSaving(false);
    }
  }

  async function handleClearKey() {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const updated = await clearPlanningApiKey(csrfToken);
      setSettings(updated);
      setForm(toForm(updated));
      setNotice("Provider API key cleared.");
    } catch (cause) {
      const text = handleError(cause);
      if (text) {
        setError(text);
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="mx-auto min-h-screen max-w-[900px] px-6 py-8">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <button
            type="button"
            onClick={onBack}
            className="text-[12px] font-medium text-pf-text-secondary transition-colors hover:text-pf-accent"
          >
            ← Back to projects
          </button>
          <p className="mt-4 font-mono text-[10px] font-medium tracking-[0.14em] text-pf-text-muted">
            PLATFORM
          </p>
          <h1 className="mt-1 text-[26px] font-semibold tracking-[-0.02em]">Platform settings</h1>
          <p className="mt-1.5 max-w-xl text-[13px] text-pf-text-secondary">
            Global configuration for planning (Pi), API rate limits, retention, and project
            defaults. These settings apply to every project.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="hidden text-right sm:block">
            <span className="block text-[12px] font-medium">{user.displayName}</span>
            <span className="block text-[11px] text-pf-text-muted">Administrator</span>
          </span>
          <button
            type="button"
            onClick={() => void onSignOut()}
            className="h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium text-pf-text-secondary transition-colors hover:border-pf-text-muted"
          >
            Sign out
          </button>
        </div>
      </div>

      {loading ? (
        <div
          role="status"
          className="mt-6 rounded-lg border border-pf-border bg-pf-surface px-4 py-12 text-center text-[13px] text-pf-text-secondary"
        >
          Loading platform settings...
        </div>
      ) : error && form === null ? (
        <div className="mt-6 rounded-lg border border-[#e8c9c4] bg-[#fbf1ef] px-4 py-5">
          <p role="alert" className="text-[13px] text-pf-error">
            {error}
          </p>
          <button
            type="button"
            onClick={() => void load()}
            className="mt-3 h-8 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium transition-colors hover:border-pf-text-muted"
          >
            Retry
          </button>
        </div>
      ) : form === null ? null : (
        <form onSubmit={handleSave} className="mt-6 space-y-6" aria-label="Platform settings">
          <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
            <h2 className="text-[14px] font-semibold">Planning (Pi)</h2>
            <p className="mt-1 text-[12px] text-pf-text-secondary">
              Without a base URL, the provider and model are validated against the pinned SDK
              catalog when a job runs. With a base URL, the endpoint is registered as a custom
              OpenAI-compatible provider. With none set, the deterministic fallback planner is used.
            </p>
            <div className="mt-4 grid gap-4 sm:grid-cols-2">
              <Field
                id="planning-provider"
                label="Provider"
                type="text"
                value={form.provider}
                placeholder="anthropic"
                onChange={(value) => update("provider", value)}
              />
              <Field
                id="planning-model"
                label="Model"
                type="text"
                value={form.model}
                placeholder="claude-sonnet-4-6"
                onChange={(value) => update("model", value)}
              />
            </div>
            <div className="mt-4 border-t border-pf-border pt-4">
              <Field
                id="planning-base-url"
                label="Base URL (custom OpenAI-compatible endpoint)"
                type="url"
                value={form.baseUrl}
                placeholder="https://api.example.com/v1"
                onChange={(value) => update("baseUrl", value)}
              />
              <p className="mt-1.5 text-[11px] text-pf-text-muted">
                Leave empty to use a pinned Pi SDK provider. When set, the worker registers this
                HTTPS endpoint as a custom OpenAI-compatible provider for the given provider id; it
                takes precedence over a built-in provider of the same id. Internal and metadata
                hosts are rejected.
              </p>
              <div className="mt-4 grid gap-4 sm:grid-cols-2">
                <Field
                  id="planning-context-window"
                  label="Context window (tokens)"
                  value={form.contextWindow}
                  placeholder="32768"
                  onChange={(value) => update("contextWindow", value)}
                />
                <Field
                  id="planning-max-output-tokens"
                  label="Max output tokens"
                  value={form.maxOutputTokens}
                  placeholder="4096"
                  onChange={(value) => update("maxOutputTokens", value)}
                />
              </div>
            </div>
            <div className="mt-4 grid gap-4 sm:grid-cols-3">
              {LIMIT_FIELDS.map((field) => (
                <Field
                  key={field.key}
                  id={`limit-${field.limit}`}
                  label={field.label}
                  value={form[field.key]}
                  onChange={(value) => update(field.key, value)}
                />
              ))}
            </div>
            <div className="mt-4 border-t border-pf-border pt-4">
              <label
                htmlFor="planning-api-key"
                className="block text-[11px] text-pf-text-secondary"
              >
                Provider API key
              </label>
              <div className="mt-1 flex flex-wrap items-center gap-2">
                <input
                  id="planning-api-key"
                  type="password"
                  value={form.apiKey}
                  autoComplete="off"
                  placeholder={
                    settings?.planning.apiKeyConfigured
                      ? "Configured — enter to replace"
                      : "Not configured"
                  }
                  onChange={(event) => update("apiKey", event.target.value)}
                  className="h-9 min-w-0 flex-1 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] placeholder:text-pf-text-muted focus:border-pf-accent focus:outline-none"
                />
                {settings?.planning.apiKeyConfigured ? (
                  <button
                    type="button"
                    onClick={() => void handleClearKey()}
                    disabled={saving}
                    className="h-9 rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[12px] font-medium text-pf-error transition-colors hover:border-pf-error disabled:opacity-55"
                  >
                    Clear key
                  </button>
                ) : null}
              </div>
              <p className="mt-1.5 text-[11px] text-pf-text-muted">
                {settings?.planning.apiKeyConfigured
                  ? `Stored as encrypted ciphertext (${settings.planning.apiKeyHint ?? "••••"}). The full key is never shown again.`
                  : "Stored encrypted at rest. It is write-only and never returned."}
                {settings?.settingsKeyConfigured === false
                  ? " Set PROBE_FORGE_SETTINGS_KEY to store a key."
                  : ""}
              </p>
            </div>
          </section>

          <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
            <h2 className="text-[14px] font-semibold">API rate limits</h2>
            <p className="mt-1 text-[12px] text-pf-text-secondary">
              Per authenticated user, fixed 60-second window. Applied live to new requests.
            </p>
            <div className="mt-4 grid gap-4 sm:grid-cols-2">
              <Field
                id="api-rate"
                label="API requests / minute"
                value={form.apiRequestsPerMinute}
                onChange={(value) => update("apiRequestsPerMinute", value)}
              />
              <Field
                id="expensive-rate"
                label="Expensive requests / minute"
                value={form.expensiveRequestsPerMinute}
                onChange={(value) => update("expensiveRequestsPerMinute", value)}
              />
            </div>
          </section>

          <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
            <h2 className="text-[14px] font-semibold">Retention</h2>
            <p className="mt-1 text-[12px] text-pf-text-secondary">
              The scheduler purges expired data on its retention interval.
            </p>
            <div className="mt-4 grid gap-4 sm:grid-cols-3">
              <Field
                id="retention-events"
                label="Job events (days)"
                value={form.retentionEventDays}
                onChange={(value) => update("retentionEventDays", value)}
              />
              <Field
                id="retention-reports"
                label="Reports (days)"
                value={form.retentionReportDays}
                onChange={(value) => update("retentionReportDays", value)}
              />
              <Field
                id="retention-counters"
                label="Counters/sessions (days)"
                value={form.retentionCounterDays}
                onChange={(value) => update("retentionCounterDays", value)}
              />
            </div>
          </section>

          <section className="rounded-lg border border-pf-border bg-pf-surface px-5 py-4">
            <h2 className="text-[14px] font-semibold">Project defaults</h2>
            <p className="mt-1 text-[12px] text-pf-text-secondary">
              Defaults used when a project has no explicit quota.
            </p>
            <div className="mt-4 grid gap-4 sm:grid-cols-3">
              <Field
                id="default-runs"
                label="Max active runs"
                value={form.defaultMaxActiveRuns}
                onChange={(value) => update("defaultMaxActiveRuns", value)}
              />
              <Field
                id="default-budget"
                label="Max requests / window"
                value={form.defaultMaxRequestsPerWindow}
                placeholder="unlimited"
                onChange={(value) => update("defaultMaxRequestsPerWindow", value)}
              />
              <Field
                id="default-window"
                label="Window seconds"
                value={form.defaultWindowSeconds}
                onChange={(value) => update("defaultWindowSeconds", value)}
              />
            </div>
          </section>

          {error ? (
            <p role="alert" className="text-[12px] text-pf-error">
              {error}
            </p>
          ) : null}
          {notice ? (
            <p role="status" className="text-[12px] text-pf-success">
              {notice}
            </p>
          ) : null}

          <div className="flex items-center gap-3">
            <button
              type="submit"
              disabled={saving}
              className="h-9 rounded-md bg-pf-accent px-4 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:opacity-55"
            >
              {saving ? "Saving..." : "Save settings"}
            </button>
            <span className="text-[11px] text-pf-text-muted">
              {settings
                ? `Last updated ${new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(settings.updatedAt))}`
                : null}
            </span>
          </div>
        </form>
      )}
    </div>
  );
}
