import type { LoadScenario, Plan, PlannedTest, Risk } from "@probe-forge/contracts";
import type { RouteFact } from "../discovery/model";
import type { PlanningContext } from "./planning-input";
import { validateCandidatePlan } from "./validate";

export type FallbackReason =
  | "model_not_configured"
  | "model_not_found"
  | "model_unavailable"
  | "model_timeout"
  | "model_invalid_output"
  | "planning_limit_exceeded"
  | "prompt_too_large"
  | "unexpected";

export const FALLBACK_PROVIDER = "probe-forge";
export const FALLBACK_MODEL = "deterministic-fallback-v1";

const RISK_RANK: Readonly<Record<Risk, number>> = {
  low: 0,
  medium: 1,
  high: 2,
  critical: 3,
};

const MAX_PROBE_ROUTE_LENGTH = 2_000;

function hasControlCharacters(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint !== undefined && (codePoint < 0x20 || codePoint === 0x7f)) return true;
  }
  return false;
}

/**
 * Mirrors the adapter-side route contract: an absolute same-origin path with no
 * query, fragment, backslash, control characters, `..` segments, or empty
 * segments. Configuration is never allowed to expand the authorized scope.
 */
function isSafeProbeRoute(path: string): boolean {
  return (
    path.length > 0 &&
    path.length <= MAX_PROBE_ROUTE_LENGTH &&
    path.startsWith("/") &&
    !path.startsWith("//") &&
    !path.includes("//") &&
    !path.includes("?") &&
    !path.includes("#") &&
    !path.includes("\\") &&
    !path.includes("..") &&
    !hasControlCharacters(path)
  );
}

/**
 * Selects the same deterministic read-only route that the load fallback uses, so
 * an adapter-backed test is only proposed when a concrete path was discovered.
 */
function selectProbeRoute(context: PlanningContext): RouteFact | null {
  const route = context.inventory.routes
    .filter(
      (candidate) =>
        candidate.pathKind === "literal" &&
        candidate.path !== null &&
        isSafeProbeRoute(candidate.path) &&
        (candidate.method === "GET" || candidate.method === "HEAD"),
    )
    .sort((left, right) => (left.path ?? "").localeCompare(right.path ?? ""))[0];
  return route ?? null;
}

function reasonText(reason: FallbackReason): string {
  switch (reason) {
    case "model_not_configured":
      return "No planning model is configured.";
    case "model_not_found":
      return "The configured planning model is not available in the pinned SDK catalog.";
    case "model_timeout":
      return "The planning model did not finish within the wall-time limit.";
    case "model_invalid_output":
      return "The model proposal did not pass strict server validation.";
    case "planning_limit_exceeded":
      return "The planning run reached a server-owned iteration, tool, or output limit.";
    case "prompt_too_large":
      return "The bounded planning context exceeded its configured size.";
    case "unexpected":
      return "The planning run ended unexpectedly.";
    default:
      return "The planning model was unavailable.";
  }
}

function fallbackTest(
  context: PlanningContext,
  entry: PlanningContext["catalog"]["entries"][number],
): PlannedTest | null {
  // Every HTTP catalog test is executed against one concrete route. Without a
  // discovered literal route the test cannot be configured, so it is omitted
  // rather than persisted as an unexecutable planned test.
  const route = entry.category === "http" ? selectProbeRoute(context) : null;
  const probePath = route?.path ?? null;
  if (entry.category === "http" && probePath === null) {
    return null;
  }
  return {
    catalogTestId: entry.id,
    name: entry.name,
    rationale:
      "Server fallback selected this vetted catalog adapter without model assistance. Confirm that it applies to the observed application surface before approval.",
    risk: entry.risk,
    prerequisites: ["Authorized target confirmation"],
    expectedEvidence:
      "Adapter-defined evidence recorded by the server for the selected vetted test.",
    targetRequirements: ["authorized in-scope target"],
    requiresExplicitConfirmation: entry.risk === "high",
    configuration: probePath === null ? {} : { route: probePath },
  };
}

const LOAD_PROFILES = {
  low: {
    virtualUsers: 2,
    rampUpSeconds: 5,
    durationSeconds: 30,
    maxRequests: 200,
    maxRequestsPerSecond: 2,
  },
  standard: {
    virtualUsers: 5,
    rampUpSeconds: 10,
    durationSeconds: 60,
    maxRequests: 1_000,
    maxRequestsPerSecond: 5,
  },
  high: {
    virtualUsers: 10,
    rampUpSeconds: 20,
    durationSeconds: 120,
    maxRequests: 3_000,
    maxRequestsPerSecond: 10,
  },
} as const;

function fallbackLoadScenario(context: PlanningContext): LoadScenario | null {
  const route = selectProbeRoute(context);
  if (route === null || route.path === null) return null;
  const path = route.path;
  const name = `Fallback baseline load for ${path}`.slice(0, 160);
  return {
    name,
    description:
      "Deterministic baseline scenario generated by the server from a literal discovered route. The profile is intentionally conservative and requires human review.",
    requests: [
      {
        id: "baseline",
        method: route.method === "HEAD" ? "HEAD" : "GET",
        path,
        expectedStatuses: route.method === "HEAD" ? [200, 204] : [200],
      },
    ],
    profile: LOAD_PROFILES[context.job.intensity],
  };
}

/**
 * Builds a conservative, deterministic draft when model output is unavailable
 * or invalid. The fallback uses only vetted catalog entries and discovered
 * literal routes. It still passes the same strict server validation as a model
 * proposal, and it never authorizes or executes work.
 */
export function buildFallbackPlan(context: PlanningContext, reason: FallbackReason): Plan | null {
  const explanation = reasonText(reason);
  let candidate: Plan;
  if (context.job.goal === "vulnerability_testing") {
    // Route-less network/static tests remain runnable when discovery found no
    // literal HTTP route; HTTP entries are dropped rather than persisted as
    // unexecutable planned tests.
    const routeAvailable = selectProbeRoute(context) !== null;
    const entries = context.catalog.entries
      .filter(
        (entry) =>
          entry.category !== "load" &&
          (routeAvailable || entry.category !== "http") &&
          entry.name.length <= 160 &&
          !hasControlCharacters(entry.name),
      )
      .sort((left, right) => {
        const risk = RISK_RANK[left.risk] - RISK_RANK[right.risk];
        return risk !== 0 ? risk : left.id.localeCompare(right.id);
      })
      .slice(0, Math.min(3, context.limits.maxPlannedTests));
    if (entries.length === 0) return null;
    const plannedTests = entries
      .map((entry) => fallbackTest(context, entry))
      .filter((test): test is PlannedTest => test !== null);
    if (plannedTests.length === 0) return null;
    const requiredInputs = [
      "Review and confirm every fallback-selected catalog test before approval.",
    ];
    if (plannedTests.length < entries.length) {
      requiredInputs.push(
        "Provide a discovered literal HTTP route so adapter-backed tests can be configured.",
      );
    }
    candidate = {
      schemaVersion: "1",
      goal: "vulnerability_testing",
      summary:
        "Deterministic fallback plan generated by the server because the configured model was unavailable or its proposal was rejected.",
      confidence: 0.2,
      plannedTests,
      loadScenarios: [],
      requiredInputs,
      limitations: [
        explanation,
        "This fallback did not use model reasoning; applicability must be verified by a human.",
      ],
    };
  } else {
    const scenario = fallbackLoadScenario(context);
    if (scenario === null) return null;
    candidate = {
      schemaVersion: "1",
      goal: "load_testing",
      summary:
        "Deterministic fallback load plan generated by the server from a literal discovered route because model output was unavailable or invalid.",
      confidence: 0.2,
      plannedTests: [],
      loadScenarios: [scenario],
      requiredInputs: [
        "Review the discovered route and conservative load profile before approval.",
      ],
      limitations: [
        explanation,
        "No target was contacted while generating this fallback.",
        "The profile is a conservative baseline, not a capacity assessment.",
      ],
    };
  }

  const validated = validateCandidatePlan(candidate, context);
  return validated.ok ? validated.plan : null;
}
