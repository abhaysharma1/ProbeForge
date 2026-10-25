import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Intensity, JobEventType, JobGoal } from "@probe-forge/contracts";
import { testCatalog } from "@probe-forge/test-catalog";
import type { SQL } from "bun";
import { generatePlan, type PlanGenerationResult } from "./agent/generate";
import { DEFAULT_PLANNING_LIMITS, type PlanningLimits } from "./agent/limits";
import {
  CUSTOM_PROVIDER_DEFAULTS,
  type CustomOpenAiCompatConfig,
  createPiPlanningRuntime,
  type PlanningRuntime,
} from "./agent/model";
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
  readonly attempt: number;
}

type JobFailureOutcome = "retry_scheduled" | "failed";

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
 * Append an ordered job event inside the caller's transaction. This mirrors
 * `probe_forge_queue::append_job_event`: the job row is locked so concurrent
 * writers cannot allocate the same sequence, and the event is written in the
 * same transaction as the status change it describes. Without this the SSE
 * stream never observes worker-driven transitions (queued -> analyzing ->
 * awaiting approval).
 */
async function appendJobEvent(
  tx: SQL,
  jobId: string,
  eventType: JobEventType,
  payload: Record<string, unknown>,
): Promise<void> {
  await tx`SELECT id FROM jobs WHERE id = ${jobId} FOR UPDATE`;
  const rows = await tx<{ sequence: string | number }[]>`
    SELECT COALESCE(MAX(sequence), 0) + 1 AS sequence
    FROM job_events
    WHERE job_id = ${jobId}
  `;
  const sequence = Number(rows[0]?.sequence ?? 1);
  await tx`
    INSERT INTO job_events (job_id, sequence, event_type, payload)
    VALUES (${jobId}, ${sequence}, ${eventType}, ${tx.json(payload)})
  `;
}

/**
 * Fail a claimed job, appending the matching lifecycle event in the same
 * transaction as the status change. A job whose lease was already taken by a
 * newer worker is left untouched and writes no event.
 */
async function failClaimedJob(
  sql: SQL,
  jobId: string,
  workerId: string,
  errorCode: string,
): Promise<JobFailureOutcome> {
  return sql.begin(async (tx) => {
    const rows = await tx<{ status: string; attempt: number; not_before: string }[]>`
      UPDATE jobs
      SET status = CASE WHEN attempt < max_attempts THEN 'queued' ELSE 'failed' END,
          worker_id = NULL,
          lease_expires_at = NULL,
          heartbeat_at = NULL,
          not_before = now() + make_interval(secs => ${RETRY_BACKOFF_SECONDS}),
          last_error_code = ${errorCode}
      WHERE id = ${jobId} AND worker_id = ${workerId}
      RETURNING status, attempt, not_before
    `;
    const row = rows[0];
    if (row === undefined) {
      return "failed";
    }
    if (row.status === "queued") {
      await appendJobEvent(tx, jobId, "job.retry_scheduled", {
        attempt: row.attempt,
        retryAt: row.not_before,
        errorCode,
      });
      return "retry_scheduled";
    }
    await appendJobEvent(tx, jobId, "job.failed", {
      attempt: row.attempt,
      errorCode,
      willRetry: false,
    });
    return "failed";
  });
}

/**
 * [TEMP DEBUG] Detailed failure diagnostics for the analysis job loop.
 *
 * Remove this helper (and every `[TEMP DEBUG]` logging block below) once the
 * worker failure is understood. Values are bounded and passed through the
 * logger, which still redacts credential markers and sensitive keys. Do not add
 * repository file contents, prompts, request bodies, or provider responses here.
 */
export interface ErrorDescription {
  readonly errorName: string;
  readonly errorMessage: string;
  readonly errorCode: string | null;
  /** Git subcommand that failed, when the error carries one (never arguments). */
  readonly operation: string | null;
  readonly errorStack: string | null;
  readonly errorCause: string | null;
}

const MAX_ERROR_TEXT = 4_000;

function clipErrorText(value: string): string {
  return value.length > MAX_ERROR_TEXT ? `${value.slice(0, MAX_ERROR_TEXT)}…[truncated]` : value;
}

/** [TEMP DEBUG] Bounded, logger-redacted description of an unknown error. */
export function describeError(error: unknown): ErrorDescription {
  if (!(error instanceof Error)) {
    return {
      errorName: typeof error,
      errorMessage: clipErrorText(String(error)),
      errorCode: null,
      operation: null,
      errorStack: null,
      errorCause: null,
    };
  }
  const maybeCode = (error as { readonly code?: unknown }).code;
  const maybeOperation = (error as { readonly operation?: unknown }).operation;
  const cause = error.cause;
  return {
    errorName: error.name,
    errorMessage: clipErrorText(error.message),
    errorCode: typeof maybeCode === "string" ? maybeCode : null,
    operation: typeof maybeOperation === "string" ? maybeOperation : null,
    errorStack: typeof error.stack === "string" ? clipErrorText(error.stack) : null,
    errorCause:
      cause === undefined
        ? null
        : clipErrorText(cause instanceof Error ? `${cause.name}: ${cause.message}` : String(cause)),
  };
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
    const job = await sql.begin(async (tx) => {
      const rows = await tx<ClaimedJobRow[]>`
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
                  j.source_reference, j.attempt, g.normalized_url
      `;

      const claimed = rows[0];
      if (claimed === undefined) {
        return undefined;
      }
      // The claim and its `job.claimed` event commit together so the SSE stream
      // never observes a status the event log does not describe.
      await appendJobEvent(tx, claimed.id, "job.claimed", {
        workerId: options.workerId,
        attempt: claimed.attempt,
        leaseSeconds: lease,
      });
      return claimed;
    });

    if (job === undefined) return null;

    logger.info("job.claimed", {
      jobId: job.id,
      projectId: job.project_id,
      workerId: options.workerId,
      goal: job.goal,
    });

    const localPath = filePathFromUrl(job.normalized_url);
    const localFixtureRoot = localPath === null ? null : dirname(localPath);
    // [TEMP DEBUG] Surface target-validation failures. A throw here happens
    // outside the reschedule block below, so the job stays `analyzing` until its
    // lease expires; the stack pinpoints the rejected URL shape.
    let target: ReturnType<typeof validateGitUrl>;
    try {
      target = validateGitUrl(job.normalized_url, {
        allowedHosts: options.allowedGitHosts,
        allowLocalFixtures: options.allowLocalFixtures,
        localFixtureRoot,
      });
    } catch (error) {
      logger.error("job.target_invalid", {
        jobId: job.id,
        sourceReference: job.source_reference,
        allowLocalFixtures: options.allowLocalFixtures,
        localFixture: localPath !== null,
        ...describeError(error),
      });
      throw error;
    }
    logger.debug("job.target_validated", {
      jobId: job.id,
      provider: target.provider,
      hostname: target.hostname,
      scheme: target.scheme,
      sourceReference: job.source_reference,
    });

    // Admin settings take precedence over the environment; the provider key is
    // decrypted only here and never logged or stored in planning metadata.
    let platform: Awaited<ReturnType<typeof loadPlatformPlanning>> | null = null;
    try {
      platform = await loadPlatformPlanning(sql);
    } catch (error) {
      // [TEMP DEBUG] Previously swallowed: a settings query/decrypt failure
      // silently forces the environment fallback (or no model at all).
      logger.error("job.platform_settings_failed", { jobId: job.id, ...describeError(error) });
      platform = null;
    }
    const provider = platform?.provider ?? options.planningProvider ?? null;
    const model = platform?.model ?? options.planningModel ?? null;
    const limits = mergePlanningLimits(
      options.planningLimits ?? DEFAULT_PLANNING_LIMITS,
      platform?.limitsOverride ?? {},
    );
    // A custom OpenAI-compatible endpoint takes precedence over the pinned
    // catalog. Missing metadata falls back to conservative defaults.
    const customConfig: CustomOpenAiCompatConfig | null =
      platform?.baseUrl != null && platform.baseUrl.trim() !== ""
        ? {
            baseUrl: platform.baseUrl,
            contextWindow: platform.contextWindow ?? CUSTOM_PROVIDER_DEFAULTS.contextWindow,
            maxOutputTokens: platform.maxOutputTokens ?? CUSTOM_PROVIDER_DEFAULTS.maxOutputTokens,
          }
        : null;
    let runtime: PlanningRuntime | null = null;
    if (provider !== null && provider.trim() !== "" && model !== null && model.trim() !== "") {
      try {
        runtime = createPiPlanningRuntime(
          provider,
          model,
          limits,
          platform?.apiKey ?? null,
          customConfig,
        );
      } catch (error) {
        // [TEMP DEBUG] Previously swallowed: a bad provider/model/custom config
        // silently degrades to the deterministic fallback planner.
        logger.error("job.runtime_creation_failed", {
          jobId: job.id,
          provider,
          model,
          hasApiKey: (platform?.apiKey ?? null) !== null,
          hasCustomBaseUrl: customConfig !== null,
          ...describeError(error),
        });
        runtime = null;
      }
    }

    // [TEMP DEBUG] Report the resolved planning configuration without ever
    // logging the decrypted API key, only whether one is present.
    logger.info("job.planning_config", {
      jobId: job.id,
      configuredProvider: provider,
      configuredModel: model,
      hasApiKey: (platform?.apiKey ?? null) !== null,
      hasCustomBaseUrl: customConfig !== null,
      customBaseUrl: customConfig?.baseUrl ?? null,
      contextWindow: customConfig?.contextWindow ?? null,
      maxOutputTokens: customConfig?.maxOutputTokens ?? null,
      runtimeCreated: runtime !== null,
      planningProviderFallback: options.planningProvider ?? null,
      planningModelFallback: options.planningModel ?? null,
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
          // [TEMP DEBUG] Snapshot summary so ingestion success/limits are visible.
          logger.info("job.snapshot_ready", {
            jobId: job.id,
            commitSha: session.snapshot.commitSha,
            fileCount: session.snapshot.analysisMetadata.fileCount,
            textFileCount: session.snapshot.analysisMetadata.textFileCount,
            totalBytes: session.snapshot.analysisMetadata.totalBytes,
            filesTruncated: session.snapshot.filesTruncated,
            detectedLanguages: session.snapshot.detectedLanguages,
            frameworks: session.snapshot.frameworks,
          });
          logger.debug("job.discovery_started", { jobId: job.id });
          const inventory = await discoverApplication(session);
          logger.info("job.discovery_completed", {
            jobId: job.id,
            frameworkCount: inventory.frameworks.length,
            frameworkNames: inventory.frameworks.map((framework) => framework.name),
            routeCount: inventory.routes.length,
            surfaceCount: inventory.surfaces.length,
            unsupportedCount: inventory.unsupported.length,
            ambiguityCount: inventory.ambiguities.length,
          });
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
          // [TEMP DEBUG] Full plan-generation result so a silent fallback or
          // retryable failure is not reduced to a single error code.
          logger.info("job.plan_result", {
            jobId: job.id,
            outcome: planResult.outcome,
            failureCode: planResult.failureCode,
            fallbackReason: planResult.fallbackReason,
            provider: planResult.provider,
            model: planResult.model,
            attempts: planResult.attempts,
            plannedTestCount: planResult.plan?.plannedTests.length ?? 0,
            loadScenarioCount: planResult.plan?.loadScenarios.length ?? 0,
            savedPlanId: planResult.savedPlan?.id ?? null,
            sessionIds: planResult.sessionIds,
            stats: planResult.stats,
          });
          return planResult;
        },
      );
    } catch (error) {
      const errorCode = jobErrorCode(error);
      // [TEMP DEBUG] The bounded row code hides the root cause; emit the full
      // (logger-redacted) error so ingestion/planning failures are diagnosable.
      logger.error("job.processing_exception", {
        jobId: job.id,
        workerId: options.workerId,
        runtimeCreated: runtime !== null,
        ...describeError(error),
        errorCode,
      });
      const outcome = await failClaimedJob(sql, job.id, options.workerId, errorCode);
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
      await sql.begin(async (tx) => {
        const updated = await tx<{ id: string }[]>`
          UPDATE jobs
          SET status = 'awaiting_approval',
              worker_id = NULL,
              lease_expires_at = NULL,
              heartbeat_at = NULL,
              last_error_code = NULL,
              current_plan_id = COALESCE(${planId}, current_plan_id)
          WHERE id = ${job.id} AND worker_id = ${options.workerId}
          RETURNING id
        `;
        if (updated[0] === undefined) {
          return;
        }
        await appendJobEvent(tx, job.id, "plan.proposed", { planId });
      });
      logger.info("job.awaiting_approval", {
        jobId: job.id,
        workerId: options.workerId,
        planId,
        planOutcome: result.outcome,
      });
      return { jobId: job.id, outcome: "awaiting_approval" };
    }

    const failureCode = result.failureCode ?? "planning_failed";
    const outcome = await failClaimedJob(sql, job.id, options.workerId, failureCode);
    logger.warn("job.plan_not_produced", {
      jobId: job.id,
      workerId: options.workerId,
      errorCode: failureCode,
      outcome,
      // [TEMP DEBUG] Retryable failures carry no thrown error; log the result
      // fields that explain why no plan was produced.
      planOutcome: result.outcome,
      fallbackReason: result.fallbackReason,
      provider: result.provider,
      model: result.model,
      attempts: result.attempts,
      sessionIds: result.sessionIds,
      stats: result.stats,
    });
    return { jobId: job.id, outcome };
  } finally {
    await store.close();
    await sql.close({ timeout: 5 });
  }
}
