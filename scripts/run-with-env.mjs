#!/usr/bin/env bun
// Runs a command with the local environment loaded from .env.
//
// Bun loads .env for a Bun file (this script), but not for shell-style package
// scripts, so `cargo`-based npm scripts run through this wrapper to inherit
// DATABASE_URL and the other PROBE_FORGE_* values.
//
//   bun scripts/run-with-env.mjs <command> [args...]
import { spawnSync } from "node:child_process";

const [command, ...args] = process.argv.slice(2);
if (command === undefined) {
  console.error("usage: bun scripts/run-with-env.mjs <command> [args...]");
  process.exit(2);
}

const result = spawnSync(command, args, { stdio: "inherit", env: process.env });
if (result.error) {
  console.error(`failed to start ${command}: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
