import type { JobEvent, JobEventType } from "@probe-forge/contracts";

import type { JobStatus } from "./api";

/**
 * Statuses a job can still move out of while the analysis worker holds it. Only
 * these are worth a live subscription; a terminal job emits nothing new, and an
 * awaiting-approval job stays put until a user records an approval.
 */
export const LIVE_JOB_STATUSES: readonly JobStatus[] = ["queued", "analyzing", "planning"];

export function isLiveJob(status: JobStatus): boolean {
  return LIVE_JOB_STATUSES.includes(status);
}

/**
 * Every event name the API can emit, mirroring the shared `JobEventType`
 * contract. `EventSource` dispatches named events to `addEventListener`, so each
 * name must be registered explicitly; the unnamed `stream.error` event arrives
 * on the default `message` channel instead.
 */
export const JOB_EVENT_TYPES: readonly JobEventType[] = [
  "job.queued",
  "job.claimed",
  "job.retry_scheduled",
  "job.failed",
  "job.cancelled",
  "job.requeued",
  "repository.clone.started",
  "repository.snapshot.ready",
  "repository.clone.failed",
  "agent.discovery.progress",
  "plan.proposed",
  "plan.approved",
  "target.validated",
  "run.scheduled",
  "run.started",
  "run.cancelled",
  "run.completed",
  "run.failed",
  "test.started",
  "test.progress",
  "test.completed",
  "test.failed",
  "finding.recorded",
  "report.completed",
  "report.failed",
];

/**
 * The status each lifecycle event implies. Other event types describe runs,
 * tests, and reports and do not change the parent job's status.
 */
const JOB_STATUS_BY_EVENT: Partial<Record<JobEventType, JobStatus>> = {
  "job.queued": "queued",
  "job.claimed": "analyzing",
  "job.retry_scheduled": "queued",
  "job.requeued": "queued",
  "job.failed": "failed",
  "job.cancelled": "cancelled",
  "plan.proposed": "awaiting_approval",
};

export function jobStatusForEvent(type: JobEventType): JobStatus | null {
  return JOB_STATUS_BY_EVENT[type] ?? null;
}

/**
 * Parse an SSE `data:` payload into the shared event envelope. Unknown or
 * malformed data yields `null` rather than surfacing a partial object.
 */
export function parseJobEvent(data: string): JobEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object") {
    return null;
  }
  const candidate = parsed as Record<string, unknown>;
  if (
    candidate.schemaVersion !== "1" ||
    typeof candidate.jobId !== "string" ||
    typeof candidate.sequence !== "number" ||
    typeof candidate.type !== "string" ||
    typeof candidate.occurredAt !== "string"
  ) {
    return null;
  }
  return parsed as JobEvent;
}
