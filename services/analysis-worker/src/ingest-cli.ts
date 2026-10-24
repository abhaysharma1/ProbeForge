import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { loadConfig, WorkerConfigError } from "./config";
import { discoverApplication } from "./discovery";
import { SERVICE_NAME } from "./health";
import { createLogger } from "./logging/logger";
import { JsonLineAuditSink } from "./repository/audit";
import { RepositoryIngestError } from "./repository/errors";
import { validateGitUrl } from "./repository/git-url";
import { withRepositorySnapshot } from "./repository/ingest";
import { type IngestionLimits, validateIngestionLimits } from "./repository/limits";
import { EnvSecretResolver } from "./repository/secrets";

const USAGE = `ProbeForge read-only repository ingestion

Usage:
  bun run ingest -- --fixture <directory> --ref <branch|tag|sha> --audit-log <file> [options]
  bun run ingest -- --url <https-url> --ref <branch|tag|sha> --audit-log <file> [options]

Options:
  --fixture <dir>          Local inert fixture repository (operator-only)
  --url <url>              HTTPS or SSH Git repository URL
  --ref <ref>              Branch, tag, or full commit SHA (required)
  --credential-ref <ref>   Secret reference, e.g. secret://git/github
  --audit-log <file>       Append-only JSONL audit log (required unless configured)
  --output <file>          Write snapshot JSON to a file instead of stdout
  --discovery-output <file> Write bounded static application inventory JSON
  --max-files <n>          Maximum tracked file entries
  --max-total-bytes <n>    Maximum tracked blob bytes
  --max-file-bytes <n>     Maximum bytes read from one file
  --max-depth <n>          Maximum repository path depth
  --max-workspace-bytes <n> Maximum on-disk workspace bytes
  --max-inventory-files <n> Maximum files included in snapshot metadata
  --max-dependencies-per-manifest <n> Maximum dependencies per manifest
  --git-timeout-ms <n>     Git command timeout
  --help                   Show this help

This command never installs dependencies, builds, tests, starts, or executes
repository content. It is not an HTTP endpoint and is not exposed by the worker.
`;

class UsageError extends Error {}

interface CliOptions {
  readonly fixture: string | null;
  readonly url: string | null;
  readonly reference: string;
  readonly credentialRef: string | null;
  readonly auditLog: string | null;
  readonly output: string | null;
  readonly discoveryOutput: string | null;
  readonly limits: Partial<IngestionLimits>;
}

function parseIntegerFlag(name: string, raw: string, minimum: number): number {
  if (!/^\d+$/.test(raw)) throw new UsageError(`${name} must be an integer`);
  const value = Number.parseInt(raw, 10);
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new UsageError(`${name} must be at least ${minimum}`);
  }
  return value;
}

function parseArgs(argv: readonly string[]): CliOptions | "help" {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === undefined) continue;
    if (arg === "--help" || arg === "-h") return "help";
    if (!arg.startsWith("--")) throw new UsageError(`Unexpected argument: ${arg}`);
    const name = arg.slice(2);
    if (name.length === 0) throw new UsageError("Invalid empty option");
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new UsageError(`Option --${name} requires a value`);
    }
    values.set(name, value);
    index += 1;
  }

  const fixture = values.get("fixture") ?? null;
  const url = values.get("url") ?? null;
  const reference = values.get("ref");
  if (reference === undefined) throw new UsageError("--ref is required");
  if ((fixture === null) === (url === null)) {
    throw new UsageError("Exactly one of --fixture or --url is required");
  }

  const limits: { -readonly [K in keyof IngestionLimits]?: IngestionLimits[K] } = {};
  const limitFlags: Array<[keyof IngestionLimits, string]> = [
    ["maxFiles", "max-files"],
    ["maxTotalBytes", "max-total-bytes"],
    ["maxFileBytes", "max-file-bytes"],
    ["maxDepth", "max-depth"],
    ["maxWorkspaceBytes", "max-workspace-bytes"],
    ["maxGitOutputBytes", "max-git-output-bytes"],
    ["gitTimeoutMs", "git-timeout-ms"],
    ["maxInventoryFiles", "max-inventory-files"],
    ["maxDependenciesPerManifest", "max-dependencies-per-manifest"],
  ];
  for (const [key, flag] of limitFlags) {
    const raw = values.get(flag);
    if (raw !== undefined) limits[key] = parseIntegerFlag(`--${flag}`, raw, 1);
  }

  const allowed = new Set([
    "fixture",
    "url",
    "ref",
    "credential-ref",
    "audit-log",
    "output",
    "discovery-output",
    ...limitFlags.map(([, flag]) => flag),
  ]);
  for (const key of values.keys()) {
    if (!allowed.has(key)) throw new UsageError(`Unknown option: --${key}`);
  }

  return {
    fixture: fixture === null ? null : resolve(fixture),
    url,
    reference,
    credentialRef: values.get("credential-ref") ?? null,
    auditLog: values.get("audit-log") ?? null,
    output: values.get("output") === undefined ? null : resolve(values.get("output") ?? ""),
    discoveryOutput:
      values.get("discovery-output") === undefined
        ? null
        : resolve(values.get("discovery-output") ?? ""),
    limits,
  };
}

async function writeSnapshot(output: string | null, value: unknown): Promise<void> {
  const text = `${JSON.stringify(value, null, 2)}\n`;
  if (output === null) {
    process.stdout.write(text);
    return;
  }
  await mkdir(dirname(output), { recursive: true, mode: 0o700 });
  await writeFile(output, text, { encoding: "utf8", mode: 0o600 });
}

export async function main(argv: readonly string[] = process.argv.slice(2)): Promise<number> {
  try {
    const parsed = parseArgs(argv);
    if (parsed === "help") {
      process.stdout.write(USAGE);
      return 0;
    }
    const config = loadConfig();
    const limits: IngestionLimits = { ...config.ingestion.limits, ...parsed.limits };
    validateIngestionLimits(limits);

    const fixtureRoot = parsed.fixture === null ? null : dirname(parsed.fixture);
    const rawUrl =
      parsed.fixture === null ? (parsed.url ?? "") : pathToFileURL(parsed.fixture).href;
    const target = validateGitUrl(rawUrl, {
      allowedHosts: config.ingestion.allowedGitHosts,
      allowLocalFixtures: parsed.fixture !== null,
      localFixtureRoot: fixtureRoot,
    });
    const auditLog = parsed.auditLog ?? config.ingestion.auditLogPath;
    if (auditLog === null) {
      throw new UsageError("--audit-log is required (or set PROBE_FORGE_ANALYSIS_AUDIT_LOG)");
    }
    const audit = new JsonLineAuditSink(auditLog);
    const logger = createLogger({
      service: SERVICE_NAME,
      level: config.logLevel,
      format: config.logFormat,
    });

    logger.info("ingest.started", {
      provider: target.provider,
      reference: parsed.reference,
      fixture: parsed.fixture !== null,
    });

    await withRepositorySnapshot(
      {
        target,
        reference: parsed.reference,
        credentialRef: parsed.credentialRef,
        secretResolver: new EnvSecretResolver(process.env),
        audit,
        limits,
        localFixtureRoot: fixtureRoot,
        allowLocalFixtures: parsed.fixture !== null,
      },
      async (session) => {
        logger.info("ingest.snapshot_created", {
          commitSha: session.snapshot.commitSha,
          fileCount: session.snapshot.analysisMetadata.fileCount,
          truncated: session.snapshot.filesTruncated,
        });
        await writeSnapshot(parsed.output, session.snapshot);
        if (parsed.discoveryOutput !== null) {
          const inventory = await discoverApplication(session);
          logger.info("ingest.discovery_completed", {
            commitSha: session.snapshot.commitSha,
          });
          await writeSnapshot(parsed.discoveryOutput, inventory);
        }
      },
    );
    return 0;
  } catch (error) {
    if (error instanceof UsageError) {
      process.stderr.write(
        `${JSON.stringify({ error: { code: "usage", message: error.message } })}\n`,
      );
      return 2;
    }
    if (error instanceof WorkerConfigError) {
      process.stderr.write(
        `${JSON.stringify({ error: { code: "invalid_config", message: error.message } })}\n`,
      );
      return 2;
    }
    if (error instanceof RepositoryIngestError) {
      process.stderr.write(
        `${JSON.stringify({ error: { code: error.code, message: error.message } })}\n`,
      );
      return 1;
    }
    process.stderr.write(
      `${JSON.stringify({
        error: { code: "internal_error", message: "Repository ingestion failed" },
      })}\n`,
    );
    return 1;
  }
}

if (import.meta.main) {
  process.exit(await main());
}
