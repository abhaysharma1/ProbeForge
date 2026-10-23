import { basename } from "node:path";
import type { HttpMethod, RouteFact } from "../model";
import {
  handlerFromRest,
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  methodsFromList,
  type SourceDocument,
} from "../shared";

function frameworkForPython(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): string {
  const text = document.text;
  if (basename(document.path).toLowerCase() === "urls.py" || /\burlpatterns\b/.test(text)) {
    return "Django";
  }
  if (/\b(?:FastAPI|APIRouter)\s*\(/.test(text) || detectedFrameworks.includes("FastAPI")) {
    return "FastAPI";
  }
  if (/\b(?:Flask|Blueprint)\s*\(/.test(text) || detectedFrameworks.includes("Flask")) {
    return "Flask";
  }
  if (/\bStarlette\s*\(/.test(text) || detectedFrameworks.includes("Starlette")) {
    return "Starlette";
  }
  if (detectedFrameworks.includes("Django")) return "Django";
  return "Python";
}

function pythonHandler(
  document: SourceDocument,
  startIndex: number,
): { name: string | null; endLine: number } {
  for (let offset = 0; offset < 6; offset += 1) {
    const index = startIndex + offset;
    const line = lineAt(document, index);
    const match = /^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(/.exec(line);
    if (match?.[1] !== undefined) return { name: match[1], endLine: index + 1 };
  }
  return { name: null, endLine: startIndex + 1 };
}

export function extractPythonRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForPython(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  if (framework === "Django") {
    for (let index = 0; index < document.lines.length; index += 1) {
      const line = lineAt(document, index);
      const match = /\b(?:path|re_path)\s*\(\s*(?:r)?(['"])([^'"]{0,300})\1\s*,\s*([^,)]*)/.exec(
        line,
      );
      if (match === null) continue;
      const rawPath = match[2] ?? "";
      const target = match[3] ?? "";
      if (/\binclude\s*\(/.test(target)) {
        ambiguities.push(
          makeAmbiguity({
            kind: "route",
            message: "Included URLconf is not expanded statically; routes from it may be missing.",
            file: document.path,
            startLine: index + 1,
          }),
        );
        continue;
      }
      const handler = pythonHandler(document, index + 1);
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          endLine: handler.endLine,
          method: "UNKNOWN",
          rawPath,
          framework,
          handler: handler.name,
          provenance: "inferred",
          confidence: "medium",
          ambiguous: true,
          notes: ["Django URL patterns do not declare HTTP methods statically."],
        }),
      );
    }
    return { routes, ambiguities, parser: "python" };
  }

  const decorator =
    /^\s*@([A-Za-z_]\w*)\.(get|post|put|patch|delete|options|head|route|api_route)\s*\(\s*(['"])([^'"]{0,300})\3(.*)$/;

  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const match = decorator.exec(line);
    if (match !== null) {
      const operation = match[2] ?? "";
      const rawPath = match[4] ?? "";
      const rest = match[5] ?? "";
      let methods: HttpMethod[] = [];
      if (operation === "route" || operation === "api_route") {
        const listMatch = /methods\s*=\s*\[([^\]]*)\]/.exec(rest);
        methods = listMatch?.[1] === undefined ? [] : methodsFromList(listMatch[1]);
        if (methods.length === 0) {
          methods = ["GET"];
        }
      } else {
        methods = [methodFromToken(operation)];
      }
      const handler = pythonHandler(document, index + 1);
      for (const method of methods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            endLine: handler.endLine,
            method,
            rawPath,
            framework,
            handler: handler.name,
            confidence: operation === "route" || operation === "api_route" ? "medium" : "high",
            notes:
              operation === "route" || operation === "api_route"
                ? ["Default GET method was inferred because no methods list was present."]
                : [],
          }),
        );
      }
      continue;
    }

    const addApiRoute = /\badd_api_route\s*\(\s*(['"])([^'"]{0,300})\1\s*,\s*([^,)]*)(.*)$/.exec(
      line,
    );
    if (addApiRoute !== null) {
      const rawPath = addApiRoute[2] ?? "";
      const rest = addApiRoute[4] ?? "";
      const listMatch = /methods\s*=\s*\[([^\]]*)\]/.exec(rest);
      const methods =
        listMatch?.[1] === undefined ? [methodFromToken("GET")] : methodsFromList(listMatch[1]);
      for (const method of methods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method,
            rawPath,
            framework,
            handler: handlerFromRest(addApiRoute[3] ?? ""),
            confidence: "medium",
          }),
        );
      }
      continue;
    }

    const starletteRoute = /\bRoute\s*\(\s*(['"])([^'"]{0,300})\1\s*,\s*([^,)]*)(.*)$/.exec(line);
    if (starletteRoute !== null && /\b(?:Starlette|Router)\s*\(/.test(document.text)) {
      const rawPath = starletteRoute[2] ?? "";
      const rest = starletteRoute[4] ?? "";
      const listMatch = /methods\s*=\s*\[([^\]]*)\]/.exec(rest);
      const methods =
        listMatch?.[1] === undefined ? [methodFromToken("GET")] : methodsFromList(listMatch[1]);
      for (const method of methods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method,
            rawPath,
            framework,
            handler: handlerFromRest(starletteRoute[3] ?? ""),
            confidence: "medium",
          }),
        );
      }
    }
  }

  if (/\b(?:websocket|websocket_route)\s*\(/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "WebSocket routes are not modeled as HTTP routes in this phase.",
        file: document.path,
      }),
    );
  }

  return { routes, ambiguities, parser: "python" };
}
