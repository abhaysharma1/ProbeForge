import { randomUUID } from "node:crypto";
import { chmod, lstat, mkdir, mkdtemp, readdir, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, isAbsolute, join, relative, resolve } from "node:path";
import { RepositoryIngestError } from "./errors";

export interface TemporaryWorkspace {
  readonly id: string;
  readonly path: string;
  readonly repoPath: string;
  readonly gitConfigPath: string;
  readonly gitHooksPath: string;
  readonly tempPath: string;
  cleanup(): Promise<void>;
}

export interface TemporaryWorkspaceOptions {
  readonly baseDir?: string;
  readonly signal?: AbortSignal;
}

async function forceRemove(path: string): Promise<void> {
  try {
    await rm(path, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
    return;
  } catch {
    // Git can leave read-only files on Windows. Clear read-only bits and retry.
  }
  await makeTreeWritable(path);
  await rm(path, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
}

async function makeTreeWritable(path: string): Promise<void> {
  let info: Awaited<ReturnType<typeof lstat>>;
  try {
    info = await lstat(path);
  } catch {
    return;
  }
  if (info.isSymbolicLink()) return;
  try {
    await chmod(path, info.isDirectory() ? 0o700 : 0o600);
  } catch {
    // Best effort; the retry below may still succeed.
  }
  if (!info.isDirectory()) return;
  try {
    const entries = await readdir(path, { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isSymbolicLink()) {
        await makeTreeWritable(join(path, entry.name));
      }
    }
  } catch {
    return;
  }
}

export async function createTemporaryWorkspace(
  options: TemporaryWorkspaceOptions = {},
): Promise<TemporaryWorkspace> {
  if (options.signal?.aborted) {
    throw new RepositoryIngestError("cancelled", "Repository ingestion was cancelled");
  }
  const requestedBase = options.baseDir ?? tmpdir();
  if (!isAbsolute(requestedBase)) {
    throw new RepositoryIngestError("internal_error", "Workspace base directory must be absolute");
  }
  await mkdir(requestedBase, { recursive: true, mode: 0o700 });
  const base = await realpath(requestedBase);
  const path = await mkdtemp(join(base, "probeforge-ingest-"));
  const realPath = await realpath(path);
  const rel = relative(base, realPath);
  if (rel.startsWith("..") || isAbsolute(rel)) {
    await forceRemove(path);
    throw new RepositoryIngestError(
      "unsafe_path",
      "Workspace escaped the configured base directory",
    );
  }

  const repoPath = join(realPath, "repo");
  const gitConfigPath = join(realPath, "git-config");
  const gitHooksPath = join(realPath, "git-hooks");
  const tempPath = join(realPath, "tmp");
  await Promise.all([
    mkdir(repoPath, { mode: 0o700 }),
    mkdir(gitConfigPath, { mode: 0o700 }),
    mkdir(gitHooksPath, { mode: 0o700 }),
    mkdir(tempPath, { mode: 0o700 }),
  ]);

  let cleaned = false;
  return {
    id: randomUUID(),
    path: realPath,
    repoPath,
    gitConfigPath,
    gitHooksPath,
    tempPath,
    cleanup: async (): Promise<void> => {
      if (cleaned) return;
      cleaned = true;
      try {
        await forceRemove(realPath);
      } catch (error) {
        throw new RepositoryIngestError(
          "workspace_cleanup_failed",
          "Temporary repository workspace could not be removed",
          { cause: error },
        );
      }
    },
  };
}

/**
 * Creates a unique workspace and guarantees cleanup on success, failure, and
 * cancellation. A cleanup failure is surfaced only when the operation succeeded,
 * so it never hides the original ingestion error.
 */
export async function withTemporaryWorkspace<T>(
  options: TemporaryWorkspaceOptions,
  operation: (workspace: TemporaryWorkspace) => Promise<T>,
): Promise<T> {
  const workspace = await createTemporaryWorkspace(options);
  let operationError: unknown;
  let result: T | undefined;
  try {
    result = await operation(workspace);
  } catch (error) {
    operationError = error;
  }
  try {
    await workspace.cleanup();
  } catch (cleanupError) {
    if (operationError === undefined) throw cleanupError;
  }
  if (operationError !== undefined) throw operationError;
  return result as T;
}

export function workspaceName(path: string): string {
  return basename(resolve(path));
}
