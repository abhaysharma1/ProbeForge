import type { Plan } from "@probe-forge/contracts";

export type AgentSessionStatus = "completed" | "failed" | "cancelled";

export interface StartAgentSessionInput {
  readonly projectId: string;
  readonly jobId: string;
  readonly provider: string;
  readonly model: string;
  readonly metadata: Readonly<Record<string, unknown>>;
}

export interface CompleteAgentSessionInput {
  readonly sessionId: string;
  readonly status: AgentSessionStatus;
  readonly metadata: Readonly<Record<string, unknown>>;
}

export interface SavePlanInput {
  readonly sessionId: string;
  readonly projectId: string;
  readonly jobId: string;
  readonly plan: Plan;
  readonly provider: string;
  readonly model: string;
  readonly createdByAgent: boolean;
  readonly fallbackReason: string | null;
}

export interface SavedPlan {
  readonly id: string;
  readonly jobId: string;
  readonly projectId: string;
  readonly version: number;
  readonly status: "proposed";
}

export interface PlanStore {
  startSession(input: StartAgentSessionInput): Promise<string>;
  completeSession(input: CompleteAgentSessionInput): Promise<void>;
  savePlan(input: SavePlanInput): Promise<SavedPlan>;
  close(): Promise<void>;
}

export type PlanStoreErrorCode =
  | "invalid_input"
  | "session_not_found"
  | "job_not_found"
  | "job_not_active"
  | "session_closed"
  | "persistence_failed";

export class PlanStoreError extends Error {
  readonly code: PlanStoreErrorCode;

  constructor(code: PlanStoreErrorCode, message: string) {
    super(message);
    this.name = "PlanStoreError";
    this.code = code;
  }
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const MAX_METADATA_BYTES = 32 * 1024;

export function assertUuid(value: string, label: string): void {
  if (!UUID_RE.test(value)) {
    throw new PlanStoreError("invalid_input", `${label} must be a UUID`);
  }
}

export function metadataJson(metadata: Readonly<Record<string, unknown>>): string {
  if (metadata === null || typeof metadata !== "object" || Array.isArray(metadata)) {
    throw new PlanStoreError("invalid_input", "session metadata must be an object");
  }
  let json: string;
  try {
    json = JSON.stringify(metadata);
  } catch {
    throw new PlanStoreError("invalid_input", "session metadata must be serializable JSON");
  }
  if (json === undefined || new TextEncoder().encode(json).byteLength > MAX_METADATA_BYTES) {
    throw new PlanStoreError("invalid_input", "session metadata exceeds the byte limit");
  }
  return json;
}

function assertPortableProvider(provider: string, model: string): void {
  if (provider.trim().length === 0 || provider.length > 120) {
    throw new PlanStoreError("invalid_input", "session provider is outside the allowed length");
  }
  if (model.trim().length === 0 || model.length > 160) {
    throw new PlanStoreError("invalid_input", "session model is outside the allowed length");
  }
}

/**
 * Deterministic in-memory store for unit and fixture tests. It mirrors the
 * transactional store's monotonic per-job versioning and session state rules.
 */
export class InMemoryPlanStore implements PlanStore {
  private readonly sessions = new Map<
    string,
    {
      projectId: string;
      jobId: string;
      provider: string;
      model: string;
      status: "running" | AgentSessionStatus;
      metadata: string;
    }
  >();
  private readonly plans = new Map<string, SavedPlan[]>();
  private readonly planPayloads = new Map<string, Plan>();
  private closed = false;

  async startSession(input: StartAgentSessionInput): Promise<string> {
    if (this.closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertUuid(input.projectId, "projectId");
    assertUuid(input.jobId, "jobId");
    assertPortableProvider(input.provider, input.model);
    const metadata = metadataJson(input.metadata);
    const id = crypto.randomUUID();
    this.sessions.set(id, {
      projectId: input.projectId,
      jobId: input.jobId,
      provider: input.provider,
      model: input.model,
      status: "running",
      metadata,
    });
    return id;
  }

  async completeSession(input: CompleteAgentSessionInput): Promise<void> {
    if (this.closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertUuid(input.sessionId, "sessionId");
    const session = this.sessions.get(input.sessionId);
    if (session === undefined) {
      throw new PlanStoreError("session_not_found", "agent session was not found");
    }
    if (session.status !== "running") {
      throw new PlanStoreError("session_closed", "agent session is already terminal");
    }
    session.status = input.status;
    session.metadata = metadataJson(input.metadata);
  }

  async savePlan(input: SavePlanInput): Promise<SavedPlan> {
    if (this.closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertUuid(input.projectId, "projectId");
    assertUuid(input.jobId, "jobId");
    assertUuid(input.sessionId, "sessionId");
    assertPortableProvider(input.provider, input.model);
    const session = this.sessions.get(input.sessionId);
    if (session === undefined) {
      throw new PlanStoreError("session_not_found", "agent session was not found");
    }
    if (session.projectId !== input.projectId || session.jobId !== input.jobId) {
      throw new PlanStoreError("job_not_found", "agent session does not belong to this job");
    }
    if (session.status !== "running") {
      throw new PlanStoreError("session_closed", "agent session is already terminal");
    }

    const existing = this.plans.get(input.jobId) ?? [];
    const saved: SavedPlan = {
      id: crypto.randomUUID(),
      jobId: input.jobId,
      projectId: input.projectId,
      version: existing.length + 1,
      status: "proposed",
    };
    existing.push(saved);
    this.plans.set(input.jobId, existing);
    this.planPayloads.set(saved.id, input.plan);
    session.status = "completed";
    session.metadata = JSON.stringify({
      outcome: "plan_saved",
      planId: saved.id,
      planVersion: saved.version,
      fallback: input.createdByAgent === false,
      fallbackReason: input.fallbackReason,
      plannedTestCount: input.plan.plannedTests.length,
      loadScenarioCount: input.plan.loadScenarios.length,
    });
    return saved;
  }

  async close(): Promise<void> {
    this.closed = true;
  }

  plansForJob(jobId: string): readonly SavedPlan[] {
    return [...(this.plans.get(jobId) ?? [])];
  }

  planFor(planId: string): Plan | null {
    return this.planPayloads.get(planId) ?? null;
  }

  session(sessionId: string): {
    projectId: string;
    jobId: string;
    provider: string;
    model: string;
    status: string;
    metadata: string;
  } | null {
    return this.sessions.get(sessionId) ?? null;
  }
}
