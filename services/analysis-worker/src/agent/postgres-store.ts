import type { SQL } from "bun";
import {
  type AgentSessionStatus,
  assertUuid,
  type CompleteAgentSessionInput,
  metadataJson,
  type PlanStore,
  PlanStoreError,
  type SavedPlan,
  type SavePlanInput,
  type StartAgentSessionInput,
} from "./store";

const MAX_PLAN_BYTES = 256 * 1024;

interface SessionRow {
  readonly job_id: string;
  readonly project_id: string;
  readonly status: string;
}

interface VersionRow {
  readonly next_version: number | string;
}

function wrapStoreError(error: unknown, message: string): never {
  if (error instanceof PlanStoreError) throw error;
  throw new PlanStoreError("persistence_failed", message);
}

function assertSavePlan(input: SavePlanInput): void {
  assertUuid(input.projectId, "projectId");
  assertUuid(input.jobId, "jobId");
  assertUuid(input.sessionId, "sessionId");
  if (input.provider.trim().length === 0 || input.provider.length > 120) {
    throw new PlanStoreError("invalid_input", "plan provider is outside the allowed length");
  }
  if (input.model.trim().length === 0 || input.model.length > 160) {
    throw new PlanStoreError("invalid_input", "plan model is outside the allowed length");
  }
  if (input.fallbackReason !== null && input.fallbackReason.length > 120) {
    throw new PlanStoreError("invalid_input", "fallback reason is outside the allowed length");
  }
  const payloadJson = JSON.stringify(input.plan);
  if (new TextEncoder().encode(payloadJson).byteLength > MAX_PLAN_BYTES) {
    throw new PlanStoreError("invalid_input", "plan payload exceeds the database limit");
  }
}

/**
 * Narrow PostgreSQL writer for planning sessions and draft plans. The agent
 * never receives this object or a database credential. Version allocation is
 * serialized by locking the job row, so concurrent attempts produce distinct
 * plan versions rather than violating the unique constraint.
 */
export class PostgresPlanStore implements PlanStore {
  readonly #sql: SQL;
  #closed = false;

  constructor(sql: SQL) {
    this.#sql = sql;
  }

  async startSession(input: StartAgentSessionInput): Promise<string> {
    if (this.#closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertUuid(input.projectId, "projectId");
    assertUuid(input.jobId, "jobId");
    if (input.provider.trim().length === 0 || input.provider.length > 120) {
      throw new PlanStoreError("invalid_input", "session provider is outside the allowed length");
    }
    if (input.model.trim().length === 0 || input.model.length > 160) {
      throw new PlanStoreError("invalid_input", "session model is outside the allowed length");
    }
    const metadata = JSON.parse(metadataJson(input.metadata)) as Record<string, unknown>;

    try {
      const rows = await this.#sql<{ id: string }[]>`
        INSERT INTO agent_sessions (project_id, job_id, provider, model, status, trace_metadata)
        SELECT p.id, j.id, ${input.provider}, ${input.model}, 'running', ${this.#sql.json(metadata)}
        FROM jobs j
        JOIN projects p ON p.id = j.project_id
        WHERE j.id = ${input.jobId} AND j.project_id = ${input.projectId}
        RETURNING id
      `;
      const id = rows[0]?.id;
      if (id === undefined) {
        throw new PlanStoreError("job_not_found", "job was not found for this project");
      }
      return id;
    } catch (error) {
      wrapStoreError(error, "unable to start the planning session");
    }
  }

  async completeSession(input: CompleteAgentSessionInput): Promise<void> {
    if (this.#closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertUuid(input.sessionId, "sessionId");
    const statuses: readonly AgentSessionStatus[] = ["completed", "failed", "cancelled"];
    if (!statuses.includes(input.status)) {
      throw new PlanStoreError("invalid_input", "session status is not supported");
    }
    const metadata = JSON.parse(metadataJson(input.metadata)) as Record<string, unknown>;
    try {
      await this.#sql`
        UPDATE agent_sessions
        SET status = ${input.status},
            finished_at = now(),
            trace_metadata = trace_metadata || ${this.#sql.json(metadata)}
        WHERE id = ${input.sessionId} AND status = 'running'
      `;
    } catch (error) {
      wrapStoreError(error, "unable to finish the planning session");
    }
  }

  async savePlan(input: SavePlanInput): Promise<SavedPlan> {
    if (this.#closed) throw new PlanStoreError("persistence_failed", "plan store is closed");
    assertSavePlan(input);

    try {
      return await this.#sql.begin(async (tx) => {
        const sessionRows = await tx<SessionRow[]>`
          SELECT job_id, project_id, status
          FROM agent_sessions
          WHERE id = ${input.sessionId}
          FOR UPDATE
        `;
        const session = sessionRows[0];
        if (session === undefined) {
          throw new PlanStoreError("session_not_found", "agent session was not found");
        }
        if (session.job_id !== input.jobId || session.project_id !== input.projectId) {
          throw new PlanStoreError("job_not_found", "agent session does not belong to this job");
        }
        if (session.status !== "running") {
          throw new PlanStoreError("session_closed", "agent session is already terminal");
        }

        const jobRows = await tx<{ id: string; status: string }[]>`
          SELECT id, status FROM jobs
          WHERE id = ${input.jobId} AND project_id = ${input.projectId}
          FOR UPDATE
        `;
        const job = jobRows[0];
        if (job === undefined) {
          throw new PlanStoreError("job_not_found", "job was not found for this project");
        }
        if (job.status === "cancelled" || job.status === "failed" || job.status === "expired") {
          throw new PlanStoreError("job_not_active", "job is terminal and cannot accept a plan");
        }

        const versionRows = await tx<VersionRow[]>`
          SELECT COALESCE(MAX(version), 0) + 1 AS next_version
          FROM test_plans
          WHERE job_id = ${input.jobId}
        `;
        const version = Number(versionRows[0]?.next_version ?? 1);
        if (!Number.isSafeInteger(version) || version < 1) {
          throw new PlanStoreError("persistence_failed", "unable to allocate a plan version");
        }

        const planRows = await tx<{ id: string }[]>`
          INSERT INTO test_plans (
            project_id, job_id, version, status, schema_version, summary, confidence,
            plan_payload, agent_provider, agent_model, created_by_agent
          ) VALUES (
            ${input.projectId}, ${input.jobId}, ${version}, 'proposed', '1',
            ${input.plan.summary}, ${input.plan.confidence}, ${tx.json(input.plan)},
            ${input.provider}, ${input.model}, ${input.createdByAgent}
          )
          RETURNING id
        `;
        const planId = planRows[0]?.id;
        if (planId === undefined) {
          throw new PlanStoreError("persistence_failed", "unable to persist the draft plan");
        }

        for (const test of input.plan.plannedTests) {
          await tx`
            INSERT INTO planned_tests (
              project_id, plan_id, catalog_test_id, name, rationale, risk,
              prerequisites, expected_evidence, target_requirements,
              requires_explicit_confirmation, configuration, rationale_references, selected
            ) VALUES (
              ${input.projectId}, ${planId}, ${test.catalogTestId}, ${test.name},
              ${test.rationale}, ${test.risk}, ${tx.json(test.prerequisites ?? [])},
              ${test.expectedEvidence}, ${tx.json(test.targetRequirements ?? [])},
              ${test.requiresExplicitConfirmation},
              ${tx.json(test.configuration ?? {})},
              ${tx.json(test.rationaleReferences ?? [])}, false
            )
          `;
        }

        for (const scenario of input.plan.loadScenarios) {
          const profile = scenario.profile;
          const profileRows = await tx<{ id: string }[]>`
            INSERT INTO load_profiles (
              project_id, name, virtual_users, arrival_rate_per_second,
              ramp_up_seconds, duration_seconds, max_requests,
              max_requests_per_second, stop_conditions
            ) VALUES (
              ${input.projectId}, ${scenario.name}, ${profile.virtualUsers ?? null},
              ${profile.arrivalRatePerSecond ?? null}, ${profile.rampUpSeconds},
              ${profile.durationSeconds}, ${profile.maxRequests},
              ${profile.maxRequestsPerSecond}, ${tx.json(profile.stopConditions ?? {})}
            )
            RETURNING id
          `;
          const profileId = profileRows[0]?.id;
          if (profileId === undefined) {
            throw new PlanStoreError("persistence_failed", "unable to persist a load profile");
          }
          await tx`
            INSERT INTO load_scenarios (
              project_id, plan_id, profile_id, name, schema_version, scenario, selected
            ) VALUES (
              ${input.projectId}, ${planId}, ${profileId}, ${scenario.name}, '1',
              ${tx.json(scenario)}, false
            )
          `;
        }

        await tx`
          UPDATE jobs SET current_plan_id = ${planId}
          WHERE id = ${input.jobId} AND project_id = ${input.projectId}
        `;

        const sessionMetadata = {
          outcome: "plan_saved",
          planId,
          planVersion: version,
          fallback: input.createdByAgent === false,
          fallbackReason: input.fallbackReason,
          plannedTestCount: input.plan.plannedTests.length,
          loadScenarioCount: input.plan.loadScenarios.length,
        };
        await tx`
          UPDATE agent_sessions
          SET status = 'completed',
              finished_at = now(),
              trace_metadata = trace_metadata || ${tx.json(sessionMetadata)}
          WHERE id = ${input.sessionId} AND status = 'running'
        `;

        const auditMetadata = {
          jobId: input.jobId,
          planVersion: version,
          goal: input.plan.goal,
          provider: input.provider,
          model: input.model,
          plannedTestCount: input.plan.plannedTests.length,
          loadScenarioCount: input.plan.loadScenarios.length,
          fallback: input.createdByAgent === false,
          fallbackReason: input.fallbackReason,
        };
        await tx`
          INSERT INTO audit_log (
            project_id, actor_id, action, resource_type, resource_id, metadata
          ) VALUES (
            ${input.projectId}, NULL,
            ${input.createdByAgent ? "plan.proposed" : "plan.fallback_proposed"},
            'test_plan', ${planId}, ${tx.json(auditMetadata)}
          )
        `;

        return {
          id: planId,
          jobId: input.jobId,
          projectId: input.projectId,
          version,
          status: "proposed",
        };
      });
    } catch (error) {
      wrapStoreError(error, "unable to persist the draft plan");
    }
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    await this.#sql.close({ timeout: 5 });
  }
}

export function createPostgresPlanStore(connectionString: string): PostgresPlanStore {
  let url: URL;
  try {
    url = new URL(connectionString);
  } catch {
    throw new PlanStoreError("invalid_input", "database URL is not valid");
  }
  if (url.protocol !== "postgres:" && url.protocol !== "postgresql:") {
    throw new PlanStoreError("invalid_input", "database URL must use the postgres scheme");
  }
  const sql = new Bun.SQL(connectionString, {
    max: 4,
    idleTimeout: 30,
    connectionTimeout: 10,
  });
  return new PostgresPlanStore(sql);
}
