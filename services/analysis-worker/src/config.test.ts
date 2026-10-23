import { describe, expect, it } from "bun:test";
import { loadConfig, WorkerConfigError } from "./config";

describe("loadConfig", () => {
  it("defaults to loopback port 8090", () => {
    expect(loadConfig({})).toEqual({ hostname: "127.0.0.1", port: 8090 });
  });

  it("accepts explicit host and port", () => {
    expect(
      loadConfig({
        PROBE_FORGE_ANALYSIS_WORKER_HOST: "localhost",
        PROBE_FORGE_ANALYSIS_WORKER_PORT: "9100",
      }),
    ).toEqual({ hostname: "localhost", port: 9100 });
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

  it("rejects an empty hostname", () => {
    expect(() => loadConfig({ PROBE_FORGE_ANALYSIS_WORKER_HOST: "  " })).toThrow(WorkerConfigError);
  });
});
