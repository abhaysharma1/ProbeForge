import { describe, expect, it } from "bun:test";
import type {
  AgentTool,
  AgentToolResult,
  BeforeToolCallContext,
} from "@earendil-works/pi-agent-core";
import { DEFAULT_PLANNING_LIMITS } from "./limits";
import { makeContext, makePlannedTest, makeValidVulnerabilityPlan } from "./test-helpers";
import { createPlanningToolSession, type PlanningToolSession } from "./tools";

function findTool(session: PlanningToolSession, name: string): AgentTool {
  const tool = session.tools.find((candidate) => candidate.name === name);
  if (tool === undefined) throw new Error(`missing tool ${name}`);
  return tool;
}

async function execute(
  session: PlanningToolSession,
  name: string,
  args: unknown,
): Promise<AgentToolResult<unknown>> {
  const tool = findTool(session, name);
  const run = tool.execute as (
    id: string,
    params: unknown,
    signal?: AbortSignal,
  ) => Promise<AgentToolResult<unknown>>;
  return run(`call-${name}`, args);
}

function callContext(name: string, args: unknown): BeforeToolCallContext {
  return {
    assistantMessage: {},
    toolCall: { type: "toolCall", id: "call-1", name, arguments: args },
    args,
    context: {},
  } as unknown as BeforeToolCallContext;
}

describe("planning tool registry", () => {
  it("exposes only the bounded read-only and proposal tools", () => {
    const session = createPlanningToolSession(makeContext());
    expect(session.tools.map((tool) => tool.name).sort()).toEqual([
      "catalog_get_test",
      "catalog_list_tests",
      "discovery_get_route",
      "discovery_get_summary",
      "discovery_list_routes",
      "discovery_list_surfaces",
      "discovery_list_unsupported",
      "inspection_list_auth_schema",
      "plan_propose",
      "plan_validate",
      "repository_list_files",
      "repository_read_file",
    ]);
    expect(session.tools.some((tool) => /shell|exec|database|http|run/i.test(tool.name))).toBe(
      false,
    );
  });

  it("lists snapshot metadata without file contents", async () => {
    const session = createPlanningToolSession(makeContext());
    const result = await execute(session, "repository_list_files", { limit: 1 });
    const text = result.content[0]?.type === "text" ? result.content[0].text : "";
    expect(text).toContain("src/server.ts");
    expect(text).not.toContain("Ignore previous instructions");
    expect(session.stats().calls).toBe(1);
  });

  it("reads one bounded text file and rejects traversal or unknown files", async () => {
    const session = createPlanningToolSession(makeContext());
    const result = await execute(session, "repository_read_file", {
      path: "README.md",
      maxBytes: 512,
    });
    const text = result.content[0]?.type === "text" ? result.content[0].text : "";
    expect(text).toContain("untrusted_repository");
    expect(text).toContain("Ignore previous instructions");
    await expect(
      execute(session, "repository_read_file", { path: "../secrets.env" }),
    ).rejects.toThrow();
    await expect(
      execute(session, "repository_read_file", { path: "missing.ts" }),
    ).rejects.toThrow();
    expect(session.stats().calls).toBe(3);
    expect(session.stats().failedCalls).toBe(2);
  });

  it("accepts a valid proposal once and then stops further tools", async () => {
    const context = makeContext();
    const session = createPlanningToolSession(context);
    const result = await execute(session, "plan_propose", {
      plan: makeValidVulnerabilityPlan(),
    });
    expect(result.isError).toBe(false);
    expect(result.terminate).toBe(true);
    expect(session.proposal()).not.toBeNull();
    const before = await session.beforeToolCall(
      callContext("repository_read_file", { path: "README.md" }),
    );
    expect(before?.block).toBe(true);
  });

  it("returns structured validation issues for an invalid proposal", async () => {
    const session = createPlanningToolSession(makeContext());
    const result = await execute(session, "plan_propose", {
      plan: { ...makeValidVulnerabilityPlan(), plannedTests: [{ catalogTestId: "invented" }] },
    });
    expect(result.isError).toBe(true);
    const text = result.content[0]?.type === "text" ? result.content[0].text : "";
    expect(text).toContain("invalid_contract");
    expect(session.proposal()).toBeNull();
  });

  it("validates a candidate plan without submitting it", async () => {
    const session = createPlanningToolSession(makeContext());
    const valid = await execute(session, "plan_validate", {
      plan: makeValidVulnerabilityPlan(),
    });
    expect(valid.isError).toBe(false);
    expect(session.proposal()).toBeNull();
    const validText = valid.content[0]?.type === "text" ? valid.content[0].text : "";
    expect(validText).toContain('"valid":true');

    const invalid = await execute(session, "plan_validate", {
      plan: makeValidVulnerabilityPlan({
        plannedTests: [makePlannedTest(), makePlannedTest()],
      }),
    });
    const invalidText = invalid.content[0]?.type === "text" ? invalid.content[0].text : "";
    expect(invalidText).toContain("duplicate_catalog_test");
    expect(session.proposal()).toBeNull();
  });

  it("blocks unknown tools, executable argument fields, and oversized arguments", async () => {
    const session = createPlanningToolSession(makeContext());
    const unknown = await session.beforeToolCall(callContext("shell_exec", { command: "whoami" }));
    expect(unknown?.block).toBe(true);

    const executable = await session.beforeToolCall(
      callContext("repository_list_files", { prefix: "src", command: "whoami" }),
    );
    expect(executable?.block).toBe(true);

    const oversized = await session.beforeToolCall(
      callContext("repository_list_files", { prefix: "x".repeat(20_000) }),
    );
    expect(oversized?.block).toBe(true);
    expect(session.stats().blockedCalls).toBe(3);
  });

  it("enforces tool-call and output budgets with visible limit state", async () => {
    const oneTool = createPlanningToolSession(
      makeContext({ limits: { ...DEFAULT_PLANNING_LIMITS, maxToolCalls: 1 } }),
    );
    await execute(oneTool, "discovery_get_summary", {});
    const blocked = await oneTool.beforeToolCall(callContext("discovery_get_summary", {}));
    expect(blocked?.block).toBe(true);
    expect(oneTool.stats().limitReached).toBe("tool_calls");

    const tinyOutput = createPlanningToolSession(
      makeContext({
        limits: {
          ...DEFAULT_PLANNING_LIMITS,
          maxToolOutputBytes: 32,
          maxTotalToolOutputBytes: 32,
        },
      }),
    );
    await expect(execute(tinyOutput, "discovery_get_summary", {})).rejects.toThrow();
    expect(tinyOutput.stats().limitReached).toBe("tool_output");
  });

  it("returns only catalog entries and rejects catalog IDs outside the catalog", async () => {
    const context = makeContext();
    const session = createPlanningToolSession(context);
    const listed = await execute(session, "catalog_list_tests", { limit: 10 });
    const text = listed.content[0]?.type === "text" ? listed.content[0].text : "";
    expect(text).toContain("http.headers.missing");
    expect(text).not.toContain("http.invented");
    await expect(execute(session, "catalog_get_test", { id: "http.invented" })).rejects.toThrow();
  });
});
