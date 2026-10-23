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

const RECEIVER = "app|router|routes|server|fastify|hono|api|koaRouter|route|r";

function isNextRouteFile(path: string): boolean {
  const normalized = path.replaceAll("\\", "/").replace(/^src\//, "");
  if (/^app\/.+\/route\.(?:js|jsx|ts|tsx)$/.test(normalized)) return true;
  if (/^pages\/api\/.+\.(?:js|jsx|ts|tsx)$/.test(normalized)) return true;
  return false;
}

function nextRoutePath(path: string): string {
  const normalized = path.replaceAll("\\", "/").replace(/^src\//, "");
  let route: string;
  if (/^app\//.test(normalized)) {
    route = normalized.replace(/^app\//, "").replace(/\/?route\.(?:js|jsx|ts|tsx)$/, "");
  } else {
    route = normalized.replace(/^pages\//, "").replace(/\.(?:js|jsx|ts|tsx)$/, "");
  }
  if (route.endsWith("/index")) route = route.slice(0, -"/index".length);
  route = route.replace(/^api$/, "api");
  return route.length === 0 ? "/" : `/${route}`;
}

export function javascriptFramework(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): string {
  const text = document.text;
  if (isNextRouteFile(document.path)) return "Next.js";
  if (/@Controller\s*\(/.test(text)) return "NestJS";
  if (/\bFastify\s*\(/.test(text)) return "Fastify";
  if (/\bHono\s*\(/.test(text)) return "Hono";
  if (/\bKoa\s*\(/.test(text)) return "Koa";
  if (/\bexpress\s*\(/.test(text) || /\bfrom\s+["']express["']/.test(text)) return "Express";
  if (detectedFrameworks.includes("Express")) return "Express";
  if (detectedFrameworks.includes("Fastify")) return "Fastify";
  if (detectedFrameworks.includes("Hono")) return "Hono";
  if (detectedFrameworks.includes("Koa")) return "Koa";
  if (detectedFrameworks.includes("NestJS")) return "NestJS";
  return "JavaScript/TypeScript";
}

function joinRoutePaths(base: string, route: string): string {
  const left = base.trim().replace(/\/+$/, "");
  const right = route.trim();
  if (left.length === 0) return right.length === 0 ? "/" : right;
  if (right.length === 0) return left.startsWith("/") ? left : `/${left}`;
  return `${left.startsWith("/") ? left : `/${left}`}/${right.replace(/^\/+/, "")}`;
}

function nextRoutes(document: SourceDocument): RouteFact[] {
  const path = nextRoutePath(document.path);
  const isAppRoute = /route\.(?:js|jsx|ts|tsx)$/.test(document.path.replaceAll("\\", "/"));
  const methods = new Map<string, number>();
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    if (isAppRoute) {
      const match =
        /\bexport\s+(?:async\s+)?(?:function|const)\s+(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\b/.exec(
          line,
        );
      if (match?.[1] !== undefined && !methods.has(match[1])) methods.set(match[1], index + 1);
    } else {
      for (const match of line.matchAll(
        /(?:req(?:uest)?\.method\s*===?\s*|case\s+)["'](GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)["']/g,
      )) {
        const method = match[1];
        if (method !== undefined && !methods.has(method)) methods.set(method, index + 1);
      }
    }
  }
  const dynamic = /\[[^\]]+\]/.test(path);
  const notes = dynamic ? ["File-derived route includes a dynamic or catch-all segment."] : [];
  if (methods.size === 0) {
    return [
      makeRoute(document, {
        startLine: 1,
        endLine: 1,
        method: "UNKNOWN",
        rawPath: path,
        framework: "Next.js",
        provenance: "inferred",
        confidence: "medium",
        fileDerived: true,
        ambiguous: true,
        notes: [...notes, "Next.js route file does not declare a static HTTP method."],
      }),
    ];
  }
  return [...methods.entries()].map(([method, line]) =>
    makeRoute(document, {
      startLine: line,
      endLine: line,
      method: methodFromToken(method),
      rawPath: path,
      framework: "Next.js",
      provenance: "inferred",
      confidence: "high",
      fileDerived: true,
      ambiguous: dynamic,
      notes,
    }),
  );
}

function nestRoutes(document: SourceDocument): RouteFact[] {
  const routes: RouteFact[] = [];
  let controllerBase = "";
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const controller = /@Controller\s*\(\s*(?:['"]([^'"]+)['"])?\s*\)/.exec(line);
    if (controller !== null) {
      controllerBase = controller[1] ?? "";
      continue;
    }
    const match =
      /@(Get|Post|Put|Patch|Delete|Options|Head|All)\s*\(\s*(?:['"]([^'"]*)['"])?\s*\)/.exec(line);
    if (match === null) continue;
    const method = methodFromToken(match[1] ?? "");
    const handlerLine = document.lines
      .slice(index, index + 5)
      .find((candidate) =>
        /(?:public|private|protected|async|\w+)\s+[A-Za-z_$][\w$]*\s*\(/.test(candidate),
      );
    routes.push(
      makeRoute(document, {
        startLine: index + 1,
        endLine: handlerLine === undefined ? index + 1 : index + 1,
        method,
        rawPath: joinRoutePaths(controllerBase, match[2] ?? ""),
        framework: "NestJS",
        handler: handlerLine?.match(/([A-Za-z_$][\w$]*)\s*\(/)?.[1] ?? null,
        confidence: "high",
      }),
    );
  }
  return routes;
}

export function extractJavaScriptRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = javascriptFramework(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];
  if (isNextRouteFile(document.path)) {
    routes.push(...nextRoutes(document));
    return { routes, ambiguities, parser: "javascript" };
  }
  if (framework === "NestJS" || /@Controller\s*\(/.test(document.text)) {
    routes.push(...nestRoutes(document));
  }

  const routeCall = new RegExp(
    `\\b(?:${RECEIVER})\\s*\\.\\s*(get|post|put|patch|delete|options|head|all|any)\\s*\\(\\s*(['"\`])([^'"\`\\r\\n]{0,300})\\2([^\\n]*)`,
  );
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const match = routeCall.exec(line);
    if (match !== null) {
      const method = methodFromToken(match[1] ?? "");
      const rawPath = match[3] ?? "";
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          method,
          rawPath,
          framework,
          handler: handlerFromRest(match[4] ?? ""),
          confidence: rawPath.includes("${") ? "medium" : "high",
        }),
      );
      continue;
    }

    const chain = new RegExp(
      `\\b(?:${RECEIVER})\\s*\\.\\s*route\\s*\\(\\s*(['"\`])([^'"\`\\r\\n]{0,300})\\1\\s*\\)([^\\n]*)`,
    ).exec(line);
    if (chain === null) continue;
    const rawPath = chain[2] ?? "";
    const windowLines = windowText(document, index, 6);
    const methodMatches = [
      ...windowLines.matchAll(/\.(get|post|put|patch|delete|options|head|all|any)\s*\(/g),
    ];
    if (methodMatches.length === 0) {
      ambiguities.push(
        makeAmbiguity({
          kind: "route",
          message: "Route chain does not declare a statically visible HTTP method.",
          file: document.path,
          startLine: index + 1,
        }),
      );
      continue;
    }
    for (const methodMatch of methodMatches) {
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          endLine: index + 1,
          method: methodFromToken(methodMatch[1] ?? ""),
          rawPath,
          framework,
          confidence: "medium",
          notes: ["Method was read from a route chain."],
        }),
      );
    }
  }

  if (
    /(?:app|router|routes|server|api)\s*\.\s*(get|post|put|patch|delete|options|head|all|any)\s*\(\s*\[/i.test(
      document.text,
    )
  ) {
    ambiguities.push(
      makeAmbiguity({
        kind: "route",
        message: "Array-valued route paths are not expanded by the static parser.",
        file: document.path,
      }),
    );
  }
  return { routes, ambiguities, parser: "javascript" };
}
