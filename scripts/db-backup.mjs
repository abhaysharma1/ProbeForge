#!/usr/bin/env bun
// PostgreSQL backup, restore, and a safe restore roundtrip check.
//
//   bun scripts/db-backup.mjs backup [--output <file>]
//   bun scripts/db-backup.mjs restore --input <file> --into <database> [--force]
//   bun scripts/db-backup.mjs check
//
// `backup` and `restore` use `pg_dump`/`psql` inside the local Compose
// PostgreSQL container (override with PROBE_FORGE_DB_CONTAINER). `restore`
// refuses to write into the database named by DATABASE_URL unless `--force` is
// given, so the primary database cannot be clobbered by accident. `check`
// dumps the source database, restores it into a throwaway scratch database,
// verifies the migration table matches, and drops the scratch database. It
// never modifies the source database.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const container = process.env.PROBE_FORGE_DB_CONTAINER ?? "probe-forge-local-postgres-1";
const scratchDatabase = "probeforge_restore_check";

function fail(message) {
  console.error(`ERROR: ${message}`);
  process.exit(1);
}

function run(args, { input } = {}) {
  const result = spawnSync("docker", ["exec", ...args], {
    input,
    maxBuffer: 512 * 1024 * 1024,
  });
  if (result.error) fail(`docker exec failed: ${result.error.message}`);
  if (result.status !== 0) {
    fail(`docker exec ${args.join(" ")} exited with ${result.status}\n${result.stderr}`);
  }
  return result.stdout?.toString("utf8") ?? "";
}

function parseDatabaseUrl() {
  const url = process.env.DATABASE_URL?.trim();
  if (!url) {
    fail("DATABASE_URL is required (see .env.example).");
  }
  const parsed = new URL(url);
  const database = parsed.pathname.replace(/^\//, "");
  if (!parsed.username || !database) {
    fail("DATABASE_URL must include a username and database name.");
  }
  return { user: decodeURIComponent(parsed.username), database };
}

function parseFlags(argv) {
  const flags = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--force") {
      flags.force = true;
      continue;
    }
    if (token.startsWith("--")) {
      flags[token.slice(2)] = argv[index + 1];
      index += 1;
    }
  }
  return flags;
}

function dumpTo(user, database, file) {
  const sql = run([
    container,
    "pg_dump",
    "-U",
    user,
    "-d",
    database,
    "--no-owner",
    "--no-privileges",
  ]);
  writeFileSync(file, sql);
  return sql.length;
}

function psqlOn(user, database, sql) {
  return run(["-i", container, "psql", "-U", user, "-d", database, "-v", "ON_ERROR_STOP=1"], {
    input: sql,
  });
}

function recreateScratch(user) {
  run([
    container,
    "psql",
    "-U",
    user,
    "-d",
    "postgres",
    "-v",
    "ON_ERROR_STOP=1",
    "-c",
    `DROP DATABASE IF EXISTS ${scratchDatabase}`,
  ]);
  run([
    container,
    "psql",
    "-U",
    user,
    "-d",
    "postgres",
    "-v",
    "ON_ERROR_STOP=1",
    "-c",
    `CREATE DATABASE ${scratchDatabase}`,
  ]);
}

function dropScratch(user) {
  run([
    container,
    "psql",
    "-U",
    user,
    "-d",
    "postgres",
    "-v",
    "ON_ERROR_STOP=1",
    "-c",
    `DROP DATABASE IF EXISTS ${scratchDatabase}`,
  ]);
}

function migrationCount(user, database) {
  const out = run([
    container,
    "psql",
    "-U",
    user,
    "-d",
    database,
    "-tA",
    "-c",
    "SELECT count(*) FROM _sqlx_migrations WHERE success",
  ]);
  return out.trim();
}

function backup(argv) {
  const flags = parseFlags(argv);
  const { user, database } = parseDatabaseUrl();
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const output = resolve(flags.output ?? join("backups", `probeforge-${stamp}.sql`));
  mkdirSync(resolve(output, ".."), { recursive: true });
  const bytes = dumpTo(user, database, output);
  console.log(`Backup written: ${output} (${bytes} bytes)`);
}

function restore(argv) {
  const flags = parseFlags(argv);
  const { user, database: primary } = parseDatabaseUrl();
  if (!flags.input) fail("restore requires --input <file>.");
  if (!flags.into) fail("restore requires --into <database>.");
  if (flags.into === primary && !flags.force) {
    fail(
      `refusing to restore into the primary database "${primary}"; pass --force if you really mean it.`,
    );
  }
  const input = resolve(flags.input);
  statSync(input);
  psqlOn(user, flags.into, readFileSync(input));
  console.log(`Restore complete: ${input} -> ${flags.into}`);
}

function check() {
  const { user, database } = parseDatabaseUrl();
  const directory = mkdtempSync(join(tmpdir(), "pf-backup-"));
  const file = join(directory, "dump.sql");
  try {
    const source = migrationCount(user, database);
    console.log(`Source ${database}: ${source} applied migrations`);
    dumpTo(user, database, file);
    recreateScratch(user);
    psqlOn(user, scratchDatabase, readFileSync(file));
    const restored = migrationCount(user, scratchDatabase);
    if (restored !== source) {
      fail(`migration count mismatch after restore: source=${source} restored=${restored}`);
    }
    console.log(`Scratch ${scratchDatabase}: ${restored} applied migrations (matches source)`);
    console.log("PASS: backup and restore roundtrip verified.");
  } finally {
    try {
      dropScratch(user);
    } catch {
      // Best-effort cleanup.
    }
    rmSync(directory, { recursive: true, force: true });
  }
}

const [mode, ...rest] = process.argv.slice(2);
switch (mode) {
  case "backup":
    backup(rest);
    break;
  case "restore":
    restore(rest);
    break;
  case "check":
    check();
    break;
  default:
    fail("usage: bun scripts/db-backup.mjs <backup|restore|check> [flags]");
}
