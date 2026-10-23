import type { DiscoveryConfidence, SurfaceFact, SurfaceKind } from "./model";
import { extractLiteralUrl, makeSurface, type SourceDocument, safeText } from "./shared";

interface SurfaceRule {
  readonly kind: SurfaceKind;
  readonly name: string;
  readonly regex: RegExp;
  readonly confidence: DiscoveryConfidence;
  readonly detail?: (line: string, match: RegExpExecArray) => string | null;
}

const AUTH_RULES: readonly SurfaceRule[] = [
  {
    kind: "auth_check",
    name: "authentication_check",
    regex:
      /\b(requireAuth|requireAuthentication|ensureAuthenticated|ensureLoggedIn|isAuthenticated|authenticate|authorize|checkAuth|verifyToken|validateToken|passport\.authenticate)\b/i,
    confidence: "high",
    detail: (_line, match) => `Authentication helper: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "login_required",
    regex: /\b(login_required|permission_required|user_passes_test)\b/,
    confidence: "high",
    detail: (_line, match) => `Authorization decorator: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "security_annotation",
    regex: /@(PreAuthorize|Secured|RolesAllowed|PermitAll|DenyAll|Authorize)\b/,
    confidence: "high",
    detail: (_line, match) => `Security annotation: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "dotnet_authorize",
    regex: /\[\s*(Authorize)\s*(?:\([^)]*\))?\s*\]/,
    confidence: "high",
    detail: (_line, match) => `Security attribute: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "dependency_security",
    regex: /\b(Depends|Security)\s*\(\s*(get_current_user|verify_|auth|security)/i,
    confidence: "high",
    detail: (_line, match) => `Dependency-based authentication: ${match[2] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "before_action_auth",
    regex: /\bbefore_action\s+:(authenticate|require_|authorize)/,
    confidence: "high",
    detail: (_line, match) => `Controller authentication hook: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "auth_check",
    name: "authorization_middleware",
    regex: /\b(UseAuthentication|UseAuthorization|RequireAuthorization)\s*\(/,
    confidence: "high",
    detail: (_line, match) => `Authorization middleware: ${match[1] ?? "unknown"}`,
  },
];

const MIDDLEWARE_RULES: readonly SurfaceRule[] = [
  {
    kind: "middleware",
    name: "express_use",
    regex: /\b(?:app|router|server|api)\s*\.\s*use\s*\(/i,
    confidence: "high",
    detail: () => "Express-compatible application middleware",
  },
  {
    kind: "middleware",
    name: "add_middleware",
    regex: /\badd_middleware\s*\(/i,
    confidence: "high",
    detail: () => "ASGI middleware registration",
  },
  {
    kind: "middleware",
    name: "django_middleware",
    regex: /^\s*MIDDLEWARE\s*=\s*\[/,
    confidence: "medium",
    detail: () => "Django middleware configuration",
  },
  {
    kind: "middleware",
    name: "rails_before_action",
    regex: /\bbefore_action\b/,
    confidence: "medium",
    detail: () => "Rails controller callback",
  },
  {
    kind: "middleware",
    name: "dotnet_use_middleware",
    regex: /\bUseMiddleware\s*\(/,
    confidence: "high",
    detail: () => "ASP.NET Core middleware registration",
  },
];

const VALIDATOR_RULES: readonly SurfaceRule[] = [
  {
    kind: "schema_validator",
    name: "zod_schema",
    regex: /\bz\.object\s*\(/,
    confidence: "high",
    detail: () => "Zod object schema",
  },
  {
    kind: "schema_validator",
    name: "joi_schema",
    regex: /\bJoi\.object\s*\(/,
    confidence: "high",
    detail: () => "Joi object schema",
  },
  {
    kind: "schema_validator",
    name: "yup_schema",
    regex: /\byup\.object\s*\(/,
    confidence: "high",
    detail: () => "Yup object schema",
  },
  {
    kind: "schema_validator",
    name: "express_validator",
    regex: /\b(body|query|param|check)\s*\(\s*['"]/,
    confidence: "medium",
    detail: (_line, match) => `Express-style request validator: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "schema_validator",
    name: "pydantic_model",
    regex: /class\s+(\w+)\s*\(\s*(?:BaseModel|Schema)\s*\)/,
    confidence: "high",
    detail: (_line, match) => `Pydantic or marshmallow schema class: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "schema_validator",
    name: "validation_annotation",
    regex: /@(Valid|Validated)\b/,
    confidence: "high",
    detail: (_line, match) => `Validation annotation: ${match[1] ?? "unknown"}`,
  },
  {
    kind: "schema_validator",
    name: "validator_call",
    regex: /\bvalidator\s*\(/,
    confidence: "medium",
    detail: () => "Validator registration",
  },
];

const CONFIG_RULES: readonly SurfaceRule[] = [
  {
    kind: "configuration",
    name: "cors_wildcard",
    regex: /\bcors\s*\([^)]*origin\s*:\s*['"]\*['"]|allow_origins\s*=\s*\[\s*['"]\*['"]/i,
    confidence: "high",
    detail: () => "CORS configured with a wildcard origin",
  },
  {
    kind: "configuration",
    name: "debug_enabled",
    regex: /\b(?:DEBUG|debug)\s*=\s*True\b/,
    confidence: "high",
    detail: () => "Debug mode enabled in source/configuration",
  },
  {
    kind: "configuration",
    name: "csrf_exempt",
    regex: /\bcsrf_exempt\b/,
    confidence: "high",
    detail: () => "CSRF exemption declared",
  },
  {
    kind: "configuration",
    name: "permit_all",
    regex: /\bpermitAll\s*\(/,
    confidence: "high",
    detail: () => "Spring Security permit-all rule",
  },
  {
    kind: "configuration",
    name: "tls_verification_disabled",
    regex:
      /\brejectUnauthorized\s*:\s*false|verify\s*=\s*False|InsecureSkipVerify\s*:\s*true|TrustServerCertificate\s*=\s*[Tt]rue/,
    confidence: "high",
    detail: () => "TLS certificate verification appears disabled",
  },
];

const OUTBOUND_RULES: readonly SurfaceRule[] = [
  {
    kind: "outbound_request",
    name: "fetch",
    regex: /\bfetch\s*\(/,
    confidence: "high",
    detail: (line) => extractLiteralUrl(line),
  },
  {
    kind: "outbound_request",
    name: "axios",
    regex: /\baxios\.(?:get|post|put|patch|delete|request)\s*\(/,
    confidence: "high",
    detail: (line) => extractLiteralUrl(line),
  },
  {
    kind: "outbound_request",
    name: "python_http_client",
    regex: /\b(?:requests|httpx)\.(?:get|post|put|patch|delete|request)\s*\(/,
    confidence: "high",
    detail: (line) => extractLiteralUrl(line),
  },
  {
    kind: "outbound_request",
    name: "rust_http_client",
    regex: /\breqwest::|\bHttpClient\b|\bRestTemplate\b|\bWebClient\b/,
    confidence: "medium",
    detail: (line) => extractLiteralUrl(line),
  },
  {
    kind: "outbound_request",
    name: "go_http_client",
    regex: /\bhttp\.(?:Get|Post|NewRequest|NewRequestWithContext)\s*\(/,
    confidence: "high",
    detail: (line) => extractLiteralUrl(line),
  },
  {
    kind: "outbound_request",
    name: "php_http_client",
    regex: /\bcurl_exec\s*\(|\bfile_get_contents\s*\(\s*['"]https?:\/\//i,
    confidence: "medium",
    detail: (line) => extractLiteralUrl(line),
  },
];

const UPLOAD_RULES: readonly SurfaceRule[] = [
  {
    kind: "file_upload",
    name: "multer_upload",
    regex: /\bmulter\s*\(|\bupload\.(?:single|array|fields)\s*\(/,
    confidence: "high",
    detail: () => "Multer upload handling",
  },
  {
    kind: "file_upload",
    name: "fastapi_upload",
    regex: /\b(?:UploadFile|File)\s*\(/,
    confidence: "high",
    detail: () => "FastAPI upload parameter",
  },
  {
    kind: "file_upload",
    name: "spring_upload",
    regex: /\bMultipartFile\b/,
    confidence: "high",
    detail: () => "Spring multipart file parameter",
  },
  {
    kind: "file_upload",
    name: "dotnet_upload",
    regex: /\bIFormFile\b/,
    confidence: "high",
    detail: () => "ASP.NET Core form file parameter",
  },
  {
    kind: "file_upload",
    name: "php_upload",
    regex: /\$_FILES\b|\$request->file\s*\(/,
    confidence: "high",
    detail: () => "PHP upload handling",
  },
  {
    kind: "file_upload",
    name: "go_upload",
    regex: /\bFormFile\s*\(|\bParseMultipartForm\s*\(/,
    confidence: "high",
    detail: () => "Go multipart upload handling",
  },
];

const ALL_RULES: readonly SurfaceRule[] = [
  ...AUTH_RULES,
  ...MIDDLEWARE_RULES,
  ...VALIDATOR_RULES,
  ...CONFIG_RULES,
  ...OUTBOUND_RULES,
  ...UPLOAD_RULES,
];

/**
 * Extracts low-level application surfaces from source lines. This is a
 * deterministic text scanner: it does not evaluate paths, imports, decorators,
 * or framework configuration, and every result is marked as source evidence.
 */
export function extractSurfaces(document: SourceDocument, framework: string): SurfaceFact[] {
  const surfaces: SurfaceFact[] = [];
  const seen = new Set<string>();
  for (let index = 0; index < document.lines.length; index += 1) {
    const line = document.lines[index] ?? "";
    for (const rule of ALL_RULES) {
      const match = rule.regex.exec(line);
      if (match === null) continue;
      const token = match[1] ?? rule.name;
      const name = safeText(token, 100) ?? rule.name;
      const key = `${document.path}:${index + 1}:${rule.kind}:${name}:${rule.name}`;
      if (seen.has(key)) continue;
      seen.add(key);
      surfaces.push(
        makeSurface(document, {
          startLine: index + 1,
          kind: rule.kind,
          framework,
          name,
          detail: rule.detail?.(line, match) ?? null,
          confidence: rule.confidence,
        }),
      );
    }
  }
  return surfaces;
}
