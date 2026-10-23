import { describe, expect, it } from "bun:test";
import { healthResponse, readinessResponse } from "./health";

describe("health responses", () => {
  it("reports ok for liveness", () => {
    expect(healthResponse()).toEqual({
      status: "ok",
      service: "probe-forge-analysis-worker",
      version: "0.1.0",
    });
  });

  it("reports ready for readiness", () => {
    expect(readinessResponse()).toEqual({
      status: "ready",
      service: "probe-forge-analysis-worker",
    });
  });
});
