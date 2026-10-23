import { describe, expect, it } from "bun:test";
import { loadConfig, WorkerConfigError } from "./config";
import { DEFAULT_INGESTION_LIMITS } from "./repository/limits";

const DEFAULT_CONFIG = {
  hostname: "127.0.0.1",
  port: 8090,
  ingestion: {
    limits: DEFAULT_INGESTION_LIMITS,
    allowedGitHosts: ["github.com", "gitlab.com", "bitbucket.org"],
    allowLocalFixtures: false,
    auditLogPath: null,
  },
};

describe("loadConfig", () => {
  it("defaults to loopback port 8090 and deny-by-default ingestion", () => {
    expect(loadConfig({})).toEqual(DEFAULT_CONFIG);
  });

  it("accepts explicit host and port", () => {
    expect(
      loadConfig({
        PROBE_FORGE_ANALYSIS_WORKER_HOST: "localhost",
        PROBE_FORGE_ANALYSIS_WORKER_PORT: "9100",
      }),
    ).toEqual({ ...DEFAULT_CONFIG, hostname: "localhost", port: 9100 });
  });

  it("accepts explicit ingestion limits, hosts, fixture mode, and audit path", () => {
    const config = loadConfig({
      PROBE_FORGE_ANALYSIS_MAX_FILES: "123",
      PROBE_FORGE_ANALYSIS_MAX_TOTAL_BYTES: "4567",
      PROBE_FORGE_ANALYSIS_MAX_FILE_BYTES: "1000",
      PROBE_FORGE_ANALYSIS_MAX_DEPTH: "12",
      PROBE_FORGE_ANALYSIS_MAX_WORKSPACE_BYTES: "9000",
      PROBE_FORGE_ANALYSIS_MAX_GIT_OUTPUT_BYTES: "8000",
      PROBE_FORGE_ANALYSIS_GIT_TIMEOUT_MS: "5000",
      PROBE_FORGE_ANALYSIS_MAX_INVENTORY_FILES: "50",
      PROBE_FORGE_ANALYSIS_MAX_DEPENDENCIES_PER_MANIFEST: "20",
      PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS: "GitHub.com, git.example.test",
      PROBE_FORGE_ANALYSIS_ALLOW_LOCAL_FIXTURES: "true",
      PROBE_FORGE_ANALYSIS_AUDIT_LOG: " audit.jsonl ",
    });
    expect(config.ingestion.limits).toEqual({
      maxFiles: 123,
      maxTotalBytes: 4567,
      maxFileBytes: 1000,
      maxDepth: 12,
      maxWorkspaceBytes: 9000,
      maxGitOutputBytes: 8000,
      gitTimeoutMs: 5000,
      maxInventoryFiles: 50,
      maxDependenciesPerManifest: 20,
    });
    expect(config.ingestion.allowedGitHosts).toEqual(["github.com", "git.example.test"]);
    expect(config.ingestion.allowLocalFixtures).toBe(true);
    expect(config.ingestion.auditLogPath).toBe("audit.jsonl");
  });

  it("rejects invalid ports", () => {
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_WORKER_PORT: "not-a-port" })).toThrow(
      WorkerConfigError,
    );
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_WORKER_PORT: "0" })).toThrow(WorkerConfigError);
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_WORKER_PORT: "70000" })).toThrow(
      WorkerConfigError,
    );
  });

  it("rejects invalid and unsafe ingestion settings", () => {
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_MAX_FILES: "0" })).toThrow(WorkerConfigError);
    expect(() =>
      loadConfig({
        PROBE_FORGE_ANALYSIS_MAX_TOTAL_BYTES: "100",
        PROBE_FORGE_ANALYSIS_MAX_FILE_BYTES: "101",
      }),
    ).toThrow(WorkerConfigError);
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_ALLOW_LOCAL_FIXTURES: "yes" })).toThrow(
      WorkerConfigError,
    );
    expect(() =>
      loadConfig({ PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS: "https://github.com" }),
    ).toThrow(WorkerConfigError);
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS: " , " })).toThrow(
      WorkerConfigError,
    );
  });

  it("rejects an empty hostname", () => {
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_WORKER_HOST: "  " })).toThrow(WorkerConfigError);
  });
});
