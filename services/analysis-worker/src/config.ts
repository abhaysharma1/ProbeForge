import {
  DEFAULT_PLANNING_LIMITS,
  type PlanningLimits,
  validatePlanningLimits,
} from "./agent/limits";
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

export interface PlanningConfig {
  readonly provider: string | null;
  readonly model: string | null;
  readonly limits: PlanningLimits;
}

export interface WorkerConfig {
  readonly hostname: string;
  readonly port: number;
  readonly ingestion: IngestionConfig;
  readonly planning: PlanningConfig;
  /** Background job loop: claim queued jobs and produce a draft plan. */
  readonly jobLoop: boolean;
  readonly jobLoopIntervalMs: number;
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
const PROVIDER_PATTERN = /^[a-z0-9][a-z0-9._-]{0,79}$/;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,159}$/;

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

function readOptionalPlanningIdentifier(
  env: Record<string, string | undefined>,
  name: string,
  pattern: RegExp,
): string | null {
  const raw = env[name]?.trim() ?? "";
  if (raw.length === 0) return null;
  if (!pattern.test(raw)) {
    throw new WorkerConfigError(`${name} is not a valid planning identifier`);
  }
  return raw;
}

function readPlanningConfig(env: Record<string, string | undefined>): PlanningConfig {
  const provider = readOptionalPlanningIdentifier(
    env,
    "PROBE_FORGE_PLANNING_PROVIDER",
    PROVIDER_PATTERN,
  );
  const model = readOptionalPlanningIdentifier(env, "PROBE_FORGE_PLANNING_MODEL", MODEL_PATTERN);
  if ((provider === null) !== (model === null)) {
    throw new WorkerConfigError(
      "PROBE_FORGE_PLANNING_PROVIDER and PROBE_FORGE_PLANNING_MODEL must be set together",
    );
  }

  const limits: PlanningLimits = {
    maxModelCalls: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_MODEL_CALLS",
      DEFAULT_PLANNING_LIMITS.maxModelCalls,
      1,
      20,
    ),
    maxToolCalls: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_TOOL_CALLS",
      DEFAULT_PLANNING_LIMITS.maxToolCalls,
      0,
      100,
    ),
    maxTurns: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_TURNS",
      DEFAULT_PLANNING_LIMITS.maxTurns,
      1,
      20,
    ),
    maxRetries: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_RETRIES",
      DEFAULT_PLANNING_LIMITS.maxRetries,
      0,
      5,
    ),
    maxResponseTokens: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_RESPONSE_TOKENS",
      DEFAULT_PLANNING_LIMITS.maxResponseTokens,
      128,
      64_000,
    ),
    maxWallTimeMs: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_WALL_TIME_MS",
      DEFAULT_PLANNING_LIMITS.maxWallTimeMs,
      1_000,
      900_000,
    ),
    maxPromptBytes: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_PROMPT_BYTES",
      DEFAULT_PLANNING_LIMITS.maxPromptBytes,
      1_024,
      256 * 1024,
    ),
    maxToolOutputBytes: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_TOOL_OUTPUT_BYTES",
      DEFAULT_PLANNING_LIMITS.maxToolOutputBytes,
      256,
      128 * 1024,
    ),
    maxTotalToolOutputBytes: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_TOTAL_TOOL_OUTPUT_BYTES",
      DEFAULT_PLANNING_LIMITS.maxTotalToolOutputBytes,
      256,
      512 * 1024,
    ),
    maxPlanBytes: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_PLAN_BYTES",
      DEFAULT_PLANNING_LIMITS.maxPlanBytes,
      1_024,
      256 * 1024,
    ),
    maxRepositoryReadBytes: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_REPOSITORY_READ_BYTES",
      DEFAULT_PLANNING_LIMITS.maxRepositoryReadBytes,
      256,
      64 * 1024,
    ),
    maxCatalogEntries: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_CATALOG_ENTRIES",
      DEFAULT_PLANNING_LIMITS.maxCatalogEntries,
      0,
      1_000,
    ),
    maxPlannedTests: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_PLANNED_TESTS",
      DEFAULT_PLANNING_LIMITS.maxPlannedTests,
      0,
      100,
    ),
    maxLoadScenarios: readInteger(
      env,
      "PROBE_FORGE_PLANNING_MAX_LOAD_SCENARIOS",
      DEFAULT_PLANNING_LIMITS.maxLoadScenarios,
      0,
      50,
    ),
  };
  try {
    validatePlanningLimits(limits);
  } catch (error) {
    throw new WorkerConfigError(error instanceof Error ? error.message : "Invalid planning limits");
  }
  return { provider, model, limits };
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
    planning: readPlanningConfig(env),
    jobLoop: readBoolean(env, "PROBE_FORGE_ANALYSIS_JOB_LOOP", false),
    jobLoopIntervalMs: readInteger(
      env,
      "PROBE_FORGE_ANALYSIS_JOB_LOOP_INTERVAL_MS",
      5_000,
      1_000,
      300_000,
    ),
  };
}
