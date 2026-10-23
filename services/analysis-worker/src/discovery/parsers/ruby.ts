import type { RouteFact } from "../model";
import {
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  methodsFromList,
  type SourceDocument,
} from "../shared";

function frameworkForRuby(document: SourceDocument, detectedFrameworks: readonly string[]): string {
  if (
    document.path.replaceAll("\\", "/").endsWith("config/routes.rb") ||
    /\bRails\.application\.routes\.draw\b/.test(document.text) ||
    detectedFrameworks.includes("Ruby on Rails")
  ) {
    return "Ruby on Rails";
  }
  if (/\bSinatra\b/.test(document.text) || detectedFrameworks.includes("Sinatra")) {
    return "Sinatra";
  }
  return "Ruby";
}

function routeTarget(line: string): string | null {
  const match = /\bto\s*:\s*['"]([^'"]+)['"]/.exec(line);
  return match?.[1] ?? null;
}

function resourceRoutes(document: SourceDocument, framework: string): RouteFact[] {
  const routes: RouteFact[] = [];
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const resource = /^\s*resources\s+:([A-Za-z_]\w*)/.exec(line);
    const singular = /^\s*resource\s+:([A-Za-z_]\w*)/.exec(line);
    const name = resource?.[1] ?? singular?.[1];
    if (name === undefined) continue;
    const base = `/${name}`;
    const entries: Array<[string, string]> = singular
      ? [
          ["GET", base],
          ["GET", `${base}/new`],
          ["POST", base],
          ["GET", `${base}/edit`],
          ["PATCH", base],
          ["PUT", base],
          ["DELETE", base],
        ]
      : [
          ["GET", base],
          ["GET", `${base}/new`],
          ["POST", base],
          ["GET", `${base}/:id`],
          ["GET", `${base}/:id/edit`],
          ["PATCH", `${base}/:id`],
          ["PUT", `${base}/:id`],
          ["DELETE", `${base}/:id`],
        ];
    for (const [method, path] of entries) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: methodFromToken(method),
          rawPath: path,
          framework,
          confidence: "medium",
          provenance: "inferred",
          notes: ["Route was inferred from a Rails resource declaration."],
        }),
      );
    }
  }
  return routes;
}

export function extractRubyRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForRuby(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const root = /^\s*root\s+['"]([^'"]+)['"]/.exec(line);
    if (root !== null) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: "GET",
          rawPath: "/",
          framework,
          handler: root[1] ?? null,
          confidence: "high",
        }),
      );
      continue;
    }

    const match =
      /^\s*(get|post|put|patch|delete|options|head|match)\s+['"]([^'"]{0,300})['"](.*)$/i.exec(
        line,
      );
    if (match !== null) {
      const operation = match[1] ?? "";
      const rawPath = match[2] ?? "";
      const rest = match[3] ?? "";
      const methods =
        operation.toLowerCase() === "match" ? methodsFromList(rest) : [methodFromToken(operation)];
      for (const method of methods.length === 0 ? [methodFromToken("GET")] : methods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method,
            rawPath,
            framework,
            handler: routeTarget(line),
            confidence: operation.toLowerCase() === "match" ? "medium" : "high",
          }),
        );
      }
    }
  }

  routes.push(...resourceRoutes(document, framework));

  if (/\b(?:namespace|scope)\s+(?::\w+|['"][^'"]+['"])/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message:
          "Rails namespace/scope prefixes are not expanded; some route paths may be relative.",
        file: document.path,
      }),
    );
  }

  return { routes, ambiguities, parser: "ruby" };
}
