import rawCatalog from "../catalog.json";

export const CATALOG_CATEGORIES = ["static", "http", "network", "load"] as const;
export type CatalogCategory = (typeof CATALOG_CATEGORIES)[number];

export const CATALOG_RISKS = ["low", "medium", "high"] as const;
export type CatalogRisk = (typeof CATALOG_RISKS)[number];

export interface TestCatalogEntry {
  readonly id: string;
  readonly name: string;
  readonly category: CatalogCategory;
  readonly risk: CatalogRisk;
}

export interface TestCatalog {
  readonly schemaVersion: string;
  readonly catalogVersion: string;
  readonly entries: readonly TestCatalogEntry[];
}

/**
 * Test definitions may only reference vetted adapters by ID. Executable fields are
 * rejected here so an arbitrary command can never enter the catalog by accident.
 */
const FORBIDDEN_FIELD_NAMES = new Set(["command", "executable", "script", "shell"]);
const STABLE_ID_PATTERN = /^[a-z0-9][a-z0-9.-]*$/;

export function assertNoExecutableFields(record: Record<string, unknown>, context: string): void {
  for (const key of Object.keys(record)) {
    if (FORBIDDEN_FIELD_NAMES.has(key.toLowerCase())) {
      throw new Error(`${context} contains forbidden executable field "${key}"`);
    }
  }
}

function requireString(record: Record<string, unknown>, key: string, context: string): string {
  const value = record[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${context} requires a non-empty string field "${key}"`);
  }
  return value;
}

function parseEntry(value: unknown, index: number): TestCatalogEntry {
  if (typeof value !== "object" || value === null) {
    throw new Error(`catalog entry ${index} must be an object`);
  }
  const entry = value as Record<string, unknown>;
  assertNoExecutableFields(entry, `catalog entry ${index}`);

  const id = requireString(entry, "id", `catalog entry ${index}`);
  if (!STABLE_ID_PATTERN.test(id)) {
    throw new Error(`catalog entry ${index} has an invalid stable id "${id}"`);
  }

  const category = requireString(entry, "category", `catalog entry ${index}`);
  if (!(CATALOG_CATEGORIES as readonly string[]).includes(category)) {
    throw new Error(`catalog entry ${index} has an unsupported category "${category}"`);
  }

  const risk = requireString(entry, "risk", `catalog entry ${index}`);
  if (!(CATALOG_RISKS as readonly string[]).includes(risk)) {
    throw new Error(`catalog entry ${index} has an unsupported risk "${risk}"`);
  }

  return {
    id,
    name: requireString(entry, "name", `catalog entry ${index}`),
    category: category as CatalogCategory,
    risk: risk as CatalogRisk,
  };
}

export function parseTestCatalog(raw: unknown): TestCatalog {
  if (typeof raw !== "object" || raw === null) {
    throw new Error("catalog must be an object");
  }
  const record = raw as Record<string, unknown>;
  assertNoExecutableFields(record, "catalog");

  const schemaVersion = requireString(record, "schemaVersion", "catalog");
  const catalogVersion = requireString(record, "catalogVersion", "catalog");

  if (!Array.isArray(record.entries)) {
    throw new Error("catalog requires an entries array");
  }

  return {
    schemaVersion,
    catalogVersion,
    entries: record.entries.map((entry, index) => parseEntry(entry, index)),
  };
}

export const testCatalog: TestCatalog = parseTestCatalog(rawCatalog);
