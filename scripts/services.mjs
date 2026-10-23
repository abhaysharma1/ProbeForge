#!/usr/bin/env bun
// Starts, health-checks, and stops the local ProbeForge services.
//
//   bun scripts/services.mjs start   run all services in the foreground; Ctrl+C stops them
//   bun scripts/services.mjs check   start, verify health endpoints, then stop (exit 0/1)
//
// Requires the Rust workspace to be built first: `bun run build:rust`.

import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const root = resolve(import.meta.dirname, "..");
const isWindows = process.platform === "win32";

function binary(name) {
  return join(root, "target", "debug", isWindows ? `${name}.exe` : name);
}

function resolveBun() {
  const candidates = [
    process.env.BUN_INSTALL
      ? join(process.env.BUN_INSTALL, "bin", isWindows ? "bun.exe" : "bun")
      : null,
    join(homedir(), ".bun", "bin", isWindows ? "bun.exe" : "bun"),
    process.env.APPDATA
      ? join(
          process.env.APPDATA,
          "npm",
          "node_modules",
          "bun",
          "bin",
          isWindows ? "bun.exe" : "bun",
        )
      : null,
  ].filter((candidate) => candidate !== null);

  return candidates.find((candidate) => existsSync(candidate)) ?? "bun";
}

const bun = resolveBun();

const apiAddr = process.env.PROBE_FORGE_API_ADDR ?? "127.0.0.1:8080";
const schedulerAddr = process.env.PROBE_FORGE_SCHEDULER_ADDR ?? "127.0.0.1:8081";
const executionAddr = process.env.PROBE_FORGE_EXECUTION_WORKER_ADDR ?? "127.0.0.1:8082";
const analysisHost = process.env.PROBE_FORGE_ANALYSIS_WORKER_HOST ?? "127.0.0.1";
const analysisPort = process.env.PROBE_FORGE_ANALYSIS_WORKER_PORT ?? "8090";

const SERVICES = [
  {
    name: "api",
    command: binary("probe-forge-api"),
    args: [],
    // Database-aware readiness, not just process liveness.
    health: { url: `http://${apiAddr}/readyz`, kind: "json", status: "ready" },
  },
  {
    name: "scheduler",
    command: binary("probe-forge-scheduler"),
    args: [],
    // The scheduler now owns a database pool and stale-lease recovery.
    health: { url: `http://${schedulerAddr}/readyz`, kind: "json", status: "ready" },
  },
  {
    name: "execution-worker",
    command: binary("probe-forge-execution-worker"),
    args: [],
    health: { url: `http://${executionAddr}/healthz`, kind: "json" },
  },
  {
    name: "analysis-worker",
    command: bun,
    args: ["run", "start"],
    cwd: join(root, "services", "analysis-worker"),
    health: { url: `http://${analysisHost}:${analysisPort}/healthz`, kind: "json" },
  },
  {
    name: "web",
    command: bun,
    args: ["run", "dev"],
    cwd: join(root, "apps", "web"),
    health: { url: "http://127.0.0.1:5173/", kind: "html" },
  },
];

let stopping = false;

function fail(message) {
  console.error(`ERROR: ${message}`);
  process.exit(1);
}

function preflight() {
  for (const service of SERVICES) {
    if (service.command !== bun && !existsSync(service.command)) {
      fail(`missing ${service.command}. Run "bun run build:rust" first.`);
    }
  }
}

async function assertPortsFree() {
  for (const service of SERVICES) {
    try {
      const response = await fetch(service.health.url, { signal: AbortSignal.timeout(1000) });
      if (response.ok) {
        fail(`${service.health.url} is already served by another process; stop it first.`);
      }
    } catch {
      // Connection refused or timed out: the port is treated as free.
    }
  }
}

function pipeWithPrefix(stream, name) {
  let buffer = "";
  stream.setEncoding("utf8");
  stream.on("data", (chunk) => {
    buffer += chunk;
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      if (line.trim().length > 0) console.log(`[${name}] ${line}`);
    }
  });
}

function startService(service) {
  const child = spawn(service.command, service.args, {
    cwd: service.cwd ?? root,
    env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? "info" },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
    detached: !isWindows,
  });

  pipeWithPrefix(child.stdout, service.name);
  pipeWithPrefix(child.stderr, service.name);

  child.on("exit", (code, signal) => {
    if (!stopping)
      console.error(`[${service.name}] exited unexpectedly (code=${code}, signal=${signal})`);
  });

  service.child = child;
  return child;
}

function stopChild(child) {
  if (!child?.pid || child.exitCode !== null) return;
  if (isWindows) {
    try {
      execFileSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    } catch {
      // Process already stopped.
    }
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      try {
        child.kill("SIGTERM");
      } catch {
        // Process already stopped.
      }
    }
  }
}

async function waitForHealthy(service, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (service.child.exitCode !== null) {
      throw new Error(`${service.name} exited before becoming healthy`);
    }
    try {
      const response = await fetch(service.health.url, { signal: AbortSignal.timeout(2000) });
      if (response.ok) {
        if (service.health.kind === "json") {
          const body = await response.json();
          if (body.status === (service.health.status ?? "ok")) return;
        } else {
          const text = await response.text();
          if (text.includes("ProbeForge")) return;
        }
      }
    } catch {
      // Not ready yet.
    }
    await delay(250);
  }
  throw new Error(`${service.name} did not become healthy at ${service.health.url}`);
}

async function runCheck() {
  preflight();
  await assertPortsFree();

  const running = [];
  stopping = false;
  try {
    for (const service of SERVICES) {
      running.push({ service, child: startService(service) });
    }
    for (const { service } of running) {
      await waitForHealthy(service);
      console.log(`ok   ${service.name.padEnd(18)} ${service.health.url}`);
    }
    console.log(`\nPASS: ${running.length}/${SERVICES.length} local services healthy.`);
  } catch (error) {
    console.error(`\nFAIL: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  } finally {
    stopping = true;
    for (const { child } of running) stopChild(child);
    await delay(300);
  }

  process.exit(process.exitCode ?? 0);
}

async function runStart() {
  preflight();
  await assertPortsFree();

  const running = SERVICES.map((service) => ({ service, child: startService(service) }));

  console.log(`Started ${running.length} local services:`);
  for (const { service } of running) {
    console.log(`  ${service.name.padEnd(18)} ${service.health.url}`);
  }
  console.log("Press Ctrl+C to stop.");

  const shutdown = () => {
    if (stopping) return;
    stopping = true;
    console.log("\nStopping local services...");
    for (const { child } of running) stopChild(child);
    setTimeout(() => process.exit(0), 300);
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
  await new Promise(() => {
    // Keep the runner alive until a signal arrives.
  });
}

const mode = process.argv[2];
if (mode === "check") {
  await runCheck();
} else if (mode === "start") {
  await runStart();
} else {
  fail("usage: node scripts/services.mjs <start|check>");
}
