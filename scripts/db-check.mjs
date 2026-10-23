// Run the schema migration and Phase 3 auth/RBAC integration tests. A real
// database is required; each test creates and drops an isolated schema and
// never touches `public`.
import { spawnSync } from "node:child_process";

const databaseUrl = process.env.DATABASE_URL?.trim();
if (!databaseUrl) {
  console.error(
    "DATABASE_URL is required. Start the local database with `bun run db:up` and export DATABASE_URL (see .env.example).",
  );
  process.exit(1);
}

const result = spawnSync(
  "cargo",
  [
    "test",
    "-p",
    "probe-forge-api",
    "--test",
    "migrations",
    "--test",
    "auth_projects",
    "--",
    "--nocapture",
  ],
  {
    stdio: "inherit",
    env: {
      ...process.env,
      DATABASE_URL: databaseUrl,
      PROBE_FORGE_REQUIRE_DB_TESTS: "1",
    },
  },
);

if (result.error) {
  console.error(`Unable to start cargo: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
