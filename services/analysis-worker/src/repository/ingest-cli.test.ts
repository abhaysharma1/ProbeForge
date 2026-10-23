import { describe, expect, it } from "bun:test";
import { readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createFixtureRepository } from "./testing";

const PACKAGE_ROOT = fileURLToPath(new URL("../../", import.meta.url));

function runCli(args: readonly string[]): ReturnType<typeof Bun.spawnSync> {
  return Bun.spawnSync([process.execPath, "src/ingest-cli.ts", ...args], {
    cwd: PACKAGE_ROOT,
    env: { ...process.env, PROBE_FORGE_ANALYSIS_ALLOW_LOCAL_FIXTURES: "false" },
    stdout: "pipe",
    stderr: "pipe",
  });
}

describe("ingest CLI", () => {
  it("analyzes a local inert fixture and writes a snapshot and audit log", async () => {
    const repo = await createFixtureRepository();
    const auditLog = join(repo.tempRoot, "audit.jsonl");
    const output = join(repo.tempRoot, "snapshot.json");
    const discoveryOutput = join(repo.tempRoot, "inventory.json");
    try {
      const result = runCli([
        "--fixture",
        repo.root,
        "--ref",
        "main",
        "--audit-log",
        auditLog,
        "--output",
        output,
        "--discovery-output",
        discoveryOutput,
      ]);
      expect(result.exitCode).toBe(0);
      const snapshot = JSON.parse(await readFile(output, "utf8")) as {
        commitSha?: string;
        analysisMetadata?: { executedRepositoryCode?: boolean };
        files?: unknown[];
      };
      expect(snapshot.commitSha).toMatch(/^[0-9a-f]{40}$/);
      expect(snapshot.analysisMetadata?.executedRepositoryCode).toBe(false);
      expect(snapshot.files?.length).toBeGreaterThan(0);
      const inventory = JSON.parse(await readFile(discoveryOutput, "utf8")) as {
        commitSha?: string;
        executedRepositoryCode?: boolean;
        routes?: Array<{ method?: string; path?: string; framework?: string }>;
        coverage?: { examinedFiles?: number };
      };
      expect(inventory.commitSha).toBe(snapshot.commitSha);
      expect(inventory.executedRepositoryCode).toBe(false);
      expect(inventory.coverage?.examinedFiles).toBeGreaterThan(0);
      expect(inventory.routes).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ method: "GET", path: "/fixture", framework: "FastAPI" }),
        ]),
      );
      const events = (await readFile(auditLog, "utf8"))
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line) as { event: string });
      expect(events.map((event) => event.event)).toEqual(
        expect.arrayContaining([
          "repository.clone.started",
          "repository.snapshot.ready",
          "repository.workspace.cleaned",
        ]),
      );
    } finally {
      await rm(output, { force: true });
      await rm(discoveryOutput, { force: true });
      await repo.cleanup();
    }
  }, 30_000);

  it("rejects an invalid URL scheme without cloning", async () => {
    const repo = await createFixtureRepository();
    try {
      const result = runCli([
        "--url",
        "http://github.com/example/repo.git",
        "--ref",
        "main",
        "--audit-log",
        join(repo.tempRoot, "audit.jsonl"),
      ]);
      expect(result.exitCode).toBe(1);
      const stderr = result.stderr?.toString() ?? "";
      expect(stderr).toContain("unsupported_scheme");
      expect(stderr).not.toContain("http://github.com");
    } finally {
      await repo.cleanup();
    }
  });
});
