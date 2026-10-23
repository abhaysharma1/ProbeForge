import { describe, expect, it } from "bun:test";
import { parseTestCatalog, testCatalog } from "./index";

const validEntry = {
  id: "http.tls.headers",
  name: "TLS and security header check",
  category: "http",
  risk: "low",
};

describe("bundled catalog", () => {
  it("loads as versioned schema 1", () => {
    expect(testCatalog.schemaVersion).toBe("1");
    expect(Array.isArray(testCatalog.entries)).toBe(true);
  });
});

describe("catalog validation", () => {
  it("rejects executable command fields", () => {
    expect(() =>
      parseTestCatalog({
        schemaVersion: "1",
        catalogVersion: "0.1.0",
        entries: [{ ...validEntry, command: "curl http://example.test" }],
      }),
    ).toThrow(/forbidden executable field/);
  });

  it("rejects unknown categories and risks", () => {
    expect(() =>
      parseTestCatalog({
        schemaVersion: "1",
        catalogVersion: "0.1.0",
        entries: [{ ...validEntry, category: "exploit" }],
      }),
    ).toThrow(/unsupported category/);

    expect(() =>
      parseTestCatalog({
        schemaVersion: "1",
        catalogVersion: "0.1.0",
        entries: [{ ...validEntry, risk: "catastrophic" }],
      }),
    ).toThrow(/unsupported risk/);
  });

  it("requires stable lowercase identifiers", () => {
    expect(() =>
      parseTestCatalog({
        schemaVersion: "1",
        catalogVersion: "0.1.0",
        entries: [{ ...validEntry, id: "Not Stable" }],
      }),
    ).toThrow(/invalid stable id/);
  });
});
