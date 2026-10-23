import { describe, expect, it } from "bun:test";
import { findCatalogEntry, parseTestCatalog, testCatalog } from "./index";

const validEntry = {
  id: "http.tls.headers",
  name: "TLS and security header check",
  category: "http",
  risk: "low",
  description: "A bounded HTTP and TLS observation against an authorized target.",
  prerequisites: ["Authorized target"],
  toolRequirements: ["http_probe>=0.1"],
  targetRequirements: ["authorized target origin"],
  timeoutSeconds: 30,
  evidenceExpectations: ["Response headers and observation timestamp"],
  requiresExplicitConfirmation: false,
};

function catalogWith(entry: unknown, version = "0.2.0") {
  return {
    schemaVersion: "1",
    catalogVersion: version,
    entries: [entry],
  };
}

describe("bundled catalog", () => {
  it("loads as versioned schema 1 with complete bounded entries", () => {
    expect(testCatalog.schemaVersion).toBe("1");
    expect(testCatalog.catalogVersion).toBe("0.2.0");
    expect(testCatalog.entries.length).toBeGreaterThan(0);
    for (const entry of testCatalog.entries) {
      expect(entry.id).toMatch(/^[a-z0-9][a-z0-9.-]+$/);
      expect(entry.name.length).toBeGreaterThan(0);
      expect(entry.description.length).toBeGreaterThan(0);
      expect(entry.toolRequirements.length).toBeGreaterThan(0);
      expect(entry.targetRequirements.length).toBeGreaterThan(0);
      expect(entry.evidenceExpectations.length).toBeGreaterThan(0);
      expect(entry.timeoutSeconds).toBeGreaterThan(0);
    }
    expect(findCatalogEntry(testCatalog, "http.headers.missing")?.risk).toBe("low");
    expect(findCatalogEntry(testCatalog, "load.baseline")?.category).toBe("load");
  });
});

describe("catalog validation", () => {
  it("rejects executable command fields and unknown fields", () => {
    expect(() =>
      parseTestCatalog(catalogWith({ ...validEntry, command: "curl http://example.test" })),
    ).toThrow(/forbidden executable field/);
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, unexpected: "value" }))).toThrow(
      /unsupported field/,
    );
  });

  it("rejects unknown categories and risks", () => {
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, category: "exploit" }))).toThrow(
      /unsupported category/,
    );
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, risk: "catastrophic" }))).toThrow(
      /unsupported risk/,
    );
  });

  it("requires stable lowercase identifiers and unique IDs", () => {
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, id: "Not Stable" }))).toThrow(
      /invalid stable id/,
    );
    expect(() =>
      parseTestCatalog({
        schemaVersion: "1",
        catalogVersion: "0.2.0",
        entries: [validEntry, validEntry],
      }),
    ).toThrow(/duplicate stable id/);
  });

  it("requires bounded arrays, timeouts, and confirmation flags", () => {
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, toolRequirements: [] }))).toThrow(
      /toolRequirements/,
    );
    expect(() => parseTestCatalog(catalogWith({ ...validEntry, timeoutSeconds: 0 }))).toThrow(
      /timeoutSeconds/,
    );
    expect(() =>
      parseTestCatalog(catalogWith({ ...validEntry, requiresExplicitConfirmation: "yes" })),
    ).toThrow(/requiresExplicitConfirmation/);
    expect(() =>
      parseTestCatalog(catalogWith({ ...validEntry, description: "x".repeat(2_001) })),
    ).toThrow(/description/);
  });

  it("rejects an unsupported schema version", () => {
    expect(() => parseTestCatalog({ ...catalogWith(validEntry), schemaVersion: "2" })).toThrow(
      /schemaVersion/,
    );
  });
});
