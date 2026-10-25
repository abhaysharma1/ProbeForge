import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";
import type { SQL } from "bun";
import { processJobOnce } from "./job-loop";
import { DEFAULT_INGESTION_LIMITS } from "./repository/limits";
import { createFixtureRepository, type TestRepository } from "./repository/testing";

/**
 * Phase 15 analysis acceptance. Runs the real job loop against an inert local
 * fixture repository. No model is configured, so the deterministic fallback
 * planner is used, and no target is ever contacted.
 */
const connectionString = process.env.PROBE_FORGE_TEST_DATABASE_URL;
const databaseDescribe = connectionString === undefined ? describe.skip : describe;

function requiredId(rows: readonly { id: string }[], label: string): string {
  const id = rows[0]?.id;
  if (id === undefined) throw new Error(`${label} insert returned no id`);
  return id;
}

databaseDescribe("analysis job loop (integration)", () => {
  let admin: SQL;
  let repo: TestRepository;
  let userId: string;
  let projectId: string;
  let jobId: string;

  beforeAll(async () => {
    if (connectionString === undefined) return;
    admin = new Bun.SQL(connectionString, { max: 2 });
    repo = await createFixtureRepository();
    const suffix = randomUUID().slice(0, 12);
    userId = requiredId(
      await admin<{ id: string }[]>`
        INSERT INTO users (email, display_name)
        VALUES (${`annloop-${suffix}@example.test`}, ${"Analysis loop fixture"})
        RETURNING id
      `,
      "user",
    );
    projectId = requiredId(
      await admin<{ id: string }[]>`
        INSERT INTO projects (name, slug, owner_id)
        VALUES (${"Analysis loop fixture"}, ${`annloop-${suffix}`}, ${userId})
        RETURNING id
      `,
      "project",
    );
    const gitSourceId = requiredId(
      await admin<{ id: string }[]>`
        INSERT INTO git_sources (project_id, provider, normalized_url, default_reference)
        VALUES (${projectId}, 'generic', ${pathToFileURL(repo.root).href}, 'main')
        RETURNING id
      `,
      "git source",
    );
    jobId = requiredId(
      await admin<{ id: string }[]>`
        INSERT INTO jobs (
          project_id, created_by, goal, intensity, status, display_name,
          git_source_id, source_reference
        ) VALUES (
          ${projectId}, ${userId}, 'vulnerability_testing', 'standard', 'queued',
          ${"Analysis loop fixture job"}, ${gitSourceId}, 'main'
        )
        RETURNING id
      `,
      "job",
    );
  });

  afterAll(async () => {
    if (connectionString === undefined) return;
    if (projectId !== undefined) {
      // audit_log and job_events are immutable by design; this disposable test
      // database clears only its own fixture rows with the triggers briefly
      // disabled. The worker now writes job_events, so that trigger must be
      // disabled too or the cascade from `projects` is rejected.
      await admin`ALTER TABLE audit_log DISABLE TRIGGER audit_log_immutable`;
      await admin`ALTER TABLE job_events DISABLE TRIGGER job_events_immutable`;
      try {
        await admin`DELETE FROM audit_log WHERE project_id = ${projectId}`;
        await admin`DELETE FROM projects WHERE id = ${projectId}`;
        await admin`DELETE FROM users WHERE id = ${userId}`;
      } finally {
        await admin`ALTER TABLE job_events ENABLE TRIGGER job_events_immutable`;
        await admin`ALTER TABLE audit_log ENABLE TRIGGER audit_log_immutable`;
      }
    }
    await repo?.cleanup();
    await admin.close({ timeout: 5 });
  });

  it("claims a queued job and persists a deterministic draft plan", async () => {
    if (connectionString === undefined) return;
    const result = await processJobOnce(connectionString, {
      workerId: "job-loop-test",
      allowLocalFixtures: true,
      allowedGitHosts: ["github.com"],
      ingestionLimits: DEFAULT_INGESTION_LIMITS,
      audit: repo.audit,
    });
    expect(result?.jobId).toBe(jobId);
    expect(result?.outcome).toBe("awaiting_approval");

    const jobs = await admin<
      { status: string; current_plan_id: string | null; worker_id: string | null }[]
    >`
      SELECT status, current_plan_id, worker_id FROM jobs WHERE id = ${jobId}
    `;
    expect(jobs[0]?.status).toBe("awaiting_approval");
    expect(jobs[0]?.current_plan_id).not.toBeNull();
    expect(jobs[0]?.worker_id).toBeNull();

    const planRows = await admin<{ version: number; created_by_agent: boolean; goal: string }[]>`
      SELECT version, created_by_agent, plan_payload->>'goal' AS goal
      FROM test_plans WHERE job_id = ${jobId}
    `;
    expect(planRows).toHaveLength(1);
    expect(planRows[0]?.created_by_agent).toBe(false);
    expect(planRows[0]?.goal).toBe("vulnerability_testing");

    const tests = await admin<{ count: string }[]>`
      SELECT count(*)::text AS count FROM planned_tests
      WHERE plan_id = ${jobs[0]?.current_plan_id}
    `;
    expect(Number(tests[0]?.count)).toBeGreaterThan(0);

    const audit = await admin<{ count: string }[]>`
      SELECT count(*)::text AS count FROM audit_log
      WHERE action = 'plan.fallback_proposed'
    `;
    expect(Number(audit[0]?.count)).toBeGreaterThan(0);

    // The worker-driven transitions are recorded in the ordered job event
    // stream so the API can replay them over SSE.
    const events = await admin<{ event_type: string; sequence: string }[]>`
      SELECT event_type, sequence FROM job_events
      WHERE job_id = ${jobId}
      ORDER BY sequence
    `;
    expect(events.map((event) => event.event_type)).toEqual(["job.claimed", "plan.proposed"]);

    // A second call finds no queued job.
    const second = await processJobOnce(connectionString, {
      workerId: "job-loop-test",
      allowLocalFixtures: true,
      allowedGitHosts: ["github.com"],
      ingestionLimits: DEFAULT_INGESTION_LIMITS,
      audit: repo.audit,
    });
    expect(second).toBeNull();
  });
});
