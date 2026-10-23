import type { Plan, PlannedTest, Risk } from "@probe-forge/contracts";
import { validatePlan } from "@probe-forge/contracts/validate";
import type { PlanningContext } from "./planning-input";

export type PlanValidationCode =
  | "invalid_contract"
  | "wrong_goal"
  | "goal_cross_population"
  | "unknown_catalog_test"
  | "duplicate_catalog_test"
  | "catalog_name_mismatch"
  | "catalog_risk_downgrade"
  | "catalog_category_mismatch"
  | "confirmation_required"
  | "unknown_reference"
  | "unsafe_load_path"
  | "duplicate_load_scenario"
  | "too_many_tests"
  | "too_many_scenarios"
  | "plan_too_large"
  | "credential_material";

export interface PlanValidationIssue {
  readonly code: PlanValidationCode;
  readonly path: string;
  readonly message: string;
}

export type PlanValidationResult =
  | { readonly ok: true; readonly plan: Plan }
  | { readonly ok: false; readonly issues: readonly PlanValidationIssue[] };

const RISK_RANK: Readonly<Record<Risk, number>> = {
  low: 0,
  medium: 1,
  high: 2,
  critical: 3,
};

const TARGET_AUTHORIZATION_INPUT =
  "Explicit authorization of an in-scope deployed target; Phase 7 only proposes a plan and never contacts targets.";
const NO_ROUTE_INPUT =
  "The repository route inventory is incomplete; provide application entrypoints or route configuration.";
const UNSUPPORTED_INPUT =
  "Confirm coverage for frameworks or languages that static discovery could not inspect.";
const CATALOG_INPUT = "Provide a vetted test catalog entry for the intended verification.";
const LOAD_PATH_INPUT =
  "Confirm that load scenario paths match the authorized deployed target before approval.";
const PHASE_SEVEN_LIMITATION =
  "Generated statically in Phase 7; no test, target request, or load execution occurred.";

const SECRET_PATTERNS: readonly RegExp[] = [
  /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----/,
  /\bBearer\s+[A-Za-z0-9._~+/-]{16,}=*/i,
  /\bAKIA[0-9A-Z]{16}\b/,
  /\bgh[pousr]_[A-Za-z0-9]{20,}\b/,
  /\bsk-[A-Za-z0-9_-]{20,}\b/,
  /\bpassword\s*[:=]\s*\S{4,}/i,
  /\bclient[_-]?secret\s*[:=]\s*\S{4,}/i,
];

function issue(code: PlanValidationCode, path: string, message: string): PlanValidationIssue {
  return { code, path, message };
}

function planBytes(plan: Plan): number {
  return new TextEncoder().encode(JSON.stringify(plan)).byteLength;
}

function isUnknownReference(reference: string, context: PlanningContext): boolean {
  return (
    reference.length === 0 ||
    reference.startsWith("/") ||
    reference.includes("\\") ||
    reference.split("/").some((segment) => segment === ".." || segment === ".") ||
    !context.repositoryFiles.has(reference)
  );
}

function checkReferences(
  references: readonly string[] | undefined,
  path: string,
  context: PlanningContext,
  issues: PlanValidationIssue[],
): void {
  references?.forEach((reference, index) => {
    if (isUnknownReference(reference, context)) {
      issues.push(
        issue(
          "unknown_reference",
          `${path}/${index}`,
          "references must name a bounded file from the repository snapshot",
        ),
      );
    }
  });
}

function checkLoadPaths(plan: Plan, issues: PlanValidationIssue[]): void {
  const names = new Set<string>();
  plan.loadScenarios.forEach((scenario, scenarioIndex) => {
    if (names.has(scenario.name)) {
      issues.push(
        issue(
          "duplicate_load_scenario",
          `/loadScenarios/${scenarioIndex}/name`,
          "load scenario names must be unique within a plan",
        ),
      );
    }
    names.add(scenario.name);
    scenario.requests.forEach((request, requestIndex) => {
      if (request.path.startsWith("//") || request.path.includes("\\")) {
        issues.push(
          issue(
            "unsafe_load_path",
            `/loadScenarios/${scenarioIndex}/requests/${requestIndex}/path`,
            "load request paths must be same-origin absolute paths",
          ),
        );
      }
    });
  });
}

function checkCredentialMaterial(
  value: unknown,
  path: string,
  issues: PlanValidationIssue[],
): void {
  if (typeof value === "string") {
    if (SECRET_PATTERNS.some((pattern) => pattern.test(value))) {
      issues.push(
        issue("credential_material", path, "plans must not contain credential or secret material"),
      );
    }
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => {
      checkCredentialMaterial(item, `${path}/${index}`, issues);
    });
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      if (key.toLowerCase() === "credentialref") continue;
      checkCredentialMaterial(item, `${path}/${key}`, issues);
    }
  }
}

function checkPlannedTests(
  plan: Plan,
  context: PlanningContext,
  issues: PlanValidationIssue[],
): void {
  const seen = new Set<string>();
  plan.plannedTests.forEach((test: PlannedTest, index) => {
    const path = `/plannedTests/${index}`;
    const catalogEntry = context.catalogById.get(test.catalogTestId);
    if (catalogEntry === undefined) {
      issues.push(
        issue(
          "unknown_catalog_test",
          `${path}/catalogTestId`,
          "catalogTestId is not present in the vetted test catalog",
        ),
      );
    } else {
      if (catalogEntry.category === "load") {
        issues.push(
          issue(
            "catalog_category_mismatch",
            `${path}/catalogTestId`,
            "load catalog entries cannot be used as vulnerability tests",
          ),
        );
      }
      if (catalogEntry.name !== test.name) {
        issues.push(
          issue(
            "catalog_name_mismatch",
            `${path}/name`,
            "the test name must match the vetted catalog entry",
          ),
        );
      }
      if (RISK_RANK[test.risk] < RISK_RANK[catalogEntry.risk]) {
        issues.push(
          issue(
            "catalog_risk_downgrade",
            `${path}/risk`,
            "the proposed risk must not be lower than the catalog risk",
          ),
        );
      }
      if (
        (test.risk === "high" || test.risk === "critical" || catalogEntry.risk === "high") &&
        !test.requiresExplicitConfirmation
      ) {
        issues.push(
          issue(
            "confirmation_required",
            `${path}/requiresExplicitConfirmation`,
            "high-risk tests require explicit confirmation",
          ),
        );
      }
    }
    if (seen.has(test.catalogTestId)) {
      issues.push(
        issue("duplicate_catalog_test", `${path}/catalogTestId`, "catalog tests must be unique"),
      );
    }
    seen.add(test.catalogTestId);
    checkReferences(test.rationaleReferences, `${path}/rationaleReferences`, context, issues);
  });
}

function appendBounded(
  values: readonly string[],
  additions: readonly string[],
  maxItems: number,
): readonly string[] {
  const result = [...values];
  const seen = new Set(result.map((value) => value.toLowerCase()));
  for (const addition of additions) {
    const key = addition.toLowerCase();
    if (seen.has(key)) continue;
    if (result.length >= maxItems) result.pop();
    result.push(addition);
    seen.add(key);
  }
  return result;
}

/**
 * Applies deterministic, server-owned requirements to an otherwise valid plan.
 * The model cannot remove these inputs, and the additions stay inside the plan
 * schema limits.
 */
export function normalizePlan(plan: Plan, context: PlanningContext): Plan {
  const requiredInputs = [TARGET_AUTHORIZATION_INPUT];
  if (context.inventory.routes.length === 0) requiredInputs.push(NO_ROUTE_INPUT);
  if (context.inventory.unsupported.length > 0) requiredInputs.push(UNSUPPORTED_INPUT);
  if (plan.goal === "vulnerability_testing" && context.catalog.entries.length === 0) {
    requiredInputs.push(CATALOG_INPUT);
  }
  if (plan.goal === "load_testing") requiredInputs.push(LOAD_PATH_INPUT);

  return {
    ...plan,
    requiredInputs: appendBounded(plan.requiredInputs, requiredInputs, 20),
    limitations: appendBounded(plan.limitations, [PHASE_SEVEN_LIMITATION], 20),
  };
}

/**
 * Strict server-side validation for a model-proposed plan. The contract schema
 * is authoritative for structure; these checks enforce Phase 7 policy and the
 * planning context. Invalid proposals are never persisted or executed.
 */
export function validateCandidatePlan(
  value: unknown,
  context: PlanningContext,
): PlanValidationResult {
  const contract = validatePlan(value);
  if (!contract.ok) {
    return {
      ok: false,
      issues: contract.issues.map((contractIssue) =>
        issue("invalid_contract", contractIssue.path, contractIssue.message),
      ),
    };
  }

  const plan = contract.value;
  const issues: PlanValidationIssue[] = [];
  if (plan.goal !== context.job.goal) {
    issues.push(issue("wrong_goal", "/goal", "the plan goal must match the job goal"));
  }
  if (plan.goal === "vulnerability_testing" && plan.loadScenarios.length > 0) {
    issues.push(
      issue(
        "goal_cross_population",
        "/loadScenarios",
        "vulnerability plans must not contain load scenarios",
      ),
    );
  }
  if (plan.goal === "load_testing" && plan.plannedTests.length > 0) {
    issues.push(
      issue(
        "goal_cross_population",
        "/plannedTests",
        "load plans must not contain vulnerability tests",
      ),
    );
  }
  if (plan.plannedTests.length > context.limits.maxPlannedTests) {
    issues.push(
      issue(
        "too_many_tests",
        "/plannedTests",
        `plans may contain at most ${context.limits.maxPlannedTests} tests`,
      ),
    );
  }
  if (plan.loadScenarios.length > context.limits.maxLoadScenarios) {
    issues.push(
      issue(
        "too_many_scenarios",
        "/loadScenarios",
        `plans may contain at most ${context.limits.maxLoadScenarios} load scenarios`,
      ),
    );
  }

  checkPlannedTests(plan, context, issues);
  checkLoadPaths(plan, issues);
  checkReferences(plan.evidenceReferences, "/evidenceReferences", context, issues);
  checkCredentialMaterial(plan, "", issues);

  const normalized = normalizePlan(plan, context);
  if (planBytes(normalized) > context.limits.maxPlanBytes) {
    issues.push(
      issue(
        "plan_too_large",
        "/",
        `the serialized plan exceeds ${context.limits.maxPlanBytes} bytes`,
      ),
    );
  }

  if (issues.length > 0) return { ok: false, issues };
  return { ok: true, plan: normalized };
}

export const PHASE_SEVEN_REQUIRED_INPUTS = Object.freeze({
  targetAuthorization: TARGET_AUTHORIZATION_INPUT,
  noRoute: NO_ROUTE_INPUT,
  unsupported: UNSUPPORTED_INPUT,
  catalog: CATALOG_INPUT,
  loadPath: LOAD_PATH_INPUT,
});
