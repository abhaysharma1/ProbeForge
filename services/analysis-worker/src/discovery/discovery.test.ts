import { describe, expect, it } from "bun:test";
import { join } from "node:path";
import { type GitTarget, validateGitUrl } from "../repository/git-url";
import type { RepositorySnapshot } from "../repository/ingest";
import { withRepositorySnapshot } from "../repository/ingest";
import type {
  ManifestEcosystem,
  ManifestInventoryItem,
  ReadRepositoryFileResult,
  SnapshotFile,
} from "../repository/inspect";
import { languageForPath } from "../repository/inspect";
import {
  createFixtureRepository,
  exists,
  fixtureFileUrl,
  fixtureRoot,
  type TestRepository,
} from "../repository/testing";
import type { ApplicationInventory, DiscoverySource, RouteFact, SurfaceFact } from "./index";
import { createDiscoveryToolset, DiscoveryToolError, discoverApplication } from "./index";

interface FixtureOptions {
  readonly files: Readonly<Record<string, string>>;
  readonly frameworks?: readonly string[];
  readonly extraLanguages?: readonly string[];
  readonly manifests?: readonly ManifestInventoryItem[];
  readonly filesTruncated?: boolean;
}

function createSource(options: FixtureOptions): DiscoverySource {
  const files: SnapshotFile[] = Object.entries(options.files).map(([path, text]) => ({
    path,
    sizeBytes: Buffer.byteLength(text),
    kind: "text",
    language: languageForPath(path),
  }));
  const detectedLanguages = [
    ...new Set([
      ...files
        .map((file) => file.language)
        .filter((language): language is string => language !== null),
      ...(options.extraLanguages ?? []),
    ]),
  ].sort();
  const manifests = options.manifests ?? [];
  const totalBytes = files.reduce((sum, file) => sum + file.sizeBytes, 0);
  const snapshot: RepositorySnapshot = {
    schemaVersion: "1",
    commitSha: "a".repeat(40),
    sourceReference: "main",
    provider: "generic",
    detectedLanguages,
    frameworks: options.frameworks ?? [],
    manifestInventory: {
      manifests,
      lockfiles: [],
      truncated: false,
    },
    analysisMetadata: {
      schemaVersion: "1",
      generatorVersion: "test",
      gitVersion: "test",
      fileCount: files.length,
      totalBytes,
      inventoryFileCount: 0,
      textFileCount: files.length,
      binaryFileCount: 0,
      oversizedFileCount: 0,
      generatedExcludedCount: 0,
      symlinkCount: 0,
      submoduleCount: 0,
      manifestCount: manifests.length,
      lockfileCount: 0,
      filesTruncated: options.filesTruncated ?? false,
      manifestInventoryTruncated: false,
      limits: {
        maxFiles: 500,
        maxTotalBytes: 10 * 1024 * 1024,
        maxFileBytes: 512 * 1024,
        maxDepth: 20,
        maxInventoryFiles: 100,
        maxDependenciesPerManifest: 100,
        maxWorkspaceBytes: 10 * 1024 * 1024,
        maxGitOutputBytes: 1024 * 1024,
        gitTimeoutMs: 30_000,
      },
      untrustedContent: true,
      executedRepositoryCode: false,
    },
    files,
    filesTruncated: options.filesTruncated ?? false,
    symlinks: [],
    submodules: [],
    untrustedContent: true,
  };
  return {
    snapshot,
    readFile: async (path, readOptions) => {
      const text = options.files[path];
      if (text === undefined) throw new Error(`missing fixture file: ${path}`);
      const sizeBytes = Buffer.byteLength(text);
      if (readOptions?.maxBytes !== undefined && sizeBytes > readOptions.maxBytes) {
        throw new Error("fixture file exceeds read limit");
      }
      return {
        path,
        text,
        sizeBytes,
        language: languageForPath(path),
      } satisfies ReadRepositoryFileResult;
    },
  };
}

function manifest(
  path: string,
  ecosystem: ManifestEcosystem,
  dependencies: ReadonlyArray<readonly [string, string | null]>,
): ManifestInventoryItem {
  return {
    path,
    ecosystem,
    name: "fixture",
    version: "1.0.0",
    packageManager: null,
    scriptNames: [],
    dependencies: dependencies.map(([name, version]) => ({ name, version })),
    truncated: false,
    parseError: null,
  };
}

function paths(routes: readonly RouteFact[]): string[] {
  return routes.map((route) => `${route.method} ${route.path}`);
}

function surfacesOf(inventory: ApplicationInventory, kind: SurfaceFact["kind"]): SurfaceFact[] {
  return inventory.surfaces.filter((surface) => surface.kind === kind);
}

function targetFor(repo: TestRepository): GitTarget {
  return validateGitUrl(fixtureFileUrl(repo.root), {
    allowedHosts: [],
    allowLocalFixtures: true,
    localFixtureRoot: fixtureRoot(repo.root),
  });
}

describe("static discovery", () => {
  it("extracts Express routes and related surfaces with evidence", async () => {
    const dynamicSegment = `\${userId}`;
    const source = createSource({
      frameworks: ["Express"],
      manifests: [manifest("package.json", "node", [["express", "5.0.0"]])],
      files: {
        "src/server.ts": [
          'import express from "express";',
          'import { requireAuth } from "./auth";',
          "const app = express();",
          "const router = express.Router();",
          "app.use(express.json());",
          'app.get("/health", function health() {});',
          'app.post("/api/users", requireAuth, body("email").isEmail(), upload.single("avatar"), createUser);',
          'cors({ origin: "*" });',
          "const agent = new https.Agent({ rejectUnauthorized: false });",
          `app.get(\`/api/users/${dynamicSegment}\`, handler);`,
          'fetch("https://api.example.test/v1/users?token=super-secret", { method: "GET" });',
        ].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    expect(inventory.executedRepositoryCode).toBe(false);
    expect(inventory.untrustedContent).toBe(true);
    expect(inventory.coverage.examinedFiles).toBe(1);
    expect(inventory.frameworks.map((framework) => framework.name)).toContain("Express");
    expect(paths(inventory.routes)).toEqual(
      expect.arrayContaining([
        "GET /health",
        "POST /api/users",
        `GET /api/users/${dynamicSegment}`,
      ]),
    );

    const dynamicRoute = inventory.routes.find((route) => route.path?.includes(dynamicSegment));
    expect(dynamicRoute?.ambiguous).toBe(true);
    expect(dynamicRoute?.pathKind).toBe("dynamic");
    expect(dynamicRoute?.evidence.file).toBe("src/server.ts");
    expect(dynamicRoute?.evidence.startLine).toBe(10);

    const health = inventory.routes.find((route) => route.path === "/health");
    expect(health?.handler).toBe("health");
    expect(health?.provenance).toBe("observed");
    expect(health?.confidence).toBe("high");

    expect(surfacesOf(inventory, "middleware").length).toBeGreaterThan(0);
    expect(
      surfacesOf(inventory, "auth_check").some((surface) => surface.name === "requireAuth"),
    ).toBe(true);
    expect(
      surfacesOf(inventory, "schema_validator").some((surface) => surface.name === "body"),
    ).toBe(true);
    expect(
      surfacesOf(inventory, "file_upload").some((surface) => surface.name === "multer_upload"),
    ).toBe(true);
    expect(
      surfacesOf(inventory, "configuration").some((surface) => surface.name === "cors_wildcard"),
    ).toBe(true);
    expect(
      surfacesOf(inventory, "configuration").some(
        (surface) => surface.name === "tls_verification_disabled",
      ),
    ).toBe(true);
    const outbound = inventory.surfaces.find((surface) => surface.name === "fetch");
    expect(outbound?.detail).toBe("https://api.example.test/v1/users");
    expect(
      inventory.surfaces.some(
        (surface) => surface.kind === "dependency" && surface.name === "express",
      ),
    ).toBe(true);
    expect(JSON.stringify(inventory)).not.toContain("super-secret");
  });

  it("extracts FastAPI, Flask, and Django routes and marks unknown Django methods ambiguous", async () => {
    const source = createSource({
      frameworks: ["FastAPI", "Flask", "Django"],
      files: {
        "app/main.py": [
          "from fastapi import FastAPI, Depends, UploadFile, File",
          "from pydantic import BaseModel",
          "app = FastAPI()",
          "class Payload(BaseModel):",
          "    name: str",
          '@app.get("/fixture")',
          "async def fixture():",
          "    return {}",
          '@app.post("/upload")',
          "async def upload(file: UploadFile = File(...), user=Depends(get_current_user)):",
          "    return {}",
        ].join("\n"),
        "app/flask_app.py": [
          "from flask import Flask",
          "app = Flask(__name__)",
          '@app.route("/flask", methods=["GET", "POST"])',
          "def flask_view():",
          "    return {}",
        ].join("\n"),
        "app/urls.py": [
          "from django.urls import path, include",
          "urlpatterns = [",
          '    path("users/<int:id>/", views.user),',
          '    path("api/", include("api.urls")),',
          "]",
        ].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    expect(paths(inventory.routes)).toEqual(
      expect.arrayContaining(["GET /fixture", "POST /upload", "GET /flask", "POST /flask"]),
    );
    const django = inventory.routes.find((route) => route.path === "/users/<int:id>/");
    expect(django?.framework).toBe("Django");
    expect(django?.method).toBe("UNKNOWN");
    expect(django?.ambiguous).toBe(true);
    expect(inventory.ambiguities.some((item) => item.message.includes("Included URLconf"))).toBe(
      true,
    );
    expect(surfacesOf(inventory, "auth_check").some((surface) => surface.name === "Depends")).toBe(
      true,
    );
    expect(
      surfacesOf(inventory, "schema_validator").some((surface) => surface.name === "Payload"),
    ).toBe(true);
    expect(surfacesOf(inventory, "file_upload").length).toBeGreaterThan(0);
  });

  it("extracts routes from Rust, Java, Go, Ruby, PHP, and C# fixtures", async () => {
    const source = createSource({
      frameworks: ["Axum", "Spring Boot", "Gin", "Ruby on Rails", "Laravel", "ASP.NET Core"],
      files: {
        "src/main.rs": [
          "use axum::{routing::{get, post}, Router};",
          'let app = Router::new().route("/rust", get(rust_get).post(rust_post));',
        ].join("\n"),
        "src/UserController.java": [
          "@RestController",
          '@RequestMapping("/api")',
          "public class UserController {",
          '  @GetMapping("/users")',
          "  public List<User> users() { return null; }",
          '  @PostMapping(value = "/users")',
          "  public User create() { return null; }",
          "}",
        ].join("\n"),
        "main.go": ["r := gin.New()", 'r.GET("/go", goGet)', 'r.POST("/go", goPost)'].join("\n"),
        "config/routes.rb": [
          "Rails.application.routes.draw do",
          '  get "/ruby", to: "home#index"',
          "  resources :users",
          "end",
        ].join("\n"),
        "routes/web.php": [
          "<?php",
          "Route::get('/php', [HomeController::class, 'index']);",
          "Route::resource('photos', PhotoController::class);",
        ].join("\n"),
        "Program.cs": [
          "var builder = WebApplication.CreateBuilder(args);",
          "var app = builder.Build();",
          'app.MapGet("/csharp", () => "ok");',
          'app.MapPost("/csharp", () => "ok");',
        ].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    expect(paths(inventory.routes)).toEqual(
      expect.arrayContaining([
        "GET /rust",
        "POST /rust",
        "GET /api/users",
        "POST /api/users",
        "GET /go",
        "POST /go",
        "GET /ruby",
        "GET /php",
        "GET /csharp",
        "POST /csharp",
      ]),
    );
    expect(
      inventory.routes.some((route) => route.path === "/users" && route.method === "POST"),
    ).toBe(true);
    expect(
      inventory.routes.some((route) => route.path === "/users/:id" && route.method === "DELETE"),
    ).toBe(true);
    expect(
      inventory.routes.some((route) => route.path === "/photos/{id}" && route.method === "PUT"),
    ).toBe(true);
    expect(inventory.frameworks.map((framework) => framework.name)).toEqual(
      expect.arrayContaining([
        "Axum",
        "Spring Boot",
        "Gin",
        "Ruby on Rails",
        "Laravel",
        "ASP.NET Core",
      ]),
    );
  });

  it("extracts additional supported patterns for Actix, net/http, Sinatra, Symfony, and Starlette", async () => {
    const source = createSource({
      frameworks: ["Actix Web", "Go net/http", "Sinatra", "Symfony", "Starlette"],
      files: {
        "src/actix.rs": [
          "use actix_web::{get, post};",
          '#[get("/actix")]',
          "async fn actix_get() -> impl Responder { todo!() }",
          '#[post("/actix")]',
          "async fn actix_post() -> impl Responder { todo!() }",
        ].join("\n"),
        "src/net.go": [
          "package main",
          'import "net/http"',
          "func main() {",
          '  http.HandleFunc("/net", handler)',
          "}",
        ].join("\n"),
        "src/app.rb": ["require 'sinatra'", "get '/sinatra' do", '  "ok"', "end"].join("\n"),
        "src/Controller.php": [
          "<?php",
          "use Symfony\\Component\\Routing\\Attribute\\Route;",
          "class Controller {",
          "  #[Route('/symfony', name: 'app_symfony', methods: ['GET'])]",
          "  public function index(): Response {}",
          "}",
        ].join("\n"),
        "src/starlette_app.py": [
          "from starlette.applications import Starlette",
          "from starlette.routing import Route",
          "async def homepage(request):",
          "    return None",
          'routes = [Route("/starlette", endpoint=homepage, methods=["GET"])]',
          "app = Starlette(routes=routes)",
        ].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    expect(paths(inventory.routes)).toEqual(
      expect.arrayContaining([
        "GET /actix",
        "POST /actix",
        "UNKNOWN /net",
        "GET /sinatra",
        "GET /symfony",
        "GET /starlette",
      ]),
    );
  });

  it("extracts NestJS, Fastify, Hono, and Koa route registrations", async () => {
    const source = createSource({
      frameworks: ["NestJS", "Fastify", "Hono", "Koa"],
      files: {
        "src/users.controller.ts": [
          '@Controller("users")',
          "export class UsersController {",
          '  @Get(":id")',
          "  async findAll() { return []; }",
          "}",
        ].join("\n"),
        "src/fastify.ts": ["const app = Fastify();", 'app.get("/fast", handler);'].join("\n"),
        "src/hono.ts": ["const app = new Hono();", 'app.get("/hono", handler);'].join("\n"),
        "src/koa.ts": [
          "const app = new Koa();",
          "const router = new Router();",
          'router.get("/koa", handler);',
        ].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    expect(paths(inventory.routes)).toEqual(
      expect.arrayContaining(["GET /users/:id", "GET /fast", "GET /hono", "GET /koa"]),
    );
    expect(inventory.routes.find((route) => route.path === "/users/:id")?.framework).toBe("NestJS");
    expect(inventory.routes.find((route) => route.path === "/fast")?.framework).toBe("Fastify");
    expect(inventory.routes.find((route) => route.path === "/hono")?.framework).toBe("Hono");
    expect(inventory.routes.find((route) => route.path === "/koa")?.framework).toBe("Koa");
  });

  it("infers Next.js file routes and marks methods or dynamic segments as ambiguous", async () => {
    const source = createSource({
      frameworks: ["Next.js"],
      files: {
        "app/api/orders/[id]/route.ts": [
          "export async function GET(request: Request) { return new Response('ok'); }",
          "export async function POST(request: Request) { return new Response('ok'); }",
        ].join("\n"),
        "pages/api/legacy.ts": [
          "export default function handler(req, res) {",
          '  if (req.method === "DELETE") return res.end();',
          "  return res.end();",
          "}",
        ].join("\n"),
        "app/api/mystery/route.ts": ["export default function handler() {}"].join("\n"),
      },
    });

    const inventory = await discoverApplication(source);
    const dynamic = inventory.routes.filter((route) => route.path === "/api/orders/[id]");
    expect(dynamic.map((route) => route.method).sort()).toEqual(["GET", "POST"]);
    expect(dynamic.every((route) => route.provenance === "inferred")).toBe(true);
    expect(dynamic.every((route) => route.ambiguous)).toBe(true);
    expect(inventory.routes.find((route) => route.path === "/api/legacy")?.method).toBe("DELETE");
    const mystery = inventory.routes.find((route) => route.path === "/api/mystery");
    expect(mystery?.method).toBe("UNKNOWN");
    expect(mystery?.ambiguous).toBe(true);
  });

  it("tracks unsupported languages and frameworks explicitly", async () => {
    const source = createSource({
      frameworks: ["React", "Angular"],
      extraLanguages: ["Kotlin"],
      files: {
        "src/App.kt": "fun main() {}",
      },
    });

    const inventory = await discoverApplication(source);
    expect(inventory.routes).toHaveLength(0);
    expect(inventory.coverage.unsupportedLanguages).toContain("Kotlin");
    expect(inventory.coverage.unsupportedFrameworks).toEqual(
      expect.arrayContaining(["React", "Angular"]),
    );
    expect(
      inventory.unsupported.some((item) => item.kind === "language" && item.name === "Kotlin"),
    ).toBe(true);
    expect(
      inventory.unsupported.some((item) => item.kind === "framework" && item.name === "React"),
    ).toBe(true);
    expect(inventory.frameworks.filter((framework) => framework.supported)).toHaveLength(0);
  });

  it("uses generic parsers but records missing framework attribution", async () => {
    const source = createSource({
      files: {
        "src/routes.ts": ["const router = makeRouter();", 'router.get("/generic", handler);'].join(
          "\n",
        ),
      },
    });

    const inventory = await discoverApplication(source);
    expect(inventory.routes.some((route) => route.path === "/generic")).toBe(true);
    expect(inventory.ambiguities.some((item) => item.kind === "framework")).toBe(true);
  });

  it("enforces route, surface, and source limits with explicit truncation", async () => {
    const lines = Array.from({ length: 12 }, (_, index) => `app.get("/r${index}", handler);`);
    const source = createSource({
      frameworks: ["Express"],
      files: {
        "src/a-server.ts": lines.join("\n"),
        "src/extra.ts": 'app.get("/extra", handler);',
      },
    });

    const inventory = await discoverApplication(source, {
      limits: {
        maxSourceFiles: 1,
        maxRoutes: 3,
        maxSurfaces: 2,
        maxFactsPerFile: 5,
      },
    });
    expect(inventory.routes).toHaveLength(3);
    expect(inventory.coverage.routesTruncated).toBe(true);
    expect(inventory.coverage.sourceFilesTruncated).toBe(true);
    expect(inventory.ambiguities.some((item) => item.message.includes("Route limit"))).toBe(true);
    expect(
      inventory.ambiguities.some((item) => item.message.includes("Source file selection")),
    ).toBe(true);
  });

  it("exposes bounded query tools for Pi", async () => {
    const source = createSource({
      frameworks: ["Express"],
      files: {
        "src/server.ts": [
          'app.get("/a", handler);',
          'app.post("/a", handler);',
          'app.get("/b", handler);',
          "app.use(requireAuth);",
        ].join("\n"),
      },
    });
    const inventory = await discoverApplication(source);
    const tools = createDiscoveryToolset(inventory);
    const summary = tools.getSummary();
    expect(summary.routeCount).toBe(3);
    expect(summary.ambiguousRouteCount).toBe(0);
    expect(summary.surfaceCounts.auth_check).toBeGreaterThan(0);

    const filtered = tools.listRoutes({ method: "GET", pathPrefix: "/a", limit: 1 });
    expect(filtered.total).toBe(1);
    expect(filtered.routes).toHaveLength(1);
    expect(filtered.truncated).toBe(false);
    expect(tools.getRoute(filtered.routes[0]?.id ?? "")).toBeDefined();
    expect(tools.getRoute("missing")).toBeNull();
    expect(tools.listSurfaces({ kind: "auth_check" }).total).toBeGreaterThan(0);
    expect(tools.listUnsupported(1).truncated).toBe(false);

    expect(() => tools.listRoutes({ limit: 0 })).toThrow(DiscoveryToolError);
    expect(() => tools.listRoutes({ pathPrefix: "bad\nprefix" })).toThrow(DiscoveryToolError);
    expect(() => tools.listSurfaces({ kind: "not-a-kind" as never })).toThrow(DiscoveryToolError);
  });

  it("produces an inventory from the inert fixture without executing repository code", async () => {
    const repo = await createFixtureRepository();
    try {
      await withRepositorySnapshot(
        {
          target: targetFor(repo),
          reference: "main",
          credentialRef: null,
          secretResolver: { resolve: async () => null },
          audit: repo.audit,
          limits: repo.limits,
          workspaceBaseDir: join(repo.tempRoot, "workspaces"),
          localFixtureRoot: fixtureRoot(repo.root),
          allowLocalFixtures: true,
        },
        async (session) => {
          const inventory = await discoverApplication(session);
          expect(inventory.commitSha).toBe(session.snapshot.commitSha);
          expect(inventory.routes.some((route) => route.path === "/fixture")).toBe(true);
          expect(inventory.routes.find((route) => route.path === "/fixture")?.framework).toBe(
            "FastAPI",
          );
          expect(inventory.executedRepositoryCode).toBe(false);
          expect(inventory.untrustedContent).toBe(true);
          expect(inventory.limits.maxRoutes).toBeGreaterThan(0);
          expect(await exists(join(session.workspacePath, "fixture-executed.txt"))).toBe(false);
        },
      );
    } finally {
      await repo.cleanup();
    }
  });
});
