import { describe, expect, it } from "bun:test";
import { buildFallbackPlan } from "./fallback";
import { makeCatalog, makeCatalogEntry, makeContext, makeRoute } from "./test-helpers";
import { validateCandidatePlan } from "./validate";

describe("buildFallbackPlan", () => {
  it("builds a low-confidence catalog-only vulnerability fallback", () => {
    const context = makeContext({
      catalog: makeCatalog([
        makeCatalogEntry({ id: "http.headers.high", name: "High risk check", risk: "high" }),
        makeCatalogEntry({ id: "http.headers.low", name: "Low risk check", risk: "low" }),
      ]),
    });
    const plan = buildFallbackPlan(context, "model_unavailable");
    expect(plan).not.toBeNull();
    if (plan === null) return;
    expect(plan.goal).toBe("vulnerability_testing");
    expect(plan.confidence).toBe(0.2);
    expect(plan.plannedTests.map((test) => test.catalogTestId)).toEqual([
      "http.headers.low",
      "http.headers.high",
    ]);
    expect(plan.plannedTests[1]?.requiresExplicitConfirmation).toBe(true);
    expect(plan.plannedTests.some((test) => test.name === "High risk check")).toBe(true);
    expect(plan.requiredInputs.join(" ")).toContain("Explicit authorization");
    expect(plan.requiredInputs.join(" ")).toContain("fallback-selected");
    expect(validateCandidatePlan(plan, context).ok).toBe(true);
  });

  it("builds a conservative load fallback from a literal GET route", () => {
    const context = makeContext({
      goal: "load_testing",
      intensity: "low",
      routes: [
        makeRoute({ id: "route.post", method: "POST", path: "/api/items" }),
        makeRoute({ id: "route.get", method: "GET", path: "/api/items" }),
        makeRoute({ id: "route.dynamic", pathKind: "parameterized", path: "/api/items/:id" }),
      ],
    });
    const plan = buildFallbackPlan(context, "model_invalid_output");
    expect(plan).not.toBeNull();
    if (plan === null) return;
    expect(plan.goal).toBe("load_testing");
    expect(plan.plannedTests).toHaveLength(0);
    expect(plan.loadScenarios).toHaveLength(1);
    expect(plan.loadScenarios[0]?.requests[0]?.path).toBe("/api/items");
    expect(plan.loadScenarios[0]?.profile.virtualUsers).toBe(2);
    expect(plan.limitations.join(" ")).toContain("did not pass strict server validation");
    expect(validateCandidatePlan(plan, context).ok).toBe(true);
  });

  it("returns null when no safe fallback exists", () => {
    const noCatalog = buildFallbackPlan(
      makeContext({ catalog: makeCatalog([]) }),
      "model_unavailable",
    );
    expect(noCatalog).toBeNull();

    const noRoute = buildFallbackPlan(
      makeContext({ goal: "load_testing", routes: [] }),
      "model_unavailable",
    );
    expect(noRoute).toBeNull();
  });
});
