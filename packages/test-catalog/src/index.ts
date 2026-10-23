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
  readonly description: string;
  readonly prerequisites: readonly string[];
  readonly toolRequirements: readonly string[];
  readonly targetRequirements: readonly string[];
  readonly timeoutSeconds: number;
  readonly evidenceExpectations: readonly string[];
  readonly requiresExplicitConfirmation: boolean;
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
const STABLE_ID_PATTERN = /^[a-z0-9][a-z0-9.-]{1,119}$/;
const MAX_ID_LENGTH = 120;
const MAX_NAME_LENGTH = 160;
const MAX_DESCRIPTION_LENGTH = 2_000;
const MAX_CATALOG_VERSION_LENGTH = 64;
const MAX_TOTAL_ENTRIES = 500;

const ENTRY_KEYS = new Set([
  "id",
  "name",
  "category",
  "risk",
  "description",
  "prerequisites",
  "toolRequirements",
  "targetRequirements",
  "timeoutSeconds",
  "evidenceExpectations",
  "requiresExplicitConfirmation",
]);
const CATALOG_KEYS = new Set(["schemaVersion", "catalogVersion", "entries"]);

export function assertNoExecutableFields(record: Record<string, unknown>, context: string): void {
  for (const key of Object.keys(record)) {
    if (FORBIDDEN_FIELD_NAMES.has(key.toLowerCase())) {
      throw new Error(`${context} contains forbidden executable field "${key}"`);
    }
  }
}

function assertExactKeys(
  record: Record<string, unknown>,
  allowed: ReadonlySet<string>,
  context: string,
): void {
  for (const key of Object.keys(record)) {
    if (!allowed.has(key)) {
      throw new Error(`${context} contains unsupported field "${key}"`);
    }
  }
}

function hasControlCharacters(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint !== undefined && (codePoint < 0x20 || codePoint === 0x7f)) return true;
  }
  return false;
}

function requireText(
  record: Record<string, unknown>,
  key: string,
  maxLength: number,
  context: string,
): string {
  const value = record[key];
  if (typeof value !== "string" || value.trim().length === 0 || value.length > maxLength) {
    throw new Error(`${context} requires a non-empty string field "${key}"`);
  }
  if (hasControlCharacters(value)) {
    throw new Error(`${context} field "${key}" contains control characters`);
  }
  return value;
}

function requireStringList(
  record: Record<string, unknown>,
  key: string,
  maxItems: number,
  maxLength: number,
  allowEmpty: boolean,
  context: string,
): readonly string[] {
  const value = record[key];
  if (!Array.isArray(value) || (!allowEmpty && value.length === 0) || value.length > maxItems) {
    throw new Error(`${context} requires a bounded string array field "${key}"`);
  }
  return value.map((item, index) => {
    if (typeof item !== "string" || item.trim().length === 0 || item.length > maxLength) {
      throw new Error(`${context} field "${key}/${index}" is not a valid string`);
    }
    if (hasControlCharacters(item)) {
      throw new Error(`${context} field "${key}/${index}" contains control characters`);
    }
    return item;
  });
}

function requireInteger(
  record: Record<string, unknown>,
  key: string,
  minimum: number,
  maximum: number,
  context: string,
): number {
  const value = record[key];
  if (!Number.isSafeInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw new Error(`${context} requires an integer field "${key}"`);
  }
  return value as number;
}

function requireBoolean(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== "boolean") {
    throw new Error(`${context} requires a boolean field "${key}"`);
  }
  return value;
}

function parseEntry(value: unknown, index: number): TestCatalogEntry {
  const context = `catalog entry ${index}`;
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${context} must be an object`);
  }
  const entry = value as Record<string, unknown>;
  assertNoExecutableFields(entry, context);
  assertExactKeys(entry, ENTRY_KEYS, context);

  const id = requireText(entry, "id", MAX_ID_LENGTH, context);
  if (!STABLE_ID_PATTERN.test(id)) {
    throw new Error(`${context} has an invalid stable id "${id}"`);
  }

  const category = requireText(entry, "category", 20, context);
  if (!(CATALOG_CATEGORIES as readonly string[]).includes(category)) {
    throw new Error(`${context} has an unsupported category "${category}"`);
  }

  const risk = requireText(entry, "risk", 20, context);
  if (!(CATALOG_RISKS as readonly string[]).includes(risk)) {
    throw new Error(`${context} has an unsupported risk "${risk}"`);
  }

  return {
    id,
    name: requireText(entry, "name", MAX_NAME_LENGTH, context),
    category: category as CatalogCategory,
    risk: risk as CatalogRisk,
    description: requireText(entry, "description", MAX_DESCRIPTION_LENGTH, context),
    prerequisites: requireStringList(entry, "prerequisites", 20, 300, true, context),
    toolRequirements: requireStringList(entry, "toolRequirements", 10, 200, false, context),
    targetRequirements: requireStringList(entry, "targetRequirements", 10, 300, false, context),
    timeoutSeconds: requireInteger(entry, "timeoutSeconds", 1, 3_600, context),
    evidenceExpectations: requireStringList(entry, "evidenceExpectations", 20, 500, false, context),
    requiresExplicitConfirmation: requireBoolean(entry, "requiresExplicitConfirmation", context),
  };
}

export function parseTestCatalog(raw: unknown): TestCatalog {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new Error("catalog must be an object");
  }
  const record = raw as Record<string, unknown>;
  assertNoExecutableFields(record, "catalog");
  assertExactKeys(record, CATALOG_KEYS, "catalog");

  const schemaVersion = requireText(record, "schemaVersion", 10, "catalog");
  if (schemaVersion !== "1") {
    throw new Error("catalog schemaVersion must be 1");
  }
  const catalogVersion = requireText(
    record,
    "catalogVersion",
    MAX_CATALOG_VERSION_LENGTH,
    "catalog",
  );

  if (!Array.isArray(record.entries)) {
    throw new Error("catalog requires an entries array");
  }
  if (record.entries.length > MAX_TOTAL_ENTRIES) {
    throw new Error(`catalog must contain at most ${MAX_TOTAL_ENTRIES} entries`);
  }

  const entries = record.entries.map((entry, index) => parseEntry(entry, index));
  const ids = new Set<string>();
  for (const entry of entries) {
    if (ids.has(entry.id)) throw new Error(`catalog contains duplicate stable id "${entry.id}"`);
    ids.add(entry.id);
  }

  return { schemaVersion, catalogVersion, entries };
}

export function findCatalogEntry(
  catalog: TestCatalog,
  catalogTestId: string,
): TestCatalogEntry | null {
  return catalog.entries.find((entry) => entry.id === catalogTestId) ?? null;
}

export const testCatalog: TestCatalog = parseTestCatalog(rawCatalog);
