// Run the schema, auth/RBAC, job API/SSE, plan approval, target authorization,
// and queue integration tests. A real database is required; each test creates
// and drops an isolated schema and never touches `public`.
import { spawnSync } from "node:child_process";

const databaseUrl = process.env.DATABASE_URL?.trim();
if (!databaseUrl) {
  console.error(
    "DATABASE_URL is required. Start the local database with `bun run db:up` and export DATABASE_URL (see .env.example).",
  );
  process.exit(1);
}

const env = {
  ...process.env,
  DATABASE_URL: databaseUrl,
  PROBE_FORGE_REQUIRE_DB_TESTS: "1",
};

const commands = [
  [
    "api schema/auth/jobs/plans/targets",
    [
      "test",
      "-p",
      "probe-forge-api",
      "--test",
      "migrations",
      "--test",
      "auth_projects",
      "--test",
      "jobs_queue",
      "--test",
      "plans_approval",
      "--test",
      "targets_authorization",
      "--test",
      "results_reports",
      "--",
      "--nocapture",
    ],
  ],
  [
    "queue leases and scheduler",
    ["test", "-p", "probe-forge-queue", "--test", "queue", "--", "--nocapture"],
  ],
];

for (const [label, args] of commands) {
  console.log(`\n==> ${label}`);
  const result = spawnSync("cargo", args, { stdio: "inherit", env });
  if (result.error) {
    console.error(`Unable to start cargo: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

process.exit(0);
