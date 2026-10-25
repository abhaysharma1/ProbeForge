import { DEFAULT_PLANNING_LIMITS } from "./agent/limits";
import { loadConfig } from "./config";
import { healthResponse, readinessResponse, SERVICE_NAME } from "./health";
import { describeError, processJobOnce } from "./job-loop";
import { createLogger } from "./logging/logger";
import { type AuditSink, JsonLineAuditSink, MemoryAuditSink } from "./repository/audit";

const config = loadConfig();

const logger = createLogger({
  service: SERVICE_NAME,
  level: config.logLevel,
  format: config.logFormat,
});

let shuttingDown = false;

const server = Bun.serve({
  hostname: config.hostname,
  port: config.port,
  fetch(request) {
    const { pathname } = new URL(request.url);

    if (request.method === "GET" && pathname === "/healthz") {
      return Response.json(healthResponse());
    }
    if (request.method === "GET" && pathname === "/readyz") {
      return Response.json(readinessResponse());
    }
    return Response.json({ error: "not_found" }, { status: 404 });
  },
});

logger.info("listening", { address: `http://${server.hostname}:${server.port}` });

/**
 * Background job loop: claim queued jobs and produce a draft plan. It is
 * disabled unless PROBE_FORGE_ANALYSIS_JOB_LOOP=true, so tests and one-off runs
 * keep full control. With no planning model configured it uses the
 * deterministic fallback planner; it never executes repository code or a
 * target.
 */
async function runJobLoop(): Promise<void> {
  const connectionString = process.env.DATABASE_URL?.trim();
  if (connectionString === undefined || connectionString.length === 0) {
    logger.warn("job_loop_disabled", { reason: "DATABASE_URL is not set" });
    return;
  }
  const audit: AuditSink =
    config.ingestion.auditLogPath === null
      ? new MemoryAuditSink()
      : new JsonLineAuditSink(config.ingestion.auditLogPath);
  logger.info("job_loop_started", { intervalMs: config.jobLoopIntervalMs });

  while (!shuttingDown) {
    try {
      const result = await processJobOnce(connectionString, {
        workerId: (process.env.PROBE_FORGE_ANALYSIS_WORKER_ID ?? "analysis-worker-1").trim(),
        allowLocalFixtures: config.ingestion.allowLocalFixtures,
        allowedGitHosts: config.ingestion.allowedGitHosts,
        ingestionLimits: config.ingestion.limits,
        planningLimits: DEFAULT_PLANNING_LIMITS,
        planningProvider: config.planning.provider,
        planningModel: config.planning.model,
        audit,
        logger,
      });
      if (result !== null) {
        logger.info("job_processed", { ...result });
      }
    } catch (error) {
      logger.warn("job_loop_error", {
        message: error instanceof Error ? error.message : "unexpected error",
        // [TEMP DEBUG] Surface the full error for loop-level failures such as an
        // invalid Git URL, which bypasses the job reschedule path.
        ...describeError(error),
      });
    }
    if (!shuttingDown) {
      await Bun.sleep(config.jobLoopIntervalMs);
    }
  }
}

if (config.jobLoop) {
  void runJobLoop();
}

async function shutdown(signal: string): Promise<void> {
  if (shuttingDown) return;
  shuttingDown = true;
  logger.info("shutdown", { signal });
  await server.stop(true);
  process.exit(0);
}

process.on("SIGINT", () => void shutdown("SIGINT"));
process.on("SIGTERM", () => void shutdown("SIGTERM"));
