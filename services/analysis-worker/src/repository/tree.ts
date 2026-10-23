import { RepositoryIngestError } from "./errors";
import { containsControlCharacter } from "./text";

export type TreeEntryKind = "blob" | "submodule" | "symlink";

export interface TreeEntry {
  readonly path: string;
  readonly mode: string;
  readonly kind: TreeEntryKind;
  readonly sha: string;
  readonly sizeBytes: number;
  readonly depth: number;
}

export interface TreeLimits {
  readonly maxFiles: number;
  readonly maxTotalBytes: number;
  readonly maxDepth: number;
}

export interface TreeInventory {
  readonly entries: readonly TreeEntry[];
  readonly symlinks: readonly string[];
  readonly submodules: readonly string[];
  readonly totalBytes: number;
  readonly fileCount: number;
}

const MAX_PATH_LENGTH = 4_096;
const WINDOWS_RESERVED_NAME = /^(?:con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\..*)?$/i;

export function assertSafeRepositoryPath(path: string): void {
  if (path.length === 0 || path.length > MAX_PATH_LENGTH) {
    throw new RepositoryIngestError("unsafe_path", "Repository path is empty or too long");
  }
  if (
    path.startsWith("/") ||
    /^[A-Za-z]:/.test(path) ||
    path.includes("\\") ||
    path.includes(":") ||
    containsControlCharacter(path)
  ) {
    throw new RepositoryIngestError(
      "unsafe_path",
      "Repository path is absolute or contains unsafe characters",
    );
  }
  const segments = path.split("/");
  for (const segment of segments) {
    if (segment.length === 0 || segment === "." || segment === "..") {
      throw new RepositoryIngestError("unsafe_path", "Repository path contains an unsafe segment");
    }
    if (segment.toLowerCase() === ".git") {
      throw new RepositoryIngestError(
        "unsafe_path",
        "Repository path contains a reserved .git segment",
      );
    }
    if (segment.endsWith(".") || segment.endsWith(" ") || WINDOWS_RESERVED_NAME.test(segment)) {
      throw new RepositoryIngestError(
        "unsafe_path",
        "Repository path uses a reserved Windows filename",
      );
    }
  }
}

function parseEntry(record: string): TreeEntry {
  const tab = record.indexOf("\t");
  if (tab <= 0) {
    throw new RepositoryIngestError("unsupported_tree_entry", "Git tree entry is malformed");
  }
  const header = record.slice(0, tab);
  const path = record.slice(tab + 1);
  assertSafeRepositoryPath(path);
  const fields = header.split(/\s+/).filter((field) => field.length > 0);
  if (fields.length !== 4) {
    throw new RepositoryIngestError("unsupported_tree_entry", "Git tree entry is malformed");
  }
  const [mode, type, sha, rawSize] = fields;
  if (mode === undefined || type === undefined || sha === undefined || rawSize === undefined) {
    throw new RepositoryIngestError("unsupported_tree_entry", "Git tree entry is malformed");
  }
  if (!/^[0-9a-f]{40}(?:[0-9a-f]{24})?$/i.test(sha)) {
    throw new RepositoryIngestError(
      "unsupported_tree_entry",
      "Git tree entry has an invalid object ID",
    );
  }

  let kind: TreeEntryKind;
  if (mode === "120000") {
    kind = "symlink";
  } else if (type === "commit" && mode === "160000") {
    kind = "submodule";
  } else if (type === "blob" && /^100[0-7]{3}$/.test(mode)) {
    kind = "blob";
  } else {
    throw new RepositoryIngestError(
      "unsupported_tree_entry",
      "Git tree entry uses an unsupported type or mode",
    );
  }

  let sizeBytes = 0;
  if (kind !== "submodule") {
    if (!/^\d+$/.test(rawSize)) {
      throw new RepositoryIngestError(
        "unsupported_tree_entry",
        "Git tree entry has an invalid size",
      );
    }
    sizeBytes = Number.parseInt(rawSize, 10);
    if (!Number.isSafeInteger(sizeBytes) || sizeBytes < 0) {
      throw new RepositoryIngestError(
        "unsupported_tree_entry",
        "Git tree entry has an invalid size",
      );
    }
  }

  return {
    path,
    mode,
    kind,
    sha: sha.toLowerCase(),
    sizeBytes,
    depth: path.split("/").length,
  };
}

/**
 * Parses `git ls-tree -r -l -z`. Limits are enforced from tree metadata before
 * checkout, so oversized file counts and blob totals are rejected without
 * reading repository content.
 */
export function parseTreeInventory(output: string, limits: TreeLimits): TreeInventory {
  const entries: TreeEntry[] = [];
  const symlinks: string[] = [];
  const submodules: string[] = [];
  let totalBytes = 0;
  let fileCount = 0;

  for (const record of output.split("\0")) {
    if (record.length === 0) continue;
    const entry = parseEntry(record);
    entries.push(entry);
    if (entry.depth > limits.maxDepth) {
      throw new RepositoryIngestError(
        "repository_too_deep",
        `Repository path exceeds the ${limits.maxDepth} segment depth limit`,
      );
    }
    if (entry.kind === "submodule") {
      submodules.push(entry.path);
      continue;
    }
    fileCount += 1;
    totalBytes += entry.sizeBytes;
    if (fileCount > limits.maxFiles) {
      throw new RepositoryIngestError(
        "too_many_files",
        `Repository exceeds the ${limits.maxFiles} file limit`,
      );
    }
    if (totalBytes > limits.maxTotalBytes) {
      throw new RepositoryIngestError(
        "repository_too_large",
        `Repository exceeds the ${limits.maxTotalBytes} byte limit`,
      );
    }
    if (entry.kind === "symlink") symlinks.push(entry.path);
  }

  // A tree that places an entry beneath a symlink or gitlink can make checkout
  // write outside the intended path on platforms that honor symlinks. Reject the
  // whole tree rather than relying solely on Git's checkout protections.
  for (const specialPath of [...symlinks, ...submodules]) {
    const prefix = `${specialPath}/`;
    if (entries.some((entry) => entry.path.startsWith(prefix))) {
      throw new RepositoryIngestError(
        "unsafe_path",
        "Repository tree places content beneath a symlink or submodule",
      );
    }
  }

  return { entries, symlinks, submodules, totalBytes, fileCount };
}
