import { describe, expect, it } from "bun:test";
import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { type GitTarget, validateGitUrl } from "./git-url";
import {
  ingestRepository,
  type RepositoryIngestOptions,
  type RepositorySnapshot,
  withRepositorySnapshot,
} from "./ingest";
import type { IngestionLimits } from "./limits";
import {
  addBinaryFile,
  addLargeFile,
  addSecondCommit,
  addSubmoduleEntry,
  addSymlinkEntry,
  commitAll,
  createFixtureRepository,
  createTag,
  exists,
  fixtureFileUrl,
  fixtureRoot,
  resolveGitSha,
  type TestRepository,
} from "./testing";

function targetFor(repo: TestRepository): GitTarget {
  return validateGitUrl(fixtureFileUrl(repo.root), {
    allowedHosts: [],
    allowLocalFixtures: true,
    localFixtureRoot: fixtureRoot(repo.root),
  });
}

function options(
  repo: TestRepository,
  overrides: Partial<RepositoryIngestOptions> = {},
  limits: Partial<IngestionLimits> = {},
): RepositoryIngestOptions {
  return {
    target: targetFor(repo),
    reference: "main",
    credentialRef: null,
    secretResolver: { resolve: async () => null },
    audit: repo.audit,
    limits: { ...repo.limits, ...limits },
    workspaceBaseDir: join(repo.tempRoot, "workspaces"),
    localFixtureRoot: fixtureRoot(repo.root),
    allowLocalFixtures: true,
    ...overrides,
  };
}

describe("read-only repository ingestion", () => {
  it("analyzes an inert fixture at a pinned commit without executing repository content", async () => {
    const repo = await createFixtureRepository();
    try {
      const expectedSha = await resolveGitSha(repo.root, "main");
      let workspacePath = "";
      let workspaceRepoPath = "";
      let snapshot: RepositorySnapshot | undefined;
      await withRepositorySnapshot(
        options(repo, {
          onWorkspaceCreated: (path) => {
            workspacePath = path;
          },
        }),
        async (session) => {
          snapshot = session.snapshot;
          workspaceRepoPath = session.workspacePath;
          const app = await session.readFile("src/app.ts");
          expect(app.text).toContain("inert");
          expect(await exists(join(workspacePath, "fixture-executed.txt"))).toBe(false);
          expect(await exists(join(workspaceRepoPath, "fixture-executed.txt"))).toBe(false);
        },
      );
      expect(snapshot).toBeDefined();
      const result = snapshot as RepositorySnapshot;
      expect(result.commitSha).toBe(expectedSha);
      expect(result.sourceReference).toBe("main");
      expect(result.untrustedContent).toBe(true);
      expect(result.analysisMetadata.executedRepositoryCode).toBe(false);
      expect(result.analysisMetadata.untrustedContent).toBe(true);
      expect(result.detectedLanguages).toEqual(
        expect.arrayContaining(["TypeScript", "Python", "Rust", "Go", "Java"]),
      );
      expect(result.frameworks).toEqual(
        expect.arrayContaining(["React", "Express", "FastAPI", "Axum"]),
      );
      expect(result.files.some((file) => file.path === "src/app.ts")).toBe(true);
      const packageManifest = result.manifestInventory.manifests.find(
        (manifest) => manifest.path === "package.json",
      );
      expect(packageManifest).toBeDefined();
      expect(packageManifest?.scriptNames).toEqual(
        expect.arrayContaining(["preinstall", "postinstall", "build", "test", "start"]),
      );
      expect(packageManifest?.dependencies.some((dependency) => dependency.name === "react")).toBe(
        true,
      );
      expect(
        result.manifestInventory.manifests.some(
          (manifest) =>
            manifest.path === "broken/package.json" && manifest.parseError === "invalid_json",
        ),
      ).toBe(true);
      expect(
        result.manifestInventory.lockfiles.some(
          (lockfile) => lockfile.path === "package-lock.json" && lockfile.parsed,
        ),
      ).toBe(true);
      expect(workspacePath.length).toBeGreaterThan(0);
      expect(await exists(workspacePath)).toBe(false);
      expect(repo.audit.events.map((event) => event.event)).toEqual(
        expect.arrayContaining([
          "repository.clone.started",
          "repository.reference.resolved",
          "repository.snapshot.ready",
          "repository.workspace.cleaned",
        ]),
      );
      const auditJson = JSON.stringify(repo.audit.events);
      expect(auditJson).not.toContain("file://");
      expect(auditJson).not.toContain("secret://");
      expect(auditJson).not.toContain("fixture-executed");
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("resolves branches, tags, and full commit SHAs to the requested snapshot", async () => {
    const repo = await createFixtureRepository();
    try {
      const baseline = await resolveGitSha(repo.root, "main");
      await createTag(repo.root, "v1", baseline);
      const second = await addSecondCommit(repo.root);

      const pinned = await ingestRepository(options(repo, { reference: baseline }));
      expect(pinned.commitSha).toBe(baseline);
      expect(pinned.files.some((file) => file.path === "src/new.ts")).toBe(false);

      const tag = await ingestRepository(options(repo, { reference: "v1" }));
      expect(tag.commitSha).toBe(baseline);
      expect(tag.files.some((file) => file.path === "src/new.ts")).toBe(false);

      const exactTag = await ingestRepository(options(repo, { reference: "refs/tags/v1" }));
      expect(exactTag.commitSha).toBe(baseline);

      const branch = await ingestRepository(options(repo, { reference: "main" }));
      expect(branch.commitSha).toBe(second);
      expect(branch.files.some((file) => file.path === "src/new.ts")).toBe(true);
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("records symlinks and submodules without following or initializing them", async () => {
    const repo = await createFixtureRepository();
    try {
      await addSymlinkEntry(repo.root);
      await addSubmoduleEntry(repo.root);
      let workspacePath = "";
      const snapshot = await withRepositorySnapshot(
        options(repo, {
          onWorkspaceCreated: (path) => {
            workspacePath = path;
          },
        }),
        async (session) => session.snapshot,
      );
      expect(snapshot.symlinks).toEqual(["escape-link"]);
      expect(snapshot.submodules).toEqual(["vendor/example"]);
      expect(snapshot.files.some((file) => file.path === "escape-link")).toBe(false);
      expect(snapshot.files.some((file) => file.path === "vendor/example")).toBe(false);
      expect(await exists(join(workspacePath, "fixture-executed.txt"))).toBe(false);
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("handles malformed, oversized, and binary files without reading unbounded content", async () => {
    const repo = await createFixtureRepository();
    try {
      const baseline = await ingestRepository(options(repo));
      expect(
        baseline.manifestInventory.manifests.some(
          (manifest) =>
            manifest.path === "broken/package.json" && manifest.parseError === "invalid_json",
        ),
      ).toBe(true);

      await addLargeFile(repo.root, 1_000);
      await addBinaryFile(repo.root);
      await commitAll(repo.root, "oversized and binary files");

      let workspacePath = "";
      await withRepositorySnapshot(
        options(
          repo,
          {
            onWorkspaceCreated: (path) => {
              workspacePath = path;
            },
          },
          { maxFileBytes: 200 },
        ),
        async (session) => {
          const oversized = session.snapshot.files.find(
            (file) => file.path === "src/oversized.txt",
          );
          expect(oversized?.kind).toBe("oversized");
          const binary = session.snapshot.files.find((file) => file.path === "src/binary.bin");
          expect(binary?.kind).toBe("binary");
          await expect(session.readFile("../outside")).rejects.toMatchObject({
            code: "unsafe_path",
          });
          await expect(session.readFile("src/app.ts", { maxBytes: 3 })).rejects.toMatchObject({
            code: "file_too_large",
          });
          await expect(session.readFile("src/binary.bin")).rejects.toMatchObject({
            code: "binary_file",
          });
        },
      );
      expect(await exists(workspacePath)).toBe(false);
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("cleans up the workspace after clone failure", async () => {
    const repo = await createFixtureRepository();
    try {
      const workspaceBase = join(repo.tempRoot, "failure-workspaces");
      await expect(
        ingestRepository(
          options(repo, {
            reference: "missing-reference",
            workspaceBaseDir: workspaceBase,
          }),
        ),
      ).rejects.toMatchObject({ code: "reference_not_found" });
      expect(await readdir(workspaceBase)).toEqual([]);
      expect(repo.audit.events.map((event) => event.event)).toEqual(
        expect.arrayContaining(["repository.ingest.failed", "repository.workspace.cleaned"]),
      );
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("cleans up the workspace when cancelled after creation", async () => {
    const repo = await createFixtureRepository();
    try {
      const controller = new AbortController();
      let workspacePath = "";
      await expect(
        withRepositorySnapshot(
          options(repo, {
            signal: controller.signal,
            onWorkspaceCreated: (path) => {
              workspacePath = path;
              controller.abort();
            },
          }),
          async () => "unreachable",
        ),
      ).rejects.toMatchObject({ code: "cancelled" });
      expect(workspacePath.length).toBeGreaterThan(0);
      expect(await exists(workspacePath)).toBe(false);
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("requires explicit local-fixture opt-in and enforces tree limits before inspection", async () => {
    const repo = await createFixtureRepository();
    try {
      await expect(
        ingestRepository(options(repo, { allowLocalFixtures: false })),
      ).rejects.toMatchObject({ code: "unsupported_scheme" });

      const workspaceBase = join(repo.tempRoot, "limit-workspaces");
      await expect(
        ingestRepository(options(repo, { workspaceBaseDir: workspaceBase }, { maxFiles: 2 })),
      ).rejects.toMatchObject({ code: "too_many_files" });
      expect(await readdir(workspaceBase)).toEqual([]);
    } finally {
      await repo.cleanup();
    }
  }, 30_000);

  it("rejects a credential reference for a local fixture before creating a workspace", async () => {
    const repo = await createFixtureRepository();
    try {
      await expect(
        ingestRepository(options(repo, { credentialRef: "secret://git/github" })),
      ).rejects.toMatchObject({ code: "unsupported_credential_scheme" });
    } finally {
      await repo.cleanup();
    }
  });
});
