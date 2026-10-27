import { describe, expect, it } from "vitest";
import {
  activeTabOf,
  DEFAULT_PROJECT_TAB,
  jobPath,
  parseRoute,
  projectPath,
  projectsPath,
  runPath,
} from "./routing";

const PROJECT_ID = "22222222-2222-4222-8222-222222222222";
const JOB_ID = "44444444-4444-4444-8444-444444444444";
const RUN_ID = "55555555-5555-4555-8555-555555555555";

describe("parseRoute", () => {
  it("treats an empty or unknown fragment as the projects list", () => {
    expect(parseRoute("")).toEqual({ kind: "projects" });
    expect(parseRoute("#")).toEqual({ kind: "projects" });
    expect(parseRoute("#/projects")).toEqual({ kind: "projects" });
    expect(parseRoute("#/unknown/path")).toEqual({ kind: "projects" });
  });

  it("defaults a project route to the repositories tab", () => {
    expect(parseRoute(`#/projects/${PROJECT_ID}`)).toEqual({
      kind: "project",
      projectId: PROJECT_ID,
      tab: DEFAULT_PROJECT_TAB,
    });
    expect(DEFAULT_PROJECT_TAB).toBe("repositories");
  });

  it("parses project tabs and falls back for unknown tabs", () => {
    expect(parseRoute(`#/projects/${PROJECT_ID}/members`)).toEqual({
      kind: "project",
      projectId: PROJECT_ID,
      tab: "members",
    });
    expect(parseRoute(`#/projects/${PROJECT_ID}/nonsense`)).toEqual({
      kind: "project",
      projectId: PROJECT_ID,
      tab: DEFAULT_PROJECT_TAB,
    });
  });

  it("parses job and run detail routes", () => {
    expect(parseRoute(`#/projects/${PROJECT_ID}/jobs/${JOB_ID}`)).toEqual({
      kind: "job",
      projectId: PROJECT_ID,
      jobId: JOB_ID,
    });
    expect(parseRoute(`#/projects/${PROJECT_ID}/runs/${RUN_ID}`)).toEqual({
      kind: "run",
      projectId: PROJECT_ID,
      runId: RUN_ID,
    });
    // A list section without a detail id stays on the tab.
    expect(parseRoute(`#/projects/${PROJECT_ID}/jobs`)).toEqual({
      kind: "project",
      projectId: PROJECT_ID,
      tab: "jobs",
    });
  });

  it("decodes percent-encoded identifiers", () => {
    expect(parseRoute("#/projects/a%20b/members")).toEqual({
      kind: "project",
      projectId: "a b",
      tab: "members",
    });
  });
});

describe("route paths", () => {
  it("round-trips project, job, and run paths", () => {
    expect(parseRoute(projectsPath())).toEqual({ kind: "projects" });
    expect(parseRoute(projectPath(PROJECT_ID, "targets"))).toEqual({
      kind: "project",
      projectId: PROJECT_ID,
      tab: "targets",
    });
    expect(parseRoute(jobPath(PROJECT_ID, JOB_ID))).toEqual({
      kind: "job",
      projectId: PROJECT_ID,
      jobId: JOB_ID,
    });
    expect(parseRoute(runPath(PROJECT_ID, RUN_ID))).toEqual({
      kind: "run",
      projectId: PROJECT_ID,
      runId: RUN_ID,
    });
  });

  it("highlights the list a detail page belongs to", () => {
    expect(activeTabOf({ kind: "projects" })).toBeNull();
    expect(activeTabOf({ kind: "project", projectId: PROJECT_ID, tab: "runs" })).toBe("runs");
    expect(activeTabOf({ kind: "job", projectId: PROJECT_ID, jobId: JOB_ID })).toBe("jobs");
    expect(activeTabOf({ kind: "run", projectId: PROJECT_ID, runId: RUN_ID })).toBe("runs");
  });
});
