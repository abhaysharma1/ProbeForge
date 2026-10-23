import { isRepositoryIngestError, RepositoryIngestError } from "../repository/errors";
import type { RepositorySession, RepositorySnapshot } from "../repository/ingest";
import type { ManifestInventoryItem, ReadRepositoryFileResult } from "../repository/inspect";
import { DEFAULT_DISCOVERY_LIMITS, validateDiscoveryLimits } from "./limits";
import type {
  AmbiguityFact,
  ApplicationInventory,
  DiscoveryLimits,
  FrameworkFact,
  RouteFact,
  SourceLocation,
  SurfaceFact,
  UnsupportedSurface,
} from "./model";
import { extractCSharpRoutes } from "./parsers/csharp";
import { extractGoRoutes } from "./parsers/go";
import { extractJavaRoutes } from "./parsers/java";
import { extractJavaScriptRoutes } from "./parsers/javascript";
import { extractPhpRoutes } from "./parsers/php";
import { extractPythonRoutes } from "./parsers/python";
import { extractRubyRoutes } from "./parsers/ruby";
import { extractRustRoutes } from "./parsers/rust";
import {
  buildSourceDocument,
  makeAmbiguity,
  makeFrameworkFact,
  makeSurface,
  makeUnsupported,
  type SourceDocument,
  sourceLocation,
} from "./shared";
import { extractSurfaces } from "./surfaces";

export interface DiscoverySource {
  readonly snapshot: RepositorySnapshot;
  readFile(
    path: string,
    options?: { readonly maxBytes?: number },
  ): Promise<ReadRepositoryFileResult>;
}

export interface DiscoveryOptions {
  readonly limits?: Partial<DiscoveryLimits>;
  readonly signal?: AbortSignal;
}

const SOURCE_LANGUAGES = new Set([
  "C#",
  "Go",
  "Java",
  "JavaScript",
  "PHP",
  "Python",
  "Ruby",
  "Rust",
  "TypeScript",
]);

const IGNORED_LANGUAGES = new Set([
  "CSS",
  "Docker",
  "HTML",
  "JSON",
  "Make",
  "Markdown",
  "SQL",
  "Shell",
  "TOML",
  "Unknown",
  "XML",
  "YAML",
]);

interface FrameworkMeta {
  readonly ecosystem: string;
  readonly supported: boolean;
  readonly dependencies: readonly string[];
  readonly markers: readonly RegExp[];
  readonly reason?: string;
}

const FRAMEWORK_META: Readonly<Record<string, FrameworkMeta>> = {
  "Actix Web": {
    ecosystem: "rust",
    supported: true,
    dependencies: ["actix-web"],
    markers: [/\bactix_web\b/],
  },
  Angular: {
    ecosystem: "node",
    supported: false,
    dependencies: ["@angular/core"],
    markers: [],
    reason: "Angular UI routes are not modeled as server HTTP routes in Phase 6.",
  },
  "ASP.NET Core": {
    ecosystem: "dotnet",
    supported: true,
    dependencies: ["microsoft.aspnetcore.openapi", "microsoft.aspnetcore.app"],
    markers: [/\.csproj$/i, /\bMicrosoft\.AspNetCore\b/],
  },
  Astro: {
    ecosystem: "node",
    supported: false,
    dependencies: ["astro"],
    markers: [],
    reason: "Astro page routes are not modeled as server HTTP routes in Phase 6.",
  },
  Axum: {
    ecosystem: "rust",
    supported: true,
    dependencies: ["axum"],
    markers: [/\baxum\b/],
  },
  Django: {
    ecosystem: "python",
    supported: true,
    dependencies: ["django"],
    markers: [/manage\.py$/i, /(^|\/)urls\.py$/i],
  },
  Echo: {
    ecosystem: "go",
    supported: true,
    dependencies: ["github.com/labstack/echo/v4"],
    markers: [/\becho\./],
  },
  Electron: {
    ecosystem: "node",
    supported: false,
    dependencies: ["electron"],
    markers: [],
    reason: "Electron is a desktop shell; no server route parser is available.",
  },
  Express: {
    ecosystem: "node",
    supported: true,
    dependencies: ["express"],
    markers: [/\bexpress\s*\(/],
  },
  FastAPI: {
    ecosystem: "python",
    supported: true,
    dependencies: ["fastapi"],
    markers: [/\bFastAPI\s*\(/, /\bAPIRouter\s*\(/],
  },
  Fastify: {
    ecosystem: "node",
    supported: true,
    dependencies: ["fastify"],
    markers: [/\bFastify\s*\(/],
  },
  Fiber: {
    ecosystem: "go",
    supported: true,
    dependencies: ["github.com/gofiber/fiber/v2"],
    markers: [/\bfiber\./],
  },
  Flask: {
    ecosystem: "python",
    supported: true,
    dependencies: ["flask"],
    markers: [/\bFlask\s*\(/, /\bBlueprint\s*\(/],
  },
  Gin: {
    ecosystem: "go",
    supported: true,
    dependencies: ["github.com/gin-gonic/gin"],
    markers: [/\bgin\./],
  },
  "Go net/http": {
    ecosystem: "go",
    supported: true,
    dependencies: [],
    markers: [/\bnet\/http\b/, /\bhttp\.Handle(?:Func)?\s*\(/],
  },
  Hono: {
    ecosystem: "node",
    supported: true,
    dependencies: ["hono", "@hono/node-server"],
    markers: [/\bHono\s*\(/],
  },
  "JAX-RS": {
    ecosystem: "java",
    supported: false,
    dependencies: ["jakarta.ws.rs-api"],
    markers: [/\bjakarta\.ws\.rs\b|\bjavax\.ws\.rs\b|@Path\s*\(/],
    reason: "JAX-RS annotations are recognized but not expanded into routes in Phase 6.",
  },
  Koa: {
    ecosystem: "node",
    supported: true,
    dependencies: ["koa", "koa-router", "@koa/router"],
    markers: [/\bKoa\s*\(/, /\bkoaRouter\b/],
  },
  Laravel: {
    ecosystem: "php",
    supported: true,
    dependencies: ["laravel/framework"],
    markers: [/artisan$/i, /(^|\/)routes\/web\.php$/i, /\bRoute::/],
  },
  NestJS: {
    ecosystem: "node",
    supported: true,
    dependencies: ["@nestjs/core"],
    markers: [/@Controller\s*\(/],
  },
  "Next.js": {
    ecosystem: "node",
    supported: true,
    dependencies: ["next"],
    markers: [/next\.config\.(?:js|mjs|ts)$/i, /(^|\/)app\/.+\/route\.(?:js|jsx|ts|tsx)$/i],
  },
  Nuxt: {
    ecosystem: "node",
    supported: false,
    dependencies: ["nuxt"],
    markers: [],
    reason: "Nuxt page/server route conventions are not modeled in Phase 6.",
  },
  React: {
    ecosystem: "node",
    supported: false,
    dependencies: ["react", "react-dom"],
    markers: [],
    reason:
      "React component trees are not server HTTP routes; no browser route parser is included.",
  },
  Remix: {
    ecosystem: "node",
    supported: false,
    dependencies: ["@remix-run/react"],
    markers: [],
    reason: "Remix route conventions are not modeled in Phase 6.",
  },
  Rocket: {
    ecosystem: "rust",
    supported: true,
    dependencies: ["rocket"],
    markers: [/\brocket\b/],
  },
  "Ruby on Rails": {
    ecosystem: "ruby",
    supported: true,
    dependencies: ["rails"],
    markers: [/(^|\/)config\/routes\.rb$/i, /\bRails\.application\.routes\.draw\b/],
  },
  Sinatra: {
    ecosystem: "ruby",
    supported: true,
    dependencies: ["sinatra"],
    markers: [/\bSinatra\b/],
  },
  "Spring Boot": {
    ecosystem: "java",
    supported: true,
    dependencies: ["spring-boot-starter-web"],
    markers: [/@(?:RestController|Controller|RequestMapping|GetMapping)\b/],
  },
  Starlette: {
    ecosystem: "python",
    supported: true,
    dependencies: ["starlette"],
    markers: [/\bStarlette\s*\(/, /\bRouter\s*\(/],
  },
  Svelte: {
    ecosystem: "node",
    supported: false,
    dependencies: ["svelte"],
    markers: [],
    reason: "Svelte component routes are not server HTTP routes; no route parser is included.",
  },
  Symfony: {
    ecosystem: "php",
    supported: true,
    dependencies: ["symfony/framework-bundle"],
    markers: [/symfony\.lock$/i, /#\[Route\s*\(/],
  },
  Tonic: {
    ecosystem: "rust",
    supported: false,
    dependencies: ["tonic"],
    markers: [],
    reason: "Tonic gRPC services are not HTTP routes in Phase 6.",
  },
  Vite: {
    ecosystem: "node",
    supported: false,
    dependencies: [],
    markers: [/vite\.config\.(?:js|mjs|ts)$/i],
    reason: "Vite is a build tool, not a server route framework.",
  },
  Vue: {
    ecosystem: "node",
    supported: false,
    dependencies: ["vue"],
    markers: [],
    reason: "Vue component routes are not server HTTP routes; no route parser is included.",
  },
  Warp: {
    ecosystem: "rust",
    supported: false,
    dependencies: ["warp"],
    markers: [],
    reason: "Warp route filters are not modeled in Phase 6.",
  },
};

const UNKNOWN_FRAMEWORK_META: FrameworkMeta = {
  ecosystem: "unknown",
  supported: false,
  dependencies: [],
  markers: [],
  reason: "No deterministic route parser is available for this framework in Phase 6.",
};

function resolveLimits(options: DiscoveryOptions): DiscoveryLimits {
  const limits: DiscoveryLimits = {
    ...DEFAULT_DISCOVERY_LIMITS,
    ...(options.limits ?? {}),
  };
  validateDiscoveryLimits(limits);
  return limits;
}

function frameworkMeta(name: string): FrameworkMeta {
  return FRAMEWORK_META[name] ?? UNKNOWN_FRAMEWORK_META;
}

function frameworkNames(snapshot: RepositorySnapshot): string[] {
  const names = new Set(snapshot.frameworks);
  const lowerPaths = snapshot.files.map((file) => file.path.toLowerCase());
  if (
    lowerPaths.some(
      (path) =>
        /(^|\/)app\/.+\/route\.(?:js|jsx|ts|tsx)$/.test(path) ||
        /(^|\/)pages\/api\/.+\.(?:js|jsx|ts|tsx)$/.test(path),
    )
  ) {
    names.add("Next.js");
  }
  if (lowerPaths.some((path) => path.endsWith("manage.py"))) names.add("Django");
  if (lowerPaths.some((path) => /(^|\/)config\/routes\.rb$/.test(path))) names.add("Ruby on Rails");
  if (lowerPaths.some((path) => /(^|\/)artisan$/.test(path))) names.add("Laravel");
  return [...names].sort((left, right) => left.localeCompare(right));
}

function frameworkEvidence(
  name: string,
  snapshot: RepositorySnapshot,
): { evidence: SourceLocation[]; confidence: "high" | "medium" } {
  const meta = frameworkMeta(name);
  const evidence: SourceLocation[] = [];
  let markerMatched = false;
  for (const file of snapshot.files) {
    if (meta.markers.some((marker) => marker.test(file.path))) {
      evidence.push(sourceLocation(file.path, 1));
      markerMatched = true;
    }
  }
  for (const manifest of snapshot.manifestInventory.manifests) {
    for (const dependency of manifest.dependencies) {
      if (meta.dependencies.includes(dependency.name.toLowerCase())) {
        evidence.push(sourceLocation(manifest.path, 1));
      }
    }
  }
  const unique = new Map<string, SourceLocation>();
  for (const location of evidence) {
    unique.set(`${location.file}:${location.startLine}`, location);
  }
  return {
    evidence: [...unique.values()].slice(0, 8),
    confidence: markerMatched ? "high" : "medium",
  };
}

function buildFrameworks(snapshot: RepositorySnapshot): {
  frameworks: FrameworkFact[];
  unsupported: UnsupportedSurface[];
} {
  const frameworks: FrameworkFact[] = [];
  const unsupported: UnsupportedSurface[] = [];
  for (const name of frameworkNames(snapshot)) {
    const meta = frameworkMeta(name);
    const evidence = frameworkEvidence(name, snapshot);
    frameworks.push(
      makeFrameworkFact({
        name,
        ecosystem: meta.ecosystem,
        supported: meta.supported,
        evidence: evidence.evidence,
        confidence: evidence.confidence,
        notes: meta.supported
          ? [routeSupportNote(name)]
          : [meta.reason ?? UNKNOWN_FRAMEWORK_META.reason ?? "Unsupported framework"],
      }),
    );
    if (!meta.supported) {
      unsupported.push(
        makeUnsupported({
          kind: "framework",
          name,
          reason: meta.reason ?? UNKNOWN_FRAMEWORK_META.reason ?? "Unsupported framework",
          evidence: evidence.evidence,
          provenance: "inferred",
          confidence: "medium",
        }),
      );
    }
  }
  return { frameworks, unsupported };
}

function routeSupportNote(name: string): string {
  return `Phase 6 includes deterministic source parsing for ${name}; coverage remains heuristic.`;
}

function fallbackFramework(language: string): string {
  switch (language) {
    case "TypeScript":
    case "JavaScript":
      return "JavaScript/TypeScript";
    case "Python":
      return "Python";
    case "Rust":
      return "Rust";
    case "Java":
      return "Java";
    case "Go":
      return "Go";
    case "Ruby":
      return "Ruby";
    case "PHP":
      return "PHP";
    case "C#":
      return "C#";
    default:
      return language;
  }
}

interface ParsedDocument {
  readonly routes: RouteFact[];
  readonly ambiguities: AmbiguityFact[];
  readonly parser: string;
  readonly framework: string;
}

function parseDocument(document: SourceDocument, snapshot: RepositorySnapshot): ParsedDocument {
  const frameworks = snapshot.frameworks;
  switch (document.language) {
    case "JavaScript":
    case "TypeScript": {
      const result = extractJavaScriptRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "Python": {
      const result = extractPythonRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "Rust": {
      const result = extractRustRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "Java": {
      const result = extractJavaRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "Go": {
      const result = extractGoRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "Ruby": {
      const result = extractRubyRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "PHP": {
      const result = extractPhpRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    case "C#": {
      const result = extractCSharpRoutes(document, frameworks);
      return {
        ...result,
        framework: result.routes[0]?.framework ?? fallbackFramework(document.language),
      };
    }
    default:
      return {
        routes: [],
        ambiguities: [],
        parser: "unsupported",
        framework: fallbackFramework(document.language),
      };
  }
}

function uniqueLocations(locations: readonly SourceLocation[]): SourceLocation[] {
  return [...new Map(locations.map((item) => [`${item.file}:${item.startLine}`, item])).values()];
}

function dependencySurfaces(
  snapshot: RepositorySnapshot,
  limits: DiscoveryLimits,
  limitReached: () => boolean,
  accept: (surface: SurfaceFact) => void,
): { truncated: boolean; created: number } {
  let created = 0;
  let truncated = false;
  const manifests: readonly ManifestInventoryItem[] = snapshot.manifestInventory.manifests;
  for (const manifest of manifests) {
    const manifestDocument: SourceDocument = {
      path: manifest.path,
      language: "manifest",
      lines: [""],
      text: "",
      lineTruncated: false,
    };
    for (const dependency of manifest.dependencies) {
      if (created >= limits.maxDependencies || limitReached()) {
        truncated = true;
        return { truncated, created };
      }
      const detailParts = [manifest.ecosystem, dependency.version ?? "version unknown"];
      accept(
        makeSurface(manifestDocument, {
          startLine: 1,
          kind: "dependency",
          framework: null,
          name: dependency.name,
          detail: detailParts.join(" "),
          provenance: "observed",
          confidence: "high",
        }),
      );
      created += 1;
    }
  }
  return { truncated, created };
}

function languageEvidence(snapshot: RepositorySnapshot, language: string): SourceLocation[] {
  const files = snapshot.files.filter((file) => file.language === language).slice(0, 5);
  return uniqueLocations(files.map((file) => sourceLocation(file.path, 1)));
}

function sortRoutes(routes: RouteFact[]): RouteFact[] {
  return routes.sort((left, right) => {
    const file = left.evidence.file.localeCompare(right.evidence.file);
    if (file !== 0) return file;
    if (left.evidence.startLine !== right.evidence.startLine) {
      return left.evidence.startLine - right.evidence.startLine;
    }
    const method = left.method.localeCompare(right.method);
    return method !== 0 ? method : (left.path ?? "").localeCompare(right.path ?? "");
  });
}

function sortSurfaces(surfaces: SurfaceFact[]): SurfaceFact[] {
  return surfaces.sort((left, right) => {
    const file = left.evidence.file.localeCompare(right.evidence.file);
    if (file !== 0) return file;
    if (left.evidence.startLine !== right.evidence.startLine) {
      return left.evidence.startLine - right.evidence.startLine;
    }
    const kind = left.kind.localeCompare(right.kind);
    return kind !== 0 ? kind : left.name.localeCompare(right.name);
  });
}

function sortUnsupported(unsupported: UnsupportedSurface[]): UnsupportedSurface[] {
  return unsupported.sort((left, right) => {
    const kind = left.kind.localeCompare(right.kind);
    if (kind !== 0) return kind;
    const name = left.name.localeCompare(right.name);
    return name !== 0 ? name : left.reason.localeCompare(right.reason);
  });
}

function sortAmbiguities(ambiguities: AmbiguityFact[]): AmbiguityFact[] {
  return ambiguities.sort((left, right) => {
    const leftFile = left.evidence[0]?.file ?? "";
    const rightFile = right.evidence[0]?.file ?? "";
    const file = leftFile.localeCompare(rightFile);
    if (file !== 0) return file;
    const leftLine = left.evidence[0]?.startLine ?? 0;
    const rightLine = right.evidence[0]?.startLine ?? 0;
    if (leftLine !== rightLine) return leftLine - rightLine;
    return left.message.localeCompare(right.message);
  });
}

/**
 * Discover a bounded, deterministic application inventory from a Phase 5
 * repository session. The function only reads repository text and runs static
 * pattern parsers. It never imports, evaluates, builds, or executes repository
 * code, and it never writes source text into the inventory.
 */
export async function discoverApplication(
  source: DiscoverySource,
  options: DiscoveryOptions = {},
): Promise<ApplicationInventory> {
  const limits = resolveLimits(options);
  const snapshot = source.snapshot;
  const { frameworks, unsupported: frameworkUnsupported } = buildFrameworks(snapshot);

  const routes: RouteFact[] = [];
  const surfaces: SurfaceFact[] = [];
  const unsupported: UnsupportedSurface[] = frameworkUnsupported;
  const ambiguities: AmbiguityFact[] = [];
  const routeIds = new Set<string>();
  const surfaceIds = new Set<string>();
  const unsupportedIds = new Set<string>();
  const ambiguityIds = new Set<string>();
  const parsers = new Set<string>();

  let routesTruncated = false;
  let surfacesTruncated = false;
  let unsupportedTruncated = false;
  let ambiguitiesTruncated = false;
  let sourceFilesTruncated = snapshot.filesTruncated;
  let sourceBytesTruncated = false;
  let examinedFiles = 0;
  let skippedFiles = 0;
  let examinedBytes = 0;
  let lineTruncatedFiles = 0;

  const addUnsupported = (item: UnsupportedSurface): void => {
    if (unsupportedIds.has(item.id)) return;
    unsupportedIds.add(item.id);
    if (unsupported.length >= limits.maxUnsupported) {
      unsupportedTruncated = true;
      return;
    }
    unsupported.push(item);
  };

  const addAmbiguity = (item: AmbiguityFact): void => {
    if (ambiguityIds.has(item.id)) return;
    ambiguityIds.add(item.id);
    if (ambiguities.length >= limits.maxAmbiguities) {
      ambiguitiesTruncated = true;
      return;
    }
    ambiguities.push(item);
  };

  const addRoute = (route: RouteFact): void => {
    if (routeIds.has(route.id)) return;
    routeIds.add(route.id);
    if (routes.length >= limits.maxRoutes) {
      routesTruncated = true;
      return;
    }
    routes.push(route);
  };

  const addSurface = (surface: SurfaceFact): void => {
    if (surfaceIds.has(surface.id)) return;
    surfaceIds.add(surface.id);
    if (surfaces.length >= limits.maxSurfaces) {
      surfacesTruncated = true;
      return;
    }
    surfaces.push(surface);
  };

  for (const language of snapshot.detectedLanguages) {
    if (SOURCE_LANGUAGES.has(language) || IGNORED_LANGUAGES.has(language)) continue;
    addUnsupported(
      makeUnsupported({
        kind: "language",
        name: language,
        reason: "No deterministic source parser is available for this language in Phase 6.",
        evidence: languageEvidence(snapshot, language),
        provenance: "inferred",
        confidence: "high",
      }),
    );
  }

  const candidates = snapshot.files
    .filter(
      (file) =>
        file.kind === "text" &&
        file.language !== null &&
        SOURCE_LANGUAGES.has(file.language) &&
        !IGNORED_LANGUAGES.has(file.language),
    )
    .sort((left, right) => left.path.localeCompare(right.path));
  if (candidates.length > limits.maxSourceFiles) sourceFilesTruncated = true;
  const selected = candidates.slice(0, limits.maxSourceFiles);

  for (const file of selected) {
    if (options.signal?.aborted) {
      throw new RepositoryIngestError("cancelled", "Application discovery was cancelled");
    }
    if (file.sizeBytes > limits.maxFileBytes) {
      skippedFiles += 1;
      addUnsupported(
        makeUnsupported({
          kind: "file",
          name: file.path,
          reason: "File exceeds the discovery read limit and was skipped.",
          evidence: [sourceLocation(file.path, 1)],
          provenance: "observed",
          confidence: "high",
        }),
      );
      continue;
    }
    if (examinedBytes + file.sizeBytes > limits.maxSourceBytes) {
      sourceBytesTruncated = true;
      break;
    }

    let read: ReadRepositoryFileResult;
    try {
      read = await source.readFile(file.path, { maxBytes: limits.maxFileBytes });
    } catch (error) {
      if (
        isRepositoryIngestError(error) &&
        (error.code === "binary_file" || error.code === "file_too_large")
      ) {
        skippedFiles += 1;
        addUnsupported(
          makeUnsupported({
            kind: "file",
            name: file.path,
            reason: "File could not be read as bounded UTF-8 text and was skipped.",
            evidence: [sourceLocation(file.path, 1)],
            provenance: "observed",
            confidence: "high",
          }),
        );
        continue;
      }
      throw error;
    }

    examinedFiles += 1;
    examinedBytes += read.sizeBytes;
    const document = buildSourceDocument(read, limits.maxLinesPerFile);
    if (document.lineTruncated) lineTruncatedFiles += 1;
    const parsed = parseDocument(document, snapshot);
    parsers.add(parsed.parser);

    let fileRoutes = parsed.routes;
    if (fileRoutes.length > limits.maxFactsPerFile) {
      fileRoutes = fileRoutes.slice(0, limits.maxFactsPerFile);
      addAmbiguity(
        makeAmbiguity({
          kind: "coverage",
          message: "Per-file route limit reached; additional routes were omitted.",
          file: document.path,
          startLine: 1,
          confidence: "medium",
        }),
      );
    }
    for (const route of fileRoutes) addRoute(route);

    const extractedSurfaces = extractSurfaces(document, parsed.framework);
    let fileSurfaces = extractedSurfaces;
    if (fileSurfaces.length > limits.maxFactsPerFile) {
      fileSurfaces = fileSurfaces.slice(0, limits.maxFactsPerFile);
      addAmbiguity(
        makeAmbiguity({
          kind: "coverage",
          message: "Per-file surface limit reached; additional surfaces were omitted.",
          file: document.path,
          startLine: 1,
          confidence: "medium",
        }),
      );
    }
    for (const surface of fileSurfaces) addSurface(surface);
    for (const ambiguity of parsed.ambiguities) addAmbiguity(ambiguity);
  }

  const knownFrameworks = new Set(frameworks.map((framework) => framework.name));
  for (const route of routes) {
    if (knownFrameworks.has(route.framework)) continue;
    const meta = FRAMEWORK_META[route.framework];
    if (meta === undefined) continue;
    knownFrameworks.add(route.framework);
    frameworks.push(
      makeFrameworkFact({
        name: route.framework,
        ecosystem: meta.ecosystem,
        supported: meta.supported,
        evidence: [route.evidence],
        confidence: "medium",
        notes: meta.supported
          ? ["Framework was inferred from a matched route parser."]
          : [meta.reason ?? UNKNOWN_FRAMEWORK_META.reason ?? "Unsupported framework"],
      }),
    );
  }

  const dependencies = dependencySurfaces(
    snapshot,
    limits,
    () => surfaces.length >= limits.maxSurfaces,
    addSurface,
  );
  if (dependencies.truncated) surfacesTruncated = true;
  if (snapshot.analysisMetadata.manifestInventoryTruncated) {
    addAmbiguity(
      makeAmbiguity({
        kind: "coverage",
        message: "Manifest inventory was truncated; dependency surfaces may be incomplete.",
        confidence: "medium",
      }),
    );
  }
  if (sourceFilesTruncated) {
    addAmbiguity(
      makeAmbiguity({
        kind: "coverage",
        message: "Source file selection was truncated; some source files were not examined.",
        confidence: "medium",
      }),
    );
  }
  if (sourceBytesTruncated) {
    addAmbiguity(
      makeAmbiguity({
        kind: "coverage",
        message: "Source byte limit was reached; later source files were not examined.",
        confidence: "medium",
      }),
    );
  }
  if (frameworks.length === 0 && routes.length > 0) {
    addAmbiguity(
      makeAmbiguity({
        kind: "framework",
        message:
          "Routes were parsed without a detected framework; framework attribution is incomplete.",
        confidence: "low",
      }),
    );
  }
  if (routesTruncated) {
    addAmbiguity(
      makeAmbiguity({
        kind: "coverage",
        message: "Route limit was reached; additional routes were omitted.",
        confidence: "medium",
      }),
    );
  }
  if (surfacesTruncated) {
    addAmbiguity(
      makeAmbiguity({
        kind: "coverage",
        message: "Surface limit was reached; additional surfaces were omitted.",
        confidence: "medium",
      }),
    );
  }

  const unsupportedLanguages = unsupported
    .filter((item) => item.kind === "language")
    .map((item) => item.name);
  const unsupportedFrameworks = unsupported
    .filter((item) => item.kind === "framework")
    .map((item) => item.name);

  return {
    schemaVersion: "1",
    generatorVersion: "1",
    commitSha: snapshot.commitSha,
    frameworks: frameworks.sort((left, right) => left.name.localeCompare(right.name)),
    routes: sortRoutes(routes),
    surfaces: sortSurfaces(surfaces),
    unsupported: sortUnsupported(unsupported),
    ambiguities: sortAmbiguities(ambiguities),
    coverage: {
      examinedFiles,
      skippedFiles,
      examinedBytes,
      sourceFilesTruncated,
      sourceBytesTruncated,
      lineTruncatedFiles,
      routesTruncated,
      surfacesTruncated,
      unsupportedTruncated,
      ambiguitiesTruncated,
      unsupportedLanguages: [...new Set(unsupportedLanguages)].sort(),
      unsupportedFrameworks: [...new Set(unsupportedFrameworks)].sort(),
      parsers: [...parsers].sort(),
    },
    limits,
    untrustedContent: true,
    executedRepositoryCode: false,
  };
}

export function discoverRepositorySession(
  session: RepositorySession,
  options: DiscoveryOptions = {},
): Promise<ApplicationInventory> {
  return discoverApplication(session, options);
}
