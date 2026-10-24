import { DEFAULT_PLANNING_LIMITS } from "./agent/limits";
import { loadConfig } from "./config";
import { processJobOnce } from "./job-loop";
import { type AuditSink, JsonLineAuditSink, MemoryAuditSink } from "./repository/audit";

/**
 * Operator-invoked analysis job loop. It processes every currently-queued job
 * (bounded by a small iteration cap) and exits. It never contacts a target and
 * never executes repository code; with no planning model configured it uses the
 * deterministic fallback planner.
 *
 *   bun run plan
 */
const MAX_ITERATIONS = 100;

const connectionString = process.env.DATABASE_URL?.trim();
if (connectionString === undefined || connectionString.length === 0) {
  process.stderr.write(
    `${JSON.stringify({ error: { code: "invalid_config", message: "DATABASE_URL must be set" } })}\n`,
  );
  process.exit(2);
}

const config = loadConfig();
const workerId = (process.env.PROBE_FORGE_ANALYSIS_WORKER_ID ?? "analysis-worker-1").trim();
const audit: AuditSink =
  config.ingestion.auditLogPath === null
    ? new MemoryAuditSink()
    : new JsonLineAuditSink(config.ingestion.auditLogPath);

let processed = 0;
while (processed < MAX_ITERATIONS) {
  const result = await processJobOnce(connectionString, {
    workerId,
    allowLocalFixtures: config.ingestion.allowLocalFixtures,
    allowedGitHosts: config.ingestion.allowedGitHosts,
    ingestionLimits: config.ingestion.limits,
    planningLimits: DEFAULT_PLANNING_LIMITS,
    audit,
  });
  if (result === null) break;
  processed += 1;
  process.stdout.write(
    `${JSON.stringify({ level: "info", service: "analysis-worker", event: "job_processed", ...result })}\n`,
  );
}

process.stdout.write(
  `${JSON.stringify({ level: "info", service: "analysis-worker", event: "loop_idle", processed })}\n`,
);
