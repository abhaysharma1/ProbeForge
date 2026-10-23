import { describe, expect, it } from "bun:test";
import { isAbsolute, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { RepositoryIngestError, type RepositoryIngestErrorCode } from "./errors";
import { assertReferenceSafe, isCommitSha, providerForHost, validateGitUrl } from "./git-url";

const OPTIONS = {
  allowedHosts: ["github.com", "gitlab.com", "bitbucket.org", "git.example.test"],
} as const;

function expectCode(callback: () => unknown, code: RepositoryIngestErrorCode): void {
  try {
    callback();
    throw new Error("expected an error");
  } catch (error) {
    expect(error).toBeInstanceOf(RepositoryIngestError);
    expect((error as RepositoryIngestError).code).toBe(code);
  }
}

describe("validateGitUrl", () => {
  it("accepts HTTPS and SSH provider URLs without credentials", () => {
    expect(validateGitUrl("https://github.com/example/repo.git", OPTIONS)).toMatchObject({
      provider: "github",
      hostname: "github.com",
      scheme: "https",
      localPath: null,
    });
    expect(validateGitUrl("ssh://git@gitlab.com/group/repo.git", OPTIONS)).toMatchObject({
      provider: "gitlab",
      scheme: "ssh",
    });
    expect(validateGitUrl("https://git.example.test/team/repo", OPTIONS)).toMatchObject({
      provider: "generic",
    });
  });

  it("rejects unsupported schemes, embedded credentials, and unsafe hosts", () => {
    expectCode(
      () => validateGitUrl("http://github.com/example/repo.git", OPTIONS),
      "unsupported_scheme",
    );
    expectCode(
      () => validateGitUrl("git://github.com/example/repo.git", OPTIONS),
      "unsupported_scheme",
    );
    expectCode(
      () => validateGitUrl("https://user:token@github.com/example/repo.git", OPTIONS),
      "credentials_in_url",
    );
    expectCode(
      () => validateGitUrl("https://token@github.com/example/repo.git", OPTIONS),
      "credentials_in_url",
    );
    expectCode(
      () => validateGitUrl("ssh://root@github.com/example/repo.git", OPTIONS),
      "credentials_in_url",
    );
    expectCode(
      () => validateGitUrl("https://localhost/example/repo.git", OPTIONS),
      "unsupported_git_host",
    );
    expectCode(
      () => validateGitUrl("https://127.0.0.1/example/repo.git", OPTIONS),
      "unsupported_git_host",
    );
    expectCode(
      () => validateGitUrl("https://169.254.169.254/latest/meta-data", OPTIONS),
      "unsupported_git_host",
    );
    expectCode(
      () => validateGitUrl("https://evil.internal/example/repo.git", OPTIONS),
      "unsupported_git_host",
    );
    expectCode(
      () => validateGitUrl("https://not-allowlisted.example/example/repo.git", OPTIONS),
      "unsupported_git_host",
    );
  });

  it("rejects non-default ports, queries, fragments, and unsafe paths", () => {
    expectCode(
      () => validateGitUrl("https://github.com:8443/example/repo.git", OPTIONS),
      "unsupported_scheme",
    );
    expectCode(
      () => validateGitUrl("https://github.com/example/repo.git?ref=main", OPTIONS),
      "invalid_url",
    );
    expectCode(
      () => validateGitUrl("https://github.com/example/repo.git#main", OPTIONS),
      "invalid_url",
    );
    expectCode(
      () => validateGitUrl("https://github.com/example/repo%2F..%2Foutside.git", OPTIONS),
      "invalid_url",
    );
    expectCode(
      () => validateGitUrl("https://github.com/example/repo%5Cescape.git", OPTIONS),
      "invalid_url",
    );
  });

  it("allows local fixtures only in explicit fixture mode and inside the fixture root", () => {
    const root = join(process.cwd(), "fixtures", "inert-repo");
    const url = pathToFileURL(root).href;
    const target = validateGitUrl(url, {
      allowedHosts: [],
      allowLocalFixtures: true,
      localFixtureRoot: root,
    });
    expect(target.scheme).toBe("file");
    expect(target.localPath).toBeDefined();
    expect(isAbsolute(target.localPath ?? "")).toBe(true);
    expect(relative(root, target.localPath ?? "").startsWith("..")).toBe(false);

    expectCode(
      () => validateGitUrl(url, { allowedHosts: [], allowLocalFixtures: false }),
      "unsupported_scheme",
    );
    expectCode(
      () =>
        validateGitUrl(pathToFileURL(resolve(root, "..", "outside")).href, {
          allowedHosts: [],
          allowLocalFixtures: true,
          localFixtureRoot: root,
        }),
      "unsafe_path",
    );
  });

  it("maps known providers and detects commit SHAs", () => {
    expect(providerForHost("github.com")).toBe("github");
    expect(providerForHost("gitlab.com")).toBe("gitlab");
    expect(providerForHost("bitbucket.org")).toBe("bitbucket");
    expect(providerForHost("git.example.test")).toBe("generic");
    expect(isCommitSha("a".repeat(40))).toBe(true);
    expect(isCommitSha("a".repeat(64))).toBe(true);
    expect(isCommitSha("A".repeat(40))).toBe(false);
    expect(isCommitSha("a".repeat(39))).toBe(false);
  });
});

describe("assertReferenceSafe", () => {
  it("accepts branches, tags, refs, and full commit SHAs", () => {
    for (const reference of [
      "main",
      "release/1.0",
      "v1.2.3",
      "refs/heads/main",
      "refs/tags/v1",
      "a".repeat(40),
      "b".repeat(64),
    ]) {
      expect(() => assertReferenceSafe(reference)).not.toThrow();
    }
  });

  it("rejects option-like, revision-expression, and traversal refs", () => {
    for (const reference of [
      "-upload-pack=evil",
      "main~1",
      "main^",
      "main:path",
      "main..other",
      "main//other",
      "/main",
      "main/",
      "refs/remotes/origin/main",
      "refs/heads/../main",
      "main.lock",
      "main@{1}",
      "main\\other",
      "",
      "main\nother",
    ]) {
      expectCode(() => assertReferenceSafe(reference), "unsafe_reference");
    }
  });
});
