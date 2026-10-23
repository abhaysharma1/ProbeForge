import type { RouteFact } from "../model";
import {
  handlerFromRest,
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  methodsFromList,
  type SourceDocument,
} from "../shared";

function frameworkForPhp(document: SourceDocument, detectedFrameworks: readonly string[]): string {
  if (
    /\bRoute::/.test(document.text) ||
    /\bLaravel\b/.test(document.text) ||
    detectedFrameworks.includes("Laravel")
  ) {
    return "Laravel";
  }
  if (/\bSymfony\b/.test(document.text) || detectedFrameworks.includes("Symfony")) {
    return "Symfony";
  }
  return "PHP";
}

function laravelResourceRoutes(document: SourceDocument, framework: string): RouteFact[] {
  const routes: RouteFact[] = [];
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const resource = /\bRoute::(resource|apiResource)\s*\(\s*['"]([^'"]+)['"]/.exec(line);
    if (resource === null) continue;
    const name = resource[2] ?? "";
    const base = `/${name}`;
    const entries: Array<readonly [string, string]> =
      resource[1] === "apiResource"
        ? [
            ["GET", base],
            ["POST", base],
            ["GET", `${base}/{id}`],
            ["PUT", `${base}/{id}`],
            ["PATCH", `${base}/{id}`],
            ["DELETE", `${base}/{id}`],
          ]
        : [
            ["GET", base],
            ["GET", `${base}/create`],
            ["POST", base],
            ["GET", `${base}/{id}`],
            ["GET", `${base}/{id}/edit`],
            ["PUT", `${base}/{id}`],
            ["PATCH", `${base}/{id}`],
            ["DELETE", `${base}/{id}`],
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
          notes: ["Route was inferred from a Laravel resource declaration."],
        }),
      );
    }
  }
  return routes;
}

export function extractPhpRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForPhp(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const match =
      /\bRoute::(get|post|put|patch|delete|options|head|any)\s*\(\s*(['"])([^'"]{0,300})\2([^)]*)/.exec(
        line,
      );
    if (match !== null) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method: methodFromToken(match[1] ?? ""),
          rawPath: match[3] ?? "",
          framework,
          handler: handlerFromRest(match[4] ?? ""),
          confidence: "high",
        }),
      );
      continue;
    }

    const matchRoute =
      /\bRoute::match\s*\(\s*\[([^\]]*)\]\s*,\s*(['"])([^'"]{0,300})\2([^)]*)/.exec(line);
    if (matchRoute !== null) {
      for (const method of methodsFromList(matchRoute[1] ?? "")) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method,
            rawPath: matchRoute[3] ?? "",
            framework,
            handler: handlerFromRest(matchRoute[4] ?? ""),
            confidence: "high",
          }),
        );
      }
      continue;
    }

    const symfony = /#\[Route\s*\(\s*(['"])([^'"]{0,300})\1([^\]]*)\]/.exec(line);
    if (symfony !== null) {
      const rest = symfony[3] ?? "";
      const methodList = /methods\s*:\s*\[([^\]]*)\]/.exec(rest);
      const methods = methodList?.[1] === undefined ? [] : methodsFromList(methodList[1]);
      for (const method of methods.length === 0 ? [methodFromToken("GET")] : methods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            method,
            rawPath: symfony[2] ?? "",
            framework: "Symfony",
            confidence: "high",
          }),
        );
      }
    }
  }

  routes.push(...laravelResourceRoutes(document, framework));

  if (/\bRoute::(?:prefix|group)\s*\(/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "Laravel route prefixes/groups are not expanded; some paths may be relative.",
        file: document.path,
      }),
    );
  }

  return { routes, ambiguities, parser: "php" };
}
