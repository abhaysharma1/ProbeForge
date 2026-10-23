import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";

import manifest from "../fixtures/v1/manifest.json";
import {
  MAX_CONTRACT_BYTES,
  type ValidatorName,
  validateContractJson,
  validateJobCreation,
  validateLoadScenario,
  validateTargetAuthorization,
  validators,
} from "./validate";

const TOKEN_PLACEHOLDER = "$" + "{token}";

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

  it("rejects missing status assertions, oversized bodies, and loop fields", () => {
    const missingStatus = fixture("load-scenario.advanced.valid.json") as {
      requests: { expectedStatuses?: number[]; bodyTemplate?: unknown; loop?: unknown }[];
    };
    const first = missingStatus.requests[0];
    if (!first) throw new Error("fixture shape changed");
    delete first.expectedStatuses;
    expect(validateLoadScenario(missingStatus).ok).toBe(false);

    const oversized = fixture("load-scenario.advanced.valid.json") as {
      requests: { bodyTemplate?: unknown }[];
    };
    const oversizedRequest = oversized.requests[0];
    if (!oversizedRequest) throw new Error("fixture shape changed");
    oversizedRequest.bodyTemplate = { payload: "x".repeat(70 * 1024) };
    expect(validateLoadScenario(oversized).ok).toBe(false);

    const loop = fixture("load-scenario.valid.json") as {
      requests: { loop?: unknown }[];
    };
    const loopRequest = loop.requests[0];
    if (!loopRequest) throw new Error("fixture shape changed");
    loopRequest.loop = { while: true };
    expect(validateLoadScenario(loop).ok).toBe(false);

    const sameStep = fixture("load-scenario.advanced.valid.json") as {
      requests: {
        extract?: unknown;
        bodyTemplate?: unknown;
      }[];
    };
    const sameStepRequest = sameStep.requests[0];
    if (!sameStepRequest) throw new Error("fixture shape changed");
    sameStepRequest.extract = [{ name: "token", source: "header", selector: "x-session-token" }];
    sameStepRequest.bodyTemplate = { token: TOKEN_PLACEHOLDER };
    expect(validateLoadScenario(sameStep).ok).toBe(false);
  });

  it("does not treat a valid plan as a load scenario", () => {
    expect(validateLoadScenario(fixture("plan.load.valid.json")).ok).toBe(false);
  });

  it("matches IPv6 allowed hosts without requiring URL brackets", () => {
    const authorization = fixture("authorization.valid.json") as {
      scope: {
        origin: string;
        allowedHosts: string[];
        allowedPorts: number[];
      };
    };
    authorization.scope.origin = "https://[::1]:8443";
    authorization.scope.allowedHosts = ["::1"];
    authorization.scope.allowedPorts = [8443];

    expect(validateTargetAuthorization(authorization).ok).toBe(true);
  });
});
