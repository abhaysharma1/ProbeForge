// Run the schema, auth/RBAC, job API/SSE, plan approval, target authorization,
// results/reports, run lifecycle, end-to-end acceptance, queue/scheduler, and
// the analysis job loop tests. A real database is required; each Rust test
// creates and drops an isolated schema and never touches `public`. The analysis
// loop test uses its own fixture rows in `public` and cleans up after itself.
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

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
      "--test",
      "operations",
      "--test",
      "runs_lifecycle",
      "--test",
      "end_to_end",
      "--test",
      "admin_settings",
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

// The analysis job loop is a TypeScript integration test against the same
// database. It skips when the database URL is unset, so it must be given one.
console.log("\n==> analysis job loop (TypeScript)");
const analysisResult = spawnSync(process.execPath, ["test", "src/job-loop.integration.test.ts"], {
  cwd: join(root, "services", "analysis-worker"),
  stdio: "inherit",
  env: { ...env, PROBE_FORGE_TEST_DATABASE_URL: databaseUrl },
});
if (analysisResult.error) {
  console.error(`Unable to start the analysis test runner: ${analysisResult.error.message}`);
  process.exit(1);
}
if (analysisResult.status !== 0) {
  process.exit(analysisResult.status ?? 1);
}

process.exit(0);
