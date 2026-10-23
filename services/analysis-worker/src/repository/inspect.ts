import { lstat, readFile, realpath } from "node:fs/promises";
import { basename, extname, isAbsolute, relative, resolve } from "node:path";
import { RepositoryIngestError } from "./errors";
import { containsControlCharacter, stripUnsafeControls } from "./text";
import { assertSafeRepositoryPath, type TreeInventory } from "./tree";

export type ManifestEcosystem =
  | "dotnet"
  | "go"
  | "java"
  | "node"
  | "php"
  | "python"
  | "ruby"
  | "rust"
  | "unknown";

export interface DependencyInventory {
  readonly name: string;
  readonly version: string | null;
}

export interface ManifestInventoryItem {
  readonly path: string;
  readonly ecosystem: ManifestEcosystem;
  readonly name: string | null;
  readonly version: string | null;
  readonly packageManager: string | null;
  readonly scriptNames: readonly string[];
  readonly dependencies: readonly DependencyInventory[];
  readonly truncated: boolean;
  readonly parseError: string | null;
}

export interface LockfileInventoryItem {
  readonly path: string;
  readonly ecosystem: ManifestEcosystem;
  readonly sizeBytes: number;
  readonly parsed: boolean;
  readonly packageCount: number;
  readonly dependencies: readonly DependencyInventory[];
  readonly truncated: boolean;
  readonly parseError: string | null;
}

export interface ManifestInventory {
  readonly manifests: readonly ManifestInventoryItem[];
  readonly lockfiles: readonly LockfileInventoryItem[];
  readonly truncated: boolean;
}

export type SnapshotFileKind = "binary" | "oversized" | "text";

export interface SnapshotFile {
  readonly path: string;
  readonly sizeBytes: number;
  readonly kind: SnapshotFileKind;
  readonly language: string | null;
}

export interface InspectionOptions {
  readonly maxFileBytes: number;
  readonly maxInventoryFiles: number;
  readonly maxDependenciesPerManifest: number;
  readonly signal?: AbortSignal;
}

export interface InspectionResult {
  readonly files: readonly SnapshotFile[];
  readonly detectedLanguages: readonly string[];
  readonly frameworks: readonly string[];
  readonly manifestInventory: ManifestInventory;
  readonly textFileCount: number;
  readonly binaryFileCount: number;
  readonly oversizedFileCount: number;
  readonly generatedExcludedCount: number;
  readonly symlinkCount: number;
  readonly submoduleCount: number;
  readonly filesTruncated: boolean;
}

export interface ReadRepositoryFileOptions {
  readonly maxBytes: number;
  readonly signal?: AbortSignal;
}

export interface ReadRepositoryFileResult {
  readonly path: string;
  readonly text: string;
  readonly sizeBytes: number;
  readonly language: string | null;
}

const GENERATED_DIRECTORIES = new Set([
  ".cache",
  ".git",
  ".next",
  ".nuxt",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "obj",
  "out",
  "target",
  "vendor",
  "__pycache__",
]);

const LANGUAGE_BY_EXTENSION: Readonly<Record<string, string>> = {
  ".c": "C",
  ".cc": "C++",
  ".cpp": "C++",
  ".cs": "C#",
  ".css": "CSS",
  ".go": "Go",
  ".h": "C",
  ".hpp": "C++",
  ".html": "HTML",
  ".java": "Java",
  ".js": "JavaScript",
  ".json": "JSON",
  ".jsx": "JavaScript",
  ".kt": "Kotlin",
  ".kts": "Kotlin",
  ".md": "Markdown",
  ".mjs": "JavaScript",
  ".cjs": "JavaScript",
  ".php": "PHP",
  ".py": "Python",
  ".rb": "Ruby",
  ".rs": "Rust",
  ".scala": "Scala",
  ".sh": "Shell",
  ".sql": "SQL",
  ".svelte": "Svelte",
  ".swift": "Swift",
  ".toml": "TOML",
  ".ts": "TypeScript",
  ".tsx": "TypeScript",
  ".vue": "Vue",
  ".xml": "XML",
  ".yaml": "YAML",
  ".yml": "YAML",
};

const LANGUAGE_BY_FILENAME: Readonly<Record<string, string>> = {
  Dockerfile: "Docker",
  Gemfile: "Ruby",
  Makefile: "Make",
  Rakefile: "Ruby",
};

const FRAMEWORK_BY_DEPENDENCY: Readonly<Record<string, string>> = {
  "@angular/core": "Angular",
  "@nestjs/core": "NestJS",
  "@remix-run/react": "Remix",
  "actix-web": "Actix Web",
  astro: "Astro",
  axum: "Axum",
  django: "Django",
  electron: "Electron",
  express: "Express",
  fastapi: "FastAPI",
  fastify: "Fastify",
  flask: "Flask",
  "github.com/gin-gonic/gin": "Gin",
  "github.com/gofiber/fiber/v2": "Fiber",
  "github.com/labstack/echo/v4": "Echo",
  koa: "Koa",
  "laravel/framework": "Laravel",
  next: "Next.js",
  nuxt: "Nuxt",
  rails: "Ruby on Rails",
  react: "React",
  "react-dom": "React",
  rocket: "Rocket",
  sinatra: "Sinatra",
  "spring-boot-starter-web": "Spring Boot",
  svelte: "Svelte",
  "symfony/framework-bundle": "Symfony",
  tonic: "Tonic",
  vue: "Vue",
  warp: "Warp",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function safeString(value: unknown, maxLength = 200): string | null {
  if (typeof value !== "string") return null;
  const cleaned = stripUnsafeControls(value).trim();
  if (cleaned.length === 0) return null;
  return cleaned.slice(0, maxLength);
}

function safeName(value: unknown): string | null {
  if (typeof value !== "string") return null;
  if (!/^[@A-Za-z0-9._/-]{1,200}$/.test(value)) return null;
  return value;
}

function safeScriptName(value: unknown): string | null {
  if (typeof value !== "string" || value.length === 0 || value.length > 120) return null;
  if (containsControlCharacter(value)) return null;
  return value;
}

function dependency(name: string, version: unknown): DependencyInventory | null {
  const safe = safeName(name);
  if (safe === null) return null;
  return { name: safe, version: safeString(version, 100) };
}

function collectObjectDependencies(value: unknown, max: number): DependencyInventory[] {
  if (!isRecord(value)) return [];
  const result: DependencyInventory[] = [];
  for (const [name, version] of Object.entries(value)) {
    if (result.length >= max) break;
    const item = dependency(name, version);
    if (item !== null) result.push(item);
  }
  return result;
}

export function languageForPath(path: string): string | null {
  const filename = basename(path);
  const byName = LANGUAGE_BY_FILENAME[filename];
  if (byName !== undefined) return byName;
  return LANGUAGE_BY_EXTENSION[extname(filename).toLowerCase()] ?? null;
}

export function manifestEcosystem(path: string): ManifestEcosystem | null {
  const name = basename(path).toLowerCase();
  if (name === "package.json") return "node";
  if (name === "cargo.toml") return "rust";
  if (name === "pyproject.toml" || /^requirements.*\.txt$/.test(name)) return "python";
  if (name === "go.mod") return "go";
  if (name === "pom.xml" || name === "build.gradle" || name === "build.gradle.kts") return "java";
  if (name === "gemfile") return "ruby";
  if (name === "composer.json") return "php";
  if (name.endsWith(".csproj") || name.endsWith(".fsproj") || name.endsWith(".vbproj")) {
    return "dotnet";
  }
  return null;
}

export function lockfileEcosystem(path: string): ManifestEcosystem | null {
  const name = basename(path).toLowerCase();
  if (
    name === "package-lock.json" ||
    name === "bun.lock" ||
    name === "bun.lockb" ||
    name === "pnpm-lock.yaml" ||
    name === "yarn.lock"
  ) {
    return "node";
  }
  if (name === "cargo.lock") return "rust";
  if (
    name === "poetry.lock" ||
    name === "pipfile.lock" ||
    name === "requirements.lock" ||
    name === "uv.lock"
  ) {
    return "python";
  }
  if (name === "go.sum") return "go";
  if (name === "composer.lock") return "php";
  if (name === "gemfile.lock") return "ruby";
  if (name === "packages.lock.json") return "dotnet";
  return null;
}

function parseNodeManifest(path: string, text: string, max: number): ManifestInventoryItem {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return {
      path,
      ecosystem: "node",
      name: null,
      version: null,
      packageManager: null,
      scriptNames: [],
      dependencies: [],
      truncated: false,
      parseError: "invalid_json",
    };
  }
  if (!isRecord(parsed)) {
    return {
      path,
      ecosystem: "node",
      name: null,
      version: null,
      packageManager: null,
      scriptNames: [],
      dependencies: [],
      truncated: false,
      parseError: "invalid_shape",
    };
  }
  const dependencies: DependencyInventory[] = [];
  let truncated = false;
  for (const section of [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
  ]) {
    const collected = collectObjectDependencies(parsed[section], max - dependencies.length);
    dependencies.push(...collected);
    if (dependencies.length >= max) {
      truncated = true;
      break;
    }
  }
  const scripts = isRecord(parsed.scripts)
    ? Object.keys(parsed.scripts)
        .map((name) => safeScriptName(name))
        .filter((name): name is string => name !== null)
    : [];
  return {
    path,
    ecosystem: "node",
    name: safeString(parsed.name),
    version: safeString(parsed.version, 100),
    packageManager: safeString(parsed.packageManager, 100),
    scriptNames: scripts.slice(0, 50),
    dependencies,
    truncated,
    parseError: null,
  };
}

function parseQuotedDependencies(text: string, max: number): DependencyInventory[] {
  const result: DependencyInventory[] = [];
  for (const match of text.matchAll(/["']([A-Za-z0-9@][A-Za-z0-9@._/-]*)([^"'\n]*)["']/g)) {
    if (result.length >= max) break;
    const name = match[1];
    if (name === undefined) continue;
    result.push({ name, version: safeString(match[2], 100) });
  }
  return result;
}

function parseCargoManifest(path: string, text: string, max: number): ManifestInventoryItem {
  const dependencies: DependencyInventory[] = [];
  let section = "";
  let name: string | null = null;
  let version: string | null = null;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[") && line.endsWith("]")) {
      section = line.slice(1, -1).trim();
      continue;
    }
    if (line.length === 0 || line.startsWith("#")) continue;
    if (section === "package") {
      const match = /^(name|version)\s*=\s*"([^"]+)"/.exec(line);
      if (match?.[1] === "name") name = safeString(match[2]);
      if (match?.[1] === "version") version = safeString(match[2], 100);
      continue;
    }
    if (
      section === "dependencies" ||
      section === "dev-dependencies" ||
      section === "build-dependencies" ||
      section === "workspace.dependencies"
    ) {
      const match = /^([A-Za-z0-9_-]+)\s*=\s*(.+)$/.exec(line);
      if (match?.[1] !== undefined) {
        const versionMatch = /version\s*=\s*"([^"]+)"/.exec(match[2] ?? "");
        dependencies.push({
          name: match[1],
          version: versionMatch?.[1] ?? safeString(match[2]?.replace(/^["']|["']$/g, ""), 100),
        });
        if (dependencies.length >= max) break;
      }
    }
  }
  return {
    path,
    ecosystem: "rust",
    name,
    version,
    packageManager: "cargo",
    scriptNames: [],
    dependencies,
    truncated: dependencies.length >= max,
    parseError: null,
  };
}

function parsePythonManifest(
  path: string,
  text: string,
  max: number,
  ecosystem: ManifestEcosystem,
): ManifestInventoryItem {
  const dependencies: DependencyInventory[] = [];
  let name: string | null = null;
  let version: string | null = null;
  const filename = basename(path).toLowerCase();
  if (filename === "pyproject.toml") {
    let inProject = false;
    for (const rawLine of text.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (line.startsWith("[") && line.endsWith("]")) {
        inProject = line === "[project]";
        continue;
      }
      if (!inProject) continue;
      const nameMatch = /^(name|version)\s*=\s*"([^"]+)"/.exec(line);
      if (nameMatch?.[1] === "name") name = safeString(nameMatch[2]);
      if (nameMatch?.[1] === "version") version = safeString(nameMatch[2], 100);
      if (line.startsWith("dependencies") || line.startsWith("[")) {
        const collected = parseQuotedDependencies(line, max - dependencies.length);
        dependencies.push(...collected);
      }
    }
  } else {
    for (const rawLine of text.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (line.length === 0 || line.startsWith("#") || line.startsWith("-")) continue;
      const match = /^([A-Za-z0-9][A-Za-z0-9._-]*)\s*(.*)$/.exec(line);
      if (match?.[1] === undefined) continue;
      dependencies.push({ name: match[1], version: safeString(match[2], 100) });
      if (dependencies.length >= max) break;
    }
  }
  return {
    path,
    ecosystem,
    name,
    version,
    packageManager: filename === "pyproject.toml" ? "python" : "pip",
    scriptNames: [],
    dependencies,
    truncated: dependencies.length >= max,
    parseError: null,
  };
}

function parseGoMod(path: string, text: string, max: number): ManifestInventoryItem {
  const dependencies: DependencyInventory[] = [];
  let name: string | null = null;
  let inRequire = false;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("module ")) name = safeString(line.slice("module ".length));
    if (line === "require (") {
      inRequire = true;
      continue;
    }
    if (inRequire && line === ")") {
      inRequire = false;
      continue;
    }
    if ((inRequire || line.startsWith("require ")) && line.length > 0) {
      const value = line.startsWith("require ") ? line.slice("require ".length) : line;
      const [depName, depVersion] = value.split(/\s+/);
      if (depName !== undefined && depName !== "(" && depName !== ")") {
        dependencies.push({ name: depName, version: safeString(depVersion, 100) });
        if (dependencies.length >= max) break;
      }
    }
  }
  return {
    path,
    ecosystem: "go",
    name,
    version: null,
    packageManager: "go",
    scriptNames: [],
    dependencies,
    truncated: dependencies.length >= max,
    parseError: null,
  };
}

function parseTextManifest(
  path: string,
  text: string,
  ecosystem: ManifestEcosystem,
  max: number,
): ManifestInventoryItem {
  const dependencies: DependencyInventory[] = [];
  let name: string | null = null;
  let version: string | null = null;
  const filename = basename(path).toLowerCase();
  if (filename === "pom.xml") {
    const artifactIds = [...text.matchAll(/<artifactId>\s*([^<\s]+)\s*<\/artifactId>/g)]
      .map((match) => match[1])
      .filter((value): value is string => value !== undefined)
      .slice(0, max);
    name = artifactIds[0] ?? null;
    for (const artifact of artifactIds) dependencies.push({ name: artifact, version: null });
  } else if (filename === "build.gradle" || filename === "build.gradle.kts") {
    for (const match of text.matchAll(
      /(?:implementation|api|compileOnly|runtimeOnly)\s*\(?\s*["']([^"']+)["']/g,
    )) {
      if (dependencies.length >= max) break;
      const value = match[1];
      if (value !== undefined) dependencies.push({ name: value, version: null });
    }
  } else if (filename === "gemfile") {
    for (const match of text.matchAll(/^\s*gem\s+["']([^"']+)["'](?:\s*,\s*["']([^"']+)["'])?/gm)) {
      if (dependencies.length >= max) break;
      const depName = match[1];
      if (depName !== undefined)
        dependencies.push({ name: depName, version: safeString(match[2], 100) });
    }
  } else if (
    filename.endsWith(".csproj") ||
    filename.endsWith(".fsproj") ||
    filename.endsWith(".vbproj")
  ) {
    for (const match of text.matchAll(
      /<PackageReference\s+Include="([^"]+)"(?:\s+Version="([^"]+)")?/g,
    )) {
      if (dependencies.length >= max) break;
      const depName = match[1];
      if (depName !== undefined)
        dependencies.push({ name: depName, version: safeString(match[2], 100) });
    }
  } else if (filename === "composer.json") {
    try {
      const parsed: unknown = JSON.parse(text);
      if (isRecord(parsed)) {
        name = safeString(parsed.name);
        version = safeString(parsed.version, 100);
        dependencies.push(...collectObjectDependencies(parsed.require, max));
      }
    } catch {
      return {
        path,
        ecosystem,
        name: null,
        version: null,
        packageManager: "composer",
        scriptNames: [],
        dependencies: [],
        truncated: false,
        parseError: "invalid_json",
      };
    }
  }
  return {
    path,
    ecosystem,
    name,
    version,
    packageManager: null,
    scriptNames: [],
    dependencies,
    truncated: dependencies.length >= max,
    parseError: null,
  };
}

export function parseManifest(
  path: string,
  text: string,
  maxDependencies: number,
): ManifestInventoryItem {
  const ecosystem = manifestEcosystem(path);
  if (ecosystem === null) {
    throw new RepositoryIngestError("invalid_manifest", "File is not a supported manifest");
  }
  const max = Math.max(1, maxDependencies);
  if (ecosystem === "node") return parseNodeManifest(path, text, max);
  if (ecosystem === "rust") return parseCargoManifest(path, text, max);
  if (ecosystem === "python") return parsePythonManifest(path, text, max, ecosystem);
  if (ecosystem === "go") return parseGoMod(path, text, max);
  if (ecosystem === "php" && basename(path).toLowerCase() === "composer.json") {
    return parseTextManifest(path, text, ecosystem, max);
  }
  return parseTextManifest(path, text, ecosystem, max);
}

function parseLockfile(path: string, text: string, maxDependencies: number): LockfileInventoryItem {
  const ecosystem = lockfileEcosystem(path) ?? "unknown";
  const max = Math.max(1, maxDependencies);
  const dependencies: DependencyInventory[] = [];
  const filename = basename(path).toLowerCase();
  let parsed = false;
  let parseError: string | null = null;
  try {
    if (filename === "package-lock.json") {
      const parsedJson: unknown = JSON.parse(text);
      if (!isRecord(parsedJson)) throw new Error("invalid shape");
      const packages = parsedJson.packages;
      if (isRecord(packages)) {
        for (const [packagePath, value] of Object.entries(packages)) {
          if (dependencies.length >= max) break;
          if (!isRecord(value) || packagePath.length === 0) continue;
          const marker = "node_modules/";
          const index = packagePath.lastIndexOf(marker);
          const name = index >= 0 ? packagePath.slice(index + marker.length) : packagePath;
          dependencies.push({ name, version: safeString(value.version, 100) });
        }
      }
      parsed = true;
    } else if (filename === "cargo.lock") {
      let currentName: string | null = null;
      let currentVersion: string | null = null;
      for (const rawLine of text.split(/\r?\n/)) {
        const line = rawLine.trim();
        if (line === "[[package]]") {
          if (currentName !== null) {
            dependencies.push({ name: currentName, version: currentVersion });
            if (dependencies.length >= max) break;
          }
          currentName = null;
          currentVersion = null;
        } else if (line.startsWith("name = ")) {
          currentName = safeName(line.slice("name = ".length).replace(/^"|"$/g, ""));
        } else if (line.startsWith("version = ")) {
          currentVersion = safeString(line.slice("version = ".length).replace(/^"|"$/g, ""), 100);
        }
      }
      if (currentName !== null && dependencies.length < max) {
        dependencies.push({ name: currentName, version: currentVersion });
      }
      parsed = true;
    } else if (filename === "go.sum") {
      for (const line of text.split(/\r?\n/)) {
        if (dependencies.length >= max) break;
        const [name, version] = line.trim().split(/\s+/);
        if (name !== undefined && version !== undefined) {
          dependencies.push({ name, version: safeString(version, 100) });
        }
      }
      parsed = true;
    } else if (filename === "composer.lock") {
      const parsedJson: unknown = JSON.parse(text);
      if (!isRecord(parsedJson)) throw new Error("invalid shape");
      for (const key of ["packages", "packages-dev"]) {
        const values = parsedJson[key];
        if (!Array.isArray(values)) continue;
        for (const value of values) {
          if (dependencies.length >= max) break;
          if (!isRecord(value)) continue;
          const name = safeName(value.name);
          if (name !== null) dependencies.push({ name, version: safeString(value.version, 100) });
        }
      }
      parsed = true;
    } else if (filename === "gemfile.lock") {
      let inSpecs = false;
      for (const line of text.split(/\r?\n/)) {
        if (line.trim() === "specs:") {
          inSpecs = true;
          continue;
        }
        if (!inSpecs) continue;
        if (dependencies.length >= max) break;
        const match = /^\s{4}([A-Za-z0-9_.-]+)\s+\(([^)]+)\)/.exec(line);
        if (match?.[1] !== undefined) {
          dependencies.push({ name: match[1], version: safeString(match[2], 100) });
        }
      }
      parsed = true;
    } else {
      parseError = "metadata_only";
    }
  } catch {
    parseError = "invalid_lockfile";
  }
  return {
    path,
    ecosystem,
    sizeBytes: Buffer.byteLength(text, "utf8"),
    parsed,
    packageCount: dependencies.length,
    dependencies,
    truncated: dependencies.length >= max,
    parseError,
  };
}

function decodeText(buffer: Buffer): string | null {
  if (buffer.includes(0)) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(buffer);
  } catch {
    return null;
  }
}

function assertInsideRoot(root: string, path: string): void {
  const rel = relative(root, path);
  if (rel.length === 0 || rel.startsWith("..") || isAbsolute(rel)) {
    throw new RepositoryIngestError("unsafe_path", "Repository path escapes the workspace");
  }
}

async function readBounded(
  root: string,
  realRoot: string,
  path: string,
  maxBytes: number,
): Promise<Buffer> {
  assertSafeRepositoryPath(path);
  const full = resolve(root, ...path.split("/"));
  assertInsideRoot(root, full);
  const info = await lstat(full);
  if (info.isSymbolicLink()) {
    throw new RepositoryIngestError("unsafe_path", "Repository path is a symbolic link");
  }
  if (!info.isFile()) {
    throw new RepositoryIngestError("not_a_file", "Repository path is not a regular file");
  }
  if (info.size > maxBytes) {
    throw new RepositoryIngestError("file_too_large", "Repository file exceeds the read limit");
  }
  const realFull = await realpath(full);
  assertInsideRoot(realRoot, realFull);
  const buffer = await readFile(realFull);
  if (buffer.byteLength > maxBytes) {
    throw new RepositoryIngestError("file_too_large", "Repository file exceeds the read limit");
  }
  return buffer;
}

export async function readRepositoryFile(
  root: string,
  path: string,
  options: ReadRepositoryFileOptions,
): Promise<ReadRepositoryFileResult> {
  if (options.signal?.aborted) {
    throw new RepositoryIngestError("cancelled", "Repository ingestion was cancelled");
  }
  if (!Number.isSafeInteger(options.maxBytes) || options.maxBytes < 1) {
    throw new RepositoryIngestError("file_too_large", "Repository read limit is invalid");
  }
  const realRoot = await realpath(root);
  const buffer = await readBounded(root, realRoot, path, options.maxBytes);
  const text = decodeText(buffer);
  if (text === null) {
    throw new RepositoryIngestError("binary_file", "Repository file is not valid UTF-8 text");
  }
  return { path, text, sizeBytes: buffer.byteLength, language: languageForPath(path) };
}

function shouldSkipGenerated(path: string): boolean {
  const segments = path.split("/");
  return segments.slice(0, -1).some((segment) => GENERATED_DIRECTORIES.has(segment.toLowerCase()));
}

function detectFrameworks(
  manifests: readonly ManifestInventoryItem[],
  files: readonly SnapshotFile[],
): string[] {
  const found = new Set<string>();
  for (const manifest of manifests) {
    for (const dependency of manifest.dependencies) {
      const framework = FRAMEWORK_BY_DEPENDENCY[dependency.name.toLowerCase()];
      if (framework !== undefined) found.add(framework);
    }
  }
  for (const file of files) {
    const lower = file.path.toLowerCase();
    if (lower.endsWith("next.config.js") || lower.endsWith("next.config.mjs")) found.add("Next.js");
    if (lower.endsWith("vite.config.ts") || lower.endsWith("vite.config.js")) found.add("Vite");
    if (lower.endsWith("manage.py")) found.add("Django");
    if (lower.endsWith("artisan")) found.add("Laravel");
  }
  return [...found].sort((left, right) => left.localeCompare(right));
}

export async function inspectTree(
  root: string,
  tree: TreeInventory,
  options: InspectionOptions,
): Promise<InspectionResult> {
  if (options.signal?.aborted) {
    throw new RepositoryIngestError("cancelled", "Repository ingestion was cancelled");
  }
  const realRoot = await realpath(root);
  const files: SnapshotFile[] = [];
  const manifests: ManifestInventoryItem[] = [];
  const lockfiles: LockfileInventoryItem[] = [];
  const languageBytes = new Map<string, { files: number; bytes: number }>();
  let textFileCount = 0;
  let binaryFileCount = 0;
  let oversizedFileCount = 0;
  let generatedExcludedCount = 0;
  let manifestTruncated = false;

  for (const entry of tree.entries) {
    if (options.signal?.aborted) {
      throw new RepositoryIngestError("cancelled", "Repository ingestion was cancelled");
    }
    if (entry.kind !== "blob" || shouldSkipGenerated(entry.path)) {
      if (entry.kind === "blob") generatedExcludedCount += 1;
      continue;
    }
    const language = languageForPath(entry.path);
    if (entry.sizeBytes > options.maxFileBytes) {
      files.push({ path: entry.path, sizeBytes: entry.sizeBytes, kind: "oversized", language });
      oversizedFileCount += 1;
      continue;
    }

    let buffer: Buffer;
    try {
      buffer = await readBounded(root, realRoot, entry.path, options.maxFileBytes);
    } catch (error) {
      if (error instanceof RepositoryIngestError && error.code === "file_too_large") {
        files.push({ path: entry.path, sizeBytes: entry.sizeBytes, kind: "oversized", language });
        oversizedFileCount += 1;
        continue;
      }
      throw error;
    }
    const text = decodeText(buffer);
    if (text === null) {
      files.push({ path: entry.path, sizeBytes: buffer.byteLength, kind: "binary", language });
      binaryFileCount += 1;
      continue;
    }
    files.push({ path: entry.path, sizeBytes: buffer.byteLength, kind: "text", language });
    textFileCount += 1;
    if (language !== null) {
      const current = languageBytes.get(language) ?? { files: 0, bytes: 0 };
      languageBytes.set(language, {
        files: current.files + 1,
        bytes: current.bytes + buffer.byteLength,
      });
    }

    if (manifestEcosystem(entry.path) !== null) {
      const manifest = parseManifest(entry.path, text, options.maxDependenciesPerManifest);
      manifests.push(manifest);
      if (manifest.truncated) manifestTruncated = true;
    } else if (lockfileEcosystem(entry.path) !== null) {
      const lockfile = parseLockfile(entry.path, text, options.maxDependenciesPerManifest);
      lockfiles.push(lockfile);
      if (lockfile.truncated) manifestTruncated = true;
    }
  }

  const detectedLanguages = [...languageBytes.entries()]
    .sort((left, right) => right[1].bytes - left[1].bytes || left[0].localeCompare(right[0]))
    .map(([language]) => language);

  files.sort((left, right) => left.path.localeCompare(right.path));
  manifests.sort((left, right) => left.path.localeCompare(right.path));
  lockfiles.sort((left, right) => left.path.localeCompare(right.path));

  const filesTruncated = files.length > options.maxInventoryFiles;
  return {
    files: files.slice(0, options.maxInventoryFiles),
    detectedLanguages,
    frameworks: detectFrameworks(manifests, files),
    manifestInventory: { manifests, lockfiles, truncated: manifestTruncated },
    textFileCount,
    binaryFileCount,
    oversizedFileCount,
    generatedExcludedCount,
    symlinkCount: tree.symlinks.length,
    submoduleCount: tree.submodules.length,
    filesTruncated,
  };
}
