import { spawn } from "node:child_process";
import { access, cp, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { MemoryAuditSink } from "./audit";
import { DEFAULT_INGESTION_LIMITS, type IngestionLimits } from "./limits";

const FIXTURE_SOURCE = fileURLToPath(new URL("../../fixtures/inert-repo", import.meta.url));

export interface TestRepository {
  readonly root: string;
  readonly tempRoot: string;
  readonly audit: MemoryAuditSink;
  readonly limits: IngestionLimits;
  cleanup(): Promise<void>;
}

interface RunGitOptions {
  readonly input?: string;
}

function runGitArgs(
  repo: string,
  args: readonly string[],
  options: RunGitOptions = {},
): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(
      "git",
      [
        "-c",
        "user.name=ProbeForge Test",
        "-c",
        "user.email=probeforge-test@example.test",
        "-c",
        "core.autocrlf=false",
        ...args,
      ],
      {
        cwd: repo,
        shell: false,
        windowsHide: true,
        stdio: [options.input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
      },
    );
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.stderr?.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve(Buffer.concat(stdout).toString("utf8").trim());
        return;
      }
      reject(
        new Error(
          `git ${args.join(" ")} failed with code ${code}: ${Buffer.concat(stderr).toString("utf8")}`,
        ),
      );
    });
    if (options.input !== undefined) child.stdin?.end(options.input);
  });
}

export async function createFixtureRepository(): Promise<TestRepository> {
  const tempRoot = await mkdtemp(join(tmpdir(), "probeforge-ingest-test-"));
  const root = join(tempRoot, "fixture");
  await cp(FIXTURE_SOURCE, root, { recursive: true });
  await runGitArgs(root, ["init", "--quiet", "--initial-branch=main"]);
  await runGitArgs(root, ["add", "-A"]);
  await runGitArgs(root, ["commit", "--quiet", "-m", "inert fixture baseline"]);
  return {
    root,
    tempRoot,
    audit: new MemoryAuditSink(),
    limits: DEFAULT_INGESTION_LIMITS,
    cleanup: async () => {
      await rm(tempRoot, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
    },
  };
}

export async function commitAll(repo: string, message: string): Promise<string> {
  await runGitArgs(repo, ["add", "-A"]);
  await runGitArgs(repo, ["commit", "--quiet", "-m", message]);
  return await runGitArgs(repo, ["rev-parse", "HEAD"]);
}

async function commitIndex(repo: string, message: string): Promise<string> {
  await runGitArgs(repo, ["commit", "--quiet", "-m", message]);
  return await runGitArgs(repo, ["rev-parse", "HEAD"]);
}

export async function addSecondCommit(repo: string): Promise<string> {
  await writeFile(join(repo, "src", "new.ts"), 'export const second = "second commit";\n', "utf8");
  return await commitAll(repo, "second commit");
}

export async function resolveGitSha(repo: string, reference: string): Promise<string> {
  return await runGitArgs(repo, ["rev-parse", reference]);
}

export async function createTag(repo: string, name: string, reference: string): Promise<void> {
  await runGitArgs(repo, ["tag", name, reference]);
}

export async function addSymlinkEntry(repo: string): Promise<string> {
  const blob = await runGitArgs(repo, ["hash-object", "-w", "--stdin"], {
    input: "../../outside\n",
  });
  await runGitArgs(repo, ["update-index", "--add", "--cacheinfo", `120000,${blob},escape-link`]);
  return await commitIndex(repo, "add symlink entry");
}

export async function addSubmoduleEntry(repo: string): Promise<string> {
  const sha = await runGitArgs(repo, ["rev-parse", "HEAD"]);
  await writeFile(
    join(repo, ".gitmodules"),
    '[submodule "example"]\n\tpath = vendor/example\n\turl = https://example.invalid/example.git\n',
    "utf8",
  );
  await runGitArgs(repo, ["add", ".gitmodules"]);
  await runGitArgs(repo, ["update-index", "--add", "--cacheinfo", `160000,${sha},vendor/example`]);
  return await commitIndex(repo, "add submodule entry");
}

export async function addLargeFile(repo: string, bytes: number): Promise<void> {
  await mkdir(join(repo, "src"), { recursive: true });
  await writeFile(join(repo, "src", "oversized.txt"), "x".repeat(bytes), "utf8");
}

export async function addBinaryFile(repo: string): Promise<void> {
  await mkdir(join(repo, "src"), { recursive: true });
  await writeFile(join(repo, "src", "binary.bin"), Buffer.from([0, 1, 2, 3, 255]));
}

export function fixtureFileUrl(repo: string): string {
  return pathToFileURL(repo).href;
}

export function fixtureRoot(repo: string): string {
  return dirname(repo);
}

export async function exists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
