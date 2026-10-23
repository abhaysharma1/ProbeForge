export type ProjectRole = "owner" | "admin" | "collaborator" | "viewer";

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
