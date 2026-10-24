import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { Job, JobPlan } from "./api";

const USER = {
  id: "11111111-1111-4111-8111-111111111111",
  email: "owner@example.test",
  displayName: "Owner",
};

const TOKEN_PLACEHOLDER = "$" + "{token}";

const PROJECT = {
  id: "22222222-2222-4222-8222-222222222222",
  name: "Payments API",
  slug: "payments-api",
  ownerId: USER.id,
  role: "owner",
  memberCount: 1,
  createdAt: "2026-10-20T12:00:00Z",
  updatedAt: "2026-10-20T12:00:00Z",
};

const MEMBER = {
  userId: USER.id,
  email: USER.email,
  displayName: USER.displayName,
  role: "owner",
  joinedAt: "2026-10-20T12:00:00Z",
};

const JOB: Job = {
  id: "55555555-5555-4555-8555-555555555555",
  projectId: PROJECT.id,
  goal: "vulnerability_testing",
  intensity: "standard",
  status: "awaiting_approval",
  displayName: "Payments plan review",
  currentPlanId: "66666666-6666-4666-8666-666666666666",
  createdAt: "2026-10-21T10:00:00Z",
  updatedAt: "2026-10-21T10:05:00Z",
};

const PLAN: JobPlan = {
  id: JOB.currentPlanId as string,
  projectId: PROJECT.id,
  jobId: JOB.id,
  version: 1,
  revision: 0,
  status: "proposed",
  goal: "vulnerability_testing",
  summary: "Review the discovered route and inspect credential handling.",
  confidence: 0.72,
  requiredInputs: ["Authorized target"],
  limitations: ["No target was contacted while generating this plan."],
  evidenceReferences: ["src/server.ts"],
  createdByAgent: true,
  agentProvider: "faux",
  agentModel: "planner-fixture",
  createdAt: "2026-10-21T10:04:00Z",
  updatedAt: "2026-10-21T10:05:00Z",
  catalogVersion: "0.2.0",
  plannedTests: [
    {
      id: "77777777-7777-4777-8777-777777777777",
      catalogTestId: "http.headers.missing",
      name: "Missing security headers",
      rationale: "The repository exposes an HTTP route that can be checked for response hardening.",
      risk: "low",
      prerequisites: ["Authorized target"],
      expectedEvidence: "Response headers and observation timestamp.",
      targetRequirements: ["authorized target origin"],
      requiresExplicitConfirmation: false,
      configuration: { route: "/api/items" },
      rationaleReferences: ["src/server.ts"],
      selected: false,
      catalog: {
        id: "http.headers.missing",
        name: "Missing security headers",
        category: "http",
        risk: "low",
        description: "Checks a bounded HTTP response for baseline security headers.",
        prerequisites: ["Authorized target"],
        toolRequirements: ["http_probe>=0.1"],
        targetRequirements: ["authorized target origin"],
        timeoutSeconds: 30,
        evidenceExpectations: ["Response headers and observation timestamp"],
        requiresExplicitConfirmation: false,
        available: true,
        availabilityNote:
          "A vetted native adapter exists for this catalog ID; run creation is not wired yet.",
      },
    },
    {
      id: "88888888-8888-4888-8888-888888888888",
      catalogTestId: "static.secrets.exposure",
      name: "Potential secret exposure",
      rationale: "The pinned snapshot can be checked with a non-printing ruleset.",
      risk: "high",
      prerequisites: ["Pinned repository snapshot"],
      expectedEvidence: "Redacted file path and pattern class.",
      targetRequirements: ["read-only repository snapshot"],
      requiresExplicitConfirmation: true,
      configuration: {},
      rationaleReferences: ["src/config.ts"],
      selected: false,
      catalog: {
        id: "static.secrets.exposure",
        name: "Potential secret exposure",
        category: "static",
        risk: "high",
        description: "Searches the pinned snapshot for high-confidence credential patterns.",
        prerequisites: ["Pinned repository snapshot"],
        toolRequirements: ["static_secret_scanner>=0.1"],
        targetRequirements: ["read-only repository snapshot"],
        timeoutSeconds: 60,
        evidenceExpectations: ["Redacted file path and pattern class"],
        requiresExplicitConfirmation: true,
        available: false,
        availabilityNote:
          "Execution adapter is not implemented yet. Approval records the selected test; it does not start a run.",
      },
    },
  ],
  loadScenarios: [],
  approval: null,
};

const LOAD_JOB: Job = {
  ...JOB,
  id: "99999999-9999-4999-8999-999999999999",
  goal: "load_testing",
  displayName: "Load plan review",
  currentPlanId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
};

const LOAD_PLAN: JobPlan = {
  ...PLAN,
  id: LOAD_JOB.currentPlanId as string,
  jobId: LOAD_JOB.id,
  goal: "load_testing",
  summary: "A conservative correlated load scenario for discovered routes.",
  plannedTests: [],
  loadScenarios: [
    {
      id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      name: "Baseline API read",
      description: "Conservative correlated profile for discovered read-only routes.",
      requests: [
        {
          id: "session",
          method: "POST",
          path: "/api/session",
          headers: { "Content-Type": "application/json" },
          secretHeaders: { Authorization: "secret://fixtures/service-token" },
          extract: [{ name: "token", source: "header", selector: "x-session-token" }],
          bodyTemplate: { username: "fixture-user" },
          expectedStatuses: [200],
        },
        {
          id: "profile",
          method: "GET",
          path: `/api/profile/${TOKEN_PLACEHOLDER}`,
          dependsOn: ["session"],
          headers: { "X-Session": TOKEN_PLACEHOLDER },
          assertions: [
            {
              source: "json_body",
              selector: "/status",
              operator: "equals",
              expected: "ok",
            },
          ],
          expectedStatuses: [200],
        },
      ],
      profile: {
        virtualUsers: 2,
        rampUpSeconds: 5,
        durationSeconds: 30,
        maxRequests: 200,
        maxRequestsPerSecond: 2,
        stopConditions: { errorRate: 0.05 },
      },
      credentialRef: null,
      selected: false,
    },
  ],
};

function planWithSelection(selectedTestIds: readonly string[]): JobPlan {
  return {
    ...PLAN,
    revision: 1,
    plannedTests: PLAN.plannedTests.map((test) => ({
      ...test,
      selected: selectedTestIds.includes(test.id),
    })),
  };
}

function approvedPlan(plan: JobPlan): JobPlan {
  return {
    ...plan,
    status: "approved",
    approval: {
      id: "99999999-9999-4999-8999-999999999999",
      planRevision: plan.revision,
      approvedBy: USER.id,
      approvedByName: USER.displayName,
      approvedByEmail: USER.email,
      approvedAt: "2026-10-21T10:10:00Z",
      approvedTestIds: plan.plannedTests.filter((test) => test.selected).map((test) => test.id),
      approvedLoadScenarioIds: [],
    },
  };
}

const RUN = {
  id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
  projectId: PROJECT.id,
  jobId: JOB.id,
  runType: "vulnerability_testing",
  status: "completed",
  planId: PLAN.id,
  planVersion: 1,
  startedAt: "2026-10-21T11:00:00Z",
  finishedAt: "2026-10-21T11:02:00Z",
  createdAt: "2026-10-21T11:00:00Z",
  resultCounts: {
    passed: 1,
    failed: 1,
    skipped: 0,
    inconclusive: 0,
    error: 0,
    cancelled: 0,
    confirmed: 1,
    suspected: 0,
  },
  reportStatus: "final",
};

const RESULT_ID = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const EVIDENCE_ID = "ffffffff-ffff-4fff-8fff-ffffffffffff";

const RUN_DETAIL = {
  run: RUN,
  reportType: "security",
  target: {
    authorizationId: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
    origin: "https://inert.example.test",
  },
  scenario: null,
  results: [
    {
      id: RESULT_ID,
      catalogTestId: "http.headers.missing",
      resultKey: "result-1",
      outcome: "failed",
      findingStatus: "confirmed",
      severity: "medium",
      confidence: 0.8,
      summary: "Missing security header observed.",
      details: {},
      observation: { observed: "x-content-type-options absent" },
      toolName: "http_probe",
      toolVersion: "0.1.0",
      durationMs: 5,
      createdAt: "2026-10-21T11:01:00Z",
      evidence: [
        {
          id: EVIDENCE_ID,
          storageRef: "evidence/1",
          sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
          mimeType: "text/plain",
          sizeBytes: 10,
          redacted: true,
          retentionUntil: null,
          createdAt: "2026-10-21T11:01:00Z",
        },
      ],
    },
  ],
  recommendations: [
    {
      id: "11111111-2222-4333-8444-555555555555",
      resultId: RESULT_ID,
      evidenceId: null,
      runMetricId: null,
      kind: "observed",
      severity: "medium",
      title: "Review finding http.headers.missing",
      body: "Missing security header observed.",
      rationale: null,
      confidence: 0.8,
      generatedBy: "system",
      createdAt: "2026-10-21T11:03:00Z",
    },
  ],
  metrics: {
    requestCount: 0,
    errorCount: 0,
    responseBytes: 0,
    latencyP95Ms: 0,
    throughputRps: 0,
  },
  report: {
    id: "aaaa1111-1111-4111-8111-111111111111",
    runId: RUN.id,
    reportType: "security",
    status: "final",
    currentVersion: 1,
    lastErrorCode: null,
    createdAt: "2026-10-21T11:03:00Z",
    updatedAt: "2026-10-21T11:03:00Z",
    latest: {
      id: "bbbb2222-2222-4222-8222-222222222222",
      version: 1,
      status: "final",
      contentHash: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
      summary: "Security run completed: 1 confirmed.",
      generatedBy: "system",
      createdAt: "2026-10-21T11:03:00Z",
      content: {
        claims: [
          {
            claimKind: "observed",
            text: "Missing security header observed.",
            references: { resultIds: [RESULT_ID], evidenceIds: [EVIDENCE_ID], metricIds: [] },
          },
          {
            claimKind: "hypothesis",
            text: "Latency may reflect target capacity.",
            references: { resultIds: [], evidenceIds: [], metricIds: [] },
          },
        ],
      },
    },
    versions: [
      {
        id: "bbbb2222-2222-4222-8222-222222222222",
        version: 1,
        status: "final",
        contentHash: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        summary: "Security run completed: 1 confirmed.",
        generatedBy: "system",
        createdAt: "2026-10-21T11:03:00Z",
      },
    ],
  },
};

const ADMIN_SETTINGS = {
  planning: {
    provider: null,
    model: null,
    limits: {},
    apiKeyConfigured: false,
    apiKeyHint: null,
    apiKeyUpdatedAt: null,
  },
  rateLimits: { apiRequestsPerMinute: 240, expensiveRequestsPerMinute: 20 },
  retention: { eventDays: 30, reportDays: 180, counterDays: 7 },
  projectDefaults: { maxActiveRuns: 2, maxRequestsPerWindow: null, windowSeconds: 86400 },
  settingsKeyConfigured: true,
  updatedBy: null,
  updatedAt: "2026-10-24T00:00:00Z",
};

function mockResponse(body: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

interface FetchOptions {
  readonly session: boolean;
  readonly loginStatus?: number;
  readonly projects?: readonly unknown[];
  readonly role?: "owner" | "admin" | "collaborator" | "viewer";
  readonly jobs?: readonly Job[];
  readonly plan?: JobPlan;
  readonly planPatchStatus?: number;
  readonly planApproveStatus?: number;
  readonly runs?: readonly unknown[];
  readonly runDetail?: unknown;
  readonly targets?: readonly unknown[];
  readonly createdJob?: unknown;
  readonly createJobStatus?: number;
  readonly admin?: boolean;
  readonly adminSettings?: unknown;
}

function installFetch(options: FetchOptions) {
  let currentPlan = options.plan ? (JSON.parse(JSON.stringify(options.plan)) as JobPlan) : null;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";

    if (url.endsWith("/api/auth/session")) {
      return options.session
        ? mockResponse({
            user: { ...USER, isAdmin: options.admin ?? false },
            sessionId: "33333333-3333-4333-8333-333333333333",
            csrfToken: "csrf-session",
            expiresAt: "2026-10-23T20:00:00Z",
          })
        : mockResponse(
            { error: { code: "authentication_required", message: "Authentication is required." } },
            401,
          );
    }

    if (url.endsWith("/api/auth/csrf")) {
      return mockResponse({ csrfToken: "csrf-bootstrap" });
    }

    if (url.endsWith("/api/auth/login")) {
      if (options.loginStatus && options.loginStatus >= 400) {
        return mockResponse(
          { error: { code: "rate_limited", message: "Too many attempts. Try again later." } },
          options.loginStatus,
        );
      }
      return mockResponse({
        user: { ...USER, isAdmin: options.admin ?? false },
        csrfToken: "csrf-login",
        expiresAt: "2026-10-23T20:00:00Z",
      });
    }

    if (url.endsWith("/api/auth/logout")) {
      return mockResponse(null, 204);
    }

    if (url.endsWith("/api/projects") && method === "GET") {
      return mockResponse(options.projects ?? []);
    }

    if (url.includes("/api/projects/") && url.endsWith("/jobs") && method === "GET") {
      return mockResponse(options.jobs ?? []);
    }

    if (url.includes("/api/projects/") && url.endsWith("/runs") && method === "GET") {
      return mockResponse(options.runs ?? []);
    }

    if (
      url.includes("/api/runs/") &&
      !url.includes("/report") &&
      !url.includes("/compare") &&
      method === "GET"
    ) {
      return options.runDetail
        ? mockResponse(options.runDetail)
        : mockResponse({ error: { code: "run_not_found", message: "Run not found." } }, 404);
    }

    if (url.includes("/jobs/") && url.endsWith("/plan") && method === "GET") {
      return currentPlan
        ? mockResponse(currentPlan)
        : mockResponse({ error: { code: "plan_not_found", message: "Plan not found." } }, 404);
    }

    if (url.includes("/api/plans/") && url.endsWith("/approve") && method === "POST") {
      if (options.planApproveStatus && options.planApproveStatus >= 400) {
        return mockResponse(
          {
            error: {
              code: "plan_conflict",
              message:
                "This plan changed since it was loaded. Reload and review the latest revision.",
            },
          },
          options.planApproveStatus,
        );
      }
      if (!currentPlan) {
        return mockResponse({ error: { code: "plan_not_found", message: "Plan not found." } }, 404);
      }
      const body = JSON.parse(String(init?.body)) as { expectedRevision: number };
      if (body.expectedRevision !== currentPlan.revision) {
        return mockResponse(
          {
            error: {
              code: "plan_conflict",
              message:
                "This plan changed since it was loaded. Reload and review the latest revision.",
            },
          },
          409,
        );
      }
      currentPlan = approvedPlan(currentPlan);
      return mockResponse(currentPlan);
    }

    if (url.includes("/api/plans/") && method === "PATCH") {
      if (options.planPatchStatus && options.planPatchStatus >= 400) {
        return mockResponse(
          {
            error: {
              code: "plan_conflict",
              message:
                "This plan changed since it was loaded. Reload and review the latest revision.",
            },
          },
          options.planPatchStatus,
        );
      }
      if (!currentPlan) {
        return mockResponse({ error: { code: "plan_not_found", message: "Plan not found." } }, 404);
      }
      const body = JSON.parse(String(init?.body)) as {
        expectedRevision: number;
        selectedTestIds?: readonly string[];
      };
      if (body.expectedRevision !== currentPlan.revision) {
        return mockResponse(
          {
            error: {
              code: "plan_conflict",
              message:
                "This plan changed since it was loaded. Reload and review the latest revision.",
            },
          },
          409,
        );
      }
      currentPlan = planWithSelection(body.selectedTestIds ?? []);
      return mockResponse(currentPlan);
    }

    if (url.includes("/api/projects/") && url.endsWith("/targets") && method === "GET") {
      return mockResponse(options.targets ?? []);
    }

    if (url.includes("/api/jobs?") && method === "POST") {
      if (options.createJobStatus && options.createJobStatus >= 400) {
        return mockResponse(
          { error: { code: "invalid_repository_url", message: "Repository URL is not valid." } },
          options.createJobStatus,
        );
      }
      return mockResponse(options.createdJob ?? JOB, 201);
    }

    if (url.endsWith("/api/admin/settings") && method === "GET") {
      return mockResponse(options.adminSettings ?? ADMIN_SETTINGS);
    }

    if (url.includes("/members")) {
      return mockResponse([{ ...MEMBER, role: options.role ?? "owner" }]);
    }

    if (url.includes("/audit")) {
      return mockResponse([
        {
          id: "44444444-4444-4444-8444-444444444444",
          actorId: USER.id,
          action: "project.created",
          resourceType: "project",
          resourceId: PROJECT.id,
          metadata: {},
          createdAt: "2026-10-20T12:00:00Z",
        },
      ]);
    }

    return mockResponse({ error: { code: "not_found", message: "Not found." } }, 404);
  });

  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

async function renderSignedIn(options: FetchOptions) {
  const fetchMock = installFetch(options);
  render(<App />);
  await screen.findByRole("heading", { name: "Projects" });
  return fetchMock;
}

async function signIn() {
  await screen.findByRole("heading", { name: "Sign in" });
  fireEvent.change(screen.getByLabelText("Email"), {
    target: { value: "owner@example.test" },
  });
  fireEvent.change(screen.getByLabelText("Password"), {
    target: { value: "correct horse battery staple" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
}

describe("App", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("signs in and shows the empty projects state", async () => {
    installFetch({ session: false });
    render(<App />);

    await signIn();

    expect(await screen.findByRole("heading", { name: "Projects" })).toBeInTheDocument();
    expect(await screen.findByText("Create your first project")).toBeInTheDocument();
    expect(screen.getByText("No projects yet.")).toBeInTheDocument();
  });

  it("shows a rate-limit error without leaving the login view", async () => {
    installFetch({ session: false, loginStatus: 429 });
    render(<App />);

    await signIn();

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Too many attempts. Try again later.",
    );
    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
  });

  it("opens a project and shows members plus the audit trail", async () => {
    await renderSignedIn({ session: true, projects: [PROJECT] });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));

    expect(await screen.findByRole("heading", { name: "Payments API" })).toBeInTheDocument();
    expect((await screen.findAllByText("owner@example.test")).length).toBeGreaterThan(0);
    expect(await screen.findByText("Audit trail")).toBeInTheDocument();
    expect(await screen.findByText("Created project")).toBeInTheDocument();
    expect(screen.getByLabelText("Member email")).toBeInTheDocument();
  });

  it("does not offer member management or audit access to a viewer", async () => {
    await renderSignedIn({
      session: true,
      projects: [{ ...PROJECT, role: "viewer" }],
      role: "viewer",
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));

    expect(
      await screen.findByText("Viewers can read project data. Member management is restricted."),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Member email")).not.toBeInTheDocument();
    expect(screen.queryByText("Audit trail")).not.toBeInTheDocument();
  });

  it("reviews a plan, saves the selection, and records an immutable approval", async () => {
    await renderSignedIn({
      session: true,
      projects: [PROJECT],
      jobs: [JOB],
      plan: PLAN,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Payments plan review/ }));

    expect(
      await screen.findByRole("heading", { name: "Payments plan review" }),
    ).toBeInTheDocument();
    expect((await screen.findAllByText("Adapter unavailable")).length).toBeGreaterThan(0);
    expect(screen.getByText("Explicit confirmation")).toBeInTheDocument();
    expect(screen.getByText("http.headers.missing")).toBeInTheDocument();
    expect(screen.getByText("http_probe>=0.1")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Missing security headers"));
    fireEvent.click(screen.getByRole("button", { name: "Save selection" }));

    expect(await screen.findByText(/Selection saved at revision 1/)).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText(/I reviewed the selected tests/));
    fireEvent.click(screen.getByRole("button", { name: "Approve selected plan" }));

    expect(await screen.findByText(/Approval recorded/)).toBeInTheDocument();
    expect(await screen.findByText("Immutable approval recorded")).toBeInTheDocument();
    expect(await screen.findByText(/Approved by Owner/)).toBeInTheDocument();
  });

  it("reviews correlated load steps and the pinned scenario version", async () => {
    await renderSignedIn({
      session: true,
      projects: [PROJECT],
      jobs: [LOAD_JOB],
      plan: LOAD_PLAN,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Load plan review/ }));

    expect(await screen.findByRole("heading", { name: "Load plan review" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "Approval pins this scenario to plan v1. No load traffic is sent by approval.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Secret-backed headers: Authorization")).toBeInTheDocument();
    expect(screen.getByText("Extracts: token from header:x-session-token")).toBeInTheDocument();
    expect(screen.getByText("Asserts: json_body:/status equals")).toBeInTheDocument();
    expect(screen.getByText("after session")).toBeInTheDocument();
    expect(screen.getByText("error rate 0.05")).toBeInTheDocument();
  });

  it("opens a run and shows traceable results and report claims", async () => {
    await renderSignedIn({
      session: true,
      projects: [PROJECT],
      runs: [RUN],
      runDetail: RUN_DETAIL,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));
    expect(await screen.findByText("Runs")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: /View results/ }));

    expect(await screen.findByRole("heading", { name: "Run results" })).toBeInTheDocument();
    expect(
      (await screen.findAllByText("Missing security header observed.")).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("HYPOTHESIS")).toBeInTheDocument();
    expect(screen.getByText(/stored reference/)).toBeInTheDocument();
    expect(screen.getByText("https://inert.example.test")).toBeInTheDocument();
  });

  it("connects a public repository and reports queued analysis", async () => {
    await renderSignedIn({
      session: true,
      projects: [PROJECT],
      targets: [],
      createdJob: JOB,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));

    const repoInput = await screen.findByLabelText("Repository URL");
    fireEvent.change(repoInput, { target: { value: "https://github.com/owner/repo" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect repository" }));

    expect(await screen.findByText(/Connected "Payments plan review"/)).toBeInTheDocument();
  });

  it("rejects a non-public repository URL before submitting", async () => {
    await renderSignedIn({ session: true, projects: [PROJECT], targets: [] });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));

    const repoInput = await screen.findByLabelText("Repository URL");
    fireEvent.change(repoInput, { target: { value: "http://192.168.0.10/repo" } });
    expect(
      await screen.findByText(/Only public HTTPS repository URLs are supported/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect repository" })).toBeDisabled();
  });

  it("shows a reload action when a concurrent edit conflicts", async () => {
    await renderSignedIn({
      session: true,
      projects: [PROJECT],
      jobs: [JOB],
      plan: PLAN,
      planPatchStatus: 409,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Payments plan review/ }));
    fireEvent.click(await screen.findByLabelText("Missing security headers"));
    fireEvent.click(await screen.findByRole("button", { name: "Save selection" }));

    expect(await screen.findByText(/This plan changed since it was loaded/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reload latest plan" })).toBeInTheDocument();
    expect(screen.queryByText(/Selection saved/)).not.toBeInTheDocument();
  });

  it("keeps a viewer from changing or approving a plan", async () => {
    await renderSignedIn({
      session: true,
      projects: [{ ...PROJECT, role: "viewer" }],
      role: "viewer",
      jobs: [JOB],
      plan: PLAN,
    });

    fireEvent.click(await screen.findByRole("button", { name: /Payments API/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Payments plan review/ }));

    expect(
      await screen.findByText(/your project role does not permit changing the selection/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Missing security headers")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Approve selected plan" })).not.toBeInTheDocument();
  });

  it("shows the platform admin dashboard for an administrator", async () => {
    await renderSignedIn({ session: true, admin: true, projects: [] });

    fireEvent.click(await screen.findByRole("button", { name: /Platform admin/ }));

    expect(await screen.findByRole("heading", { name: "Platform settings" })).toBeInTheDocument();
    expect(screen.getByLabelText("Provider")).toBeInTheDocument();
    expect(screen.getByLabelText("API requests / minute")).toBeInTheDocument();
  });

  it("hides the platform admin entry from non-administrators", async () => {
    await renderSignedIn({ session: true, projects: [] });

    expect(screen.queryByRole("button", { name: /Platform admin/ })).toBeNull();
  });

  it("signs out and returns to the login view", async () => {
    await renderSignedIn({ session: true, projects: [] });

    const signOutButtons = screen.getAllByRole("button", { name: "Sign out" });
    const firstSignOut = signOutButtons[0];
    if (!firstSignOut) {
      throw new Error("expected a sign-out button");
    }
    fireEvent.click(firstSignOut);

    expect(await screen.findByRole("heading", { name: "Sign in" })).toBeInTheDocument();
  });
});
