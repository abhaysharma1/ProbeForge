import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Intensity, JobGoal } from "@probe-forge/contracts";
import { testCatalog } from "@probe-forge/test-catalog";
import { generatePlan, type PlanGenerationResult } from "./agent/generate";
import { DEFAULT_PLANNING_LIMITS, type PlanningLimits } from "./agent/limits";
import { createPiPlanningRuntime, type PlanningRuntime } from "./agent/model";
import { createPostgresPlanStore } from "./agent/postgres-store";
import { discoverApplication } from "./discovery";
import { SERVICE_NAME } from "./health";
import { createLogger, type Logger } from "./logging/logger";
import { loadPlatformPlanning, mergePlanningLimits } from "./platform-settings";
import type { AuditSink } from "./repository/audit";
import { validateGitUrl } from "./repository/git-url";
import { withRepositorySnapshot } from "./repository/ingest";
import type { IngestionLimits } from "./repository/limits";
import { EnvSecretResolver } from "./repository/secrets";

/**
 * Analysis job loop. It claims one queued job, ingests the pinned repository
 * read-only, runs deterministic discovery, and produces a draft plan. Model
 * execution is optional: with no configured runtime the deterministic fallback
 * planner is used, so this path never contacts a model and never executes
 * repository content or a target.
 *
 * This is an operator-invoked capability (like the ingestion CLI), not an HTTP
 * endpoint. It is exercised end to end by the Phase 15 acceptance test.
 */

export interface JobLoopOptions {
  readonly workerId: string;
  readonly leaseSeconds?: number;
  readonly allowLocalFixtures: boolean;
  readonly allowedGitHosts: readonly string[];
  readonly ingestionLimits: IngestionLimits;
  readonly planningLimits?: PlanningLimits;
  /** Environment fallback when the admin has not configured a provider/model. */
  readonly planningProvider?: string | null;
  readonly planningModel?: string | null;
  readonly audit: AuditSink;
  /** Structured logger. Defaults to a silent sink so callers stay test-safe. */
  readonly logger?: Logger;
}

export interface JobLoopResult {
  readonly jobId: string;
  readonly outcome: "awaiting_approval" | "retry_scheduled" | "failed";
}

interface ClaimedJobRow {
  readonly id: string;
  readonly project_id: string;
  readonly goal: string;
  readonly intensity: string;
  readonly display_name: string;
  readonly source_reference: string;
  readonly normalized_url: string;
}

const DEFAULT_LEASE_SECONDS = 300;
const RETRY_BACKOFF_SECONDS = 5;

function filePathFromUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "file:" ? fileURLToPath(parsed) : null;
  } catch {
    return null;
  }
}

/** A bounded, safe error code for the job row; never the raw error text. */
function jobErrorCode(error: unknown): string {
  if (error !== null && typeof error === "object" && "code" in error) {
    const code = (error as { readonly code?: unknown }).code;
    if (typeof code === "string" && /^[a-z0-9_]{1,120}$/.test(code)) {
      return code;
    }
  }
  return "planning_failed";
}

/**
 * Claim and process at most one queued job. Returns `null` when the queue is
 * empty. The claim commits before ingestion or planning begins.
 */
export async function processJobOnce(
  connectionString: string,
  options: JobLoopOptions,
): Promise<JobLoopResult | null> {
  const sql = new Bun.SQL(connectionString, { max: 4, idleTimeout: 30, connectionTimeout: 10 });
  const store = createPostgresPlanStore(connectionString);
  const lease = options.leaseSeconds ?? DEFAULT_LEASE_SECONDS;
  const logger =
    options.logger ?? createLogger({ service: SERVICE_NAME, level: "error", sink: () => {} });

  try {
    const claimed = await sql<ClaimedJobRow[]>`
      UPDATE jobs AS j
      SET status = 'analyzing',
          worker_id = ${options.workerId},
          attempt = j.attempt + 1,
          heartbeat_at = now(),
          lease_expires_at = now() + make_interval(secs => ${lease}),
          last_error_code = NULL
      FROM (
        SELECT id FROM jobs
        WHERE status = 'queued' AND not_before <= now() AND attempt < max_attempts
        ORDER BY priority DESC, not_before ASC, created_at ASC
        FOR UPDATE SKIP LOCKED
        LIMIT 1
      ) AS candidate, git_sources AS g
      WHERE j.id = candidate.id AND g.id = j.git_source_id
      RETURNING j.id, j.project_id, j.goal, j.intensity, j.display_name,
                j.source_reference, g.normalized_url
    `;

    const job = claimed[0];
    if (job === undefined) return null;

    logger.info("job.claimed", {
      jobId: job.id,
      projectId: job.project_id,
      workerId: options.workerId,
      goal: job.goal,
    });

    const localPath = filePathFromUrl(job.normalized_url);
    const localFixtureRoot = localPath === null ? null : dirname(localPath);
    const target = validateGitUrl(job.normalized_url, {
      allowedHosts: options.allowedGitHosts,
      allowLocalFixtures: options.allowLocalFixtures,
      localFixtureRoot,
    });

    // Admin settings take precedence over the environment; the provider key is
    // decrypted only here and never logged or stored in planning metadata.
    let platform: Awaited<ReturnType<typeof loadPlatformPlanning>> | null = null;
    try {
      platform = await loadPlatformPlanning(sql);
    } catch {
      platform = null;
    }
    const provider = platform?.provider ?? options.planningProvider ?? null;
    const model = platform?.model ?? options.planningModel ?? null;
    const limits = mergePlanningLimits(
      options.planningLimits ?? DEFAULT_PLANNING_LIMITS,
      platform?.limitsOverride ?? {},
    );
    let runtime: PlanningRuntime | null = null;
    if (provider !== null && provider.trim() !== "" && model !== null && model.trim() !== "") {
      try {
        runtime = createPiPlanningRuntime(provider, model, limits, platform?.apiKey ?? null);
      } catch {
        runtime = null;
      }
    }

    let result: PlanGenerationResult;
    try {
      result = await withRepositorySnapshot(
        {
          target,
          reference: job.source_reference,
          credentialRef: null,
          secretResolver: new EnvSecretResolver(process.env),
          audit: options.audit,
          limits: options.ingestionLimits,
          localFixtureRoot,
          allowLocalFixtures: options.allowLocalFixtures,
        },
        async (session) => {
          logger.debug("job.discovery_started", { jobId: job.id });
          const inventory = await discoverApplication(session);
          logger.debug("job.discovery_completed", { jobId: job.id });
          const planResult = await generatePlan(
            {
              job: {
                id: job.id,
                projectId: job.project_id,
                goal: job.goal as JobGoal,
                intensity: job.intensity as Intensity,
                displayName: job.display_name,
              },
              repository: session,
              inventory,
              catalog: testCatalog,
            },
            {
              runtime,
              store,
              limits,
            },
          );
          logger.debug("job.plan_generated", {
            jobId: job.id,
            planOutcome: planResult.outcome,
          });
          return planResult;
        },
      );
    } catch (error) {
      const errorCode = jobErrorCode(error);
      const rows = await sql<{ status: string }[]>`
        UPDATE jobs
        SET status = CASE WHEN attempt < max_attempts THEN 'queued' ELSE 'failed' END,
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            not_before = now() + make_interval(secs => ${RETRY_BACKOFF_SECONDS}),
            last_error_code = ${errorCode}
        WHERE id = ${job.id} AND worker_id = ${options.workerId}
        RETURNING status
      `;
      const outcome = rows[0]?.status === "queued" ? "retry_scheduled" : "failed";
      logger.warn("job.processing_failed", {
        jobId: job.id,
        workerId: options.workerId,
        errorCode,
        outcome,
      });
      return { jobId: job.id, outcome };
    }

    if (result.outcome === "planned" || result.outcome === "fallback") {
      const planId = result.savedPlan?.id ?? null;
      await sql`
        UPDATE jobs
        SET status = 'awaiting_approval',
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = NULL,
            current_plan_id = COALESCE(${planId}, current_plan_id)
        WHERE id = ${job.id} AND worker_id = ${options.workerId}
      `;
      logger.info("job.awaiting_approval", {
        jobId: job.id,
        workerId: options.workerId,
        planId,
        planOutcome: result.outcome,
      });
      return { jobId: job.id, outcome: "awaiting_approval" };
    }

    const rows = await sql<{ status: string }[]>`
      UPDATE jobs
      SET status = CASE WHEN attempt < max_attempts THEN 'queued' ELSE 'failed' END,
          worker_id = NULL,
          lease_expires_at = NULL,
          heartbeat_at = NULL,
          not_before = now() + make_interval(secs => ${RETRY_BACKOFF_SECONDS}),
          last_error_code = ${result.failureCode ?? "planning_failed"}
      WHERE id = ${job.id} AND worker_id = ${options.workerId}
      RETURNING status
    `;
    const outcome = rows[0]?.status === "queued" ? "retry_scheduled" : "failed";
    logger.warn("job.plan_not_produced", {
      jobId: job.id,
      workerId: options.workerId,
      errorCode: result.failureCode ?? "planning_failed",
      outcome,
    });
    return { jobId: job.id, outcome };
  } finally {
    await store.close();
    await sql.close({ timeout: 5 });
  }
}
