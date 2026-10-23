import { describe, expect, it } from "bun:test";
import { PlanningError } from "./errors";
import { DEFAULT_PLANNING_LIMITS, validatePlanningLimits } from "./limits";
import { assertRuntimeIdentifier, createPiPlanningRuntime } from "./model";

describe("planning limits", () => {
  it("accepts the documented defaults", () => {
    expect(() => validatePlanningLimits(DEFAULT_PLANNING_LIMITS)).not.toThrow();
  });

  it("rejects unbounded or inconsistent limits", () => {
    expect(() => validatePlanningLimits({ ...DEFAULT_PLANNING_LIMITS, maxModelCalls: 0 })).toThrow(
      RangeError,
    );
    expect(() =>
      validatePlanningLimits({ ...DEFAULT_PLANNING_LIMITS, maxPlanBytes: 512 * 1024 }),
    ).toThrow(RangeError);
    expect(() =>
      validatePlanningLimits({
        ...DEFAULT_PLANNING_LIMITS,
        maxToolOutputBytes: 4_096,
        maxTotalToolOutputBytes: 2_048,
      }),
    ).toThrow(RangeError);
  });
});

describe("planning runtime", () => {
  it("validates provider and model identifiers", () => {
    expect(() => assertRuntimeIdentifier("anthropic", "provider")).not.toThrow();
    expect(() => assertRuntimeIdentifier("anthropic", "model")).not.toThrow();
    expect(() => assertRuntimeIdentifier("Anthropic ", "provider")).toThrow(PlanningError);
    expect(() => assertRuntimeIdentifier("bad model", "model")).toThrow(PlanningError);
  });

  it("fails closed without a configured model", () => {
    expect(() => createPiPlanningRuntime(null, null, DEFAULT_PLANNING_LIMITS)).toThrow(
      PlanningError,
    );
    expect(() =>
      createPiPlanningRuntime("anthropic", "not-a-real-model", DEFAULT_PLANNING_LIMITS),
    ).toThrow(/not present/);
  });
});
