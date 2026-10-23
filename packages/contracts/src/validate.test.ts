import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";

import manifest from "../fixtures/v1/manifest.json";
import {
  MAX_CONTRACT_BYTES,
  type ValidatorName,
  validateContractJson,
  validateJobCreation,
  validateLoadScenario,
  validators,
} from "./validate";

function fixture(path: string): unknown {
  return JSON.parse(readFileSync(new URL(`../fixtures/v1/${path}`, import.meta.url), "utf8"));
}

describe("shared contract fixtures", () => {
  for (const testCase of manifest.cases) {
    it(`${testCase.valid ? "accepts" : "rejects"} ${testCase.name}`, () => {
      const validator = validators[testCase.validator as ValidatorName];
      const result = validator(fixture(testCase.path));

      expect(result.ok).toBe(testCase.valid);
      if (!result.ok) {
        expect(result.issues.length).toBeGreaterThan(0);
      }
    });
  }
});

describe("contract validation boundaries", () => {
  it("rejects a payload above the byte ceiling before parsing", () => {
    const result = validateContractJson(" ".repeat(MAX_CONTRACT_BYTES + 1), validateJobCreation);

    expect(result.ok).toBe(false);
  });

  it("rejects malformed JSON without echoing the payload", () => {
    const secret = "not-json-secret-value";
    const result = validateContractJson(`{"token":"${secret}"`, validateJobCreation);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.issues[0]?.message).not.toContain(secret);
    }
  });

  it("rejects executable keys regardless of case", () => {
    const scenario = fixture("load-scenario.valid.json") as {
      requests: { bodyTemplate?: unknown }[];
    };
    const request = scenario.requests[0];
    if (!request) throw new Error("fixture shape changed");
    request.bodyTemplate = { CoMmAnD: "curl https://example.test" };

    expect(validateLoadScenario(scenario).ok).toBe(false);
  });

  it("does not treat a valid plan as a load scenario", () => {
    expect(validateLoadScenario(fixture("plan.load.valid.json")).ok).toBe(false);
  });
});
