export type DiscoveryConfidence = "high" | "medium" | "low";

export type DiscoveryProvenance = "observed" | "inferred";

export type HttpMethod =
  | "ANY"
  | "CONNECT"
  | "DELETE"
  | "GET"
  | "HEAD"
  | "OPTIONS"
  | "PATCH"
  | "POST"
  | "PUT"
  | "TRACE"
  | "UNKNOWN";

export type RoutePathKind = "dynamic" | "file-derived" | "literal" | "parameterized";

export interface SourceLocation {
  readonly file: string;
  readonly startLine: number;
  readonly endLine: number;
}

export interface FrameworkFact {
  readonly id: string;
  readonly name: string;
  readonly ecosystem: string;
  readonly supported: boolean;
  readonly provenance: DiscoveryProvenance;
  readonly confidence: DiscoveryConfidence;
  readonly evidence: readonly SourceLocation[];
  readonly notes: readonly string[];
}

export interface RouteFact {
  readonly id: string;
  readonly framework: string;
  readonly method: HttpMethod;
  readonly path: string | null;
  readonly pathKind: RoutePathKind;
  readonly handler: string | null;
  readonly provenance: DiscoveryProvenance;
  readonly confidence: DiscoveryConfidence;
  readonly ambiguous: boolean;
  readonly notes: readonly string[];
  readonly evidence: SourceLocation;
}

export type SurfaceKind =
  | "auth_check"
  | "configuration"
  | "dependency"
  | "file_upload"
  | "middleware"
  | "outbound_request"
  | "schema_validator";

export interface SurfaceFact {
  readonly id: string;
  readonly kind: SurfaceKind;
  readonly framework: string | null;
  readonly name: string;
  readonly detail: string | null;
  readonly provenance: DiscoveryProvenance;
  readonly confidence: DiscoveryConfidence;
  readonly evidence: SourceLocation;
}

export type UnsupportedKind = "file" | "framework" | "language" | "parser";

export interface UnsupportedSurface {
  readonly id: string;
  readonly kind: UnsupportedKind;
  readonly name: string;
  readonly reason: string;
  readonly provenance: DiscoveryProvenance;
  readonly confidence: DiscoveryConfidence;
  readonly evidence: readonly SourceLocation[];
}

export interface AmbiguityFact {
  readonly id: string;
  readonly kind: "coverage" | "framework" | "route";
  readonly message: string;
  readonly provenance: DiscoveryProvenance;
  readonly confidence: DiscoveryConfidence;
  readonly evidence: readonly SourceLocation[];
}

export interface DiscoveryLimits {
  readonly maxSourceFiles: number;
  readonly maxSourceBytes: number;
  readonly maxFileBytes: number;
  readonly maxLinesPerFile: number;
  readonly maxRoutes: number;
  readonly maxSurfaces: number;
  readonly maxFactsPerFile: number;
  readonly maxDependencies: number;
  readonly maxUnsupported: number;
  readonly maxAmbiguities: number;
}

export interface DiscoveryCoverage {
  readonly examinedFiles: number;
  readonly skippedFiles: number;
  readonly examinedBytes: number;
  readonly sourceFilesTruncated: boolean;
  readonly sourceBytesTruncated: boolean;
  readonly lineTruncatedFiles: number;
  readonly routesTruncated: boolean;
  readonly surfacesTruncated: boolean;
  readonly unsupportedTruncated: boolean;
  readonly ambiguitiesTruncated: boolean;
  readonly unsupportedLanguages: readonly string[];
  readonly unsupportedFrameworks: readonly string[];
  readonly parsers: readonly string[];
}

export interface ApplicationInventory {
  readonly schemaVersion: "1";
  readonly generatorVersion: "1";
  readonly commitSha: string;
  readonly frameworks: readonly FrameworkFact[];
  readonly routes: readonly RouteFact[];
  readonly surfaces: readonly SurfaceFact[];
  readonly unsupported: readonly UnsupportedSurface[];
  readonly ambiguities: readonly AmbiguityFact[];
  readonly coverage: DiscoveryCoverage;
  readonly limits: DiscoveryLimits;
  readonly untrustedContent: true;
  readonly executedRepositoryCode: false;
}

export interface DiscoverySummary {
  readonly schemaVersion: "1";
  readonly commitSha: string;
  readonly frameworks: readonly string[];
  readonly supportedFrameworks: readonly string[];
  readonly routeCount: number;
  readonly ambiguousRouteCount: number;
  readonly surfaceCounts: Readonly<Record<SurfaceKind, number>>;
  readonly unsupportedCount: number;
  readonly ambiguityCount: number;
  readonly coverage: DiscoveryCoverage;
}
