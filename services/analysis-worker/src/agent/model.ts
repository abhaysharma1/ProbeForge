import type { StreamFn } from "@earendil-works/pi-agent-core";
import type { Api, Model } from "@earendil-works/pi-ai";
import { builtinModels } from "@earendil-works/pi-ai/providers/all";
import { PlanningError } from "./errors";
import type { PlanningLimits } from "./limits";

/**
 * A deliberately small model handle. The orchestrator never receives provider
 * credentials; `streamFn` resolves authentication inside the Pi SDK from the
 * configured provider environment.
 */
export interface PlanningRuntime {
  readonly provider: string;
  readonly model: string;
  readonly modelRef: Model<Api>;
  readonly streamFn: StreamFn;
}

const PROVIDER_PATTERN = /^[a-z0-9][a-z0-9._-]{0,79}$/;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,159}$/;

export function assertRuntimeIdentifier(value: string, kind: "provider" | "model"): void {
  const pattern = kind === "provider" ? PROVIDER_PATTERN : MODEL_PATTERN;
  if (!pattern.test(value)) {
    throw new PlanningError("invalid_input", `Planning ${kind} is not a valid identifier`);
  }
}

/**
 * Builds the real Pi SDK runtime for a provider/model that is present in the
 * pinned SDK catalog. Provider API keys remain in the provider environment and
 * are never copied into planning metadata.
 */
export function createPiPlanningRuntime(
  providerValue: string | null,
  modelValue: string | null,
  limits: PlanningLimits,
  apiKey: string | null = null,
): PlanningRuntime {
  const provider = providerValue?.trim() ?? "";
  const model = modelValue?.trim() ?? "";
  if (provider.length === 0 || model.length === 0) {
    throw new PlanningError(
      "model_not_configured",
      "Planning model is not configured; set the provider and model environment variables",
    );
  }
  assertRuntimeIdentifier(provider, "provider");
  assertRuntimeIdentifier(model, "model");

  const models = builtinModels();
  const modelRef = models.getModel(provider, model);
  if (modelRef === undefined) {
    throw new PlanningError(
      "model_not_found",
      "The configured planning provider/model is not present in the pinned Pi SDK catalog",
    );
  }

  const streamFn: StreamFn = (requestModel, context, options) => {
    const requestedTokens = options?.maxTokens;
    const maxTokens =
      requestedTokens === undefined
        ? limits.maxResponseTokens
        : Math.min(requestedTokens, limits.maxResponseTokens);
    return models.streamSimple(requestModel, context, {
      ...options,
      ...(apiKey !== null && apiKey.trim().length > 0 ? { apiKey } : {}),
      maxTokens,
      temperature: 0,
    });
  };

  return { provider, model, modelRef, streamFn };
}
