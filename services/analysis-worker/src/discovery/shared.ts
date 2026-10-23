import { createHash } from "node:crypto";
import type { ReadRepositoryFileResult } from "../repository/inspect";
import { stripUnsafeControls } from "../repository/text";
import type {
  AmbiguityFact,
  DiscoveryConfidence,
  DiscoveryProvenance,
  FrameworkFact,
  HttpMethod,
  RouteFact,
  RoutePathKind,
  SourceLocation,
  SurfaceFact,
  SurfaceKind,
  UnsupportedSurface,
} from "./model";

export interface SourceDocument {
  readonly path: string;
  readonly language: string;
  readonly lines: readonly string[];
  readonly text: string;
  readonly lineTruncated: boolean;
}

const MAX_LINE_LENGTH = 4_000;
const MAX_PATH_LENGTH = 300;
const MAX_IDENTIFIER_LENGTH = 120;
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

export function buildSourceDocument(
  file: ReadRepositoryFileResult,
  maxLines: number,
): SourceDocument {
  const allLines = file.text.split(/\r?\n/);
  const lineTruncated =
    allLines.length > maxLines || allLines.some((line) => line.length > MAX_LINE_LENGTH);
  const lines = (lineTruncated ? allLines.slice(0, maxLines) : allLines).map((line) =>
    line.length > MAX_LINE_LENGTH ? line.slice(0, MAX_LINE_LENGTH) : line,
  );
  return {
    path: file.path,
    language: file.language ?? "Unknown",
    lines,
    text: file.text,
    lineTruncated,
  };
}

export function factId(prefix: string, ...parts: readonly (string | number | null)[]): string {
  const digest = createHash("sha256")
    .update(JSON.stringify([prefix, ...parts]))
    .digest("hex")
    .slice(0, 16);
  return `${prefix}_${digest}`;
}

export function sourceLocation(
  file: string,
  startLine: number,
  endLine: number = startLine,
): SourceLocation {
  const start = Math.max(1, Math.trunc(startLine));
  const end = Math.max(start, Math.trunc(endLine));
  return { file, startLine: start, endLine: end };
}

export function safeText(value: string, maxLength = MAX_PATH_LENGTH): string | null {
  const cleaned = stripUnsafeControls(value)
    .replace(/[\r\n\t]+/g, " ")
    .trim();
  if (cleaned.length === 0) return null;
  return cleaned.slice(0, maxLength);
}

export function safeIdentifier(value: string): string | null {
  const cleaned = value.trim();
  if (!/^[A-Za-z_$@][A-Za-z0-9_$.:@/-]{0,199}$/.test(cleaned)) return null;
  return cleaned.slice(0, MAX_IDENTIFIER_LENGTH);
}

export function methodFromToken(token: string): HttpMethod {
  const upper = token.trim().toUpperCase();
  if (upper === "ALL" || upper === "ANY") return "ANY";
  return HTTP_METHODS.has(upper as HttpMethod) && upper !== "UNKNOWN" && upper !== "ANY"
    ? (upper as HttpMethod)
    : "UNKNOWN";
}

export function methodsFromList(raw: string): HttpMethod[] {
  const methods: HttpMethod[] = [];
  for (const match of raw.matchAll(
    /['":]?\b(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD|TRACE|CONNECT|ANY|ALL)\b/gi,
  )) {
    const token = match[1];
    if (token === undefined) continue;
    const method = methodFromToken(token);
    if (!methods.includes(method)) methods.push(method);
  }
  return methods;
}

export interface NormalizedRoutePath {
  readonly path: string;
  readonly pathKind: RoutePathKind;
  readonly ambiguous: boolean;
  readonly notes: readonly string[];
}

export function normalizeRoutePath(raw: string, fileDerived = false): NormalizedRoutePath {
  const cleaned = stripUnsafeControls(raw).trim();
  const notes: string[] = [];
  let path = cleaned;
  if (path.length === 0) {
    path = "/";
    notes.push("Route path was empty; treated as the application root.");
  } else if (path.includes("?")) {
    path = path.slice(0, path.indexOf("?"));
    notes.push("Query string was omitted from the route path.");
  }
  if (!path.startsWith("/") && !path.startsWith("*")) {
    path = `/${path}`;
    notes.push("Route path did not start with a slash; a leading slash was added.");
  }
  const dynamic =
    path.includes("${") ||
    path.includes("#{") ||
    path.includes("{{") ||
    path.includes("<%") ||
    /^https?:\/\//i.test(path);
  const parameterized = /[:{<][A-Za-z_*][^/]*|\[[^\]]+\]|\*/.test(path);
  const pathTruncated = cleaned.length > MAX_PATH_LENGTH;
  const ambiguous = dynamic || cleaned.length === 0 || pathTruncated;
  if (dynamic) notes.push("Path contains a runtime expression; the exact route is ambiguous.");
  if (pathTruncated) notes.push("Route path exceeded the maximum stored length and was truncated.");
  const pathKind: RoutePathKind = dynamic
    ? "dynamic"
    : fileDerived
      ? "file-derived"
      : parameterized
        ? "parameterized"
        : "literal";
  return {
    path: path.slice(0, MAX_PATH_LENGTH),
    pathKind,
    ambiguous,
    notes,
  };
}

export interface RouteInput {
  readonly startLine: number;
  readonly endLine?: number;
  readonly method: HttpMethod;
  readonly rawPath: string;
  readonly framework: string;
  readonly handler?: string | null;
  readonly provenance?: DiscoveryProvenance;
  readonly confidence?: DiscoveryConfidence;
  readonly fileDerived?: boolean;
  readonly ambiguous?: boolean;
  readonly notes?: readonly string[];
}

export function makeRoute(document: SourceDocument, input: RouteInput): RouteFact {
  const normalized = normalizeRoutePath(input.rawPath, input.fileDerived === true);
  const notes = [
    ...normalized.notes,
    ...(input.notes ?? []),
    ...(input.method === "UNKNOWN" ? ["HTTP method was not declared statically."] : []),
  ]
    .map((note) => safeText(note, 240))
    .filter((note): note is string => note !== null)
    .slice(0, 8);
  const ambiguous = input.ambiguous === true || normalized.ambiguous || input.method === "UNKNOWN";
  return {
    id: factId(
      "route",
      document.path,
      input.startLine,
      input.method,
      normalized.path,
      input.framework,
    ),
    framework: input.framework,
    method: input.method,
    path: normalized.path,
    pathKind: normalized.pathKind,
    handler:
      input.handler === undefined || input.handler === null ? null : safeIdentifier(input.handler),
    provenance: input.provenance ?? "observed",
    confidence: input.confidence ?? "high",
    ambiguous,
    notes,
    evidence: sourceLocation(document.path, input.startLine, input.endLine ?? input.startLine),
  };
}

export interface SurfaceInput {
  readonly startLine: number;
  readonly endLine?: number;
  readonly kind: SurfaceKind;
  readonly framework: string | null;
  readonly name: string;
  readonly detail?: string | null;
  readonly provenance?: DiscoveryProvenance;
  readonly confidence?: DiscoveryConfidence;
}

export function makeSurface(document: SourceDocument, input: SurfaceInput): SurfaceFact {
  return {
    id: factId("surface", input.kind, document.path, input.startLine, input.name, input.framework),
    kind: input.kind,
    framework: input.framework,
    name: safeText(input.name, 120) ?? "unknown_surface",
    detail:
      input.detail === undefined || input.detail === null ? null : safeText(input.detail, 240),
    provenance: input.provenance ?? "observed",
    confidence: input.confidence ?? "high",
    evidence: sourceLocation(document.path, input.startLine, input.endLine ?? input.startLine),
  };
}

export interface UnsupportedInput {
  readonly kind: UnsupportedSurface["kind"];
  readonly name: string;
  readonly reason: string;
  readonly evidence?: readonly SourceLocation[];
  readonly provenance?: DiscoveryProvenance;
  readonly confidence?: DiscoveryConfidence;
}

export function makeUnsupported(input: UnsupportedInput): UnsupportedSurface {
  return {
    id: factId("unsupported", input.kind, input.name, input.reason),
    kind: input.kind,
    name: safeText(input.name, 120) ?? "unknown",
    reason: safeText(input.reason, 240) ?? "Unsupported surface",
    provenance: input.provenance ?? "inferred",
    confidence: input.confidence ?? "medium",
    evidence: input.evidence ?? [],
  };
}

export function makeAmbiguity(input: {
  readonly kind: AmbiguityFact["kind"];
  readonly message: string;
  readonly file?: string;
  readonly startLine?: number;
  readonly endLine?: number;
  readonly provenance?: DiscoveryProvenance;
  readonly confidence?: DiscoveryConfidence;
}): AmbiguityFact {
  const evidence =
    input.file === undefined
      ? []
      : [sourceLocation(input.file, input.startLine ?? 1, input.endLine ?? input.startLine ?? 1)];
  return {
    id: factId("ambiguity", input.kind, input.file ?? "", input.startLine ?? 0, input.message),
    kind: input.kind,
    message: safeText(input.message, 300) ?? "Ambiguous discovery result",
    provenance: input.provenance ?? "inferred",
    confidence: input.confidence ?? "medium",
    evidence,
  };
}

export function makeFrameworkFact(input: {
  readonly name: string;
  readonly ecosystem: string;
  readonly supported: boolean;
  readonly provenance?: DiscoveryProvenance;
  readonly confidence?: DiscoveryConfidence;
  readonly evidence?: readonly SourceLocation[];
  readonly notes?: readonly string[];
}): FrameworkFact {
  const notes = (input.notes ?? [])
    .map((note) => safeText(note, 240))
    .filter((note): note is string => note !== null);
  return {
    id: factId("framework", input.name, input.ecosystem),
    name: input.name,
    ecosystem: input.ecosystem,
    supported: input.supported,
    provenance: input.provenance ?? "inferred",
    confidence: input.confidence ?? "medium",
    evidence: input.evidence ?? [],
    notes,
  };
}

export function lineAt(document: SourceDocument, index: number): string {
  return document.lines[index] ?? "";
}

export function windowText(document: SourceDocument, index: number, count: number): string {
  return document.lines.slice(index, index + count).join("\n");
}

export function firstLineMatching(
  document: SourceDocument,
  startIndex: number,
  count: number,
  pattern: RegExp,
): { index: number; match: RegExpExecArray } | null {
  for (let offset = 0; offset < count; offset += 1) {
    const index = startIndex + offset;
    const line = lineAt(document, index);
    const match = pattern.exec(line);
    if (match !== null) return { index, match };
  }
  return null;
}

export function handlerFromRest(rest: string): string | null {
  const candidates = [
    ...rest.matchAll(/\b([A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*)\b/g),
  ]
    .map((match) => match[1])
    .filter((value): value is string => value !== undefined)
    .filter(
      (value) =>
        ![
          "async",
          "await",
          "req",
          "res",
          "request",
          "response",
          "next",
          "body",
          "query",
          "param",
          "upload",
          "single",
          "array",
          "fields",
          "isEmail",
          "optional",
          "required",
        ].includes(value),
    );
  return candidates.length === 0 ? null : (candidates.at(-1) ?? null);
}

export function extractLiteralUrl(value: string): string | null {
  const match = /(?:https?):\/\/[^\s'"`<>)]+/i.exec(value);
  if (match === null) return null;
  try {
    const url = new URL(match[0]);
    if (url.username.length > 0 || url.password.length > 0) return null;
    const path = url.pathname.length > 0 ? url.pathname : "/";
    return `${url.protocol}//${url.host}${path}`.slice(0, 240);
  } catch {
    return null;
  }
}
