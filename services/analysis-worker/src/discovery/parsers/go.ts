import type { RouteFact } from "../model";
import {
  handlerFromRest,
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  type SourceDocument,
} from "../shared";

function frameworkForGo(document: SourceDocument, detectedFrameworks: readonly string[]): string {
  if (/\bgin\./.test(document.text) || detectedFrameworks.includes("Gin")) return "Gin";
  if (/\becho\./.test(document.text) || detectedFrameworks.includes("Echo")) return "Echo";
  if (/\bfiber\./.test(document.text) || detectedFrameworks.includes("Fiber")) return "Fiber";
  if (/\bnet\/http\b/.test(document.text) || /\bhttp\.Handle(?:Func)?\s*\(/.test(document.text)) {
    return "Go net/http";
  }
  return "Go";
}

export function extractGoRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForGo(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  const routeCall =
    /\b(?:r|router|e|app|api|group|v1|s|server|g|mux)\s*\.\s*(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD|Any|Get|Post|Put|Patch|Delete|Options|Head)\s*\(\s*"([^"]{0,300})"([^)]*)/;
  const handleFunc =
    /\b(?:http|mux|r|router)\s*\.\s*(HandleFunc|Handle)\s*\(\s*"([^"]{0,300})"([^)]*)/;

  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const match = routeCall.exec(line);
    if (match !== null) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: methodFromToken(match[1] ?? ""),
          rawPath: match[2] ?? "",
          framework,
          handler: handlerFromRest(match[3] ?? ""),
          confidence: "high",
        }),
      );
      continue;
    }

    const handle = handleFunc.exec(line);
    if (handle !== null) {
      const rawTarget = handle[2] ?? "";
      const prefix = /^(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\s+(\/.+)$/i.exec(rawTarget);
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: prefix?.[1] === undefined ? "UNKNOWN" : methodFromToken(prefix[1]),
          rawPath: prefix?.[2] ?? rawTarget,
          framework,
          handler: handlerFromRest(handle[3] ?? ""),
          confidence: prefix?.[1] === undefined ? "medium" : "high",
          ambiguous: prefix?.[1] === undefined,
          notes:
            prefix?.[1] === undefined
              ? ["http.Handle/HandleFunc does not declare an HTTP method directly."]
              : ["Method was read from a Go 1.22 method-qualified pattern."],
        }),
      );
    }
  }

  if (/\.\s*(?:Group|Route)\s*\(\s*"/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "Go route groups are not expanded; nested route paths may be relative.",
        file: document.path,
      }),
    );
  }
  if (/\.\s*Methods\s*\(\s*"/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "Method chaining on a Go route handler is not fully expanded by the parser.",
        file: document.path,
        confidence: "low",
      }),
    );
  }

  return { routes, ambiguities, parser: "go" };
}
