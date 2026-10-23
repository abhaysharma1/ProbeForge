import { lstat, readdir, realpath, stat } from "node:fs/promises";
import { dirname, isAbsolute, join, relative } from "node:path";
import { pathToFileURL } from "node:url";
import { SERVICE_VERSION } from "../health";
import type { AuditSink, SnapshotAuditEvent } from "./audit";
import { RepositoryIngestError, toRepositoryIngestError } from "./errors";
import { GitClient } from "./git";
import { assertReferenceSafe, type GitProvider, type GitTarget } from "./git-url";
import {
  inspectTree,
  type ManifestInventory,
  type ReadRepositoryFileResult,
  readRepositoryFile,
  type SnapshotFile,
} from "./inspect";
import { type IngestionLimits, validateIngestionLimits } from "./limits";
import { assertRemoteHostSafe, type HostResolver } from "./net";
import { assertCredentialRefSafe, type GitCredential, type SecretResolver } from "./secrets";
import { parseTreeInventory, type TreeInventory } from "./tree";
import { createTemporaryWorkspace, type TemporaryWorkspace } from "./workspace";

export interface SnapshotLimits {
  readonly maxFiles: number;
  readonly maxTotalBytes: number;
  readonly maxFileBytes: number;
  readonly maxDepth: number;
  readonly maxInventoryFiles: number;
  readonly maxDependenciesPerManifest: number;
  readonly maxWorkspaceBytes: number;
  readonly maxGitOutputBytes: number;
  readonly gitTimeoutMs: number;
}

export interface SnapshotAnalysisMetadata {
  readonly schemaVersion: "1";
  readonly generatorVersion: string;
  readonly gitVersion: string;
  readonly fileCount: number;
  readonly totalBytes: number;
  readonly inventoryFileCount: number;
  readonly textFileCount: number;
  readonly binaryFileCount: number;
  readonly oversizedFileCount: number;
  readonly generatedExcludedCount: number;
  readonly symlinkCount: number;
  readonly submoduleCount: number;
  readonly manifestCount: number;
  readonly lockfileCount: number;
  readonly filesTruncated: boolean;
  readonly manifestInventoryTruncated: boolean;
  readonly limits: SnapshotLimits;
  readonly untrustedContent: true;
  readonly executedRepositoryCode: false;
}

export interface RepositorySnapshot {
  readonly schemaVersion: "1";
  readonly commitSha: string;
  readonly sourceReference: string;
  readonly provider: GitProvider;
  readonly detectedLanguages: readonly string[];
  readonly frameworks: readonly string[];
  readonly manifestInventory: ManifestInventory;
  readonly analysisMetadata: SnapshotAnalysisMetadata;
  readonly files: readonly SnapshotFile[];
  readonly filesTruncated: boolean;
  readonly symlinks: readonly string[];
  readonly submodules: readonly string[];
  readonly untrustedContent: true;
}

export interface RepositorySession {
  readonly snapshot: RepositorySnapshot;
  /**
   * Internal workspace path. It is intentionally not part of the snapshot or
   * audit events and is removed when the callback returns.
   */
  readonly workspacePath: string;
  readFile(
    path: string,
    options?: { readonly maxBytes?: number },
  ): Promise<ReadRepositoryFileResult>;
}

export interface RepositoryIngestOptions {
  readonly target: GitTarget;
  readonly reference: string;
  readonly credentialRef: string | null;
  readonly secretResolver: SecretResolver;
  readonly audit: AuditSink;
  readonly limits: IngestionLimits;
  readonly signal?: AbortSignal;
  readonly workspaceBaseDir?: string;
  readonly localFixtureRoot?: string | null;
  /** Must be explicitly true to ingest a local `file:` fixture. */
  readonly allowLocalFixtures?: boolean;
  readonly hostResolver?: HostResolver;
  /** Internal test/observability hook; the path is not written to audit events. */
  readonly onWorkspaceCreated?: (workspacePath: string) => void;
}

const MAX_SNAPSHOT_BYTES = 60 * 1024;

function snapshotLimits(limits: IngestionLimits): SnapshotLimits {
  return {
    maxFiles: limits.maxFiles,
    maxTotalBytes: limits.maxTotalBytes,
    maxFileBytes: limits.maxFileBytes,
    maxDepth: limits.maxDepth,
    maxInventoryFiles: limits.maxInventoryFiles,
    maxDependenciesPerManifest: limits.maxDependenciesPerManifest,
    maxWorkspaceBytes: limits.maxWorkspaceBytes,
    maxGitOutputBytes: limits.maxGitOutputBytes,
    gitTimeoutMs: limits.gitTimeoutMs,
  };
}

async function measureWorkspace(workspace: TemporaryWorkspace, maxBytes: number): Promise<void> {
  const stack = [workspace.path];
  let totalBytes = 0;
  let entries = 0;
  while (stack.length > 0) {
    const current = stack.pop();
    if (current === undefined) break;
    const children = await readdir(current, { withFileTypes: true });
    for (const child of children) {
      entries += 1;
      if (entries > 50_000) {
        throw new RepositoryIngestError(
          "workspace_too_large",
          "Workspace entry limit was exceeded",
        );
      }
      const full = join(current, child.name);
      if (child.isSymbolicLink()) continue;
      if (child.isDirectory()) {
        stack.push(full);
        continue;
      }
      if (!child.isFile()) continue;
      const info = await lstat(full);
      totalBytes += info.size;
      if (totalBytes > maxBytes) {
        throw new RepositoryIngestError(
          "workspace_too_large",
          "Workspace exceeds the configured disk limit",
        );
      }
    }
  }
}

async function resolveLocalFixture(
  target: GitTarget,
  localFixtureRoot: string | null,
): Promise<GitTarget> {
  if (target.localPath === null) {
    throw new RepositoryIngestError("invalid_url", "Local fixture path is missing");
  }
  const realPath = await realpath(target.localPath);
  const info = await stat(realPath);
  if (!info.isDirectory()) {
    throw new RepositoryIngestError("invalid_url", "Local fixture path is not a directory");
  }
  if (localFixtureRoot !== null) {
    const realRoot = await realpath(localFixtureRoot);
    const rel = relative(realRoot, realPath);
    if (rel.startsWith("..") || isAbsolute(rel)) {
      throw new RepositoryIngestError(
        "unsafe_path",
        "Local fixture path escapes the configured fixture root",
      );
    }
  }
  return {
    ...target,
    url: pathToFileURL(realPath).href,
    localPath: realPath,
  };
}

function auditEvent(
  event: SnapshotAuditEvent["event"],
  workspaceId: string,
  details: Omit<SnapshotAuditEvent, "event" | "at" | "workspaceId"> = {},
): SnapshotAuditEvent {
  return { event, at: new Date().toISOString(), workspaceId, ...details };
}

async function writeAudit(sink: AuditSink, event: SnapshotAuditEvent): Promise<void> {
  try {
    await sink.write(event);
  } catch (error) {
    throw new RepositoryIngestError(
      "audit_failed",
      "Repository audit event could not be recorded",
      {
        cause: error,
      },
    );
  }
}

function mutableInventory(inventory: ManifestInventory): {
  manifests: Array<{
    path: string;
    ecosystem: ManifestInventory["manifests"][number]["ecosystem"];
    name: string | null;
    version: string | null;
    packageManager: string | null;
    scriptNames: readonly string[];
    dependencies: Array<{ name: string; version: string | null }>;
    truncated: boolean;
    parseError: string | null;
  }>;
  lockfiles: Array<{
    path: string;
    ecosystem: ManifestInventory["lockfiles"][number]["ecosystem"];
    sizeBytes: number;
    parsed: boolean;
    packageCount: number;
    dependencies: Array<{ name: string; version: string | null }>;
    truncated: boolean;
    parseError: string | null;
  }>;
  truncated: boolean;
} {
  return {
    manifests: inventory.manifests.map((manifest) => ({
      ...manifest,
      dependencies: [...manifest.dependencies],
    })),
    lockfiles: inventory.lockfiles.map((lockfile) => ({
      ...lockfile,
      dependencies: [...lockfile.dependencies],
    })),
    truncated: inventory.truncated,
  };
}

function compactSnapshot(
  base: Omit<
    RepositorySnapshot,
    "files" | "filesTruncated" | "manifestInventory" | "symlinks" | "submodules"
  >,
  files: readonly SnapshotFile[],
  tree: TreeInventory,
  inventory: ManifestInventory,
): RepositorySnapshot {
  let compactFiles = [...files];
  const manifestInventory = mutableInventory(inventory);
  let symlinks = tree.symlinks.slice(0, 100);
  let submodules = tree.submodules.slice(0, 100);
  let filesTruncated = files.length < tree.fileCount;
  let manifestTruncated = inventory.truncated;

  const build = (): RepositorySnapshot => ({
    ...base,
    manifestInventory: {
      manifests: manifestInventory.manifests,
      lockfiles: manifestInventory.lockfiles,
      truncated: manifestInventory.truncated,
    },
    files: compactFiles,
    filesTruncated,
    symlinks,
    submodules,
    untrustedContent: true,
    analysisMetadata: {
      ...base.analysisMetadata,
      inventoryFileCount: compactFiles.length,
      filesTruncated,
      manifestInventoryTruncated: manifestInventory.truncated,
    },
  });

  let snapshot = build();
  while (Buffer.byteLength(JSON.stringify(snapshot), "utf8") > MAX_SNAPSHOT_BYTES) {
    if (compactFiles.length > 0) {
      compactFiles = compactFiles.slice(0, Math.floor(compactFiles.length / 2));
      filesTruncated = true;
      snapshot = build();
      continue;
    }
    const withDependencies = manifestInventory.manifests.find(
      (manifest) => manifest.dependencies.length > 0,
    );
    if (withDependencies !== undefined) {
      for (const manifest of manifestInventory.manifests) {
        if (manifest.dependencies.length > 0) {
          manifest.dependencies = manifest.dependencies.slice(
            0,
            Math.floor(manifest.dependencies.length / 2),
          );
          manifest.truncated = true;
        }
      }
      manifestInventory.truncated = true;
      manifestTruncated = true;
      snapshot = build();
      continue;
    }
    const lockfileWithDependencies = manifestInventory.lockfiles.find(
      (lockfile) => lockfile.dependencies.length > 0,
    );
    if (lockfileWithDependencies !== undefined) {
      for (const lockfile of manifestInventory.lockfiles) {
        if (lockfile.dependencies.length > 0) {
          lockfile.dependencies = lockfile.dependencies.slice(
            0,
            Math.floor(lockfile.dependencies.length / 2),
          );
          lockfile.truncated = true;
        }
      }
      manifestInventory.truncated = true;
      manifestTruncated = true;
      snapshot = build();
      continue;
    }
    if (manifestInventory.lockfiles.length > 0) {
      manifestInventory.lockfiles = manifestInventory.lockfiles.slice(
        0,
        Math.floor(manifestInventory.lockfiles.length / 2),
      );
      manifestInventory.truncated = true;
      manifestTruncated = true;
      snapshot = build();
      continue;
    }
    if (manifestInventory.manifests.length > 0) {
      manifestInventory.manifests = manifestInventory.manifests.slice(
        0,
        Math.floor(manifestInventory.manifests.length / 2),
      );
      manifestInventory.truncated = true;
      manifestTruncated = true;
      snapshot = build();
      continue;
    }
    if (symlinks.length > 0) {
      symlinks = symlinks.slice(0, Math.floor(symlinks.length / 2));
      snapshot = build();
      continue;
    }
    if (submodules.length > 0) {
      submodules = submodules.slice(0, Math.floor(submodules.length / 2));
      snapshot = build();
      continue;
    }
    throw new RepositoryIngestError(
      "snapshot_metadata_too_large",
      "Repository snapshot metadata exceeds the storage limit",
    );
  }

  if (manifestTruncated && !snapshot.manifestInventory.truncated) {
    return { ...snapshot, manifestInventory: { ...snapshot.manifestInventory, truncated: true } };
  }
  return snapshot;
}

async function resolveCredential(
  target: GitTarget,
  credentialRef: string | null,
  resolver: SecretResolver,
): Promise<GitCredential | null> {
  if (credentialRef === null) return null;
  assertCredentialRefSafe(credentialRef);
  if (target.scheme !== "https" && target.scheme !== "ssh") {
    throw new RepositoryIngestError(
      "unsupported_credential_scheme",
      "Credential references are supported only for HTTPS and SSH repositories in this phase",
    );
  }
  const credential = await resolver.resolve(credentialRef);
  if (credential === null) {
    throw new RepositoryIngestError(
      "credential_unavailable",
      "The repository credential reference could not be resolved",
    );
  }
  if (target.scheme === "https" && credential.kind !== "http_basic") {
    throw new RepositoryIngestError(
      "unsupported_credential_kind",
      "HTTPS repositories require an HTTP basic credential",
    );
  }
  if (target.scheme === "ssh" && credential.kind !== "ssh_private_key") {
    throw new RepositoryIngestError(
      "unsupported_credential_kind",
      "SSH repositories require an SSH private-key credential",
    );
  }
  return credential;
}

/**
 * Creates a pinned snapshot, invokes the callback while the workspace exists, and
 * always attempts cleanup. The callback is the only place where bounded file
 * reads are available; after it returns, the repository is gone.
 */
export async function withRepositorySnapshot<T>(
  options: RepositoryIngestOptions,
  operation: (session: RepositorySession) => Promise<T>,
): Promise<T> {
  if (options.target.scheme === "file" && options.allowLocalFixtures !== true) {
    throw new RepositoryIngestError(
      "unsupported_scheme",
      "Local fixture ingestion requires explicit operator opt-in",
    );
  }
  validateIngestionLimits(options.limits);
  assertReferenceSafe(options.reference);
  const credential = await resolveCredential(
    options.target,
    options.credentialRef,
    options.secretResolver,
  );
  if (options.target.scheme !== "file") {
    await assertRemoteHostSafe(options.target.hostname, options.hostResolver);
  }

  const workspace = await createTemporaryWorkspace({
    ...(options.workspaceBaseDir === undefined ? {} : { baseDir: options.workspaceBaseDir }),
    ...(options.signal === undefined ? {} : { signal: options.signal }),
  });
  options.onWorkspaceCreated?.(workspace.path);
  let failure: RepositoryIngestError | undefined;
  let result: T | undefined;
  try {
    await writeAudit(
      options.audit,
      auditEvent("repository.clone.started", workspace.id, {
        provider: options.target.provider,
        sourceReference: options.reference,
      }),
    );
    const target =
      options.target.scheme === "file"
        ? await resolveLocalFixture(
            options.target,
            options.localFixtureRoot ??
              (options.target.localPath === null ? null : dirname(options.target.localPath)),
          )
        : options.target;
    const client = await GitClient.create(workspace, target, credential, {
      timeoutMs: options.limits.gitTimeoutMs,
      maxOutputBytes: options.limits.maxGitOutputBytes,
      ...(options.signal === undefined ? {} : { signal: options.signal }),
    });
    const gitVersion = await client.checkVersion();
    const resolved = await client.resolveReference(options.reference);
    await writeAudit(
      options.audit,
      auditEvent("repository.reference.resolved", workspace.id, {
        commitSha: resolved.sha,
      }),
    );
    await client.fetch(resolved.sha);
    const tree = parseTreeInventory(await client.listTree(resolved.sha), {
      maxFiles: options.limits.maxFiles,
      maxTotalBytes: options.limits.maxTotalBytes,
      maxDepth: options.limits.maxDepth,
    });
    await client.checkout(resolved.sha);
    await measureWorkspace(workspace, options.limits.maxWorkspaceBytes);
    const inspection = await inspectTree(workspace.repoPath, tree, {
      maxFileBytes: options.limits.maxFileBytes,
      maxInventoryFiles: options.limits.maxInventoryFiles,
      maxDependenciesPerManifest: options.limits.maxDependenciesPerManifest,
      ...(options.signal === undefined ? {} : { signal: options.signal }),
    });
    const snapshot = compactSnapshot(
      {
        schemaVersion: "1",
        commitSha: resolved.sha,
        sourceReference: options.reference,
        provider: options.target.provider,
        detectedLanguages: inspection.detectedLanguages,
        frameworks: inspection.frameworks,
        analysisMetadata: {
          schemaVersion: "1",
          generatorVersion: SERVICE_VERSION,
          gitVersion,
          fileCount: tree.fileCount,
          totalBytes: tree.totalBytes,
          inventoryFileCount: 0,
          textFileCount: inspection.textFileCount,
          binaryFileCount: inspection.binaryFileCount,
          oversizedFileCount: inspection.oversizedFileCount,
          generatedExcludedCount: inspection.generatedExcludedCount,
          symlinkCount: inspection.symlinkCount,
          submoduleCount: inspection.submoduleCount,
          manifestCount: inspection.manifestInventory.manifests.length,
          lockfileCount: inspection.manifestInventory.lockfiles.length,
          filesTruncated: inspection.filesTruncated,
          manifestInventoryTruncated: inspection.manifestInventory.truncated,
          limits: snapshotLimits(options.limits),
          untrustedContent: true,
          executedRepositoryCode: false,
        },
        untrustedContent: true,
      },
      inspection.files,
      tree,
      inspection.manifestInventory,
    );
    await writeAudit(
      options.audit,
      auditEvent("repository.snapshot.ready", workspace.id, {
        commitSha: snapshot.commitSha,
        fileCount: tree.fileCount,
        totalBytes: tree.totalBytes,
      }),
    );
    result = await operation({
      snapshot,
      workspacePath: workspace.repoPath,
      readFile: async (path, readOptions) =>
        await readRepositoryFile(workspace.repoPath, path, {
          maxBytes: readOptions?.maxBytes ?? options.limits.maxFileBytes,
          ...(options.signal === undefined ? {} : { signal: options.signal }),
        }),
    });
  } catch (error) {
    failure = toRepositoryIngestError(error, "internal_error", "Repository ingestion failed");
    try {
      await writeAudit(
        options.audit,
        auditEvent("repository.ingest.failed", workspace.id, {
          errorCode: failure.code,
        }),
      );
    } catch {
      // Preserve the ingestion failure; the audit sink can be retried by callers.
    }
  }

  try {
    await workspace.cleanup();
    try {
      await writeAudit(options.audit, auditEvent("repository.workspace.cleaned", workspace.id));
    } catch {
      if (failure === undefined) {
        failure = new RepositoryIngestError(
          "audit_failed",
          "Repository cleanup audit event could not be recorded",
        );
      }
    }
  } catch (cleanupError) {
    try {
      await writeAudit(
        options.audit,
        auditEvent("repository.workspace.cleanup_failed", workspace.id),
      );
    } catch {
      // Preserve the cleanup failure below.
    }
    if (failure === undefined) {
      failure = toRepositoryIngestError(
        cleanupError,
        "workspace_cleanup_failed",
        "Temporary repository workspace could not be removed",
      );
    }
  }

  if (failure !== undefined) throw failure;
  return result as T;
}

export async function ingestRepository(
  options: RepositoryIngestOptions,
): Promise<RepositorySnapshot> {
  return await withRepositorySnapshot(options, async (session) => session.snapshot);
}
