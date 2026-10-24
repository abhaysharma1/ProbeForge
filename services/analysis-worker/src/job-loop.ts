import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Intensity, JobGoal } from "@probe-forge/contracts";
import { testCatalog } from "@probe-forge/test-catalog";
import { generatePlan, type PlanGenerationResult } from "./agent/generate";
import { DEFAULT_PLANNING_LIMITS, type PlanningLimits } from "./agent/limits";
import { createPostgresPlanStore } from "./agent/postgres-store";
import { discoverApplication } from "./discovery";
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
  readonly audit: AuditSink;
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

    const localPath = filePathFromUrl(job.normalized_url);
    const localFixtureRoot = localPath === null ? null : dirname(localPath);
    const target = validateGitUrl(job.normalized_url, {
      allowedHosts: options.allowedGitHosts,
      allowLocalFixtures: options.allowLocalFixtures,
      localFixtureRoot,
    });

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
          const inventory = await discoverApplication(session);
          return await generatePlan(
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
              runtime: null,
              store,
              limits: options.planningLimits ?? DEFAULT_PLANNING_LIMITS,
            },
          );
        },
      );
    } catch (error) {
      const rows = await sql<{ status: string }[]>`
        UPDATE jobs
        SET status = CASE WHEN attempt < max_attempts THEN 'queued' ELSE 'failed' END,
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            not_before = now() + make_interval(secs => ${RETRY_BACKOFF_SECONDS}),
            last_error_code = ${jobErrorCode(error)}
        WHERE id = ${job.id} AND worker_id = ${options.workerId}
        RETURNING status
      `;
      return {
        jobId: job.id,
        outcome: rows[0]?.status === "queued" ? "retry_scheduled" : "failed",
      };
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
    return {
      jobId: job.id,
      outcome: rows[0]?.status === "queued" ? "retry_scheduled" : "failed",
    };
  } finally {
    await store.close();
    await sql.close({ timeout: 5 });
  }
}
