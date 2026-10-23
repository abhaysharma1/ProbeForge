/**
 * Runtime contract validation for server-side TypeScript (analysis worker,
 * scripts, future API adapters). Import this module through
 * `@probe-forge/contracts/validate`; the root entry point stays free of Ajv so
 * the browser bundle does not include a validator.
 */

import Ajv2020, { type ErrorObject, type ValidateFunction } from "ajv/dist/2020.js";

import schema from "../schemas/probeforge.v1.schema.json";
import type {
  JobCreation,
  JobEvent,
  JsonValue,
  LoadAssertion,
  LoadExtraction,
  LoadProfile,
  LoadScenario,
  Plan,
  PlannedTest,
  RunRequest,
  TargetAuthorization,
  TestResult,
} from "./model";

export const MAX_CONTRACT_BYTES = 256 * 1024;

const SCHEMA_ID = schema.$id;
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const NIL_UUID_RE = /^00000000-0000-0000-0000-000000000000$/i;
const RFC3339_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const FORBIDDEN_KEYS = new Set([
  "command",
  "cmd",
  "script",
  "scripts",
  "executable",
  "shell",
  "entrypoint",
]);
const CREDENTIAL_HEADERS = new Set([
  "authorization",
  "cookie",
  "proxy-authorization",
  "set-cookie",
]);
const FORBIDDEN_LOAD_HEADERS = new Set([
  "host",
  "content-length",
  "transfer-encoding",
  "connection",
  "keep-alive",
]);
const HEADER_NAME_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
const LOAD_STEP_ID_RE = /^[a-z][a-z0-9_-]{0,63}$/;
const LOAD_VARIABLE_RE = /^[a-z][a-z0-9_]{0,63}$/;
const LOAD_MAX_BODY_BYTES = 64 * 1024;
const TEMPLATE_FORM = "$" + "{name}";

export interface ValidationIssue {
  readonly path: string;
  readonly message: string;
}

export type ValidationResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

const ajv = new Ajv2020({
  allErrors: true,
  strict: true,
  strictRequired: false,
});

ajv.addFormat("uuid", {
  type: "string",
  validate: (value: string) => UUID_RE.test(value),
});
ajv.addFormat("uri", {
  type: "string",
  validate: (value: string) => {
    try {
      new URL(value);
      return true;
    } catch {
      return false;
    }
  },
});
ajv.addFormat("date-time", {
  type: "string",
  validate: (value: string) => RFC3339_RE.test(value) && !Number.isNaN(Date.parse(value)),
});

ajv.addSchema(schema);

function compiled<T>(definition: string): ValidateFunction<T> {
  const validator = ajv.getSchema<T>(`${SCHEMA_ID}#/$defs/${definition}`);
  if (!validator) {
    throw new Error(`contract schema definition is missing: ${definition}`);
  }
  return validator;
}

const jobCreationValidator = compiled<JobCreation>("jobCreation");
const planValidator = compiled<Plan>("plan");
const loadScenarioValidator = compiled<LoadScenario>("loadScenario");
const loadProfileValidator = compiled<LoadProfile>("loadProfile");
const targetAuthorizationValidator = compiled<TargetAuthorization>("targetAuthorization");
const runRequestValidator = compiled<RunRequest>("runRequest");
const testResultValidator = compiled<TestResult>("testResult");
const jobEventValidator = compiled<JobEvent>("jobEvent");

function issue(path: string, message: string): ValidationIssue {
  return { path, message };
}

function finish<T>(value: T, issues: readonly ValidationIssue[]): ValidationResult<T> {
  return issues.length === 0 ? { ok: true, value } : { ok: false, issues };
}

function structural<T>(validator: ValidateFunction<T>, value: unknown): ValidationResult<T> {
  if (validator(value as T)) {
    return { ok: true, value: value as T };
  }

  return {
    ok: false,
    issues: (validator.errors ?? []).map((error: ErrorObject) => {
      const property =
        error.keyword === "additionalProperties" &&
        typeof error.params.additionalProperty === "string"
          ? `: ${error.params.additionalProperty}`
          : "";
      return issue(error.instancePath || "/", `${error.message ?? "is invalid"}${property}`);
    }),
  };
}

export function validateJobCreation(value: unknown): ValidationResult<JobCreation> {
  const result = structural(jobCreationValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const job = result.value;
  checkText(job.displayName, "/displayName", 120, issues);
  checkRepositoryUrl(job.git.repositoryUrl, issues);
  checkRepositoryReference(job.git.reference, issues);
  if (job.git.credentialRef !== undefined) {
    checkCredentialRef(job.git.credentialRef, "/git/credentialRef", issues);
  }
  return finish(job, issues);
}

export function validatePlan(value: unknown): ValidationResult<Plan> {
  const result = structural(planValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const plan = result.value;
  checkText(plan.summary, "/summary", 8000, issues);
  if (!Number.isFinite(plan.confidence) || plan.confidence < 0 || plan.confidence > 1) {
    issues.push(issue("/confidence", "confidence must be a finite number between 0 and 1"));
  }

  if (plan.goal === "vulnerability_testing") {
    if (plan.plannedTests.length === 0) {
      issues.push(issue("/plannedTests", "vulnerability plans must propose at least one test"));
    }
    if (plan.loadScenarios.length > 0) {
      issues.push(issue("/loadScenarios", "vulnerability plans must not include load scenarios"));
    }
  } else {
    if (plan.loadScenarios.length === 0) {
      issues.push(issue("/loadScenarios", "load plans must propose at least one scenario"));
    }
    if (plan.plannedTests.length > 0) {
      issues.push(issue("/plannedTests", "load plans must not include vulnerability tests"));
    }
  }

  plan.plannedTests.forEach((test, index) => {
    checkPlannedTest(test, `/plannedTests/${index}`, issues);
  });
  plan.loadScenarios.forEach((scenario, index) => {
    checkLoadScenario(scenario, `/loadScenarios/${index}`, issues);
  });

  return finish(plan, issues);
}

export function validateLoadScenario(value: unknown): ValidationResult<LoadScenario> {
  const result = structural(loadScenarioValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  checkLoadScenario(result.value, "", issues);
  return finish(result.value, issues);
}

export function validateLoadProfile(value: unknown): ValidationResult<LoadProfile> {
  const result = structural(loadProfileValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  checkLoadProfile(result.value, "", issues);
  return finish(result.value, issues);
}

export function validateTargetAuthorization(value: unknown): ValidationResult<TargetAuthorization> {
  const result = structural(targetAuthorizationValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const authorization = result.value;
  checkNonNilUuid(authorization.targetId, "/targetId", issues);

  if (!authorization.confirmed) {
    issues.push(issue("/confirmed", "target authorization requires explicit confirmation"));
  }

  checkScope(authorization, issues);
  checkLimits(authorization, issues);

  if (authorization.validFrom !== undefined && authorization.expiresAt !== undefined) {
    if (Date.parse(authorization.expiresAt) <= Date.parse(authorization.validFrom)) {
      issues.push(issue("/expiresAt", "expiresAt must be later than validFrom"));
    }
  }

  return finish(authorization, issues);
}

export function validateRunRequest(value: unknown): ValidationResult<RunRequest> {
  const result = structural(runRequestValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const request = result.value;
  checkNonNilUuid(request.jobId, "/jobId", issues);
  checkNonNilUuid(request.planId, "/planId", issues);
  checkNonNilUuid(request.targetAuthorizationId, "/targetAuthorizationId", issues);
  checkNonNilUuid(request.idempotencyKey, "/idempotencyKey", issues);

  const plannedTests = request.plannedTestIds ?? [];
  const scenarios = request.loadScenarioIds ?? [];

  if (request.runType === "vulnerability_testing") {
    if (plannedTests.length === 0) {
      issues.push(issue("/plannedTestIds", "vulnerability runs require at least one planned test"));
    }
    if (scenarios.length > 0) {
      issues.push(issue("/loadScenarioIds", "vulnerability runs must not include load scenarios"));
    }
  } else {
    if (scenarios.length === 0) {
      issues.push(issue("/loadScenarioIds", "load runs require at least one load scenario"));
    }
    if (plannedTests.length > 0) {
      issues.push(issue("/plannedTestIds", "load runs must not include vulnerability tests"));
    }
  }

  checkUniqueUuids(plannedTests, "/plannedTestIds", issues);
  checkUniqueUuids(scenarios, "/loadScenarioIds", issues);

  return finish(request, issues);
}

export function validateTestResult(value: unknown): ValidationResult<TestResult> {
  const result = structural(testResultValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const testResult = result.value;
  checkNonNilUuid(testResult.runId, "/runId", issues);
  checkText(testResult.summary, "/summary", 8000, issues);

  if (testResult.outcome === "failed" && testResult.severity === undefined) {
    issues.push(issue("/severity", "failed results must include a severity"));
  }
  if (
    testResult.confidence !== undefined &&
    (!Number.isFinite(testResult.confidence) ||
      testResult.confidence < 0 ||
      testResult.confidence > 1)
  ) {
    issues.push(issue("/confidence", "confidence must be a finite number between 0 and 1"));
  }

  if (testResult.details !== undefined) {
    checkSafeJson(testResult.details, "/details", 0, issues);
  }

  return finish(testResult, issues);
}

export function validateJobEvent(value: unknown): ValidationResult<JobEvent> {
  const result = structural(jobEventValidator, value);
  if (!result.ok) return result;

  const issues: ValidationIssue[] = [];
  const event = result.value;
  checkNonNilUuid(event.jobId, "/jobId", issues);
  checkSafeJson(event.payload ?? null, "/payload", 0, issues);
  return finish(event, issues);
}

/**
 * Validate a raw JSON string, enforcing the payload ceiling before parsing.
 * `JSON.parse` failures produce a stable issue without echoing payload data.
 */
export function validateContractJson<T>(
  raw: string,
  validate: (value: unknown) => ValidationResult<T>,
): ValidationResult<T> {
  const bytes = new TextEncoder().encode(raw).byteLength;
  if (bytes > MAX_CONTRACT_BYTES) {
    return {
      ok: false,
      issues: [issue("/", `payload exceeds ${MAX_CONTRACT_BYTES} bytes`)],
    };
  }

  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return { ok: false, issues: [issue("/", "payload is not valid JSON")] };
  }
  return validate(value);
}

export const validators = {
  jobCreation: validateJobCreation,
  plan: validatePlan,
  loadScenario: validateLoadScenario,
  loadProfile: validateLoadProfile,
  targetAuthorization: validateTargetAuthorization,
  runRequest: validateRunRequest,
  testResult: validateTestResult,
  jobEvent: validateJobEvent,
} satisfies Record<string, (value: unknown) => ValidationResult<unknown>>;

export type ValidatorName = keyof typeof validators;

function checkPlannedTest(test: PlannedTest, path: string, issues: ValidationIssue[]): void {
  checkCatalogTestId(test.catalogTestId, `${path}/catalogTestId`, issues);
  checkText(test.name, `${path}/name`, 160, issues);
  checkText(test.rationale, `${path}/rationale`, 2000, issues);
  checkText(test.expectedEvidence, `${path}/expectedEvidence`, 1000, issues);
  checkStringList(test.prerequisites ?? [], `${path}/prerequisites`, 20, 300, issues);
  checkStringList(test.targetRequirements ?? [], `${path}/targetRequirements`, 20, 300, issues);
  checkStringList(test.rationaleReferences ?? [], `${path}/rationaleReferences`, 20, 500, issues);
  checkSafeJson(test.configuration ?? null, `${path}/configuration`, 0, issues);
}

function checkLoadScenario(scenario: LoadScenario, path: string, issues: ValidationIssue[]): void {
  checkText(scenario.name, `${path}/name`, 160, issues);
  checkText(scenario.description, `${path}/description`, 2000, issues);

  if (scenario.requests.length === 0) {
    issues.push(issue(`${path}/requests`, "a load scenario requires at least one request"));
  }
  if (scenario.requests.length > 50) {
    issues.push(issue(`${path}/requests`, "at most 50 requests are allowed"));
  }

  const priorStepIds = new Set<string>();
  const seenStepIds = new Set<string>();
  const extractedNames = new Set<string>();

  scenario.requests.forEach((request, index) => {
    const requestPath = `${path}/requests/${index}`;

    if (request.id !== undefined) {
      if (!LOAD_STEP_ID_RE.test(request.id)) {
        issues.push(
          issue(
            `${requestPath}/id`,
            "step ids must start with a lowercase letter and contain only lowercase letters, digits, underscores, or hyphens",
          ),
        );
      }
      if (seenStepIds.has(request.id)) {
        issues.push(issue(`${requestPath}/id`, "step ids must be unique within a load scenario"));
      }
      seenStepIds.add(request.id);
    }

    const dependencies = request.dependsOn ?? [];
    if (dependencies.length > 20) {
      issues.push(issue(`${requestPath}/dependsOn`, "at most 20 dependencies are allowed"));
    }
    const seenDependencies = new Set<string>();
    dependencies.forEach((dependency) => {
      if (seenDependencies.has(dependency)) {
        issues.push(issue(`${requestPath}/dependsOn`, "dependencies must be unique"));
        return;
      }
      seenDependencies.add(dependency);
      if (!priorStepIds.has(dependency)) {
        issues.push(
          issue(`${requestPath}/dependsOn`, "dependencies must refer to an earlier step id"),
        );
      }
    });

    checkRequestPath(request.path, `${requestPath}/path`, issues);
    checkTemplateValue(request.path, extractedNames, `${requestPath}/path`, issues);

    for (const [name, value] of Object.entries(request.headers ?? {})) {
      const headerPath = `${requestPath}/headers/${name}`;
      if (name.length > 120 || !HEADER_NAME_RE.test(name)) {
        issues.push(issue(headerPath, "header names must be valid HTTP token characters"));
      }
      if (CREDENTIAL_HEADERS.has(name.toLowerCase())) {
        issues.push(issue(headerPath, "credential-bearing headers must use credentialRef instead"));
      }
      if (FORBIDDEN_LOAD_HEADERS.has(name.toLowerCase())) {
        issues.push(issue(headerPath, "hop-by-hop and length-managed headers are not allowed"));
      }
      if (value.length > 1024) {
        issues.push(issue(headerPath, "header values are limited to 1024 characters"));
      }
      if (hasControlCharacters(value)) {
        issues.push(issue(headerPath, "header values must not contain control characters"));
      }
      checkTemplateValue(value, extractedNames, headerPath, issues);
    }

    checkSecretHeaders(
      request.headers ?? {},
      request.secretHeaders ?? {},
      `${requestPath}/secretHeaders`,
      issues,
    );

    const extractions = request.extract ?? [];
    if (extractions.length > 20) {
      issues.push(issue(`${requestPath}/extract`, "at most 20 extraction rules are allowed"));
    }
    extractions.forEach((extraction, extractionIndex) => {
      const extractionPath = `${requestPath}/extract/${extractionIndex}`;
      checkLoadExtraction(extraction, extractionPath, issues);
    });

    const assertions = request.assertions ?? [];
    if (assertions.length > 20) {
      issues.push(issue(`${requestPath}/assertions`, "at most 20 assertions are allowed"));
    }
    assertions.forEach((assertion, assertionIndex) => {
      checkLoadAssertion(assertion, `${requestPath}/assertions/${assertionIndex}`, issues);
    });

    if (request.bodyTemplate !== undefined) {
      const bodyPath = `${requestPath}/bodyTemplate`;
      checkSafeJson(request.bodyTemplate, bodyPath, 0, issues);
      checkTemplateJson(request.bodyTemplate, extractedNames, bodyPath, issues);
      if (
        new TextEncoder().encode(JSON.stringify(request.bodyTemplate) ?? "").byteLength >
        LOAD_MAX_BODY_BYTES
      ) {
        issues.push(
          issue(bodyPath, `load request bodies must not exceed ${LOAD_MAX_BODY_BYTES} bytes`),
        );
      }
      if (request.method === "GET" || request.method === "HEAD" || request.method === "OPTIONS") {
        issues.push(
          issue(bodyPath, "GET, HEAD, and OPTIONS load requests must not include a body template"),
        );
      }
    }

    if (request.expectedStatuses.length === 0) {
      issues.push(
        issue(
          `${requestPath}/expectedStatuses`,
          "each load request requires at least one expected status",
        ),
      );
    }
    if (request.expectedStatuses.length > 20) {
      issues.push(
        issue(`${requestPath}/expectedStatuses`, "at most 20 expected statuses are allowed"),
      );
    }
    for (const status of request.expectedStatuses) {
      if (!Number.isInteger(status) || status < 100 || status > 599) {
        issues.push(
          issue(`${requestPath}/expectedStatuses`, "statuses must be between 100 and 599"),
        );
      }
    }

    extractions.forEach((extraction, extractionIndex) => {
      if (extractedNames.has(extraction.name)) {
        issues.push(
          issue(
            `${requestPath}/extract/${extractionIndex}/name`,
            "extraction variable names must be unique within a load scenario",
          ),
        );
      }
      extractedNames.add(extraction.name);
    });

    if (request.id !== undefined) {
      priorStepIds.add(request.id);
    }
  });

  checkLoadProfile(scenario.profile, `${path}/profile`, issues);
  if (scenario.credentialRef !== undefined) {
    checkCredentialRef(scenario.credentialRef, `${path}/credentialRef`, issues);
  }
}

function checkLoadExtraction(
  extraction: LoadExtraction,
  path: string,
  issues: ValidationIssue[],
): void {
  if (!LOAD_VARIABLE_RE.test(extraction.name)) {
    issues.push(
      issue(
        `${path}/name`,
        "extraction names must start with a lowercase letter and contain only lowercase letters, digits, or underscores",
      ),
    );
  }
  if (extraction.source === "header") {
    if (!HEADER_NAME_RE.test(extraction.selector)) {
      issues.push(
        issue(`${path}/selector`, "header extraction selectors must be valid HTTP header names"),
      );
    }
  } else {
    checkJsonPointer(extraction.selector, `${path}/selector`, issues);
  }
}

function checkLoadAssertion(
  assertion: LoadAssertion,
  path: string,
  issues: ValidationIssue[],
): void {
  if (assertion.source === "header") {
    if (!HEADER_NAME_RE.test(assertion.selector)) {
      issues.push(
        issue(`${path}/selector`, "header assertion selectors must be valid HTTP header names"),
      );
    }
  } else {
    checkJsonPointer(assertion.selector, `${path}/selector`, issues);
  }

  if (assertion.operator === "exists") {
    if (assertion.expected !== undefined) {
      issues.push(
        issue(`${path}/expected`, "exists assertions must not include an expected value"),
      );
    }
    return;
  }

  if (assertion.expected === undefined) {
    issues.push(
      issue(`${path}/expected`, "equals and contains assertions require an expected value"),
    );
    return;
  }
  if (assertion.operator === "contains" && typeof assertion.expected !== "string") {
    issues.push(issue(`${path}/expected`, "contains assertions require a string expected value"));
  }
  if (assertion.source === "header" && typeof assertion.expected !== "string") {
    issues.push(issue(`${path}/expected`, "header assertions require a string expected value"));
  }
  if (
    typeof assertion.expected !== "string" &&
    typeof assertion.expected !== "number" &&
    typeof assertion.expected !== "boolean" &&
    assertion.expected !== null
  ) {
    issues.push(
      issue(
        `${path}/expected`,
        "assertion expected values must be strings, numbers, booleans, or null",
      ),
    );
  }
  if (
    typeof assertion.expected === "string" &&
    (assertion.expected.length > 1024 || hasControlCharacters(assertion.expected))
  ) {
    issues.push(
      issue(`${path}/expected`, "assertion string values must be bounded and control-free"),
    );
  }
}

function checkSecretHeaders(
  literalHeaders: Readonly<Record<string, string>>,
  secretHeaders: Readonly<Record<string, string>>,
  path: string,
  issues: ValidationIssue[],
): void {
  const entries = Object.entries(secretHeaders);
  if (entries.length > 20) {
    issues.push(issue(path, "at most 20 secret header references are allowed"));
  }
  const seen = new Set<string>();
  for (const [name, reference] of entries) {
    if (!HEADER_NAME_RE.test(name) || name.length > 120) {
      issues.push(issue(`${path}/${name}`, "header names must be valid HTTP token characters"));
    }
    const lower = name.toLowerCase();
    if (seen.has(lower)) {
      issues.push(issue(`${path}/${name}`, "secret header names must be unique ignoring case"));
    }
    seen.add(lower);
    if (FORBIDDEN_LOAD_HEADERS.has(lower)) {
      issues.push(
        issue(`${path}/${name}`, "hop-by-hop and length-managed headers are not allowed"),
      );
    }
    if (Object.keys(literalHeaders).some((literal) => literal.toLowerCase() === lower)) {
      issues.push(
        issue(`${path}/${name}`, "a header cannot be supplied as both literal and secret-backed"),
      );
    }
    checkCredentialRef(reference, `${path}/${name}`, issues);
  }
}

function checkTemplateValue(
  value: string,
  extractedNames: ReadonlySet<string>,
  path: string,
  issues: ValidationIssue[],
): void {
  let remainder = value;
  while (true) {
    const start = remainder.indexOf("${");
    if (start === -1) return;
    const afterStart = remainder.slice(start + 2);
    const end = afterStart.indexOf("}");
    if (end === -1) {
      issues.push(issue(path, `template placeholders must use the form ${TEMPLATE_FORM}`));
      return;
    }
    const name = afterStart.slice(0, end);
    if (!LOAD_VARIABLE_RE.test(name)) {
      issues.push(issue(path, "template placeholders must use a bounded lowercase variable name"));
      return;
    }
    if (!extractedNames.has(name)) {
      issues.push(issue(path, "template variables must be extracted by an earlier step"));
    }
    remainder = afterStart.slice(end + 1);
  }
}

function checkTemplateJson(
  value: JsonValue,
  extractedNames: ReadonlySet<string>,
  path: string,
  issues: ValidationIssue[],
): void {
  if (typeof value === "string") {
    checkTemplateValue(value, extractedNames, path, issues);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => {
      checkTemplateJson(item, extractedNames, `${path}/${index}`, issues);
    });
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      checkTemplateJson(item, extractedNames, `${path}/${key}`, issues);
    }
  }
}

function checkJsonPointer(value: string, path: string, issues: ValidationIssue[]): void {
  if (
    !value.startsWith("/") ||
    value.length > 500 ||
    hasControlCharacters(value) ||
    value.includes("${")
  ) {
    issues.push(issue(path, "JSON selectors must be bounded JSON pointers beginning with /"));
  }
}

function checkLoadProfile(profile: LoadProfile, path: string, issues: ValidationIssue[]): void {
  const hasVirtualUsers = profile.virtualUsers !== undefined;
  const hasArrivalRate = profile.arrivalRatePerSecond !== undefined;
  if (hasVirtualUsers === hasArrivalRate) {
    issues.push(
      issue(`${path}/virtualUsers`, "provide exactly one of virtualUsers or arrivalRatePerSecond"),
    );
  }
  if (
    profile.virtualUsers !== undefined &&
    (!Number.isInteger(profile.virtualUsers) ||
      profile.virtualUsers < 1 ||
      profile.virtualUsers > 100)
  ) {
    issues.push(issue(`${path}/virtualUsers`, "virtualUsers must be between 1 and 100"));
  }
  if (
    profile.arrivalRatePerSecond !== undefined &&
    (!Number.isFinite(profile.arrivalRatePerSecond) ||
      profile.arrivalRatePerSecond < 0.1 ||
      profile.arrivalRatePerSecond > 100)
  ) {
    issues.push(
      issue(`${path}/arrivalRatePerSecond`, "arrivalRatePerSecond must be between 0.1 and 100"),
    );
  }
  if (profile.durationSeconds < 1 || profile.durationSeconds > 900) {
    issues.push(issue(`${path}/durationSeconds`, "durationSeconds must be between 1 and 900"));
  }
  if (profile.rampUpSeconds < 0 || profile.rampUpSeconds > 300) {
    issues.push(issue(`${path}/rampUpSeconds`, "rampUpSeconds must not exceed 300"));
  }
  if (profile.maxRequests < 1 || profile.maxRequests > 100_000) {
    issues.push(issue(`${path}/maxRequests`, "maxRequests must be between 1 and 100000"));
  }
  if (
    !Number.isFinite(profile.maxRequestsPerSecond) ||
    profile.maxRequestsPerSecond < 0.1 ||
    profile.maxRequestsPerSecond > 100
  ) {
    issues.push(
      issue(`${path}/maxRequestsPerSecond`, "maxRequestsPerSecond must be between 0.1 and 100"),
    );
  }

  const conditions = profile.stopConditions;
  if (conditions !== undefined) {
    if (
      conditions.errorRate !== undefined &&
      (!Number.isFinite(conditions.errorRate) ||
        conditions.errorRate < 0 ||
        conditions.errorRate > 1)
    ) {
      issues.push(issue(`${path}/stopConditions/errorRate`, "errorRate must be between 0 and 1"));
    }
    if (
      conditions.p95LatencyMs !== undefined &&
      (!Number.isInteger(conditions.p95LatencyMs) ||
        conditions.p95LatencyMs < 1 ||
        conditions.p95LatencyMs > 60_000)
    ) {
      issues.push(
        issue(`${path}/stopConditions/p95LatencyMs`, "p95LatencyMs must be between 1 and 60000"),
      );
    }
    if (
      conditions.maxFailedRequests !== undefined &&
      (!Number.isInteger(conditions.maxFailedRequests) ||
        conditions.maxFailedRequests < 1 ||
        conditions.maxFailedRequests > 100_000)
    ) {
      issues.push(
        issue(
          `${path}/stopConditions/maxFailedRequests`,
          "maxFailedRequests must be between 1 and 100000",
        ),
      );
    }
  }
}

function checkScope(authorization: TargetAuthorization, issues: ValidationIssue[]): void {
  const origin = checkOrigin(authorization.scope.origin, "/scope/origin", issues);
  if (origin) {
    const originHost = origin.hostname.replace(/^\[/, "").replace(/\]$/, "");
    if (
      !authorization.scope.allowedHosts.some(
        (host) => host.toLowerCase() === originHost.toLowerCase(),
      )
    ) {
      issues.push(issue("/scope/allowedHosts", "allowedHosts must include the origin host"));
    }
    const port = Number(origin.port || (origin.protocol === "https:" ? 443 : 80));
    if (!authorization.scope.allowedPorts.includes(port)) {
      issues.push(issue("/scope/allowedPorts", "allowedPorts must include the origin port"));
    }
  }

  authorization.scope.allowedHosts.forEach((host, index) => {
    checkHost(host, `/scope/allowedHosts/${index}`, issues);
  });
  authorization.scope.allowedPaths.forEach((path, index) => {
    checkRequestPath(path, `/scope/allowedPaths/${index}`, issues);
  });
  if (new Set(authorization.scope.allowedPorts).size !== authorization.scope.allowedPorts.length) {
    issues.push(issue("/scope/allowedPorts", "ports must be unique"));
  }
}

function checkLimits(authorization: TargetAuthorization, issues: ValidationIssue[]): void {
  const limits = authorization.limits;
  if (
    !Number.isFinite(limits.maxRequestsPerSecond) ||
    limits.maxRequestsPerSecond < 0.1 ||
    limits.maxRequestsPerSecond > 1000
  ) {
    issues.push(
      issue("/limits/maxRequestsPerSecond", "maxRequestsPerSecond must be between 0.1 and 1000"),
    );
  }
  if (limits.maxConcurrency < 1 || limits.maxConcurrency > 100) {
    issues.push(issue("/limits/maxConcurrency", "maxConcurrency must be between 1 and 100"));
  }
  if (limits.maxDurationSeconds < 1 || limits.maxDurationSeconds > 3600) {
    issues.push(
      issue("/limits/maxDurationSeconds", "maxDurationSeconds must be between 1 and 3600"),
    );
  }
  if (limits.maxRequests < 1 || limits.maxRequests > 1_000_000) {
    issues.push(issue("/limits/maxRequests", "maxRequests must be between 1 and 1000000"));
  }
}

function checkText(
  value: string,
  path: string,
  maxLength: number,
  issues: ValidationIssue[],
): void {
  if (value.trim().length === 0) {
    issues.push(issue(path, "must not be blank"));
  }
  if (value.length > maxLength) {
    issues.push(issue(path, `must be at most ${maxLength} characters`));
  }
  if (hasControlCharacters(value)) {
    issues.push(issue(path, "must not contain control characters"));
  }
}

function checkStringList(
  values: readonly string[],
  path: string,
  maxItems: number,
  maxLength: number,
  issues: ValidationIssue[],
): void {
  if (values.length > maxItems) {
    issues.push(issue(path, `must contain at most ${maxItems} items`));
  }
  values.forEach((value, index) => {
    checkText(value, `${path}/${index}`, maxLength, issues);
  });
}

function checkRepositoryUrl(value: string, issues: ValidationIssue[]): void {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    issues.push(issue("/git/repositoryUrl", "must be a valid URL"));
    return;
  }
  if (url.protocol !== "https:" && url.protocol !== "ssh:") {
    issues.push(issue("/git/repositoryUrl", "must use https or ssh"));
  }
  if (url.password !== "") {
    issues.push(issue("/git/repositoryUrl", "must not embed a password; use credentialRef"));
  }
  if (url.protocol === "https:" && url.username !== "") {
    issues.push(issue("/git/repositoryUrl", "https repository URLs must not include a username"));
  }
  if (url.protocol === "ssh:" && url.username !== "" && url.username !== "git") {
    issues.push(issue("/git/repositoryUrl", "ssh repository URLs may only use the git username"));
  }
  if (url.hostname === "") {
    issues.push(issue("/git/repositoryUrl", "must include a host"));
  }
  if (url.search !== "" || url.hash !== "") {
    issues.push(issue("/git/repositoryUrl", "must not include a query or fragment"));
  }
}

function checkRepositoryReference(value: string, issues: ValidationIssue[]): void {
  checkText(value, "/git/reference", 200, issues);
  if (
    value.startsWith("-") ||
    value.startsWith("/") ||
    value.endsWith("/") ||
    value.includes("..") ||
    value.includes("//") ||
    !/^[A-Za-z0-9._/-]+$/.test(value)
  ) {
    issues.push(issue("/git/reference", "must be a safe branch, tag, or commit reference"));
  }
}

function checkCredentialRef(value: string, path: string, issues: ValidationIssue[]): void {
  const reference = value.startsWith("secret://") ? value.slice("secret://".length) : "";
  if (reference === "" || reference.length > 190 || !/^[a-z0-9][a-z0-9/_.-]*$/.test(reference)) {
    issues.push(issue(path, "has an invalid secret reference format"));
  }
}

function checkCatalogTestId(value: string, path: string, issues: ValidationIssue[]): void {
  if (value.length < 2 || value.length > 119 || !/^[a-z0-9][a-z0-9.-]+$/.test(value)) {
    issues.push(issue(path, "has an invalid catalog test id"));
  }
}

function checkOrigin(value: string, path: string, issues: ValidationIssue[]): URL | undefined {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    issues.push(issue(path, "must be a valid URL origin"));
    return undefined;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    issues.push(issue(path, "must use http or https"));
  }
  if (url.username !== "" || url.password !== "") {
    issues.push(issue(path, "must not include credentials"));
  }
  if (url.pathname !== "/" || url.search !== "" || url.hash !== "") {
    issues.push(issue(path, "must be an origin without a path, query, or fragment"));
  }
  if (url.hostname === "") {
    issues.push(issue(path, "must include a host"));
    return undefined;
  }
  return url;
}

function checkHost(value: string, path: string, issues: ValidationIssue[]): void {
  if (
    value.length === 0 ||
    value.length > 253 ||
    value.startsWith(".") ||
    value.endsWith(".") ||
    !/^[A-Za-z0-9.:[\]-]+$/.test(value)
  ) {
    issues.push(issue(path, "has an invalid host format"));
  }
}

function checkRequestPath(value: string, path: string, issues: ValidationIssue[]): void {
  if (
    !value.startsWith("/") ||
    value.length > 2000 ||
    value.includes("..") ||
    value.includes("//") ||
    value.includes("\\") ||
    hasControlCharacters(value)
  ) {
    issues.push(issue(path, "must be an absolute path without traversal or separator escapes"));
  }
}

function checkSafeJson(
  value: JsonValue,
  path: string,
  depth: number,
  issues: ValidationIssue[],
): void {
  const MAX_DEPTH = 20;
  if (depth > MAX_DEPTH) {
    issues.push(issue(path, "JSON nesting is too deep"));
    return;
  }

  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      issues.push(issue(path, "numbers must be finite and representable"));
    }
    return;
  }
  if (typeof value === "string") {
    if (value.length > 16384) {
      issues.push(issue(path, "strings must be at most 16384 characters"));
    }
    return;
  }
  if (Array.isArray(value)) {
    if (value.length > 100) {
      issues.push(issue(path, "arrays may contain at most 100 items"));
    }
    value.forEach((item, index) => {
      checkSafeJson(item, `${path}/${index}`, depth + 1, issues);
    });
    return;
  }
  if (value !== null && typeof value === "object") {
    const entries = Object.entries(value);
    if (entries.length > 50) {
      issues.push(issue(path, "objects may contain at most 50 properties"));
    }
    for (const [key, item] of entries) {
      if (FORBIDDEN_KEYS.has(key.toLowerCase())) {
        issues.push(issue(path, "executable command or script fields are not allowed"));
      }
      checkSafeJson(item, `${path}/${key}`, depth + 1, issues);
    }
  }
}

function checkNonNilUuid(value: string, path: string, issues: ValidationIssue[]): void {
  if (NIL_UUID_RE.test(value)) {
    issues.push(issue(path, "must not be a nil UUID"));
  }
}

function checkUniqueUuids(
  values: readonly string[],
  path: string,
  issues: ValidationIssue[],
): void {
  const seen = new Set<string>();
  for (const value of values) {
    if (NIL_UUID_RE.test(value)) {
      issues.push(issue(path, "must not contain a nil UUID"));
    }
    if (seen.has(value)) {
      issues.push(issue(path, "must not contain duplicate ids"));
    }
    seen.add(value);
  }
}

function hasControlCharacters(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (
      codePoint !== undefined &&
      (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f))
    ) {
      return true;
    }
  }
  return false;
}
