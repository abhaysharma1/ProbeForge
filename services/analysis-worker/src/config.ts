export interface WorkerConfig {
  readonly hostname: string;
  readonly port: number;
}

export class WorkerConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WorkerConfigError";
  }
}

const DEFAULT_HOSTNAME = "127.0.0.1";
const DEFAULT_PORT = 8090;

/**
 * The analysis worker binds to loopback by default. Phase 1 does not expose
 * planning tools or repository access yet; it only serves health probes.
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

  return { hostname, port };
}
