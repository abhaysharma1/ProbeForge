import {
  DEFAULT_INGESTION_LIMITS,
  type IngestionLimits,
  validateIngestionLimits,
} from "./repository/limits";

export interface IngestionConfig {
  readonly limits: IngestionLimits;
  readonly allowedGitHosts: readonly string[];
  readonly allowLocalFixtures: boolean;
  readonly auditLogPath: string | null;
}

export interface WorkerConfig {
  readonly hostname: string;
  readonly port: number;
  readonly ingestion: IngestionConfig;
}

export class WorkerConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WorkerConfigError";
  }
}

const DEFAULT_HOSTNAME = "127.0.0.1";
const DEFAULT_PORT = 8090;
const DEFAULT_ALLOWED_GIT_HOSTS = ["github.com", "gitlab.com", "bitbucket.org"] as const;

function readInteger(
  env: Record<string, string | undefined>,
  name: string,
  fallback: number,
  minimum: number,
  maximum: number,
): number {
  const raw = (env[name] ?? `${fallback}`).trim();
  if (!/^\d+$/.test(raw)) {
    throw new WorkerConfigError(`${name} must be an integer, received "${raw}"`);
  }
  const value = Number.parseInt(raw, 10);
  if (value < minimum || value > maximum) {
    throw new WorkerConfigError(
      `${name} must be between ${minimum} and ${maximum}, received ${value}`,
    );
  }
  return value;
}

function readBoolean(
  env: Record<string, string | undefined>,
  name: string,
  fallback: boolean,
): boolean {
  const raw = env[name];
  if (raw === undefined || raw.trim().length === 0) return fallback;
  if (raw === "true" || raw === "1") return true;
  if (raw === "false" || raw === "0") return false;
  throw new WorkerConfigError(`${name} must be true or false`);
}

function readAllowedGitHosts(env: Record<string, string | undefined>): readonly string[] {
  const raw = env.PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS;
  const hosts = (raw === undefined ? DEFAULT_ALLOWED_GIT_HOSTS : raw.split(","))
    .map((host) => host.trim().toLowerCase().replace(/\.$/, ""))
    .filter((host) => host.length > 0);
  if (hosts.length === 0) {
    throw new WorkerConfigError("PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS must not be empty");
  }
  const unique = [...new Set(hosts)];
  for (const host of unique) {
    if (!/^[a-z0-9](?:[a-z0-9.-]{0,251}[a-z0-9])?$/.test(host)) {
      throw new WorkerConfigError(
        "PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS contains an invalid hostname",
      );
    }
  }
  return unique;
}

/**
 * The analysis worker binds to loopback by default. Phase 5 adds read-only
 * ingestion limits and a host allowlist; it still exposes only health probes.
 */
export function loadConfig(env: Record<string, string | undefined> = Bun.env): WorkerConfig {
  const hostname = (env.PROBE_FORGE_ANALYSIS_WORKER_HOST ?? DEFAULT_HOSTNAME).trim();
  if (hostname.length === 0) {
    throw new WorkerConfigError("PROBE_FORGE_ANALYSIS_WORKER_HOST must not be empty");
  }

  const rawPort = (env.PROBE_FORGE_ANALYSIS_WORKER_PORT ?? `${DEFAULT_PORT}`).trim();
  if (!/^\d+$/.test(rawPort)) {
    throw new WorkerConfigError(
      `PROBE_FORGE_ANALYSIS_WORKER_PORT must be an integer, received "${rawPort}"`,
    );
  }

  const port = Number.parseInt(rawPort, 10);
  if (port < 1 || port > 65535) {
    throw new WorkerConfigError(
      `PROBE_FORGE_ANALYSIS_WORKER_PORT must be between 1 and 65535, received ${port}`,
    );
  }

  const limits: IngestionLimits = {
    maxFiles: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_FILES",
      DEFAULT_INGESTION_LIMITS.maxFiles,
      1,
      50_000,
    ),
    maxTotalBytes: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_TOTAL_BYTES",
      DEFAULT_INGESTION_LIMITS.maxTotalBytes,
      1,
      1024 * 1024 * 1024,
    ),
    maxFileBytes: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_FILE_BYTES",
      DEFAULT_INGESTION_LIMITS.maxFileBytes,
      1,
      64 * 1024 * 1024,
    ),
    maxDepth: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_DEPTH",
      DEFAULT_INGESTION_LIMITS.maxDepth,
      1,
      128,
    ),
    maxWorkspaceBytes: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_WORKSPACE_BYTES",
      DEFAULT_INGESTION_LIMITS.maxWorkspaceBytes,
      1,
      2 * 1024 * 1024 * 1024,
    ),
    maxGitOutputBytes: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_GIT_OUTPUT_BYTES",
      DEFAULT_INGESTION_LIMITS.maxGitOutputBytes,
      1,
      64 * 1024 * 1024,
    ),
    gitTimeoutMs: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_GIT_TIMEOUT_MS",
      DEFAULT_INGESTION_LIMITS.gitTimeoutMs,
      1_000,
      600_000,
    ),
    maxInventoryFiles: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_INVENTORY_FILES",
      DEFAULT_INGESTION_LIMITS.maxInventoryFiles,
      1,
      10_000,
    ),
    maxDependenciesPerManifest: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_MAX_DEPENDENCIES_PER_MANIFEST",
      DEFAULT_INGESTION_LIMITS.maxDependenciesPerManifest,
      1,
      1_000,
    ),
  };
  try {
    validateIngestionLimits(limits);
  } catch (error) {
    throw new WorkerConfigError(
      error instanceof Error ? error.message : "Invalid ingestion limits",
    );
  }

  const auditLogPathRaw = env.PROBE_FORGE_ANALYSIS_AUDIT_LOG?.trim();
  return {
    hostname,
    port,
    ingestion: {
      limits,
      allowedGitHosts: readAllowedGitHosts(env),
      allowLocalFixtures: readBoolean(env, "PROBE_FORGE_ANALYSIS_ALLOW_LOCAL_FIXTURES", false),
      auditLogPath:
        auditLogPathRaw === undefined || auditLogPathRaw.length === 0 ? null : auditLogPathRaw,
    },
  };
}
