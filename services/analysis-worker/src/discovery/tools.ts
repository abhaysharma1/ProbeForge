import type {
  ApplicationInventory,
  DiscoverySummary,
  HttpMethod,
  RouteFact,
  SurfaceFact,
  SurfaceKind,
  UnsupportedSurface,
} from "./model";

export class DiscoveryToolError extends Error {
  readonly code: "invalid_query" | "not_found";

  constructor(code: DiscoveryToolError["code"], message: string) {
    super(message);
    this.name = "DiscoveryToolError";
    this.code = code;
  }
}

export interface RouteQuery {
  readonly method?: HttpMethod;
  readonly pathPrefix?: string;
  readonly framework?: string;
  readonly limit?: number;
}

export interface SurfaceQuery {
  readonly kind?: SurfaceKind;
  readonly limit?: number;
}

export interface RouteQueryResult {
  readonly routes: readonly RouteFact[];
  readonly total: number;
  readonly truncated: boolean;
}

export interface SurfaceQueryResult {
  readonly surfaces: readonly SurfaceFact[];
  readonly total: number;
  readonly truncated: boolean;
}

export interface UnsupportedQueryResult {
  readonly unsupported: readonly UnsupportedSurface[];
  readonly total: number;
  readonly truncated: boolean;
}

export interface DiscoveryToolset {
  readonly getSummary: () => DiscoverySummary;
  readonly listRoutes: (query?: RouteQuery) => RouteQueryResult;
  readonly getRoute: (id: string) => RouteFact | null;
  readonly listSurfaces: (query?: SurfaceQuery) => SurfaceQueryResult;
  readonly listUnsupported: (limit?: number) => UnsupportedQueryResult;
}

export const DISCOVERY_TOOL_NAMES = Object.freeze({
  summary: "discovery_get_summary",
  routes: "discovery_list_routes",
  route: "discovery_get_route",
  surfaces: "discovery_list_surfaces",
  unsupported: "discovery_list_unsupported",
} as const);

const MAX_LIMIT = 100;
const DEFAULT_LIMIT = 50;
const HTTP_METHODS = new Set<HttpMethod>([
  "ANY",
  "CONNECT",
  "DELETE",
  "GET",
  "HEAD",
  "OPTIONS",
  "PATCH",
  "POST",
  "PUT",
  "TRACE",
  "UNKNOWN",
]);
const SURFACE_KINDS = new Set<SurfaceKind>([
  "auth_check",
  "configuration",
  "dependency",
  "file_upload",
  "middleware",
  "outbound_request",
  "schema_validator",
]);

function boundedLimit(value: number | undefined): number {
  if (value === undefined) return DEFAULT_LIMIT;
  if (!Number.isSafeInteger(value) || value < 1 || value > MAX_LIMIT) {
    throw new DiscoveryToolError(
      "invalid_query",
      `limit must be an integer from 1 to ${MAX_LIMIT}`,
    );
  }
  return value;
}

function boundedText(value: string, maxLength: number, label: string): string {
  if (value.length === 0 || value.length > maxLength) {
    throw new DiscoveryToolError("invalid_query", `${label} is outside its allowed length`);
  }
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint !== undefined && (codePoint < 0x20 || codePoint === 0x7f)) {
      throw new DiscoveryToolError("invalid_query", `${label} contains control characters`);
    }
  }
  return value;
}

function routeMatches(route: RouteFact, query: RouteQuery): boolean {
  if (query.method !== undefined && route.method !== query.method) return false;
  if (query.framework !== undefined && route.framework !== query.framework) return false;
  if (query.pathPrefix !== undefined) {
    if (route.path === null || !route.path.startsWith(query.pathPrefix)) return false;
  }
  return true;
}

/**
 * Builds a small, read-only query surface over a completed inventory. The
 * methods never read repository files, run parsers, or accept free-form source
 * expressions; they only filter the bounded inventory with validated limits.
 *
 * Phase 7 can adapt these methods to Pi tool definitions. The return values are
 * intentionally plain JSON-compatible data and every list result reports
 * whether it was truncated.
 */
export function createDiscoveryToolset(inventory: ApplicationInventory): DiscoveryToolset {
  return {
    getSummary: () => summary(inventory),
    listRoutes: (query = {}) => {
      if (query.method !== undefined && !HTTP_METHODS.has(query.method)) {
        throw new DiscoveryToolError("invalid_query", "method is not a supported HTTP method");
      }
      if (query.pathPrefix !== undefined) {
        boundedText(query.pathPrefix, 200, "pathPrefix");
      }
      if (query.framework !== undefined) {
        boundedText(query.framework, 100, "framework");
      }
      const limit = boundedLimit(query.limit);
      const matches = inventory.routes.filter((route) => routeMatches(route, query));
      return {
        routes: matches.slice(0, limit),
        total: matches.length,
        truncated: matches.length > limit,
      };
    },
    getRoute: (id) => {
      boundedText(id, 100, "id");
      return inventory.routes.find((route) => route.id === id) ?? null;
    },
    listSurfaces: (query = {}) => {
      if (query.kind !== undefined && !SURFACE_KINDS.has(query.kind)) {
        throw new DiscoveryToolError("invalid_query", "kind is not a supported surface kind");
      }
      const limit = boundedLimit(query.limit);
      const matches =
        query.kind === undefined
          ? inventory.surfaces
          : inventory.surfaces.filter((surface) => surface.kind === query.kind);
      return {
        surfaces: matches.slice(0, limit),
        total: matches.length,
        truncated: matches.length > limit,
      };
    },
    listUnsupported: (limitValue) => {
      const limit = boundedLimit(limitValue);
      return {
        unsupported: inventory.unsupported.slice(0, limit),
        total: inventory.unsupported.length,
        truncated: inventory.unsupported.length > limit,
      };
    },
  };
}

function summary(inventory: ApplicationInventory): DiscoverySummary {
  const surfaceCounts: Record<SurfaceKind, number> = {
    auth_check: 0,
    configuration: 0,
    dependency: 0,
    file_upload: 0,
    middleware: 0,
    outbound_request: 0,
    schema_validator: 0,
  };
  for (const surface of inventory.surfaces) surfaceCounts[surface.kind] += 1;
  return {
    schemaVersion: "1",
    commitSha: inventory.commitSha,
    frameworks: inventory.frameworks.map((framework) => framework.name),
    supportedFrameworks: inventory.frameworks
      .filter((framework) => framework.supported)
      .map((framework) => framework.name),
    routeCount: inventory.routes.length,
    ambiguousRouteCount: inventory.routes.filter((route) => route.ambiguous).length,
    surfaceCounts,
    unsupportedCount: inventory.unsupported.length,
    ambiguityCount: inventory.ambiguities.length,
    coverage: inventory.coverage,
  };
}
