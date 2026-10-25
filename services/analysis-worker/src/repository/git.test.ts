import { describe, expect, it } from "bun:test";
import { spawn } from "node:child_process";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { createGitEnvironment, fetchArguments, GitClient } from "./git";
import { validateGitUrl } from "./git-url";
import { createFixtureRepository, fixtureFileUrl, fixtureRoot, resolveGitSha } from "./testing";
import { createTemporaryWorkspace } from "./workspace";

const SSH_KEY = `${["-----BEGIN", "OPENSSH", "PRIVATE", "KEY-----"].join(" ")}\n${"A".repeat(120)}\n${["-----END", "OPENSSH", "PRIVATE", "KEY-----"].join(" ")}\n`;

describe("createGitEnvironment", () => {
  it("injects HTTPS basic credentials through Git config environment, not arguments", async () => {
    const workspace = await createTemporaryWorkspace();
    try {
      const target = validateGitUrl("https://github.com/example/repo.git", {
        allowedHosts: ["github.com"],
      });
      const command = await createGitEnvironment(workspace, target, {
        kind: "http_basic",
        username: "x-access-token",
        password: "fixture-password",
      });
      expect(command.env.GIT_CONFIG_KEY_0).toBe("http.extraHeader");
      expect(command.env.GIT_CONFIG_VALUE_0).toContain("Authorization: Basic ");
      expect(command.env.GIT_CONFIG_VALUE_0).not.toContain("fixture-password");
      expect(command.args.join(" ")).not.toContain("fixture-password");
      expect(command.args.join(" ")).not.toContain("Authorization");
      expect(command.args).toContain("credential.helper=");
      expect(command.args).toContain("http.followRedirects=false");
    } finally {
      await workspace.cleanup();
    }
  });

  it("writes an isolated SSH key and references it only in the SSH command", async () => {
    const workspace = await createTemporaryWorkspace();
    try {
      const target = validateGitUrl("ssh://git@github.com/example/repo.git", {
        allowedHosts: ["github.com"],
      });
      const command = await createGitEnvironment(workspace, target, {
        kind: "ssh_private_key",
        privateKey: SSH_KEY,
      });
      const keyPath = join(workspace.gitConfigPath, "id_key");
      expect(await readFile(keyPath, "utf8")).toBe(SSH_KEY);
      expect((await stat(keyPath)).isFile()).toBe(true);
      expect(command.env.GIT_SSH_COMMAND).toContain("-i");
      expect(command.env.GIT_SSH_COMMAND).toContain("id_key");
      expect(command.env.GIT_SSH_COMMAND).toContain("StrictHostKeyChecking=yes");
      expect(command.env.GIT_CONFIG_COUNT).toBeUndefined();
      expect(command.args.join(" ")).not.toContain("PRIVATE KEY");
      expect(command.env.GIT_SSH_COMMAND).not.toContain("PRIVATE KEY");
    } finally {
      await workspace.cleanup();
    }
  });

  it("disables git lazy fetching so a missing object fails fast", async () => {
    const workspace = await createTemporaryWorkspace();
    try {
      const target = validateGitUrl("https://github.com/example/repo.git", {
        allowedHosts: ["github.com"],
      });
      const command = await createGitEnvironment(workspace, target, null);
      expect(command.env.GIT_NO_LAZY_FETCH).toBe("1");
    } finally {
      await workspace.cleanup();
    }
  });
});

function runGit(
  env: NodeJS.ProcessEnv,
  configArgs: readonly string[],
  args: readonly string[],
  cwd: string,
): Promise<{ code: number; stdout: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn("git", [...configArgs, ...args], {
      cwd,
      env,
      shell: false,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.on("error", reject);
    child.on("close", (code) =>
      resolve({ code: code ?? -1, stdout: Buffer.concat(stdout).toString("utf8") }),
    );
  });
}

describe("repository fetch", () => {
  it("fetches a complete clone so ls-tree -l never needs the network", async () => {
    const repo = await createFixtureRepository();
    const workspace = await createTemporaryWorkspace();
    try {
      const sha = await resolveGitSha(repo.root, "main");
      const target = validateGitUrl(fixtureFileUrl(repo.root), {
        allowedHosts: [],
        allowLocalFixtures: true,
        localFixtureRoot: fixtureRoot(repo.root),
      });
      const client = await GitClient.create(workspace, target, null, {
        timeoutMs: 30_000,
        maxOutputBytes: 8 * 1024 * 1024,
      });
      await client.fetch(sha);

      const command = await createGitEnvironment(workspace, target, null);
      // The pinned commit must be a complete clone, not a blobs-less partial one.
      const filter = await runGit(
        command.env,
        command.args,
        ["config", "--get", "remote.origin.partialclonefilter"],
        workspace.repoPath,
      );
      expect(filter.stdout.trim()).toBe("");
      // `ls-tree -l` needs blob sizes. With lazy fetching disabled (set by the
      // git environment) it can only succeed if every blob is present locally.
      const tree = await runGit(
        command.env,
        command.args,
        ["ls-tree", "-r", "-l", "-z", sha],
        workspace.repoPath,
      );
      expect(tree.code).toBe(0);
      expect(tree.stdout).toContain("blob");
    } finally {
      await workspace.cleanup();
      await repo.cleanup();
    }
  });

  it("requests a complete fetch without a partial-clone filter", () => {
    const args = fetchArguments("a".repeat(40));
    const joined = args.join(" ");
    expect(args[0]).toBe("fetch");
    expect(joined).toContain("--depth=1");
    expect(joined).toContain("--no-tags");
    expect(args.some((arg) => arg.startsWith("--filter"))).toBe(false);
  });
});
