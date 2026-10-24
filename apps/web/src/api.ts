export type ProjectRole = "owner" | "admin" | "collaborator" | "viewer";

import type { LoadProfile, LoadRequest, Risk } from "@probe-forge/contracts";

export interface AuthUser {
  readonly id: string;
  readonly email: string;
  readonly displayName: string;
}

export interface SessionInfo {
  readonly user: AuthUser;
  readonly sessionId: string;
  readonly csrfToken: string;
  readonly expiresAt: string;
}

export interface LoginResult {
  readonly user: AuthUser;
  readonly csrfToken: string;
  readonly expiresAt: string;
}

export interface Project {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly ownerId: string;
  readonly role: ProjectRole;
  readonly memberCount: number;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface ProjectMember {
  readonly userId: string;
  readonly email: string;
  readonly displayName: string;
  readonly role: ProjectRole;
  readonly joinedAt: string;
}

export interface AuditEvent {
  readonly id: string;
  readonly actorId: string | null;
  readonly action: string;
  readonly resourceType: string;
  readonly resourceId: string | null;
  readonly metadata: Record<string, unknown>;
  readonly createdAt: string;
}

export type JobGoal = "vulnerability_testing" | "load_testing";
export type JobStatus =
  | "draft"
  | "queued"
  | "analyzing"
  | "planning"
  | "awaiting_approval"
  | "failed"
  | "cancelled"
  | "expired";

export interface Job {
  readonly id: string;
  readonly projectId: string;
  readonly goal: JobGoal;
  readonly intensity: string;
  readonly status: JobStatus;
  readonly displayName: string;
  readonly currentPlanId: string | null;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export type CatalogCategory = "static" | "http" | "network" | "load";

export interface CatalogEntry {
  readonly id: string;
  readonly name: string;
  readonly category: CatalogCategory;
  readonly risk: Risk;
  readonly description: string;
  readonly prerequisites: readonly string[];
  readonly toolRequirements: readonly string[];
  readonly targetRequirements: readonly string[];
  readonly timeoutSeconds: number;
  readonly evidenceExpectations: readonly string[];
  readonly requiresExplicitConfirmation: boolean;
  readonly available: boolean;
  readonly availabilityNote: string | null;
}

export interface PlannedTest {
  readonly id: string;
  readonly catalogTestId: string;
  readonly name: string;
  readonly rationale: string;
  readonly risk: Risk;
  readonly prerequisites: readonly string[];
  readonly expectedEvidence: string;
  readonly targetRequirements: readonly string[];
  readonly requiresExplicitConfirmation: boolean;
  readonly configuration: Record<string, unknown>;
  readonly rationaleReferences: readonly string[];
  readonly selected: boolean;
  readonly catalog: CatalogEntry | null;
}

export interface PlanLoadScenario {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly requests: readonly LoadRequest[];
  readonly profile: LoadProfile;
  readonly credentialRef: string | null;
  readonly selected: boolean;
}

export interface PlanApprovalSummary {
  readonly id: string;
  readonly planRevision: number;
  readonly approvedBy: string;
  readonly approvedByName: string;
  readonly approvedByEmail: string;
  readonly approvedAt: string;
  readonly approvedTestIds: readonly string[];
  readonly approvedLoadScenarioIds: readonly string[];
}

export interface JobPlan {
  readonly id: string;
  readonly projectId: string;
  readonly jobId: string;
  readonly version: number;
  readonly revision: number;
  readonly status: "draft" | "proposed" | "approved" | "superseded" | "rejected";
  readonly goal: JobGoal;
  readonly summary: string;
  readonly confidence: number;
  readonly requiredInputs: readonly string[];
  readonly limitations: readonly string[];
  readonly evidenceReferences: readonly string[];
  readonly createdByAgent: boolean;
  readonly agentProvider: string | null;
  readonly agentModel: string | null;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly catalogVersion: string;
  readonly plannedTests: readonly PlannedTest[];
  readonly loadScenarios: readonly PlanLoadScenario[];
  readonly approval: PlanApprovalSummary | null;
}

export interface RunResultCounts {
  readonly passed: number;
  readonly failed: number;
  readonly skipped: number;
  readonly inconclusive: number;
  readonly error: number;
  readonly cancelled: number;
  readonly confirmed: number;
  readonly suspected: number;
}

export interface RunSummaryEntry {
  readonly id: string;
  readonly projectId: string;
  readonly jobId: string;
  readonly runType: JobGoal;
  readonly status: string;
  readonly planId: string;
  readonly planVersion: number;
  readonly startedAt: string | null;
  readonly finishedAt: string | null;
  readonly createdAt: string;
  readonly resultCounts: RunResultCounts;
  readonly reportStatus: string | null;
}

export interface EvidenceSummary {
  readonly id: string;
  readonly storageRef: string;
  readonly sha256: string;
  readonly mimeType: string;
  readonly sizeBytes: number;
  readonly redacted: boolean;
  readonly retentionUntil: string | null;
  readonly createdAt: string;
}

export interface TestResultView {
  readonly id: string;
  readonly catalogTestId: string;
  readonly resultKey: string;
  readonly outcome: string;
  readonly findingStatus: string | null;
  readonly severity: string | null;
  readonly confidence: number | null;
  readonly summary: string;
  readonly details: Record<string, unknown>;
  readonly observation: Record<string, unknown>;
  readonly toolName: string | null;
  readonly toolVersion: string | null;
  readonly durationMs: number;
  readonly createdAt: string;
  readonly evidence: readonly EvidenceSummary[];
}

export interface RecommendationView {
  readonly id: string;
  readonly resultId: string | null;
  readonly evidenceId: string | null;
  readonly runMetricId: string | null;
  readonly kind: string;
  readonly severity: string;
  readonly title: string;
  readonly body: string;
  readonly rationale: string | null;
  readonly confidence: number | null;
  readonly generatedBy: string;
  readonly createdAt: string;
}

export interface RunMetricSummary {
  readonly requestCount: number;
  readonly errorCount: number;
  readonly responseBytes: number;
  readonly latencyP95Ms: number;
  readonly throughputRps: number;
}

export interface RunScenarioProfile {
  readonly id: string;
  readonly name: string;
  readonly virtualUsers: number | null;
  readonly arrivalRatePerSecond: number | null;
  readonly rampUpSeconds: number;
  readonly durationSeconds: number;
  readonly maxRequests: number;
  readonly maxRequestsPerSecond: number;
}

export interface ReportVersionMeta {
  readonly id: string;
  readonly version: number;
  readonly status: string;
  readonly contentHash: string;
  readonly summary: string;
  readonly generatedBy: string;
  readonly createdAt: string;
}

export interface ReportVersionContent extends ReportVersionMeta {
  readonly content: Record<string, unknown>;
}

export interface RunReportView {
  readonly id: string;
  readonly runId: string;
  readonly reportType: string;
  readonly status: string;
  readonly currentVersion: number;
  readonly lastErrorCode: string | null;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly latest: ReportVersionContent | null;
  readonly versions: readonly ReportVersionMeta[];
}

export interface RunDetail {
  readonly run: RunSummaryEntry;
  readonly reportType: string;
  readonly target: {
    readonly authorizationId: string;
    readonly origin: string | null;
  };
  readonly scenario: RunScenarioProfile | null;
  readonly results: readonly TestResultView[];
  readonly recommendations: readonly RecommendationView[];
  readonly metrics: RunMetricSummary;
  readonly report: RunReportView | null;
}

export interface ComparisonSide {
  readonly runId: string;
  readonly runType: JobGoal;
  readonly status: string;
  readonly planVersion: number;
  readonly scenarioId: string | null;
  readonly scenarioName: string | null;
  readonly targetOrigin: string | null;
  readonly profile: RunScenarioProfile | null;
  readonly startedAt: string | null;
  readonly finishedAt: string | null;
  readonly createdAt: string;
  readonly resultCounts: RunResultCounts;
  readonly metrics: RunMetricSummary;
}

export interface RunComparison {
  readonly left: ComparisonSide;
  readonly right: ComparisonSide;
  readonly delta: {
    readonly requestCount: number;
    readonly errorCount: number;
    readonly latencyP95Ms: number;
    readonly throughputRps: number;
  };
}

interface ErrorEnvelope {
  readonly error?: {
    readonly code?: string;
    readonly message?: string;
  };
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

const API_BASE = "/api";

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    credentials: "same-origin",
    ...init,
  });

  if (!response.ok) {
    let envelope: ErrorEnvelope = {};
    try {
      envelope = (await response.json()) as ErrorEnvelope;
    } catch {
      // The API always returns JSON, but a proxy failure may not.
    }
    throw new ApiError(
      response.status,
      envelope.error?.code ?? "request_failed",
      envelope.error?.message ?? "The request could not be completed.",
    );
  }

  if (response.status === 204) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

function unsafeRequest<T>(method: string, path: string, csrfToken: string, body?: unknown) {
  return request<T>(path, {
    method,
    headers: {
      "Content-Type": "application/json",
      "X-CSRF-Token": csrfToken,
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
}

export function getSession(): Promise<SessionInfo> {
  return request<SessionInfo>("/auth/session");
}

export function getCsrfToken(): Promise<string> {
  return request<{ csrfToken: string }>("/auth/csrf").then((response) => response.csrfToken);
}

export function login(email: string, password: string, csrfToken: string): Promise<LoginResult> {
  return unsafeRequest<LoginResult>("POST", "/auth/login", csrfToken, { email, password });
}

export function logout(csrfToken: string): Promise<void> {
  return unsafeRequest<void>("POST", "/auth/logout", csrfToken);
}

export function listProjects(): Promise<Project[]> {
  return request<Project[]>("/projects");
}

export function listJobs(projectId: string): Promise<Job[]> {
  return request<Job[]>(`/projects/${projectId}/jobs`);
}

export function getJobPlan(jobId: string): Promise<JobPlan> {
  return request<JobPlan>(`/jobs/${jobId}/plan`);
}

export function listRuns(projectId: string): Promise<RunSummaryEntry[]> {
  return request<RunSummaryEntry[]>(`/projects/${projectId}/runs`);
}

export function getRun(runId: string): Promise<RunDetail> {
  return request<RunDetail>(`/runs/${runId}`);
}

export function generateRunReport(runId: string, csrfToken: string): Promise<RunReportView> {
  return unsafeRequest<RunReportView>("POST", `/runs/${runId}/report`, csrfToken);
}

export function compareRuns(runId: string, otherRunId: string): Promise<RunComparison> {
  return request<RunComparison>(
    `/runs/${runId}/compare?otherRunId=${encodeURIComponent(otherRunId)}`,
  );
}

export function runReportExportPath(runId: string, format: "json" | "markdown"): string {
  return `${API_BASE}/runs/${runId}/report/export?format=${format}`;
}

export type JobIntensity = "low" | "standard" | "high";

export interface StartJobInput {
  readonly goal: JobGoal;
  readonly intensity: JobIntensity;
  readonly displayName: string;
  readonly repositoryUrl: string;
  readonly reference: string;
}

export function createJob(
  projectId: string,
  input: StartJobInput,
  csrfToken: string,
): Promise<Job> {
  return unsafeRequest<Job>("POST", `/jobs?projectId=${encodeURIComponent(projectId)}`, csrfToken, {
    schemaVersion: "1",
    goal: input.goal,
    intensity: input.intensity,
    displayName: input.displayName,
    git: { repositoryUrl: input.repositoryUrl, reference: input.reference },
  });
}

export function cancelJob(jobId: string, csrfToken: string): Promise<Job> {
  return unsafeRequest<Job>("POST", `/jobs/${jobId}/cancel`, csrfToken);
}

export type TargetEnvironment = "local" | "staging" | "production";

export interface Target {
  readonly id: string;
  readonly projectId: string;
  readonly normalizedOrigin: string;
  readonly environment: TargetEnvironment;
  readonly allowedHosts: readonly string[];
  readonly allowedPaths: readonly string[];
  readonly allowedPorts: readonly number[];
  readonly verificationStatus: string;
  readonly verificationCheckedAt: string | null;
  readonly verifiedAddresses: readonly string[];
  readonly createdBy: string;
  readonly createdAt: string;
  readonly updatedAt: string;
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
  readonly id: string;
  readonly projectId: string;
  readonly targetId: string;
  readonly authorizationMethod: string;
  readonly scope: TargetScope;
  readonly limits: TargetLimits;
  readonly confirmedAt: string;
  readonly validFrom: string;
  readonly expiresAt: string | null;
  readonly revokedAt: string | null;
  readonly active: boolean;
}

export interface CreateTargetInput {
  readonly origin: string;
  readonly environment: TargetEnvironment;
  readonly allowedHosts?: readonly string[];
  readonly allowedPaths?: readonly string[];
  readonly allowedPorts?: readonly number[];
}

export function listTargets(projectId: string): Promise<Target[]> {
  return request<Target[]>(`/projects/${projectId}/targets`);
}

export function createTarget(
  projectId: string,
  input: CreateTargetInput,
  csrfToken: string,
): Promise<Target> {
  return unsafeRequest<Target>("POST", `/projects/${projectId}/targets`, csrfToken, input);
}

export function validateTarget(targetId: string, csrfToken: string): Promise<Target> {
  return unsafeRequest<Target>("POST", `/targets/${targetId}/validate`, csrfToken);
}

export function listAuthorizations(targetId: string): Promise<TargetAuthorization[]> {
  return request<TargetAuthorization[]>(`/targets/${targetId}/authorizations`);
}

export function createAuthorization(
  targetId: string,
  limits: TargetLimits,
  csrfToken: string,
  expiresAt?: string | null,
): Promise<TargetAuthorization> {
  return unsafeRequest<TargetAuthorization>(
    "POST",
    `/targets/${targetId}/authorizations`,
    csrfToken,
    {
      authorizationMethod: "explicit_confirmation",
      confirmed: true,
      limits,
      ...(expiresAt ? { expiresAt } : {}),
    },
  );
}

export function revokeAuthorization(
  authorizationId: string,
  csrfToken: string,
): Promise<TargetAuthorization> {
  return unsafeRequest<TargetAuthorization>(
    "POST",
    `/target-authorizations/${authorizationId}/revoke`,
    csrfToken,
  );
}

export interface RunRef {
  readonly id: string;
  readonly runType: JobGoal;
  readonly status: string;
}

export function createRuns(
  planId: string,
  targetAuthorizationId: string,
  csrfToken: string,
): Promise<RunRef[]> {
  return unsafeRequest<RunRef[]>("POST", `/plans/${planId}/runs`, csrfToken, {
    targetAuthorizationId,
  });
}

export function cancelRun(runId: string, csrfToken: string): Promise<RunRef> {
  return unsafeRequest<RunRef>("POST", `/runs/${runId}/cancel`, csrfToken);
}

export function updatePlanSelection(
  planId: string,
  expectedRevision: number,
  selectedTestIds: readonly string[],
  selectedLoadScenarioIds: readonly string[],
  csrfToken: string,
): Promise<JobPlan> {
  return unsafeRequest<JobPlan>("PATCH", `/plans/${planId}`, csrfToken, {
    expectedRevision,
    selectedTestIds,
    selectedLoadScenarioIds,
  });
}

export function approvePlan(
  planId: string,
  expectedRevision: number,
  csrfToken: string,
): Promise<JobPlan> {
  return unsafeRequest<JobPlan>("POST", `/plans/${planId}/approve`, csrfToken, {
    expectedRevision,
    confirmed: true,
  });
}

export function createProject(name: string, slug: string, csrfToken: string): Promise<Project> {
  const body = slug.trim() === "" ? { name } : { name, slug };
  return unsafeRequest<Project>("POST", "/projects", csrfToken, body);
}

export function listMembers(projectId: string): Promise<ProjectMember[]> {
  return request<ProjectMember[]>(`/projects/${projectId}/members`);
}

export function addMember(
  projectId: string,
  email: string,
  role: ProjectRole,
  csrfToken: string,
): Promise<ProjectMember> {
  return unsafeRequest<ProjectMember>("POST", `/projects/${projectId}/members`, csrfToken, {
    email,
    role,
  });
}

export function listAudit(projectId: string): Promise<AuditEvent[]> {
  return request<AuditEvent[]>(`/projects/${projectId}/audit?limit=50`);
}

export function canManageMembers(role: ProjectRole): boolean {
  return role === "owner" || role === "admin";
}

export function canViewAudit(role: ProjectRole): boolean {
  return role === "owner" || role === "admin";
}

export function assignableRoles(role: ProjectRole): ProjectRole[] {
  if (role === "owner") {
    return ["admin", "collaborator", "viewer"];
  }
  if (role === "admin") {
    return ["collaborator", "viewer"];
  }
  return [];
}
