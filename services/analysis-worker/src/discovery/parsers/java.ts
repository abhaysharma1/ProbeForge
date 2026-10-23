import type { HttpMethod, RouteFact } from "../model";
import {
  lineAt,
  makeAmbiguity,
  makeRoute,
  methodFromToken,
  methodsFromList,
  type SourceDocument,
} from "../shared";

function frameworkForJava(document: SourceDocument, detectedFrameworks: readonly string[]): string {
  if (
    /@(?:RestController|Controller|RequestMapping|GetMapping)\b/.test(document.text) ||
    /\borg\.springframework\b/.test(document.text) ||
    detectedFrameworks.includes("Spring Boot")
  ) {
    return "Spring Boot";
  }
  if (/\bjakarta\.ws\.rs\b|\bjavax\.ws\.rs\b|@Path\s*\(/.test(document.text)) {
    return "JAX-RS";
  }
  return "Java";
}

function javaHandler(
  document: SourceDocument,
  startIndex: number,
): { name: string | null; endLine: number } {
  for (let offset = 0; offset < 6; offset += 1) {
    const index = startIndex + offset;
    const line = lineAt(document, index);
    const match =
      /(?:public|protected|private)\s+(?:static\s+)?[\w<>[\], ?]+\s+([A-Za-z_$]\w*)\s*\(/.exec(
        line,
      );
    if (match?.[1] !== undefined) return { name: match[1], endLine: index + 1 };
  }
  return { name: null, endLine: startIndex + 1 };
}

function joinSpringPath(base: string, route: string): string {
  const left = base.trim().replace(/\/+$/, "");
  const right = route.trim();
  if (left.length === 0) return right.length === 0 ? "/" : right;
  if (right.length === 0) return left.startsWith("/") ? left : `/${left}`;
  return `${left.startsWith("/") ? left : `/${left}`}/${right.replace(/^\/+/, "")}`;
}

export function extractJavaRoutes(
  document: SourceDocument,
  detectedFrameworks: readonly string[],
): { routes: RouteFact[]; ambiguities: ReturnType<typeof makeAmbiguity>[]; parser: string } {
  const framework = frameworkForJava(document, detectedFrameworks);
  const routes: RouteFact[] = [];
  const ambiguities: ReturnType<typeof makeAmbiguity>[] = [];

  let classBase = "";
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = lineAt(document, index);
    const classMapping = /@RequestMapping\s*\(\s*(?:value\s*=\s*)?(?:['"]([^'"]*)['"])?/.exec(line);
    if (classMapping !== null) {
      const isClassLevel = document.lines
        .slice(index + 1, index + 5)
        .some((candidate) => /\bclass\s+[A-Za-z_$]\w*/.test(candidate));
      if (isClassLevel) {
        classBase = classMapping[1] ?? "";
        continue;
      }
    }

    const mapping =
      /@(Get|Post|Put|Patch|Delete|Head|Options)Mapping\s*\(\s*(?:value\s*=\s*)?(?:['"]([^'"]*)['"])?([^)]*)\)?/.exec(
        line,
      );
    if (mapping !== null) {
      const operation = mapping[1] ?? "";
      const method = methodFromToken(operation);
      const handler = javaHandler(document, index + 1);
      routes.push(
        makeRoute(document, {
          startLine: index + 1,
          endLine: handler.endLine,
          method,
          rawPath: joinSpringPath(classBase, mapping[2] ?? ""),
          framework,
          handler: handler.name,
          confidence: "high",
        }),
      );
      continue;
    }

    const requestMapping =
      /@RequestMapping\s*\(\s*(?:value\s*=\s*)?(?:['"]([^'"]*)['"])?([^)]*)\)?/.exec(line);
    if (requestMapping !== null) {
      const rawPath = requestMapping[1] ?? "";
      const rest = requestMapping[2] ?? "";
      const methods = methodsFromList(rest);
      const handler = javaHandler(document, index + 1);
      const finalMethods: HttpMethod[] = methods.length === 0 ? ["ANY"] : methods;
      for (const method of finalMethods) {
        routes.push(
          makeRoute(document, {
            startLine: index + 1,
            endLine: handler.endLine,
            method,
            rawPath: joinSpringPath(classBase, rawPath),
            framework,
            handler: handler.name,
            confidence: "medium",
            notes: methods.length === 0 ? ["@RequestMapping did not restrict an HTTP method."] : [],
          }),
        );
      }
    }
  }

  if (routes.length === 0 && /@(?:Path|GET|POST|RequestMapping)\b/.test(document.text)) {
    ambiguities.push(
      makeAmbiguity({
        kind: "coverage",
        message: "Java route-like annotations were found but no supported mapping was parsed.",
        file: document.path,
        confidence: "low",
      }),
    );
  }

  return { routes, ambiguities, parser: "java" };
}
