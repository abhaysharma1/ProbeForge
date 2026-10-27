import { type AnchorHTMLAttributes, type MouseEvent, useMemo, useSyncExternalStore } from "react";

/**
 * Hash-based navigation.
 *
 * The web app ships as a static build with no defined server-side SPA fallback,
 * so every screen is addressed by a fragment (`#/projects/...`). Fragments need
 * no rewrite rules, keep Back/Forward working, and survive a refresh without a
 * new routing dependency.
 */

export type ProjectTab = "repositories" | "jobs" | "runs" | "targets" | "members" | "activity";

export const DEFAULT_PROJECT_TAB: ProjectTab = "repositories";

export const PROJECT_TABS: readonly ProjectTab[] = [
  "repositories",
  "jobs",
  "runs",
  "targets",
  "members",
  "activity",
];

export type Route =
  | { readonly kind: "projects" }
  | { readonly kind: "project"; readonly projectId: string; readonly tab: ProjectTab }
  | { readonly kind: "job"; readonly projectId: string; readonly jobId: string }
  | { readonly kind: "run"; readonly projectId: string; readonly runId: string };

function decodeSegment(value: string | undefined): string | null {
  if (value === undefined || value === "") {
    return null;
  }
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
}

function isProjectTab(value: string | undefined): value is ProjectTab {
  return value !== undefined && (PROJECT_TABS as readonly string[]).includes(value);
}

export function parseRoute(hash: string): Route {
  const raw = hash.startsWith("#") ? hash.slice(1) : hash;
  const segments = raw.split("/").filter((segment) => segment !== "");
  if (segments[0] !== "projects") {
    return { kind: "projects" };
  }
  const projectId = decodeSegment(segments[1]);
  if (projectId === null) {
    return { kind: "projects" };
  }
  if (segments.length === 2) {
    return { kind: "project", projectId, tab: DEFAULT_PROJECT_TAB };
  }
  const section = segments[2];
  if (segments.length >= 4 && section === "jobs") {
    const jobId = decodeSegment(segments[3]);
    if (jobId !== null) {
      return { kind: "job", projectId, jobId };
    }
  }
  if (segments.length >= 4 && section === "runs") {
    const runId = decodeSegment(segments[3]);
    if (runId !== null) {
      return { kind: "run", projectId, runId };
    }
  }
  if (segments.length === 3 && isProjectTab(section)) {
    return { kind: "project", projectId, tab: section };
  }
  return { kind: "project", projectId, tab: DEFAULT_PROJECT_TAB };
}

export function projectsPath(): string {
  return "#/projects";
}

export function projectPath(projectId: string, tab: ProjectTab = DEFAULT_PROJECT_TAB): string {
  return `#/projects/${encodeURIComponent(projectId)}/${tab}`;
}

export function jobPath(projectId: string, jobId: string): string {
  return `#/projects/${encodeURIComponent(projectId)}/jobs/${encodeURIComponent(jobId)}`;
}

export function runPath(projectId: string, runId: string): string {
  return `#/projects/${encodeURIComponent(projectId)}/runs/${encodeURIComponent(runId)}`;
}

/** The tab a project route should highlight; detail routes highlight their list. */
export function activeTabOf(route: Route): ProjectTab | null {
  switch (route.kind) {
    case "project":
      return route.tab;
    case "job":
      return "jobs";
    case "run":
      return "runs";
    default:
      return null;
  }
}

export function navigate(to: string): void {
  if (typeof window === "undefined" || window.location.hash === to) {
    return;
  }
  // Assigning the fragment pushes a history entry, so Back returns to the
  // previous screen and the `hashchange` listener renders the new route.
  window.location.hash = to;
}

function subscribeToHash(onChange: () => void): () => void {
  const handle = () => {
    // A route change is a page change: start the new page at the top. The
    // shell itself is not animated.
    document.documentElement.scrollTop = 0;
    onChange();
  };
  window.addEventListener("hashchange", handle);
  return () => window.removeEventListener("hashchange", handle);
}

function hashSnapshot(): string {
  return window.location.hash;
}

export function useHashRoute(): Route {
  const hash = useSyncExternalStore(subscribeToHash, hashSnapshot, () => "");
  return useMemo(() => parseRoute(hash), [hash]);
}

interface RouteLinkProps extends Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> {
  readonly to: string;
}

/**
 * An anchor that keeps real link semantics (href, middle-click, copy address)
 * while navigating in-page. Modified clicks are left to the browser.
 */
export function RouteLink({ to, onClick, ...rest }: RouteLinkProps) {
  function handleClick(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (event.defaultPrevented) {
      return;
    }
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    event.preventDefault();
    navigate(to);
  }

  return <a href={to} onClick={handleClick} {...rest} />;
}
