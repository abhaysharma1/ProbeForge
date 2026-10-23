import type { FinishTurn } from "@earendil-works/pi-agent-core";
import { Agent } from "@earendil-works/pi-agent-core";
import type { Plan } from "@probe-forge/contracts";
import { PlanningError } from "./errors";
import {
  buildFallbackPlan,
  FALLBACK_MODEL,
  FALLBACK_PROVIDER,
  type FallbackReason,
} from "./fallback";
import { DEFAULT_PLANNING_LIMITS, type PlanningLimits, validatePlanningLimits } from "./limits";
import type { PlanningRuntime } from "./model";
import {
  createPlanningContext,
  createPlanningPrompt,
  type PlanningContext,
  type PlanningInput,
  planningSystemPrompt,
} from "./planning-input";
import { type PlanStore, PlanStoreError, type SavedPlan } from "./store";
import { createPlanningToolSession, type PlanningToolStats } from "./tools";
import { validateCandidatePlan } from "./validate";

export type PlanGenerationOutcome = "planned" | "fallback" | "retryable_failure" | "cancelled";

export interface PlanGenerationOptions {
  readonly runtime: PlanningRuntime | null;
  readonly store: PlanStore;
  readonly limits?: PlanningLimits;
  readonly signal?: AbortSignal;
}

export interface PlanGenerationStats {
  readonly attempts: number;
  readonly modelCalls: number;
  readonly turns: number;
  readonly toolCalls: number;
  readonly toolOutputBytes: number;
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly elapsedMs: number;
}

export interface PlanGenerationResult {
  readonly outcome: PlanGenerationOutcome;
  readonly plan: Plan | null;
  readonly savedPlan: SavedPlan | null;
  readonly provider: string | null;
  readonly model: string | null;
  readonly attempts: number;
  readonly fallbackReason: FallbackReason | null;
  readonly failureCode: string | null;
  readonly stats: PlanGenerationStats;
  readonly sessionIds: readonly string[];
}

type AttemptFailureCode =
  | "agent_unavailable"
  | "model_invalid_output"
  | "model_runtime_error"
  | "model_timeout"
  | "planning_limit_exceeded";

interface AttemptFailure {
  readonly code: AttemptFailureCode;
  readonly retryable: boolean;
  readonly fallbackReason: FallbackReason;
}

interface AttemptStats {
  readonly modelCalls: number;
  readonly turns: number;
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly elapsedMs: number;
  readonly tools: PlanningToolStats;
}

type AttemptOutcome =
  | { readonly status: "proposed"; readonly plan: Plan; readonly stats: AttemptStats }
  | { readonly status: "failed"; readonly failure: AttemptFailure; readonly stats: AttemptStats }
  | { readonly status: "cancelled"; readonly stats: AttemptStats };

const ZERO_TOOL_STATS: PlanningToolStats = {
  calls: 0,
  blockedCalls: 0,
  failedCalls: 0,
  outputBytes: 0,
  toolNames: [],
  limitReached: null,
};

function isAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

function storeFailureCode(error: unknown): string {
  return error instanceof PlanStoreError ? error.code : "persistence_failed";
}

function zeroAttemptStats(): AttemptStats {
  return {
    modelCalls: 0,
    turns: 0,
    inputTokens: 0,
    outputTokens: 0,
    elapsedMs: 0,
    tools: ZERO_TOOL_STATS,
  };
}

function failureForStopReason(stopReason: string): AttemptFailure {
  if (stopReason === "error" || stopReason === "aborted") {
    return {
      code: "model_runtime_error",
      retryable: true,
      fallbackReason: "model_unavailable",
    };
  }
  if (stopReason === "length") {
    return {
      code: "planning_limit_exceeded",
      retryable: false,
      fallbackReason: "planning_limit_exceeded",
    };
  }
  return {
    code: "model_invalid_output",
    retryable: false,
    fallbackReason: "model_invalid_output",
  };
}

function safeNumber(value: number): number {
  return Number.isFinite(value) && value >= 0 ? value : 0;
}

function sessionMetadata(
  context: PlanningContext,
  attempt: number,
  maxAttempts: number,
  extra: Readonly<Record<string, unknown>>,
): Record<string, unknown> {
  return {
    phase: "planning",
    attempt,
    maxAttempts,
    goal: context.job.goal,
    commitSha: context.repository.snapshot.commitSha,
    catalogVersion: context.catalog.catalogVersion,
    limits: {
      maxModelCalls: context.limits.maxModelCalls,
      maxToolCalls: context.limits.maxToolCalls,
      maxTurns: context.limits.maxTurns,
      maxWallTimeMs: context.limits.maxWallTimeMs,
      maxResponseTokens: context.limits.maxResponseTokens,
      maxToolOutputBytes: context.limits.maxToolOutputBytes,
      maxTotalToolOutputBytes: context.limits.maxTotalToolOutputBytes,
      maxPlanBytes: context.limits.maxPlanBytes,
    },
    ...extra,
  };
}

function toolMetadata(tools: PlanningToolStats): Record<string, unknown> {
  return {
    toolCalls: tools.calls,
    blockedToolCalls: tools.blockedCalls,
    failedToolCalls: tools.failedCalls,
    toolOutputBytes: tools.outputBytes,
    toolNames: tools.toolNames,
    toolLimitReached: tools.limitReached,
  };
}

/**
 * Runs one Pi agent attempt with server-owned budgets. No model error text,
 * prompt text, repository text, or provider response is returned or persisted.
 */
async function runAgentAttempt(
  context: PlanningContext,
  runtime: PlanningRuntime,
  store: PlanStore,
  prompt: string,
  attempt: number,
  maxAttempts: number,
  externalSignal?: AbortSignal,
): Promise<{ readonly outcome: AttemptOutcome; readonly sessionId: string | null }> {
  const startedAt = Date.now();
  let sessionId: string | null = null;
  try {
    sessionId = await store.startSession({
      projectId: context.job.projectId,
      jobId: context.job.id,
      provider: runtime.provider,
      model: runtime.model,
      metadata: sessionMetadata(context, attempt, maxAttempts, { status: "started" }),
    });
  } catch {
    return {
      outcome: {
        status: "failed",
        failure: {
          code: "agent_unavailable",
          retryable: true,
          fallbackReason: "model_unavailable",
        },
        stats: zeroAttemptStats(),
      },
      sessionId: null,
    };
  }

  const toolSession = createPlanningToolSession(context);
  let modelCalls = 0;
  let turns = 0;
  let inputTokens = 0;
  let outputTokens = 0;
  let failure: AttemptFailure | null = null;
  let timedOut = false;
  let cancelled = isAborted(externalSignal);
  let agent: Agent | null = null;
  let promptSettled = false;
  let releaseAbortWait: (() => void) | null = null;

  const abortWait = new Promise<void>((resolve) => {
    releaseAbortWait = resolve;
  });
  const onExternalAbort = (): void => {
    cancelled = true;
    agent?.abort();
    releaseAbortWait?.();
  };
  if (externalSignal !== undefined) {
    externalSignal.addEventListener("abort", onExternalAbort, { once: true });
  }
  const timer = setTimeout(() => {
    timedOut = true;
    agent?.abort();
    releaseAbortWait?.();
  }, context.limits.maxWallTimeMs);

  const finishTurn: FinishTurn = ({ message }) => {
    turns += 1;
    inputTokens += safeNumber(message.usage.input);
    outputTokens += safeNumber(message.usage.output);
    if (
      message.stopReason === "error" ||
      message.stopReason === "aborted" ||
      message.stopReason === "length"
    ) {
      failure ??= failureForStopReason(message.stopReason);
    }
    if (toolSession.proposal() !== null) return { action: "end" };
    if (failure !== null) return { action: "end" };
    const tools = toolSession.stats();
    if (tools.limitReached !== null) {
      failure ??= {
        code: "planning_limit_exceeded",
        retryable: false,
        fallbackReason: "planning_limit_exceeded",
      };
      return { action: "end" };
    }
    if (turns >= context.limits.maxTurns) {
      failure ??= {
        code: "planning_limit_exceeded",
        retryable: false,
        fallbackReason: "planning_limit_exceeded",
      };
      return { action: "end" };
    }
    return undefined;
  };

  try {
    agent = new Agent({
      initialState: {
        systemPrompt: planningSystemPrompt(),
        model: runtime.modelRef,
        tools: [...toolSession.tools],
      },
      streamFn: runtime.streamFn,
      toolExecution: "sequential",
      maxRetryDelayMs: 1_000,
      beforeToolCall: toolSession.beforeToolCall,
      finishTurn,
      prepareRequest: () => {
        modelCalls += 1;
        if (modelCalls > context.limits.maxModelCalls) {
          failure ??= {
            code: "planning_limit_exceeded",
            retryable: false,
            fallbackReason: "planning_limit_exceeded",
          };
          agent?.abort();
        }
      },
    });

    const promptPromise = agent
      .prompt(prompt)
      .catch(() => {
        failure ??= {
          code: "model_runtime_error",
          retryable: true,
          fallbackReason: "model_unavailable",
        };
      })
      .finally(() => {
        promptSettled = true;
        releaseAbortWait?.();
      });

    await Promise.race([promptPromise, abortWait]);
    if (!promptSettled) {
      agent.abort();
      await Promise.race([promptPromise, new Promise<void>((resolve) => setTimeout(resolve, 250))]);
    }

    const elapsedMs = Date.now() - startedAt;
    const tools = toolSession.stats();
    const stats: AttemptStats = {
      modelCalls,
      turns,
      inputTokens,
      outputTokens,
      elapsedMs,
      tools,
    };

    if (cancelled || isAborted(externalSignal)) {
      await bestEffortComplete(
        store,
        sessionId,
        "cancelled",
        sessionMetadata(context, attempt, maxAttempts, {
          status: "cancelled",
          ...toolMetadata(tools),
          elapsedMs,
        }),
      );
      return { outcome: { status: "cancelled", stats }, sessionId };
    }
    if (timedOut || elapsedMs > context.limits.maxWallTimeMs) {
      failure = {
        code: "model_timeout",
        retryable: true,
        fallbackReason: "model_timeout",
      };
    }
    if (failure === null && agent.state.errorMessage !== undefined) {
      failure = {
        code: "model_runtime_error",
        retryable: true,
        fallbackReason: "model_unavailable",
      };
    }
    if (failure === null && tools.limitReached !== null) {
      failure = {
        code: "planning_limit_exceeded",
        retryable: false,
        fallbackReason: "planning_limit_exceeded",
      };
    }
    if (failure === null && modelCalls > context.limits.maxModelCalls) {
      failure = {
        code: "planning_limit_exceeded",
        retryable: false,
        fallbackReason: "planning_limit_exceeded",
      };
    }

    const proposal = toolSession.proposal();
    if (failure === null && proposal !== null) {
      const revalidated = validateCandidatePlan(proposal, context);
      if (revalidated.ok) {
        // The session stays `running` until savePlan completes it atomically
        // with the draft plan and audit record.
        return {
          outcome: { status: "proposed", plan: revalidated.plan, stats },
          sessionId,
        };
      }
      failure = {
        code: "model_invalid_output",
        retryable: false,
        fallbackReason: "model_invalid_output",
      };
    }
    if (failure === null) {
      failure = {
        code: "model_invalid_output",
        retryable: false,
        fallbackReason: "model_invalid_output",
      };
    }

    await bestEffortComplete(
      store,
      sessionId,
      "failed",
      sessionMetadata(context, attempt, maxAttempts, {
        status: "failed",
        failureCode: failure.code,
        retryable: failure.retryable,
        ...toolMetadata(tools),
        modelCalls,
        turns,
        inputTokens,
        outputTokens,
        elapsedMs,
      }),
    );
    return { outcome: { status: "failed", failure, stats }, sessionId };
  } finally {
    clearTimeout(timer);
    if (externalSignal !== undefined) {
      externalSignal.removeEventListener("abort", onExternalAbort);
    }
  }
}

async function bestEffortComplete(
  store: PlanStore,
  sessionId: string | null,
  status: "completed" | "failed" | "cancelled",
  metadata: Readonly<Record<string, unknown>>,
): Promise<void> {
  if (sessionId === null) return;
  try {
    await store.completeSession({ sessionId, status, metadata });
  } catch {
    // Session cleanup is best effort. The caller receives a retryable failure
    // when plan persistence itself cannot be confirmed.
  }
}

function addStats(left: PlanGenerationStats, right: AttemptStats): PlanGenerationStats {
  return {
    attempts: left.attempts + 1,
    modelCalls: left.modelCalls + right.modelCalls,
    turns: left.turns + right.turns,
    toolCalls: left.toolCalls + right.tools.calls,
    toolOutputBytes: left.toolOutputBytes + right.tools.outputBytes,
    inputTokens: left.inputTokens + right.inputTokens,
    outputTokens: left.outputTokens + right.outputTokens,
    elapsedMs: left.elapsedMs + right.elapsedMs,
  };
}

function emptyStats(): PlanGenerationStats {
  return {
    attempts: 0,
    modelCalls: 0,
    turns: 0,
    toolCalls: 0,
    toolOutputBytes: 0,
    inputTokens: 0,
    outputTokens: 0,
    elapsedMs: 0,
  };
}

function result(
  outcome: PlanGenerationOutcome,
  fields: Partial<PlanGenerationResult>,
  stats: PlanGenerationStats,
  sessions: string[],
): PlanGenerationResult {
  return {
    outcome,
    plan: fields.plan ?? null,
    savedPlan: fields.savedPlan ?? null,
    provider: fields.provider ?? null,
    model: fields.model ?? null,
    attempts: stats.attempts,
    fallbackReason: fields.fallbackReason ?? null,
    failureCode: fields.failureCode ?? null,
    stats,
    sessionIds: sessions,
  };
}

async function persistFallback(
  context: PlanningContext,
  store: PlanStore,
  reason: FallbackReason,
  stats: PlanGenerationStats,
  sessions: string[],
): Promise<PlanGenerationResult> {
  const plan = buildFallbackPlan(context, reason);
  if (plan === null) {
    return result(
      "retryable_failure",
      { fallbackReason: reason, failureCode: "fallback_unavailable" },
      stats,
      sessions,
    );
  }
  let sessionId: string;
  try {
    sessionId = await store.startSession({
      projectId: context.job.projectId,
      jobId: context.job.id,
      provider: FALLBACK_PROVIDER,
      model: FALLBACK_MODEL,
      metadata: sessionMetadata(context, 0, 1, {
        phase: "fallback",
        status: "started",
        fallbackReason: reason,
        plannedTestCount: plan.plannedTests.length,
        loadScenarioCount: plan.loadScenarios.length,
      }),
    });
    sessions.push(sessionId);
  } catch (error) {
    return result(
      "retryable_failure",
      { fallbackReason: reason, failureCode: storeFailureCode(error) },
      stats,
      sessions,
    );
  }

  try {
    const savedPlan = await store.savePlan({
      sessionId,
      projectId: context.job.projectId,
      jobId: context.job.id,
      plan,
      provider: FALLBACK_PROVIDER,
      model: FALLBACK_MODEL,
      createdByAgent: false,
      fallbackReason: reason,
    });
    return result(
      "fallback",
      {
        plan,
        savedPlan,
        provider: FALLBACK_PROVIDER,
        model: FALLBACK_MODEL,
        fallbackReason: reason,
      },
      stats,
      sessions,
    );
  } catch (error) {
    const failureCode = storeFailureCode(error);
    await bestEffortComplete(
      store,
      sessionId,
      "failed",
      sessionMetadata(context, 0, 1, {
        phase: "fallback",
        status: "failed",
        failureCode,
        fallbackReason: reason,
      }),
    );
    return result("retryable_failure", { fallbackReason: reason, failureCode }, stats, sessions);
  }
}

/**
 * Generates one draft plan. A model outage or invalid model proposal never
 * marks the job failed and never executes tests; it either yields a
 * deterministic fallback draft or a retryable result for the queue layer.
 */
export async function generatePlan(
  input: PlanningInput,
  options: PlanGenerationOptions,
): Promise<PlanGenerationResult> {
  const limits = options.limits ?? DEFAULT_PLANNING_LIMITS;
  try {
    validatePlanningLimits(limits);
  } catch (error) {
    throw new PlanningError(
      "invalid_limits",
      error instanceof Error ? error.message : "planning limits are invalid",
    );
  }
  const context = createPlanningContext(input, limits);
  const sessions: string[] = [];
  let stats = emptyStats();

  if (isAborted(options.signal)) {
    return result("cancelled", {}, stats, sessions);
  }
  if (options.runtime === null) {
    return persistFallback(context, options.store, "model_not_configured", stats, sessions);
  }

  let prompt: string;
  try {
    prompt = createPlanningPrompt(context);
  } catch (error) {
    const reason: FallbackReason =
      error instanceof PlanningError && error.message.includes("byte limit")
        ? "prompt_too_large"
        : "unexpected";
    return persistFallback(context, options.store, reason, stats, sessions);
  }

  const maxAttempts = limits.maxRetries + 1;
  let lastReason: FallbackReason = "model_unavailable";
  let lastFailureCode: string | null = null;

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    if (isAborted(options.signal)) {
      return result("cancelled", {}, stats, sessions);
    }
    const run = await runAgentAttempt(
      context,
      options.runtime,
      options.store,
      prompt,
      attempt,
      maxAttempts,
      options.signal,
    );
    if (run.sessionId !== null) sessions.push(run.sessionId);
    stats = addStats(stats, run.outcome.stats);

    if (run.outcome.status === "proposed") {
      if (run.sessionId === null) {
        return result("retryable_failure", { failureCode: "session_missing" }, stats, sessions);
      }
      if (isAborted(options.signal)) {
        await bestEffortComplete(options.store, run.sessionId, "cancelled", {
          status: "cancelled",
          failureCode: "cancelled_after_proposal",
        });
        return result("cancelled", {}, stats, sessions);
      }
      try {
        const savedPlan = await options.store.savePlan({
          sessionId: run.sessionId,
          projectId: context.job.projectId,
          jobId: context.job.id,
          plan: run.outcome.plan,
          provider: options.runtime.provider,
          model: options.runtime.model,
          createdByAgent: true,
          fallbackReason: null,
        });
        return result(
          "planned",
          {
            plan: run.outcome.plan,
            savedPlan,
            provider: options.runtime.provider,
            model: options.runtime.model,
          },
          stats,
          sessions,
        );
      } catch (error) {
        const failureCode = storeFailureCode(error);
        await bestEffortComplete(options.store, run.sessionId, "failed", {
          status: "failed",
          failureCode,
        });
        return result("retryable_failure", { failureCode }, stats, sessions);
      }
    }

    if (run.outcome.status === "cancelled") {
      return result("cancelled", {}, stats, sessions);
    }

    lastReason = run.outcome.failure.fallbackReason;
    lastFailureCode = run.outcome.failure.code;
    if (!run.outcome.failure.retryable) break;
  }

  if (isAborted(options.signal)) {
    return result("cancelled", {}, stats, sessions);
  }
  const fallback = await persistFallback(context, options.store, lastReason, stats, sessions);
  return {
    ...fallback,
    failureCode: fallback.outcome === "fallback" ? lastFailureCode : fallback.failureCode,
  };
}
