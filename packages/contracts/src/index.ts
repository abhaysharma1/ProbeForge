/**
 * Stable shared contracts for job goals, domain payloads, and service health.
 * Domain payload schemas live in `schemas/probeforge.v1.schema.json`; runtime
 * validation lives in the `./validate` entry point so browser bundles do not
 * pull in the validator.
 */

export type * from "./model";
export { isJobGoal, JOB_GOALS } from "./model";

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
