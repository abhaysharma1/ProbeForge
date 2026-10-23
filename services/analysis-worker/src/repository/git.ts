import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { RepositoryIngestError } from "./errors";
import { type GitTarget, isCommitSha } from "./git-url";
import { BoundedProcessError, runBoundedProcess } from "./process";
import { basicAuthorizationValue, type GitCredential, redactGitError } from "./secrets";
import type { TemporaryWorkspace } from "./workspace";

export interface GitClientOptions {
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
  readonly signal?: AbortSignal;
}

export interface ResolvedReference {
  readonly sha: string;
  readonly kind: "branch" | "sha" | "tag";
  readonly reference: string;
}

export interface GitCommandEnvironment {
  readonly args: readonly string[];
  readonly env: NodeJS.ProcessEnv;
}

const MAX_VERSION_OUTPUT = 4_096;

function fixedGitArguments(target: GitTarget, hooksPath: string): string[] {
  const args = [
    "-c",
    "protocol.allow=never",
    "-c",
    "protocol.https.allow=always",
    "-c",
    "protocol.ssh.allow=always",
    "-c",
    "protocol.version=2",
    "-c",
    "http.followRedirects=false",
    "-c",
    "http.sslVerify=true",
    "-c",
    "fetch.fsckObjects=true",
    "-c",
    "transfer.fsckObjects=true",
    "-c",
    "credential.helper=",
    "-c",
    "credential.interactive=false",
    "-c",
    "submodule.recurse=false",
    "-c",
    `core.hooksPath=${hooksPath}`,
    "-c",
    "core.symlinks=false",
    "-c",
    "core.autocrlf=false",
    "-c",
    "core.eol=lf",
    "-c",
    "gc.auto=0",
    "-c",
    "maintenance.auto=false",
    "-c",
    "advice.detachedHead=false",
  ];
  if (target.scheme === "file") {
    args.push("-c", "protocol.file.allow=always");
  }
  return args;
}

export async function createGitEnvironment(
  workspace: TemporaryWorkspace,
  target: GitTarget,
  credential: GitCredential | null,
): Promise<GitCommandEnvironment> {
  const globalConfig = join(workspace.gitConfigPath, "global.config");
  const systemConfig = join(workspace.gitConfigPath, "system.config");
  await Promise.all([
    writeFile(globalConfig, "", { mode: 0o600 }),
    writeFile(systemConfig, "", { mode: 0o600 }),
  ]);

  let sshCommand = "ssh -o BatchMode=yes -o StrictHostKeyChecking=yes";
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH ?? "",
    HOME: workspace.gitConfigPath,
    XDG_CONFIG_HOME: workspace.gitConfigPath,
    TMP: workspace.tempPath,
    TEMP: workspace.tempPath,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: globalConfig,
    GIT_CONFIG_SYSTEM: systemConfig,
    GIT_TERMINAL_PROMPT: "0",
    GIT_PAGER: "cat",
    PAGER: "cat",
    GIT_OPTIONAL_LOCKS: "0",
    GIT_LFS_SKIP_SMUDGE: "1",
    GIT_SSH_COMMAND: sshCommand,
    LC_ALL: "C",
    LANG: "C",
  };
  if (process.platform === "win32") {
    env.SystemRoot = process.env.SystemRoot ?? "";
    env.WINDIR = process.env.WINDIR ?? "";
    env.COMSPEC = process.env.COMSPEC ?? "";
    env.PATHEXT = process.env.PATHEXT ?? "";
  }
  if (credential?.kind === "http_basic") {
    env.GIT_CONFIG_COUNT = "1";
    env.GIT_CONFIG_KEY_0 = "http.extraHeader";
    env.GIT_CONFIG_VALUE_0 = `Authorization: ${basicAuthorizationValue(credential)}`;
  }
  if (credential?.kind === "ssh_private_key") {
    const keyPath = join(workspace.gitConfigPath, "id_key");
    await writeFile(keyPath, credential.privateKey, { mode: 0o600 });
    sshCommand = `ssh -i ${shellSingleQuote(keyPath.replaceAll("\\", "/"))} -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes`;
    env.GIT_SSH_COMMAND = sshCommand;
  }

  return { args: fixedGitArguments(target, workspace.gitHooksPath), env };
}

function shellSingleQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function mapProcessError(error: unknown, fallbackMessage: string): RepositoryIngestError {
  if (error instanceof RepositoryIngestError) return error;
  if (error instanceof BoundedProcessError) {
    if (error.code === "process_aborted") {
      return new RepositoryIngestError("cancelled", "Repository ingestion was cancelled", {
        cause: error,
      });
    }
    if (error.code === "process_timeout") {
      return new RepositoryIngestError("git_timeout", "Git command timed out", {
        cause: error,
        retryable: true,
      });
    }
    if (error.code === "process_output_limit") {
      return new RepositoryIngestError(
        "git_output_limit",
        "Git command output exceeded its limit",
        {
          cause: error,
        },
      );
    }
    if (error.code === "process_spawn_failed") {
      return new RepositoryIngestError("git_unavailable", "Git is not available", {
        cause: error,
      });
    }
    return new RepositoryIngestError("git_failed", fallbackMessage, {
      cause: error,
      retryable: true,
    });
  }
  return new RepositoryIngestError("git_failed", fallbackMessage, {
    cause: error,
    retryable: true,
  });
}

export class GitClient {
  readonly #workspace: TemporaryWorkspace;
  readonly #target: GitTarget;
  readonly #command: GitCommandEnvironment;
  readonly #options: GitClientOptions;

  private constructor(
    workspace: TemporaryWorkspace,
    target: GitTarget,
    command: GitCommandEnvironment,
    options: GitClientOptions,
  ) {
    this.#workspace = workspace;
    this.#target = target;
    this.#command = command;
    this.#options = options;
  }

  static async create(
    workspace: TemporaryWorkspace,
    target: GitTarget,
    credential: GitCredential | null,
    options: GitClientOptions,
  ): Promise<GitClient> {
    const command = await createGitEnvironment(workspace, target, credential);
    return new GitClient(workspace, target, command, options);
  }

  async checkVersion(): Promise<string> {
    const result = await this.#run(["--version"], MAX_VERSION_OUTPUT);
    const match = /^git version ([0-9][0-9.\-A-Za-z]*)/m.exec(result);
    if (match?.[1] === undefined) {
      throw new RepositoryIngestError("git_unavailable", "Git version could not be determined");
    }
    return match[1];
  }

  async resolveReference(reference: string): Promise<ResolvedReference> {
    if (isCommitSha(reference)) {
      return { sha: reference, kind: "sha", reference };
    }

    const patterns = referencePatterns(reference);
    const output = await this.#run(
      ["ls-remote", "--quiet", this.#target.url, ...patterns],
      1_048_576,
    );
    const refs = new Map<string, string>();
    for (const line of output.split(/\r?\n/)) {
      if (line.length === 0) continue;
      const tab = line.indexOf("\t");
      if (tab <= 0) continue;
      const sha = line.slice(0, tab).trim().toLowerCase();
      const name = line.slice(tab + 1).trim();
      if (!isCommitSha(sha)) continue;
      refs.set(name, sha);
    }

    if (reference.startsWith("refs/heads/") || reference.startsWith("refs/tags/")) {
      const branchName = reference.startsWith("refs/heads/") ? reference : null;
      const tagName = reference.startsWith("refs/tags/") ? reference : null;
      const branchSha = branchName === null ? undefined : refs.get(branchName);
      const peeledTagName = tagName === null ? null : `${tagName}^{}`;
      const peeledTagSha = peeledTagName === null ? undefined : refs.get(peeledTagName);
      const tagSha = peeledTagSha ?? (tagName === null ? undefined : refs.get(tagName));
      const candidates = new Set<string>();
      const kind: ResolvedReference["kind"] = branchSha === undefined ? "tag" : "branch";
      if (branchSha !== undefined) candidates.add(branchSha);
      if (tagSha !== undefined) candidates.add(tagSha);
      if (candidates.size === 0) {
        throw new RepositoryIngestError("reference_not_found", "Source reference was not found", {
          retryable: true,
        });
      }
      if (candidates.size > 1) {
        throw new RepositoryIngestError(
          "ambiguous_reference",
          "Source reference matches a branch and tag with different commits",
        );
      }
      const sha = [...candidates][0];
      if (sha === undefined) {
        throw new RepositoryIngestError("reference_not_found", "Source reference was not found", {
          retryable: true,
        });
      }
      return { sha, kind, reference };
    }

    const branchName = `refs/heads/${reference}`;
    const tagName = `refs/tags/${reference}`;
    const peeledTagName = `${tagName}^{}`;
    const branchSha = refs.get(branchName);
    const peeledTagSha = refs.get(peeledTagName);
    const tagSha = peeledTagSha ?? refs.get(tagName);

    const candidates = new Set<string>();
    let kind: ResolvedReference["kind"] = "branch";
    if (branchSha !== undefined) {
      candidates.add(branchSha);
      kind = "branch";
    }
    if (tagSha !== undefined) {
      candidates.add(tagSha);
      if (branchSha === undefined) kind = "tag";
    }
    if (candidates.size === 0) {
      throw new RepositoryIngestError("reference_not_found", "Source reference was not found", {
        retryable: true,
      });
    }
    if (candidates.size > 1) {
      throw new RepositoryIngestError(
        "ambiguous_reference",
        "Source reference matches a branch and tag with different commits",
      );
    }
    const sha = [...candidates][0];
    if (sha === undefined) {
      throw new RepositoryIngestError("reference_not_found", "Source reference was not found", {
        retryable: true,
      });
    }
    return { sha, kind, reference };
  }

  async fetch(sha: string): Promise<void> {
    await this.#run(["init", "--quiet"], 8_192, this.#workspace.repoPath);
    await this.#run(["remote", "add", "origin", this.#target.url], 8_192, this.#workspace.repoPath);
    try {
      await this.#run(
        ["fetch", "--quiet", "--depth=1", "--no-tags", "--filter=blob:none", "origin", sha],
        16_384,
        this.#workspace.repoPath,
      );
    } catch (error) {
      if (isMissingCommitError(error)) {
        throw new RepositoryIngestError(
          "commit_unavailable",
          "The resolved commit could not be fetched from the repository",
          { cause: error, retryable: true },
        );
      }
      throw error;
    }
  }

  async checkout(sha: string): Promise<void> {
    await this.#run(["checkout", "--detach", "--quiet", sha], 16_384, this.#workspace.repoPath);
    const head = (await this.#run(["rev-parse", "HEAD"], 256, this.#workspace.repoPath))
      .trim()
      .toLowerCase();
    if (head !== sha.toLowerCase()) {
      throw new RepositoryIngestError(
        "commit_mismatch",
        "Checked-out commit does not match the resolved commit",
      );
    }
  }

  async fetchAndCheckout(sha: string): Promise<void> {
    await this.fetch(sha);
    await this.checkout(sha);
  }

  async listTree(sha: string): Promise<string> {
    return await this.#run(
      ["ls-tree", "-r", "-l", "-z", sha],
      this.#options.maxOutputBytes,
      this.#workspace.repoPath,
    );
  }

  async #run(args: readonly string[], maxOutputBytes: number, cwd?: string): Promise<string> {
    try {
      const result = await runBoundedProcess("git", [...this.#command.args, ...args], {
        ...(cwd === undefined ? {} : { cwd }),
        env: this.#command.env,
        timeoutMs: this.#options.timeoutMs,
        maxOutputBytes,
        ...(this.#options.signal === undefined ? {} : { signal: this.#options.signal }),
      });
      return result.stdout.toString("utf8");
    } catch (error) {
      if (error instanceof BoundedProcessError && error.stderr.length > 0) {
        const redacted = redactGitError(error.stderr.toString("utf8"));
        if (redacted.length > 0) {
          throw new RepositoryIngestError(
            error.code === "process_timeout"
              ? "git_timeout"
              : error.code === "process_aborted"
                ? "cancelled"
                : error.code === "process_output_limit"
                  ? "git_output_limit"
                  : error.code === "process_spawn_failed"
                    ? "git_unavailable"
                    : "git_failed",
            redacted,
            { cause: error, retryable: error.code !== "process_output_limit" },
          );
        }
      }
      throw mapProcessError(error, "Git command failed");
    }
  }
}

function referencePatterns(reference: string): string[] {
  if (reference.startsWith("refs/heads/") || reference.startsWith("refs/tags/")) {
    return [reference, `${reference}^{}`];
  }
  return [`refs/heads/${reference}`, `refs/tags/${reference}`, `refs/tags/${reference}^{}`];
}

function isMissingCommitError(error: unknown): boolean {
  if (!(error instanceof RepositoryIngestError)) return false;
  const message = error.message.toLowerCase();
  return (
    message.includes("not our ref") ||
    message.includes("couldn't find remote ref") ||
    message.includes("unadvertised object") ||
    message.includes("no such remote ref") ||
    message.includes("upload-pack: not our ref")
  );
}
