#!/usr/bin/env bun
// ProbeForge repository contract check.
//
// Validates the repository contract: required files, internal links, phase
// coverage, environment-file hygiene, and high-signal secret patterns.
// Uses Node.js built-ins only, so it runs without installing dependencies.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const failures = [];
const fail = (message) => failures.push(message);
const abs = (relPath) => join(root, relPath);
const read = (relPath) => readFileSync(abs(relPath), "utf8");

const REQUIRED_FILES = [
  "README.md",
  ".gitignore",
  ".gitattributes",
  ".editorconfig",
  ".env.example",
  ".nvmrc",
  "Agents.md",
  "package.json",
  "tsconfig.base.json",
  "biome.json",
  "Cargo.toml",
  "rust-toolchain.toml",
  "docs/README.md",
  "docs/IMPLEMENTATION_CHECKLIST.md",
  "docs/DECISION_LOG.md",
  "docs/CONTRACTS.md",
  "docs/DATABASE.md",
  "docs/AUTH.md",
  "docs/JOBS.md",
  "docs/REPOSITORY_INGESTION.md",
  "docs/DISCOVERY.md",
  "docs/PLANNING.md",
  "docs/PLAN_APPROVAL.md",
  "docs/TARGET_VALIDATION.md",
  "docs/EXECUTION_ADAPTERS.md",
  "docs/06_frontend_design.md",
  "docs/frontend_design.md",
  "docs/01_what_we_are_developing.md",
  "docs/02_broad_plan.md",
  "docs/03_phase_by_phase_implementation_plan.md",
  "docs/04_tech_stack.md",
  "docs/05_starting_prompt.md",
  "docs/phases/00_recon_report.md",
  "docs/phases/01_monorepo_report.md",
  "docs/phases/02_contracts_db_report.md",
  "docs/phases/03_auth_projects_report.md",
  "docs/phases/04_jobs_queue_report.md",
  "docs/phases/05_git_ingestion_report.md",
  "docs/phases/06_static_discovery_report.md",
  "docs/phases/07_planning_report.md",
  "docs/phases/08_plan_approval_report.md",
  "docs/phases/09_target_validation_report.md",
  "docs/phases/10_execution_adapters_report.md",
  "infra/local/compose.yaml",
  "crates/contracts/Cargo.toml",
  "crates/execution-adapters/Cargo.toml",
  "crates/execution-adapters/src/lib.rs",
  "crates/queue/Cargo.toml",
  "crates/queue/src/lib.rs",
  "crates/target-policy/Cargo.toml",
  "crates/target-policy/src/lib.rs",
  "crates/queue/tests/queue.rs",
  "migrations/0001_identity.sql",
  "migrations/0002_sources_jobs.sql",
  "migrations/0003_plans.sql",
  "migrations/0004_targets.sql",
  "migrations/0005_runs.sql",
  "migrations/0006_approval_scenarios.sql",
  "migrations/0007_auth_sessions.sql",
  "migrations/0008_plan_review.sql",
  "migrations/0009_approval_run_match.sql",
  "migrations/0010_target_validation.sql",
  "packages/contracts/schemas/probeforge.v1.schema.json",
  "packages/contracts/fixtures/v1/manifest.json",
  "services/api/src/bin/migrate.rs",
  "services/api/src/bin/create-user.rs",
  "services/api/src/jobs.rs",
  "services/api/tests/migrations.rs",
  "services/api/tests/auth_projects.rs",
  "services/api/tests/jobs_queue.rs",
  "packages/contracts/fixtures/v1/event.claimed.valid.json",
  "scripts/db-check.mjs",
  "scripts/services.mjs",
  "apps/web/package.json",
  "services/analysis-worker/package.json",
  "packages/contracts/package.json",
  "packages/test-catalog/package.json",
];

const LINKED_DOCS = [
  "README.md",
  "docs/README.md",
  "docs/IMPLEMENTATION_CHECKLIST.md",
  "docs/CONTRACTS.md",
  "docs/DATABASE.md",
  "docs/AUTH.md",
  "docs/JOBS.md",
  "docs/REPOSITORY_INGESTION.md",
  "docs/DISCOVERY.md",
  "docs/PLANNING.md",
  "docs/PLAN_APPROVAL.md",
  "docs/TARGET_VALIDATION.md",
  "docs/EXECUTION_ADAPTERS.md",
];

// High-signal patterns only; generic words like "password" appear in policy prose on purpose.
const SECRET_PATTERNS = [
  { name: "private key block", re: /-----BEGIN [A-Z ]*PRIVATE KEY-----/ },
  { name: "AWS access key id", re: /\bAKIA[0-9A-Z]{16}\b/ },
  { name: "GitHub token", re: /\bgh[pousr]_[A-Za-z0-9]{36,}\b/ },
  { name: "Slack token", re: /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/ },
  { name: "bearer token literal", re: /\bBearer\s+[A-Za-z0-9._~+/-]{20,}=*/ },
];

const IGNORED_DIRS = new Set([".git", "node_modules", "target", "dist", "coverage"]);
const TEXT_FILE_RE = /\.(md|txt|json|ya?ml|toml|rs|ts|tsx|js|mjs|ps1|sh|example)$/i;

// 1. Required contract files.
let presentFiles = 0;
for (const relPath of REQUIRED_FILES) {
  if (existsSync(abs(relPath))) presentFiles += 1;
  else fail(`missing required file: ${relPath}`);
}

// 2. Internal Markdown links must resolve to files or directories.
function checkLinks(relPath) {
  const text = read(relPath);
  for (const match of text.matchAll(/\[[^\]]*\]\(([^)\s]+)\)/g)) {
    const target = match[1];
    if (/^(https?:|mailto:|#)/.test(target)) continue;
    const clean = target.split("#")[0].trim();
    if (!clean) continue;
    if (!existsSync(resolve(dirname(abs(relPath)), clean))) {
      fail(`broken link in ${relPath}: ${target}`);
    }
  }
}
let linkedDocs = 0;
for (const relPath of LINKED_DOCS) {
  if (!existsSync(abs(relPath))) continue;
  checkLinks(relPath);
  linkedDocs += 1;
}

// 3. The checklist must cover Phase 0 through Phase 15 exactly once.
let phaseCount = 0;
if (existsSync(abs("docs/IMPLEMENTATION_CHECKLIST.md"))) {
  const checklist = read("docs/IMPLEMENTATION_CHECKLIST.md");
  const headings = [...checklist.matchAll(/^## Phase (\d+) — /gm)].map((m) => Number(m[1]));
  phaseCount = headings.length;
  const expected = Array.from({ length: 16 }, (_, i) => i);
  const missing = expected.filter((n) => !headings.includes(n));
  const duplicates = [...new Set(headings.filter((n, i) => headings.indexOf(n) !== i))];
  if (missing.length) fail(`checklist missing phases: ${missing.join(", ")}`);
  if (duplicates.length) fail(`checklist has duplicate phases: ${duplicates.join(", ")}`);
}

// 4. Environment-file hygiene: a real .env must never be tracked by git.
function trackedPaths() {
  try {
    return execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
      .split("\0")
      .filter(Boolean);
  } catch {
    return null;
  }
}

const tracked = trackedPaths();
let committedEnvViolations = 0;

if (tracked !== null) {
  for (const path of tracked) {
    const name = path.split("/").pop();
    if (name === ".env" || (name?.startsWith(".env.") && name !== ".env.example")) {
      fail(`real environment file is tracked by git: ${path}`);
      committedEnvViolations += 1;
    }
  }
} else {
  // No git repository available: fall back to checking the working tree.
  for (const name of readdirSync(root)) {
    if (name === ".env" || (name.startsWith(".env.") && name !== ".env.example")) {
      fail(`real environment file must not be committed: ${name}`);
      committedEnvViolations += 1;
    }
  }
}

if (existsSync(abs(".gitignore"))) {
  const rules = read(".gitignore").split(/\r?\n/);
  for (const rule of [".env", "!.env.example"]) {
    if (!rules.includes(rule)) fail(`.gitignore must contain the rule: ${rule}`);
  }
}

// 5. High-signal secret patterns in repository text files.
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (IGNORED_DIRS.has(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full);
    } else if (entry.isFile() && TEXT_FILE_RE.test(entry.name)) {
      const text = readFileSync(full, "utf8");
      for (const { name, re } of SECRET_PATTERNS) {
        if (re.test(text)) fail(`possible ${name} in ${relative(root, full)}`);
      }
    }
  }
}
walk(root);

// Report.
console.log("ProbeForge repository contract check");
console.log(`- required files: ${presentFiles}/${REQUIRED_FILES.length} present`);
console.log(`- internal links: ${linkedDocs} document(s) checked`);
console.log(`- checklist phases: ${phaseCount} heading(s) found (expected 16: Phase 0-15)`);
console.log(
  `- environment hygiene: ${
    committedEnvViolations === 0 ? "no tracked .env files" : "violations found"
  }`,
);
console.log("- secret scan: completed");

if (failures.length > 0) {
  console.error(`\nFAIL: ${failures.length} contract issue(s)`);
  for (const message of failures) console.error(`  - ${message}`);
  process.exit(1);
}

console.log("\nPASS: repository contract is satisfied.");
