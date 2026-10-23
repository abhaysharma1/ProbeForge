export interface IngestionLimits {
  readonly maxFiles: number;
  readonly maxTotalBytes: number;
  readonly maxFileBytes: number;
  readonly maxDepth: number;
  readonly maxWorkspaceBytes: number;
  readonly maxGitOutputBytes: number;
  readonly gitTimeoutMs: number;
  readonly maxInventoryFiles: number;
  readonly maxDependenciesPerManifest: number;
}

export const DEFAULT_INGESTION_LIMITS: IngestionLimits = {
  maxFiles: 5_000,
  maxTotalBytes: 64 * 1024 * 1024,
  maxFileBytes: 2 * 1024 * 1024,
  maxDepth: 32,
  maxWorkspaceBytes: 128 * 1024 * 1024,
  maxGitOutputBytes: 8 * 1024 * 1024,
  gitTimeoutMs: 60_000,
  maxInventoryFiles: 500,
  maxDependenciesPerManifest: 200,
};

export function validateIngestionLimits(limits: IngestionLimits): void {
  const entries: Array<[keyof IngestionLimits, number, number]> = [
    ["maxFiles", limits.maxFiles, 1],
    ["maxTotalBytes", limits.maxTotalBytes, 1],
    ["maxFileBytes", limits.maxFileBytes, 1],
    ["maxDepth", limits.maxDepth, 1],
    ["maxWorkspaceBytes", limits.maxWorkspaceBytes, 1],
    ["maxGitOutputBytes", limits.maxGitOutputBytes, 1],
    ["gitTimeoutMs", limits.gitTimeoutMs, 1],
    ["maxInventoryFiles", limits.maxInventoryFiles, 1],
    ["maxDependenciesPerManifest", limits.maxDependenciesPerManifest, 1],
  ];
  for (const [name, value, minimum] of entries) {
    if (!Number.isSafeInteger(value) || value < minimum) {
      throw new Error(`${name} must be an integer of at least ${minimum}`);
    }
  }
  if (limits.maxFileBytes > limits.maxTotalBytes) {
    throw new Error("maxFileBytes must not exceed maxTotalBytes");
  }
}
