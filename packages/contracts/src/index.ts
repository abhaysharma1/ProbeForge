/**
 * Stable shared contracts for the two supported job goals and service health.
 * Phase 2 adds versioned API/event schemas; keep these identifiers stable.
 */

export const JOB_GOALS = ["vulnerability_testing", "load_testing"] as const;

export type JobGoal = (typeof JOB_GOALS)[number];

export function isJobGoal(value: string): value is JobGoal {
  return (JOB_GOALS as readonly string[]).includes(value);
}

export const SERVICE_NAMES = [
  "probe-forge-api",
  "probe-forge-scheduler",
  "probe-forge-execution-worker",
  "probe-forge-analysis-worker",
] as const;

export type ServiceName = (typeof SERVICE_NAMES)[number];

export interface HealthResponse {
  readonly status: "ok";
  readonly service: ServiceName;
  readonly version: string;
}

export interface ReadinessResponse {
  readonly status: "ready" | "not_ready";
  readonly service: ServiceName;
}
