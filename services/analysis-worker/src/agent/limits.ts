import { MAX_CONTRACT_BYTES } from "@probe-forge/contracts/validate";

/**
 * Hard limits for one planning attempt. Limits are server-owned: the model and
 * repository content cannot widen them. The wall-time limit covers the whole
 * agent run, including model calls and tool execution.
 */
export interface PlanningLimits {
  readonly maxModelCalls: number;
  readonly maxToolCalls: number;
  readonly maxTurns: number;
  readonly maxRetries: number;
  readonly maxResponseTokens: number;
  readonly maxWallTimeMs: number;
  readonly maxPromptBytes: number;
  readonly maxToolOutputBytes: number;
  readonly maxTotalToolOutputBytes: number;
  readonly maxPlanBytes: number;
  readonly maxRepositoryReadBytes: number;
  readonly maxCatalogEntries: number;
  readonly maxPlannedTests: number;
  readonly maxLoadScenarios: number;
}

export const DEFAULT_PLANNING_LIMITS: PlanningLimits = {
  maxModelCalls: 6,
  maxToolCalls: 24,
  maxTurns: 8,
  maxRetries: 1,
  maxResponseTokens: 4_096,
  maxWallTimeMs: 120_000,
  maxPromptBytes: 64 * 1024,
  maxToolOutputBytes: 32 * 1024,
  maxTotalToolOutputBytes: 128 * 1024,
  maxPlanBytes: MAX_CONTRACT_BYTES,
  maxRepositoryReadBytes: 8 * 1024,
  maxCatalogEntries: 200,
  maxPlannedTests: 20,
  maxLoadScenarios: 5,
};

export function validatePlanningLimits(limits: PlanningLimits): void {
  const entries: Array<[keyof PlanningLimits, number, number, number]> = [
    ["maxModelCalls", limits.maxModelCalls, 1, 20],
    ["maxToolCalls", limits.maxToolCalls, 0, 100],
    ["maxTurns", limits.maxTurns, 1, 20],
    ["maxRetries", limits.maxRetries, 0, 5],
    ["maxResponseTokens", limits.maxResponseTokens, 128, 64_000],
    ["maxWallTimeMs", limits.maxWallTimeMs, 1_000, 900_000],
    ["maxPromptBytes", limits.maxPromptBytes, 1_024, 256 * 1024],
    ["maxToolOutputBytes", limits.maxToolOutputBytes, 256, 128 * 1024],
    ["maxTotalToolOutputBytes", limits.maxTotalToolOutputBytes, 256, 512 * 1024],
    ["maxPlanBytes", limits.maxPlanBytes, 1_024, MAX_CONTRACT_BYTES],
    ["maxRepositoryReadBytes", limits.maxRepositoryReadBytes, 256, 64 * 1024],
    ["maxCatalogEntries", limits.maxCatalogEntries, 0, 1_000],
    ["maxPlannedTests", limits.maxPlannedTests, 0, 100],
    ["maxLoadScenarios", limits.maxLoadScenarios, 0, 50],
  ];

  for (const [name, value, minimum, maximum] of entries) {
    if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
      throw new RangeError(`${name} must be an integer between ${minimum} and ${maximum}`);
    }
  }
  if (limits.maxToolOutputBytes > limits.maxTotalToolOutputBytes) {
    throw new RangeError("maxToolOutputBytes must not exceed maxTotalToolOutputBytes");
  }
}
