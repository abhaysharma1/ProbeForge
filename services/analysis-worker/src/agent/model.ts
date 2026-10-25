import type { StreamFn } from "@earendil-works/pi-agent-core";
import {
  type Api,
  createModels,
  createProvider,
  envApiKeyAuth,
  type Model,
  type Models,
} from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { builtinModels } from "@earendil-works/pi-ai/providers/all";
import { isPublicHost } from "../repository/net";
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

/**
 * An admin-configured OpenAI-compatible endpoint. When present, the runtime
 * registers a custom provider for this base URL instead of resolving the
 * provider/model from the pinned SDK catalog. The base URL is validated here
 * again so an out-of-band change cannot point the worker at an internal address.
 */
export interface CustomOpenAiCompatConfig {
  readonly baseUrl: string;
  readonly contextWindow: number;
  readonly maxOutputTokens: number;
}

const PROVIDER_PATTERN = /^[a-z0-9][a-z0-9._-]{0,79}$/;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,159}$/;

const CUSTOM_API = "openai-completions" as const;
const MAX_BASE_URL_LENGTH = 300;
const MIN_CONTEXT_WINDOW = 1_024;
const MAX_CONTEXT_WINDOW = 2_000_000;
const MIN_OUTPUT_TOKENS = 128;
const MAX_OUTPUT_TOKENS = 200_000;

/** Applied when the admin leaves the custom model metadata blank. */
export const CUSTOM_PROVIDER_DEFAULTS = {
  contextWindow: 32_768,
  maxOutputTokens: 4_096,
} as const;

export function assertRuntimeIdentifier(value: string, kind: "provider" | "model"): void {
  const pattern = kind === "provider" ? PROVIDER_PATTERN : MODEL_PATTERN;
  if (!pattern.test(value)) {
    throw new PlanningError("invalid_input", `Planning ${kind} is not a valid identifier`);
  }
}

/**
 * Rejects base URLs that are not HTTPS or that point at an internal or metadata
 * host. This is the worker-side half of a two-layer check; the API validates at
 * save time. DNS-rebinding is only mitigated best-effort because the Pi SDK
 * performs the actual fetch.
 */
function normalizeBaseUrl(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0 || trimmed.length > MAX_BASE_URL_LENGTH) {
    throw new PlanningError("invalid_input", "The planning base URL is not a valid URL");
  }
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new PlanningError("invalid_input", "The planning base URL is not a valid URL");
  }
  if (parsed.protocol !== "https:") {
    throw new PlanningError("invalid_input", "The planning base URL must use HTTPS");
  }
  if (parsed.username !== "" || parsed.password !== "") {
    throw new PlanningError("invalid_input", "The planning base URL must not embed credentials");
  }
  if (parsed.search !== "" || parsed.hash !== "") {
    throw new PlanningError(
      "invalid_input",
      "The planning base URL must not include a query or fragment",
    );
  }
  if (!isPublicHost(parsed.hostname)) {
    throw new PlanningError("invalid_input", "The planning base URL must be a public host");
  }
  return trimmed;
}

/** Clamps the requested tokens to the server-owned response budget. */
function buildStreamFn(models: Models, limits: PlanningLimits, apiKey: string | null): StreamFn {
  return (requestModel, context, options) => {
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
}

function buildCustomRuntime(
  provider: string,
  model: string,
  limits: PlanningLimits,
  apiKey: string | null,
  custom: CustomOpenAiCompatConfig,
): PlanningRuntime {
  const baseUrl = normalizeBaseUrl(custom.baseUrl);
  const contextWindow = custom.contextWindow;
  const maxOutputTokens = custom.maxOutputTokens;
  if (
    !Number.isSafeInteger(contextWindow) ||
    contextWindow < MIN_CONTEXT_WINDOW ||
    contextWindow > MAX_CONTEXT_WINDOW
  ) {
    throw new PlanningError("invalid_input", "The planning context window is out of range");
  }
  if (
    !Number.isSafeInteger(maxOutputTokens) ||
    maxOutputTokens < MIN_OUTPUT_TOKENS ||
    maxOutputTokens > MAX_OUTPUT_TOKENS ||
    maxOutputTokens > contextWindow
  ) {
    throw new PlanningError("invalid_input", "The planning max output tokens is out of range");
  }

  const modelRef: Model<Api> = {
    id: model,
    name: model,
    api: CUSTOM_API,
    provider,
    baseUrl,
    reasoning: false,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow,
    maxTokens: maxOutputTokens,
    type: "chat",
  };

  const models = createModels();
  models.setProvider(
    createProvider({
      id: provider,
      name: provider,
      baseUrl,
      // Keyless-by-default auth: the explicit per-request key (the admin-stored
      // provider secret) is what authenticates. No ambient env is consulted.
      auth: { apiKey: envApiKeyAuth("Planning provider API key", []) },
      models: [modelRef],
      api: openAICompletionsApi(),
    }),
  );

  return { provider, model, modelRef, streamFn: buildStreamFn(models, limits, apiKey) };
}

/**
 * Builds the Pi SDK runtime for the configured provider/model. Provider API keys
 * remain in the provider environment (or the admin-stored secret) and are never
 * copied into planning metadata.
 *
 * When `custom` is present, a custom OpenAI-compatible provider is registered
 * for `custom.baseUrl` and takes precedence over the pinned catalog, so an
 * admin can point any OpenAI-compatible endpoint at the planner. Otherwise the
 * lookup is restricted to the pinned built-in catalog.
 */
export function createPiPlanningRuntime(
  providerValue: string | null,
  modelValue: string | null,
  limits: PlanningLimits,
  apiKey: string | null = null,
  custom: CustomOpenAiCompatConfig | null = null,
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

  if (custom !== null) {
    return buildCustomRuntime(provider, model, limits, apiKey, custom);
  }

  const models = builtinModels();
  const modelRef = models.getModel(provider, model);
  if (modelRef === undefined) {
    throw new PlanningError(
      "model_not_found",
      "The configured planning provider/model is not present in the pinned Pi SDK catalog",
    );
  }

  return { provider, model, modelRef, streamFn: buildStreamFn(models, limits, apiKey) };
}
