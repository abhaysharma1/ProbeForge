import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const USER = {
  id: "11111111-1111-4111-8111-111111111111",
  email: "owner@example.test",
  displayName: "Owner",
};

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
}

function installFetch(options: FetchOptions) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";

    if (url.endsWith("/api/auth/session")) {
      return options.session
        ? mockResponse({
            user: USER,
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
        user: USER,
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
