import type { RouteFact } from "../model";
import {
  handlerFromRest,
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  type SourceDocument,
  windowText,
} from "../shared";

function frameworkForRust(document: SourceDocument, detectedFrameworks: readonly string[]): string {
  if (/\bactix_web\b/.test(document.text)) return "Actix Web";
  if (/\brocket\b/.test(document.text)) return "Rocket";
  if (/\baxum\b/.test(document.text)) return "Axum";
  if (detectedFrameworks.includes("Axum")) return "Axum";
  if (detectedFrameworks.includes("Actix Web")) return "Actix Web";
  if (detectedFrameworks.includes("Rocket")) return "Rocket";
  return "Rust";
}

function rustHandlerEverywhere(document: SourceDocument, startIndex: number): string | null {
  for (let offset = 0; offset < 6; offset += 1) {
    const line = lineAt(document, startIndex + offset);
    const match = /\bfn\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(/.exec(line);
    if (match?.[1] !== undefined) return match[1];
  }
  return null;
}

export function extractRustRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForRust(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);

    const axum = /\.route\s*\(\s*"([^"]{1,300})"\s*,\s*([^;]{0,800})/.exec(
      windowText(document, index, 5),
    );
    if (axum !== null && line.includes(".route")) {
      const rawPath = axum[1] ?? "";
      const rest = axum[2] ?? "";
      const methodMatches = [
        ...rest.matchAll(/\b(get|post|put|patch|delete|options|head|any)\s*\(/g),
      ];
      if (methodMatches.length === 0) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method: "UNKNOWN",
            rawPath,
            framework,
            handler: rustHandlerEverywhere(document, index + 1),
            confidence: "medium",
            ambiguous: true,
          }),
        );
      } else {
        for (const methodMatch of methodMatches) {
          routes.push(
            makeRoute(document, {
              startLine: index + 1,
              method: methodFromToken(methodMatch[1] ?? ""),
              rawPath,
              framework,
              handler: handlerFromRest(rest),
              confidence: "high",
            }),
          );
        }
      }
      continue;
    }

    const attribute = /#\[(get|post|put|patch|delete|options|head)\s*\(\s*"([^"]+)"\s*\)\s*\]/.exec(
      line,
    );
    if (attribute !== null) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: methodFromToken(attribute[1] ?? ""),
          rawPath: attribute[2] ?? "",
          framework,
          handler: rustHandlerEverywhere(document, index + 1),
          confidence: "high",
        }),
      );
      continue;
    }

    const routeAttribute =
      /#\[route\s*\(\s*"([^"]+)"\s*,\s*method\s*=\s*"([A-Za-z]+)"\s*\)\s*\]/.exec(line);
    if (routeAttribute !== null) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: methodFromToken(routeAttribute[2] ?? ""),
          rawPath: routeAttribute[1] ?? "",
          framework,
          handler: rustHandlerEverywhere(document, index + 1),
          confidence: "high",
        }),
      );
    }
  }

  if (/\bweb::resource\s*\(/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "Actix resource registration is not expanded into individual methods.",
        file: document.path,
      }),
    );
  }
  if (routes.length === 0 && /\b(?:Router::new|route)\s*\(/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "coverage",
        message: "Rust router-like source was found but no route pattern matched the parser.",
        file: document.path,
        confidence: "low",
      }),
    );
  }

  return { routes, ambiguities, parser: "rust" };
}
