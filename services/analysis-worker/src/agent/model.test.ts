import { describe, expect, it } from "bun:test";
import { DEFAULT_PLANNING_LIMITS } from "./limits";
import { CUSTOM_PROVIDER_DEFAULTS, createPiPlanningRuntime } from "./model";

const custom = {
  baseUrl: "https://api.example.test/v1",
  contextWindow: 32_768,
  maxOutputTokens: 4_096,
} as const;

describe("custom OpenAI-compatible runtime", () => {
  it("registers a custom provider and resolves the requested model", () => {
    const runtime = createPiPlanningRuntime(
      "nebius",
      "deepseek-ai/DeepSeek-V3",
      DEFAULT_PLANNING_LIMITS,
      null,
      custom,
    );
    expect(runtime.provider).toBe("nebius");
    expect(runtime.model).toBe("deepseek-ai/DeepSeek-V3");
    expect(runtime.modelRef.api).toBe("openai-completions");
    expect(runtime.modelRef.baseUrl).toBe("https://api.example.test/v1");
    expect(runtime.modelRef.maxTokens).toBe(4_096);
  });

  it("takes precedence over a built-in provider id", () => {
    // "anthropic" is a built-in catalog id; the custom base URL still wins.
    const runtime = createPiPlanningRuntime(
      "anthropic",
      "any-model",
      DEFAULT_PLANNING_LIMITS,
      null,
      custom,
    );
    expect(runtime.modelRef.baseUrl).toBe("https://api.example.test/v1");
  });

  it("falls back to the pinned catalog when no custom endpoint is configured", () => {
    expect(() =>
      createPiPlanningRuntime("anthropic", "not-a-real-model", DEFAULT_PLANNING_LIMITS),
    ).toThrow(/not present/);
  });

  it("rejects insecure or internal base URLs", () => {
    for (const baseUrl of [
      "http://api.example.test/v1",
      "https://localhost/v1",
      "https://127.0.0.1/v1",
      "https://10.0.0.5/v1",
      "https://169.254.169.254/latest/meta-data",
      "https://metadata.google.internal/computeMetadata/v1",
      "https://api.example.test/v1?token=1",
      "not-a-url",
    ]) {
      expect(() =>
        createPiPlanningRuntime("provider", "model", DEFAULT_PLANNING_LIMITS, null, {
          ...custom,
          baseUrl,
        }),
      ).toThrow(/base URL/);
    }
  });

  it("rejects out-of-range model metadata", () => {
    expect(() =>
      createPiPlanningRuntime("provider", "model", DEFAULT_PLANNING_LIMITS, null, {
        ...custom,
        contextWindow: 512,
      }),
    ).toThrow(/context window/);
    expect(() =>
      createPiPlanningRuntime("provider", "model", DEFAULT_PLANNING_LIMITS, null, {
        ...custom,
        maxOutputTokens: 99_999,
      }),
    ).toThrow(/max output tokens/);
  });

  it("documents conservative defaults that fit inside the context window", () => {
    expect(CUSTOM_PROVIDER_DEFAULTS.contextWindow).toBeGreaterThanOrEqual(1_024);
    expect(CUSTOM_PROVIDER_DEFAULTS.maxOutputTokens).toBeGreaterThanOrEqual(128);
    expect(CUSTOM_PROVIDER_DEFAULTS.maxOutputTokens).toBeLessThanOrEqual(
      CUSTOM_PROVIDER_DEFAULTS.contextWindow,
    );
  });
});
