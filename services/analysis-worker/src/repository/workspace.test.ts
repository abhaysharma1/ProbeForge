import { describe, expect, it } from "bun:test";
import { chmod, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { RepositoryIngestError } from "./errors";
import { createTemporaryWorkspace, withTemporaryWorkspace } from "./workspace";

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function withBase<T>(operation: (base: string) => Promise<T>): Promise<T> {
  const base = await mkdtemp(join(tmpdir(), "probeforge-workspace-test-"));
  try {
    return await operation(base);
  } finally {
    await rm(base, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
  }
}

describe("temporary workspaces", () => {
  it("creates a unique workspace and cleans it up after success", async () => {
    await withBase(async (base) => {
      let firstPath = "";
      let secondPath = "";
      await withTemporaryWorkspace({ baseDir: base }, async (workspace) => {
        firstPath = workspace.path;
        expect(await exists(workspace.repoPath)).toBe(true);
        expect(await exists(workspace.gitHooksPath)).toBe(true);
        expect(await exists(workspace.tempPath)).toBe(true);
      });
      await withTemporaryWorkspace({ baseDir: base }, async (workspace) => {
        secondPath = workspace.path;
      });
      expect(firstPath).not.toBe(secondPath);
      expect(await exists(firstPath)).toBe(false);
      expect(await exists(secondPath)).toBe(false);
    });
  });

  it("cleans up after a failed operation", async () => {
    await withBase(async (base) => {
      let workspacePath = "";
      await expect(
        withTemporaryWorkspace({ baseDir: base }, async (workspace) => {
          workspacePath = workspace.path;
          throw new RepositoryIngestError("git_failed", "expected test failure");
        }),
      ).rejects.toBeInstanceOf(RepositoryIngestError);
      expect(workspacePath.length).toBeGreaterThan(0);
      expect(await exists(workspacePath)).toBe(false);
    });
  });

  it("cleans up on cancellation", async () => {
    await withBase(async (base) => {
      const controller = new AbortController();
      let workspacePath = "";
      await expect(
        withTemporaryWorkspace({ baseDir: base }, async (workspace) => {
          workspacePath = workspace.path;
          controller.abort();
          if (controller.signal.aborted) {
            throw new RepositoryIngestError("cancelled", "cancelled by test");
          }
        }),
      ).rejects.toMatchObject({ code: "cancelled" });
      expect(await exists(workspacePath)).toBe(false);
    });
  });

  it("removes read-only files during cleanup", async () => {
    await withBase(async (base) => {
      let workspacePath = "";
      await withTemporaryWorkspace({ baseDir: base }, async (workspace) => {
        workspacePath = workspace.path;
        const file = join(workspace.repoPath, "read-only.txt");
        await writeFile(file, "read-only", { mode: 0o400 });
        await chmod(file, 0o400);
      });
      expect(await exists(workspacePath)).toBe(false);
    });
  });

  it("rejects a relative workspace base", async () => {
    await expect(createTemporaryWorkspace({ baseDir: "relative" })).rejects.toMatchObject({
      code: "internal_error",
    });
  });
});
