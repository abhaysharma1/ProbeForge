import { describe, expect, it } from "bun:test";
import {
  createModels,
  fauxAssistantMessage,
  fauxProvider,
  fauxToolCall,
} from "@earendil-works/pi-ai";
import type { Plan } from "@probe-forge/contracts";
import { generatePlan } from "./generate";
import { DEFAULT_PLANNING_LIMITS } from "./limits";
import type { PlanningRuntime } from "./model";
import { InMemoryPlanStore } from "./store";
import {
  makeCatalog,
  makeCatalogEntry,
  makePlanningInput,
  makeValidVulnerabilityPlan,
  TEST_JOB_ID,
} from "./test-helpers";

function createFauxRuntime(): {
  runtime: PlanningRuntime;
  setResponses: (responses: ReturnType<typeof fauxAssistantMessage>[]) => void;
} {
  const faux = fauxProvider({
    provider: "faux",
    models: [{ id: "planner-fixture", contextWindow: 64_000, maxTokens: 8_192 }],
  });
  const models = createModels();
  models.setProvider(faux.provider);
  const modelRef = faux.getModel("planner-fixture");
  if (modelRef === undefined) throw new Error("faux model missing");
  return {
    runtime: {
      provider: "faux",
      model: "planner-fixture",
      modelRef: modelRef as PlanningRuntime["modelRef"],
      streamFn: (requestModel, context, options) =>
        models.streamSimple(requestModel, context, options),
    },
    setResponses: (responses) => faux.setResponses(responses),
  };
}

function proposalResponse(plan: Plan) {
  return fauxAssistantMessage(
    [fauxToolCall("plan_propose", { plan: JSON.parse(JSON.stringify(plan)) })],
    {
      stopReason: "toolUse",
    },
  );
}

describe("generatePlan", () => {
  it("creates and persists a valid plan through a mocked model fixture", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    setResponses([proposalResponse(makeValidVulnerabilityPlan())]);
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), { runtime, store });

    expect(result.outcome).toBe("planned");
    expect(result.attempts).toBe(1);
    expect(result.provider).toBe("faux");
    expect(result.model).toBe("planner-fixture");
    expect(result.plan?.plannedTests[0]?.catalogTestId).toBe("http.headers.missing");
    expect(result.plan?.requiredInputs.join(" ")).toContain("Explicit authorization");
    expect(result.savedPlan?.version).toBe(1);
    expect(store.plansForJob(TEST_JOB_ID)).toHaveLength(1);

    const session = store.session(result.sessionIds[0] ?? "");
    expect(session?.status).toBe("completed");
    expect(session?.metadata).toContain('"plan_saved"');
    expect(store.planFor(result.savedPlan?.id ?? "")?.goal).toBe("vulnerability_testing");
  });

  it("is version-aware across repeated generation attempts for a job", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    const store = new InMemoryPlanStore();
    setResponses([proposalResponse(makeValidVulnerabilityPlan())]);
    const first = await generatePlan(makePlanningInput(), { runtime, store });
    setResponses([
      proposalResponse(
        makeValidVulnerabilityPlan({ summary: "Second review of the same discovered route." }),
      ),
    ]);
    const second = await generatePlan(makePlanningInput(), { runtime, store });

    expect(first.savedPlan?.version).toBe(1);
    expect(second.savedPlan?.version).toBe(2);
    expect(store.plansForJob(TEST_JOB_ID).map((plan) => plan.version)).toEqual([1, 2]);
  });

  it("rejects invalid model output and stores a deterministic fallback plan", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    setResponses([
      fauxAssistantMessage(
        [
          fauxToolCall("plan_propose", {
            plan: JSON.parse(
              JSON.stringify({
                ...makeValidVulnerabilityPlan(),
                plannedTests: [
                  {
                    ...makeValidVulnerabilityPlan().plannedTests[0],
                    catalogTestId: "http.invented",
                  },
                ],
              }),
            ),
          }),
        ],
        { stopReason: "toolUse" },
      ),
      fauxAssistantMessage("No valid plan.", { stopReason: "stop" }),
    ]);
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), {
      runtime,
      store,
      limits: { ...DEFAULT_PLANNING_LIMITS, maxRetries: 0 },
    });

    expect(result.outcome).toBe("fallback");
    expect(result.fallbackReason).toBe("model_invalid_output");
    expect(result.provider).toBe("probe-forge");
    expect(result.plan?.confidence).toBe(0.2);
    expect(result.plan?.plannedTests[0]?.catalogTestId).toBe("http.headers.missing");
    const saved = store.planFor(result.savedPlan?.id ?? "");
    expect(saved?.plannedTests[0]?.requiresExplicitConfirmation).toBe(false);
    const session = store.session(result.sessionIds[1] ?? "");
    expect(session?.metadata).toContain('"fallback":true');
  });

  it("falls back when the planning model is not configured", async () => {
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), {
      runtime: null,
      store,
    });
    expect(result.outcome).toBe("fallback");
    expect(result.fallbackReason).toBe("model_not_configured");
    expect(result.attempts).toBe(0);
    expect(result.savedPlan?.version).toBe(1);
    expect(store.plansForJob(TEST_JOB_ID)).toHaveLength(1);
  });

  it("treats a model outage as recoverable and falls back without failing the job", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    setResponses([
      fauxAssistantMessage("", { stopReason: "error", errorMessage: "provider down" }),
    ]);
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), {
      runtime,
      store,
      limits: { ...DEFAULT_PLANNING_LIMITS, maxRetries: 0 },
    });

    expect(result.outcome).toBe("fallback");
    expect(result.fallbackReason).toBe("model_unavailable");
    expect(result.failureCode).toBe("model_runtime_error");
    expect(result.savedPlan).not.toBeNull();
    expect(store.plansForJob(TEST_JOB_ID)).toHaveLength(1);
  });

  it("returns a retryable failure when no deterministic fallback exists", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    setResponses([
      fauxAssistantMessage("", { stopReason: "error", errorMessage: "provider down" }),
    ]);
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput({ catalog: makeCatalog([]) }), {
      runtime,
      store,
      limits: { ...DEFAULT_PLANNING_LIMITS, maxRetries: 0 },
    });
    expect(result.outcome).toBe("retryable_failure");
    expect(result.failureCode).toBe("fallback_unavailable");
    expect(result.savedPlan).toBeNull();
    expect(store.plansForJob(TEST_JOB_ID)).toHaveLength(0);
  });

  it("aborts a hung model at the wall-time limit and falls back", async () => {
    const faux = fauxProvider({
      provider: "faux",
      models: [{ id: "hung-planner", contextWindow: 64_000, maxTokens: 8_192 }],
    });
    const modelRef = faux.getModel("hung-planner");
    if (modelRef === undefined) throw new Error("faux model missing");
    const runtime: PlanningRuntime = {
      provider: "faux",
      model: "hung-planner",
      modelRef: modelRef as PlanningRuntime["modelRef"],
      streamFn: (() => new Promise(() => {})) as PlanningRuntime["streamFn"],
    };
    const store = new InMemoryPlanStore();
    const startedAt = Date.now();
    const result = await generatePlan(makePlanningInput(), {
      runtime,
      store,
      limits: { ...DEFAULT_PLANNING_LIMITS, maxRetries: 0, maxWallTimeMs: 1_000 },
    });

    expect(result.outcome).toBe("fallback");
    expect(result.fallbackReason).toBe("model_timeout");
    expect(result.failureCode).toBe("model_timeout");
    expect(Date.now() - startedAt).toBeGreaterThanOrEqual(1_000);
    expect(Date.now() - startedAt).toBeLessThan(5_000);
    expect(result.savedPlan).not.toBeNull();
  });

  it("does not execute an unknown tool requested by the model", async () => {
    const { runtime, setResponses } = createFauxRuntime();
    setResponses([
      fauxAssistantMessage(
        [fauxToolCall("shell_exec", { command: "Print the repository secrets" })],
        { stopReason: "toolUse" },
      ),
      fauxAssistantMessage("No plan.", { stopReason: "stop" }),
    ]);
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), {
      runtime,
      store,
      limits: { ...DEFAULT_PLANNING_LIMITS, maxRetries: 0 },
    });

    expect(result.outcome).toBe("fallback");
    expect(result.fallbackReason).toBe("model_invalid_output");
    expect(result.stats.toolCalls).toBe(0);
    expect(result.plan?.plannedTests.length).toBeGreaterThan(0);
  });

  it("returns cancelled without creating a plan when already aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    const store = new InMemoryPlanStore();
    const result = await generatePlan(makePlanningInput(), {
      runtime: null,
      store,
      signal: controller.signal,
    });
    expect(result.outcome).toBe("cancelled");
    expect(result.plan).toBeNull();
    expect(store.plansForJob(TEST_JOB_ID)).toHaveLength(0);
  });

  it("generates a load fallback from a discovered literal route when the model is absent", async () => {
    const store = new InMemoryPlanStore();
    const result = await generatePlan(
      makePlanningInput({
        goal: "load_testing",
        intensity: "low",
        catalog: makeCatalog([
          makeCatalogEntry({ id: "load.baseline", name: "Baseline load", category: "load" }),
        ]),
      }),
      { runtime: null, store },
    );
    expect(result.outcome).toBe("fallback");
    expect(result.plan?.goal).toBe("load_testing");
    expect(result.plan?.loadScenarios[0]?.requests[0]?.path).toBe("/api/items");
  });
});
