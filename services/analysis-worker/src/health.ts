import type { HealthResponse, ReadinessResponse } from "@probe-forge/contracts";

export const SERVICE_NAME = "probe-forge-analysis-worker" as const;
export const SERVICE_VERSION = "0.1.0";

export function healthResponse(): HealthResponse {
  return { status: "ok", service: SERVICE_NAME, version: SERVICE_VERSION };
}

/**
 * Phase 1 has no external dependencies to check. When the worker starts claiming
 * analysis jobs (Phase 4/7), readiness must include queue connectivity.
 */
export function readinessResponse(): ReadinessResponse {
  return { status: "ready", service: SERVICE_NAME };
}
