import type { DiscoveryLimits } from "./model";

export const DEFAULT_DISCOVERY_LIMITS: DiscoveryLimits = {
  maxSourceFiles: 250,
  maxSourceBytes: 8 * 1024 * 1024,
  maxFileBytes: 512 * 1024,
  maxLinesPerFile: 10_000,
  maxRoutes: 500,
  maxSurfaces: 1_000,
  maxFactsPerFile: 100,
  maxDependencies: 500,
  maxUnsupported: 200,
  maxAmbiguities: 200,
};

export function validateDiscoveryLimits(limits: DiscoveryLimits): void {
  const entries: Array<[keyof DiscoveryLimits, number]> = [
    ["maxSourceFiles", limits.maxSourceFiles],
    ["maxSourceBytes", limits.maxSourceBytes],
    ["maxFileBytes", limits.maxFileBytes],
    ["maxLinesPerFile", limits.maxLinesPerFile],
    ["maxRoutes", limits.maxRoutes],
    ["maxSurfaces", limits.maxSurfaces],
    ["maxFactsPerFile", limits.maxFactsPerFile],
    ["maxDependencies", limits.maxDependencies],
    ["maxUnsupported", limits.maxUnsupported],
    ["maxAmbiguities", limits.maxAmbiguities],
  ];
  for (const [name, value] of entries) {
    if (!Number.isSafeInteger(value) || value < 1) {
      throw new Error(`${name} must be an integer of at least 1`);
    }
  }
  if (limits.maxFileBytes > limits.maxSourceBytes) {
    throw new Error("maxFileBytes must not exceed maxSourceBytes");
  }
}
