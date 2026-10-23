import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import type { SQL } from "bun";
import { createPostgresPlanStore, type PostgresPlanStore } from "./postgres-store";
import { makeContext, makeValidLoadPlan, makeValidVulnerabilityPlan } from "./test-helpers";

const connectionString = process.env.PROBE_FORGE_TEST_DATABASE_URL;
const databaseDescribe = connectionString === undefined ? describe.skip : describe;

function requiredId(rows: readonly { id: string }[], label: string): string {
  const id = rows[0]?.id;
  if (id === undefined) throw new Error(`${label} fixture insert returned no id`);
  return id;
}

databaseDescribe("PostgresPlanStore integration", () => {
  let admin: SQL;
  let store: PostgresPlanStore;
  let userId: string;
  let projectId: string;
  let gitSourceId: string;
  let jobId: string;
  let loadJobId: string;

  beforeAll(async () => {
    if (connectionString === undefined) return;
    admin = new Bun.SQL(connectionString, { max: 2 });
    store = createPostgresPlanStore(connectionString);
    const suffix = crypto.randomUUID().slice(0, 12);
    const userRows = await admin<{ id: string }[]>`
      INSERT INTO users (email, display_name)
      VALUES (${`planning-${suffix}@example.test`}, ${"Planning fixture"})
      RETURNING id
    `;
    userId = requiredId(userRows, "user");
    const projectRows = await admin<{ id: string }[]>`
      INSERT INTO projects (name, slug, owner_id)
      VALUES (${"Planning fixture"}, ${`pf-${suffix}`}, ${userId})
      RETURNING id
    `;
    projectId = requiredId(projectRows, "project");
    const sourceRows = await admin<{ id: string }[]>`
      INSERT INTO git_sources (project_id, provider, normalized_url, default_reference)
      VALUES (
        ${projectId}, 'generic', ${`https://example.test/${suffix}`}, 'refs/heads/main'
      )
      RETURNING id
    `;
    gitSourceId = requiredId(sourceRows, "git source");
    const jobRows = await admin<{ id: string }[]>`
      INSERT INTO jobs (
        project_id, created_by, goal, intensity, status, display_name,
        git_source_id, source_reference
      ) VALUES (
        ${projectId}, ${userId}, 'vulnerability_testing', 'standard', 'planning',
        ${"Planning fixture job"}, ${gitSourceId}, 'refs/heads/main'
      )
      RETURNING id
    `;
    jobId = requiredId(jobRows, "job");
    const loadJobRows = await admin<{ id: string }[]>`
      INSERT INTO jobs (
        project_id, created_by, goal, intensity, status, display_name,
        git_source_id, source_reference
      ) VALUES (
        ${projectId}, ${userId}, 'load_testing', 'low', 'planning',
        ${"Load planning fixture job"}, ${gitSourceId}, 'refs/heads/main'
      )
      RETURNING id
    `;
    loadJobId = requiredId(loadJobRows, "load job");
  });

  afterAll(async () => {
    if (connectionString === undefined) return;
    if (projectId !== undefined) {
      // The audit table is immutable by design. This integration test uses the
      // operator's disposable local database, so cleanup temporarily disables
      // the trigger, deletes only its own fixture rows, and re-enables it.
      await admin`ALTER TABLE audit_log DISABLE TRIGGER audit_log_immutable`;
      try {
        await admin`DELETE FROM audit_log WHERE project_id = ${projectId}`;
        await admin`DELETE FROM projects WHERE id = ${projectId}`;
        await admin`DELETE FROM users WHERE id = ${userId}`;
      } finally {
        await admin`ALTER TABLE audit_log ENABLE TRIGGER audit_log_immutable`;
      }
    }
    await admin.close({ timeout: 5 });
    await store.close();
  });

  it("starts a session, persists a proposed plan and planned tests atomically", async () => {
    const context = makeContext();
    const sessionId = await store.startSession({
      projectId,
      jobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 1 },
    });
    const saved = await store.savePlan({
      sessionId,
      projectId,
      jobId,
      plan: makeValidVulnerabilityPlan(),
      provider: "faux",
      model: "planner-fixture",
      createdByAgent: true,
      fallbackReason: null,
    });

    expect(saved.version).toBe(1);
    expect(saved.status).toBe("proposed");
    const sessionRows = await admin<{ status: string; trace_metadata: { outcome?: string } }[]>`
      SELECT status, trace_metadata FROM agent_sessions WHERE id = ${sessionId}
    `;
    expect(sessionRows[0]?.status).toBe("completed");
    expect(sessionRows[0]?.trace_metadata.outcome).toBe("plan_saved");

    const planRows = await admin<
      {
        version: number;
        created_by_agent: boolean;
        agent_provider: string;
        agent_model: string;
        plan_payload: { goal: string };
      }[]
    >`SELECT version, created_by_agent, agent_provider, agent_model, plan_payload
       FROM test_plans WHERE id = ${saved.id}`;
    expect(planRows[0]?.version).toBe(1);
    expect(planRows[0]?.created_by_agent).toBe(true);
    expect(planRows[0]?.agent_provider).toBe("faux");
    expect(planRows[0]?.agent_model).toBe("planner-fixture");
    expect(planRows[0]?.plan_payload.goal).toBe("vulnerability_testing");

    const testRows = await admin<{ count: string }[]>`
      SELECT count(*)::text AS count FROM planned_tests WHERE plan_id = ${saved.id}
    `;
    expect(testRows[0]?.count).toBe("1");

    const jobRows = await admin<{ current_plan_id: string }[]>`
      SELECT current_plan_id FROM jobs WHERE id = ${jobId}
    `;
    expect(jobRows[0]?.current_plan_id).toBe(saved.id);

    const auditRows = await admin<{ action: string; resource_id: string }[]>`
      SELECT action, resource_id FROM audit_log
      WHERE resource_id = ${saved.id} AND action = 'plan.proposed'
    `;
    expect(auditRows[0]?.resource_id).toBe(saved.id);
    expect(context.job.goal).toBe("vulnerability_testing");
  });

  it("allocates distinct versions for concurrent saves on the same job", async () => {
    const firstSession = await store.startSession({
      projectId,
      jobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 2 },
    });
    const secondSession = await store.startSession({
      projectId,
      jobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 3 },
    });
    const base = {
      projectId,
      jobId,
      plan: makeValidVulnerabilityPlan(),
      provider: "faux",
      model: "planner-fixture",
      fallbackReason: null,
    } as const;
    const [first, second] = await Promise.all([
      store.savePlan({ ...base, sessionId: firstSession, createdByAgent: true }),
      store.savePlan({ ...base, sessionId: secondSession, createdByAgent: false }),
    ]);
    expect(new Set([first.version, second.version])).toEqual(new Set([2, 3]));

    const rows = await admin<{ version: number }[]>`
      SELECT version FROM test_plans WHERE job_id = ${jobId} ORDER BY version
    `;
    expect(rows.map((row) => row.version)).toEqual([1, 2, 3]);

    const sessions = await admin<{ status: string }[]>`
      SELECT status FROM agent_sessions
      WHERE id IN (${firstSession}, ${secondSession})
      ORDER BY id
    `;
    expect(sessions.every((session) => session.status === "completed")).toBe(true);
  });

  it("rejects a second save on a terminal session without duplicating the plan", async () => {
    const context = makeContext();
    const sessionId = await store.startSession({
      projectId,
      jobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 4 },
    });
    await store.savePlan({
      sessionId,
      projectId,
      jobId,
      plan: makeValidVulnerabilityPlan(),
      provider: "faux",
      model: "planner-fixture",
      createdByAgent: true,
      fallbackReason: null,
    });
    await expect(
      store.savePlan({
        sessionId,
        projectId,
        jobId,
        plan: makeValidVulnerabilityPlan({ summary: context.job.displayName }),
        provider: "faux",
        model: "planner-fixture",
        createdByAgent: true,
        fallbackReason: null,
      }),
    ).rejects.toMatchObject({ code: "session_closed" });

    const rows = await admin<{ count: string }[]>`
      SELECT count(*)::text AS count FROM test_plans WHERE job_id = ${jobId}
    `;
    expect(rows[0]?.count).toBe("4");
  });

  it("persists load profiles and selectable load scenarios for load plans", async () => {
    const sessionId = await store.startSession({
      projectId,
      jobId: loadJobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 1 },
    });
    const saved = await store.savePlan({
      sessionId,
      projectId,
      jobId: loadJobId,
      plan: makeValidLoadPlan(),
      provider: "faux",
      model: "planner-fixture",
      createdByAgent: true,
      fallbackReason: null,
    });
    expect(saved.version).toBe(1);

    const scenarioRows = await admin<
      {
        scenario_id: string;
        profile_id: string;
        selected: boolean;
        virtual_users: number;
        name: string;
      }[]
    >`
      SELECT s.id AS scenario_id, s.profile_id, s.selected, p.virtual_users, s.name
      FROM load_scenarios s
      JOIN load_profiles p ON p.id = s.profile_id
      WHERE s.plan_id = ${saved.id}
    `;
    expect(scenarioRows).toHaveLength(1);
    expect(scenarioRows[0]?.name).toBe("Baseline API read");
    expect(scenarioRows[0]?.virtual_users).toBe(2);
    expect(scenarioRows[0]?.selected).toBe(false);
  });

  it("rejects a plan when the job was cancelled while the session was running", async () => {
    const sessionId = await store.startSession({
      projectId,
      jobId,
      provider: "faux",
      model: "planner-fixture",
      metadata: { phase: "planning", attempt: 5 },
    });
    await admin`UPDATE jobs SET status = 'cancelled' WHERE id = ${jobId}`;
    try {
      await expect(
        store.savePlan({
          sessionId,
          projectId,
          jobId,
          plan: makeValidVulnerabilityPlan(),
          provider: "faux",
          model: "planner-fixture",
          createdByAgent: true,
          fallbackReason: null,
        }),
      ).rejects.toMatchObject({ code: "job_not_active" });
    } finally {
      await admin`UPDATE jobs SET status = 'planning' WHERE id = ${jobId}`;
    }

    const rows = await admin<{ count: string }[]>`
      SELECT count(*)::text AS count FROM test_plans WHERE job_id = ${jobId}
    `;
    expect(rows[0]?.count).toBe("4");
  });
});
