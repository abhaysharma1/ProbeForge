import type { HttpMethod, RouteFact } from "../model";
import {
  handlerFromRest,
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  type SourceDocument,
} from "../shared";

function frameworkForCSharp(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): string {
  if (
    /\bMicrosoft\.AspNetCore\b|\bApiController\b|\bMap(?:Get|Post|Put|Patch|Delete)\b/.test(
      document.text,
    ) ||
    detectedFrameworks.includes("ASP.NET Core")
  ) {
    return "ASP.NET Core";
  }
  return "C#";
}

function csharpHandler(
  document: SourceDocument,
  startIndex: number,
): { name: string | null; endLine: number } {
  for (let offset = 0; offset < 6; offset += 1) {
    const index = startIndex + offset;
    const line = lineAt(document, index);
    const match =
      /(?:public|private|protected|internal)\s+(?:async\s+)?[\w<>[\], ?]+\s+([A-Za-z_$]\w*)\s*\(/.exec(
        line,
      );
    if (match?.[1] !== undefined) return { name: match[1], endLine: index + 1 };
  }
  return { name: null, endLine: startIndex + 1 };
}

function joinPath(base: string, route: string): string {
  const left = base.trim().replace(/\/+$/, "");
  const right = route.trim();
  if (left.length === 0) return right.length === 0 ? "/" : right;
  if (right.length === 0) return left.startsWith("/") ? left : `/${left}`;
  return `${left.startsWith("/") ? left : `/${left}`}/${right.replace(/^\/+/, "")}`;
}

function classControllerName(document: SourceDocument, startIndex: number): string | null {
  for (let offset = 0; offset < 6; offset += 1) {
    const line = lineAt(document, startIndex + offset);
    const match = /\bclass\s+([A-Za-z_$]\w*)/.exec(line);
    if (match?.[1] !== undefined) {
      return match[1].replace(/Controller$/i, "").toLowerCase();
    }
  }
  return null;
}

export function extractCSharpRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForCSharp(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  let classBase = "";
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const classRoute = /^\s*\[Route\s*\(\s*"([^"]*)"\s*\)\s*\]/.exec(line);
    if (classRoute !== null) {
      const controller = classControllerName(document, index + 1) ?? "";
      classBase = (classRoute[1] ?? "")
        .replace(/\[controller\]/gi, controller)
        .replace(/\[action\]/gi, "{action}");
      continue;
    }

    const map = /\bMap(Get|Post|Put|Patch|Delete|Methods)\s*\(\s*"([^"]{0,300})"([^)]*)/.exec(line);
    if (map !== null) {
      const operation = map[1] ?? "";
      const method: HttpMethod =
        operation === "Methods" ? "UNKNOWN" : methodFromToken(operation.replace(/^Map/, ""));
      const handler = csharpHandler(document, index + 1);
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          endLine: handler.endLine,
          method,
          rawPath: map[2] ?? "",
          framework,
          handler: handler.name ?? handlerFromRest(map[3] ?? ""),
          confidence: operation === "Methods" ? "medium" : "high",
          ambiguous: operation === "Methods",
          notes: operation === "Methods" ? ["MapMethods method set was not expanded."] : [],
        }),
      );
      continue;
    }

    const attribute =
      /\[\s*Http(Get|Post|Put|Patch|Delete|Head|Options)\s*(?:\(\s*"([^"]*)"\s*\))?\s*\]/.exec(
        line,
      );
    if (attribute !== null) {
      const handler = csharpHandler(document, index + 1);
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          endLine: handler.endLine,
          method: methodFromToken(attribute[1] ?? ""),
          rawPath: joinPath(classBase, attribute[2] ?? ""),
          framework,
          handler: handler.name,
          confidence: attribute[2] === undefined && classBase.length === 0 ? "medium" : "high",
          ambiguous: attribute[2] === undefined && classBase.length === 0,
          notes:
            attribute[2] === undefined && classBase.length === 0
              ? ["Controller route has no static path."]
              : [],
        }),
      );
    }
  }

  if (/\bMapGroup\s*\(/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "ASP.NET route groups are not expanded; grouped paths may be relative.",
        file: document.path,
      }),
    );
  }

  return { routes, ambiguities, parser: "csharp" };
}
