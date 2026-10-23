import { describe, expect, it } from "bun:test";
import { DEFAULT_PLANNING_LIMITS } from "./limits";
import {
  makeCatalog,
  makeCatalogEntry,
  makeContext,
  makePlannedTest,
  makeValidLoadPlan,
  makeValidVulnerabilityPlan,
} from "./test-helpers";
import { PHASE_SEVEN_REQUIRED_INPUTS, validateCandidatePlan } from "./validate";

const TOKEN_PLACEHOLDER = "$" + "{token}";
const MISSING_PLACEHOLDER = "$" + "{missing}";

function issueCodes(result: ReturnType<typeof validateCandidatePlan>): string[] {
  return result.ok ? [] : result.issues.map((issue) => issue.code);
}

describe("validateCandidatePlan", () => {
  it("accepts a catalog-backed plan and adds server-owned required inputs", () => {
    const result = validateCandidatePlan(makeValidVulnerabilityPlan(), makeContext());
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.requiredInputs).toContain(PHASE_SEVEN_REQUIRED_INPUTS.targetAuthorization);
    expect(result.plan.limitations.join(" ")).toContain("no test, target request");
    expect(result.plan.evidenceReferences).toEqual(["src/server.ts"]);
  });

  it("accepts a bounded load plan and adds load review requirements", () => {
    const result = validateCandidatePlan(
      makeValidLoadPlan(),
      makeContext({ goal: "load_testing" }),
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.requiredInputs).toContain(PHASE_SEVEN_REQUIRED_INPUTS.loadPath);
    expect(result.plan.loadScenarios).toHaveLength(1);
  });

  it("rejects a goal that does not match the job", () => {
    const result = validateCandidatePlan(
      makeValidLoadPlan(),
      makeContext({ goal: "vulnerability_testing" }),
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("wrong_goal");
  });

  it("rejects unknown catalog IDs, name mismatches, and risk downgrades", () => {
    const context = makeContext({
      catalog: makeCatalog([makeCatalogEntry({ risk: "high" })]),
    });
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        plannedTests: [
          makePlannedTest({
            catalogTestId: "http.invented",
            name: "Invented test",
            risk: "low",
            requiresExplicitConfirmation: false,
          }),
          makePlannedTest({
            name: "Different name",
            risk: "low",
            requiresExplicitConfirmation: false,
          }),
        ],
      }),
      context,
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("unknown_catalog_test");
    expect(issueCodes(result)).toContain("catalog_name_mismatch");
    expect(issueCodes(result)).toContain("catalog_risk_downgrade");
    expect(issueCodes(result)).toContain("confirmation_required");
  });

  it("rejects duplicate catalog tests and load entries used as vulnerability tests", () => {
    const context = makeContext({
      catalog: makeCatalog([
        makeCatalogEntry({ id: "load.baseline", name: "Baseline load", category: "load" }),
      ]),
    });
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        plannedTests: [
          makePlannedTest({
            catalogTestId: "load.baseline",
            name: "Baseline load",
            risk: "low",
            rationaleReferences: [],
          }),
          makePlannedTest({
            catalogTestId: "load.baseline",
            name: "Baseline load",
            risk: "low",
            rationaleReferences: [],
          }),
        ],
      }),
      context,
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("catalog_category_mismatch");
    expect(issueCodes(result)).toContain("duplicate_catalog_test");
  });

  it("rejects mixing vulnerability tests and load scenarios", () => {
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        loadScenarios: makeValidLoadPlan().loadScenarios,
      }),
      makeContext(),
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("invalid_contract");

    const load = validateCandidatePlan(
      makeValidLoadPlan({ plannedTests: [makePlannedTest()] }),
      makeContext({ goal: "load_testing" }),
    );
    expect(load.ok).toBe(false);
    expect(issueCodes(load)).toContain("invalid_contract");
  });

  it("rejects duplicate load scenario names and unsafe load paths", () => {
    const base = makeValidLoadPlan().loadScenarios[0];
    if (base === undefined) throw new Error("load fixture is missing a scenario");
    const request = base.requests[0];
    if (request === undefined) throw new Error("load fixture is missing a request");

    const duplicate = validateCandidatePlan(
      makeValidLoadPlan({ loadScenarios: [base, base] }),
      makeContext({
        goal: "load_testing",
        limits: { ...DEFAULT_PLANNING_LIMITS, maxLoadScenarios: 5 },
      }),
    );
    expect(duplicate.ok).toBe(false);
    expect(issueCodes(duplicate)).toContain("duplicate_load_scenario");

    const unsafe = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [{ ...base, requests: [{ ...request, path: "//metadata.test/latest" }] }],
      }),
      makeContext({ goal: "load_testing" }),
    );
    expect(unsafe.ok).toBe(false);
    expect(issueCodes(unsafe)).toContain("invalid_contract");
  });

  it("rejects references outside the bounded snapshot", () => {
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        evidenceReferences: ["src/server.ts", "../secrets.env", "README.md"],
      }),
      makeContext(),
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("unknown_reference");
  });

  it("rejects embedded credentials and executable configuration", () => {
    const credential = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        summary: `Use Authorization: ${"Bearer"} ${"abcdefghijklmnopqrstuvwxyz012345"} for the check.`,
      }),
      makeContext(),
    );
    expect(credential.ok).toBe(false);
    expect(issueCodes(credential)).toContain("credential_material");

    const executable = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        plannedTests: [
          makePlannedTest({
            configuration: { command: "curl https://example.test" },
          }),
        ],
      }),
      makeContext(),
    );
    expect(executable.ok).toBe(false);
    expect(issueCodes(executable)).toContain("invalid_contract");
  });

  it("rejects plans above the configured byte limit", () => {
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({ summary: "x".repeat(4_000) }),
      makeContext({
        limits: { ...DEFAULT_PLANNING_LIMITS, maxPlanBytes: 2_000 },
      }),
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("plan_too_large");
  });

  it("keeps required inputs bounded when the model fills all slots", () => {
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({
        requiredInputs: Array.from({ length: 20 }, (_, index) => `Model input ${index + 1}`),
      }),
      makeContext(),
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.requiredInputs).toHaveLength(20);
    expect(result.plan.requiredInputs).toContain(PHASE_SEVEN_REQUIRED_INPUTS.targetAuthorization);
  });

  it("accepts a correlated load scenario with secret headers and assertions", () => {
    const base = makeValidLoadPlan().loadScenarios[0];
    if (base === undefined) throw new Error("load fixture is missing a scenario");
    const result = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [
          {
            ...base,
            requests: [
              {
                id: "session",
                method: "POST",
                path: "/api/session",
                headers: { "Content-Type": "application/json" },
                secretHeaders: { Authorization: "secret://fixtures/service-token" },
                extract: [{ name: "token", source: "header", selector: "x-session-token" }],
                bodyTemplate: { username: "fixture-user" },
                expectedStatuses: [200],
              },
              {
                id: "profile",
                method: "GET",
                path: `/api/profile/${TOKEN_PLACEHOLDER}`,
                dependsOn: ["session"],
                headers: { "X-Session": TOKEN_PLACEHOLDER },
                assertions: [
                  {
                    source: "json_body",
                    selector: "/status",
                    operator: "equals",
                    expected: "ok",
                  },
                ],
                expectedStatuses: [200],
              },
            ],
            profile: {
              ...base.profile,
              stopConditions: { errorRate: 0.05, p95LatencyMs: 1000 },
            },
          },
        ],
      }),
      makeContext({ goal: "load_testing" }),
    );
    expect(result.ok).toBe(true);
  });

  it("rejects load dependencies, templates, raw secrets, and loop fields", () => {
    const base = makeValidLoadPlan().loadScenarios[0];
    if (base === undefined) throw new Error("load fixture is missing a scenario");
    const request = base.requests[0];
    if (request === undefined) throw new Error("load fixture is missing a request");

    const dependency = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [
          {
            ...base,
            requests: [{ ...request, dependsOn: ["later"], path: `/api/${MISSING_PLACEHOLDER}` }],
          },
        ],
      }),
      makeContext({ goal: "load_testing" }),
    );
    expect(dependency.ok).toBe(false);
    expect(issueCodes(dependency)).toContain("invalid_contract");

    const rawSecret = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [
          {
            ...base,
            requests: [
              {
                ...request,
                secretHeaders: { Authorization: "Bearer raw-token-value" },
              },
            ],
          },
        ],
      }),
      makeContext({ goal: "load_testing" }),
    );
    expect(rawSecret.ok).toBe(false);
    expect(issueCodes(rawSecret)).toContain("invalid_contract");

    const loop = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [
          {
            ...base,
            requests: [{ ...request, loop: { while: true } } as typeof request],
          },
        ],
      }),
      makeContext({ goal: "load_testing" }),
    );
    expect(loop.ok).toBe(false);
    expect(issueCodes(loop)).toContain("invalid_contract");
  });

  it("rejects excessively many planned tests and load scenarios", () => {
    const tests = Array.from({ length: 2 }, () => makePlannedTest());
    const result = validateCandidatePlan(
      makeValidVulnerabilityPlan({ plannedTests: tests }),
      makeContext({
        limits: { ...DEFAULT_PLANNING_LIMITS, maxPlannedTests: 1 },
      }),
    );
    expect(result.ok).toBe(false);
    expect(issueCodes(result)).toContain("too_many_tests");

    const scenario = makeValidLoadPlan().loadScenarios[0];
    if (scenario === undefined) throw new Error("load fixture is missing a scenario");
    const scenarios = validateCandidatePlan(
      makeValidLoadPlan({
        loadScenarios: [scenario, scenario],
      }),
      makeContext({
        goal: "load_testing",
        limits: { ...DEFAULT_PLANNING_LIMITS, maxLoadScenarios: 1 },
      }),
    );
    expect(scenarios.ok).toBe(false);
    expect(issueCodes(scenarios)).toContain("too_many_scenarios");
  });
});
