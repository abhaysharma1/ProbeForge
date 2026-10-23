/**
 * Data-only contract types mirroring
 * `packages/contracts/schemas/probeforge.v1.schema.json`.
 *
 * Optional fields correspond to JSON properties that may be absent. Validation
 * never accepts extra properties, so these interfaces must stay closed.
 */

export const JOB_GOALS = ["vulnerability_testing", "load_testing"] as const;

export type JobGoal = (typeof JOB_GOALS)[number];

export function isJobGoal(value: string): value is JobGoal {
  return (JOB_GOALS as readonly string[]).includes(value);
}

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { readonly [key: string]: JsonValue };

export type JsonObject = { readonly [key: string]: JsonValue };

export type Intensity = "low" | "standard" | "high";
export type Risk = "low" | "medium" | "high" | "critical";
export type Severity = "info" | "low" | "medium" | "high" | "critical";
export type Environment = "local" | "staging" | "production";
export type HttpMethod = "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS";
export type AuthorizationMethod = "explicit_confirmation" | "ownership_verification";
export type TestOutcome = "passed" | "failed" | "skipped" | "inconclusive" | "error" | "cancelled";

export type JobEventType =
  | "job.queued"
  | "job.failed"
  | "job.cancelled"
  | "repository.clone.started"
  | "repository.snapshot.ready"
  | "repository.clone.failed"
  | "agent.discovery.progress"
  | "plan.proposed"
  | "plan.approved"
  | "target.validated"
  | "run.scheduled"
  | "run.started"
  | "run.cancelled"
  | "run.completed"
  | "run.failed"
  | "test.started"
  | "test.progress"
  | "test.completed"
  | "test.failed"
  | "finding.recorded"
  | "report.completed"
  | "report.failed";

export interface GitSource {
  readonly repositoryUrl: string;
  readonly reference: string;
  readonly credentialRef?: string;
}

export interface JobCreation {
  readonly schemaVersion: "1";
  readonly goal: JobGoal;
  readonly intensity: Intensity;
  readonly displayName: string;
  readonly git: GitSource;
}

export interface LoadProfile {
  readonly virtualUsers?: number;
  readonly arrivalRatePerSecond?: number;
  readonly rampUpSeconds: number;
  readonly durationSeconds: number;
  readonly maxRequests: number;
  readonly maxRequestsPerSecond: number;
  readonly stopConditions?: JsonObject;
}

export interface LoadRequest {
  readonly method: HttpMethod;
  readonly path: string;
  readonly headers?: Readonly<Record<string, string>>;
  readonly bodyTemplate?: JsonValue;
  readonly expectedStatuses?: readonly number[];
}

export interface LoadScenario {
  readonly name: string;
  readonly description: string;
  readonly requests: readonly LoadRequest[];
  readonly profile: LoadProfile;
  readonly credentialRef?: string;
}

export interface PlannedTest {
  readonly catalogTestId: string;
  readonly name: string;
  readonly rationale: string;
  readonly risk: Risk;
  readonly prerequisites?: readonly string[];
  readonly expectedEvidence: string;
  readonly targetRequirements?: readonly string[];
  readonly requiresExplicitConfirmation: boolean;
  readonly configuration?: JsonObject;
  readonly rationaleReferences?: readonly string[];
}

export interface Plan {
  readonly schemaVersion: "1";
  readonly goal: JobGoal;
  readonly summary: string;
  readonly confidence: number;
  readonly plannedTests: readonly PlannedTest[];
  readonly loadScenarios: readonly LoadScenario[];
  readonly requiredInputs: readonly string[];
  readonly limitations: readonly string[];
  readonly evidenceReferences?: readonly string[];
}

export interface TargetScope {
  readonly origin: string;
  readonly allowedHosts: readonly string[];
  readonly allowedPaths: readonly string[];
  readonly allowedPorts: readonly number[];
}

export interface TargetLimits {
  readonly maxRequestsPerSecond: number;
  readonly maxConcurrency: number;
  readonly maxDurationSeconds: number;
  readonly maxRequests: number;
}

export interface TargetAuthorization {
  readonly schemaVersion: "1";
  readonly targetId: string;
  readonly authorizationMethod: AuthorizationMethod;
  readonly confirmed: true;
  readonly scope: TargetScope;
  readonly limits: TargetLimits;
  readonly validFrom?: string;
  readonly expiresAt?: string;
}

export interface RunRequest {
  readonly schemaVersion: "1";
  readonly runType: JobGoal;
  readonly jobId: string;
  readonly planId: string;
  readonly planVersion: number;
  readonly plannedTestIds?: readonly string[];
  readonly loadScenarioIds?: readonly string[];
  readonly targetAuthorizationId: string;
  readonly idempotencyKey: string;
}

export interface ToolInfo {
  readonly name: string;
  readonly version: string;
}

export interface TestResult {
  readonly schemaVersion: "1";
  readonly runId: string;
  readonly catalogTestId: string;
  readonly outcome: TestOutcome;
  readonly severity?: Severity;
  readonly confidence?: number;
  readonly summary: string;
  readonly details?: JsonObject;
  readonly durationMs: number;
  readonly tool?: ToolInfo;
}

export interface JobEvent {
  readonly schemaVersion: "1";
  readonly jobId: string;
  readonly sequence: number;
  readonly type: JobEventType;
  readonly occurredAt: string;
  readonly payload?: JsonValue;
}
