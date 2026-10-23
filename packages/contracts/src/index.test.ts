import { describe, expect, it } from "bun:test";
import { isJobGoal, JOB_GOALS, SERVICE_NAMES } from "./index";

describe("job goals", () => {
  it("exposes exactly the two supported goals", () => {
    expect(JOB_GOALS).toEqual(["vulnerability_testing", "load_testing"]);
  });

  it("rejects unsupported goal values", () => {
    expect(isJobGoal("penetration_testing")).toBe(false);
    expect(isJobGoal("load_testing")).toBe(true);
  });
});

describe("service names", () => {
  it("lists each service once", () => {
    expect(new Set(SERVICE_NAMES).size).toBe(SERVICE_NAMES.length);
    expect(SERVICE_NAMES).toHaveLength(4);
  });
});
