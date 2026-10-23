import { describe, expect, it } from "bun:test";
import { RepositoryIngestError, type RepositoryIngestErrorCode } from "./errors";
import {
  assertCredentialRefSafe,
  basicAuthorizationValue,
  EnvSecretResolver,
  redactGitError,
  secretEnvironmentKey,
} from "./secrets";

function expectCode(callback: () => unknown, code: RepositoryIngestErrorCode): void {
  try {
    callback();
    throw new Error("expected an error");
  } catch (error) {
    expect(error).toBeInstanceOf(RepositoryIngestError);
    expect((error as RepositoryIngestError).code).toBe(code);
  }
}

describe("credential references", () => {
  it("accepts secret references and derives an environment key", () => {
    expect(() => assertCredentialRefSafe("secret://git/github")).not.toThrow();
    expect(secretEnvironmentKey("secret://git/github")).toBe("PROBE_FORGE_SECRET_GIT_GITHUB");
  });

  it("rejects credentials embedded in other forms", () => {
    for (const reference of [
      "ghp_fixture",
      "secret://",
      "secret://GIT/GITHUB",
      "secret://git/github\n",
    ]) {
      expectCode(() => assertCredentialRefSafe(reference), "invalid_credential_ref");
    }
  });

  it("resolves HTTP basic credentials without exposing them in the reference", async () => {
    const resolver = new EnvSecretResolver({
      PROBE_FORGE_SECRET_GIT_GITHUB: JSON.stringify({
        kind: "http_basic",
        username: "x-access-token",
        password: "fixture-token",
      }),
    });
    await expect(resolver.resolve("secret://git/github")).resolves.toEqual({
      kind: "http_basic",
      username: "x-access-token",
      password: "fixture-token",
    });
    expect(
      basicAuthorizationValue({
        kind: "http_basic",
        username: "x-access-token",
        password: "fixture-token",
      }),
    ).toBe(`Basic ${Buffer.from("x-access-token:fixture-token").toString("base64")}`);
  });

  it("returns null for a missing secret and fails closed for malformed secrets", async () => {
    await expect(new EnvSecretResolver({}).resolve("secret://git/missing")).resolves.toBeNull();
    await expect(
      new EnvSecretResolver({ PROBE_FORGE_SECRET_GIT_BAD: "not-json" }).resolve("secret://git/bad"),
    ).rejects.toBeInstanceOf(RepositoryIngestError);
    await expect(
      new EnvSecretResolver({
        PROBE_FORGE_SECRET_GIT_BAD: JSON.stringify({ kind: "ssh_private_key", path: "/tmp/key" }),
      }).resolve("secret://git/bad"),
    ).rejects.toBeInstanceOf(RepositoryIngestError);
    await expect(
      new EnvSecretResolver({
        PROBE_FORGE_SECRET_GIT_BAD: JSON.stringify({
          kind: "http_basic",
          username: "user\nname",
          password: "secret",
        }),
      }).resolve("secret://git/bad"),
    ).rejects.toBeInstanceOf(RepositoryIngestError);
  });

  it("resolves an unencrypted SSH private key from a secret reference", async () => {
    const privateKey = `${["-----BEGIN", "OPENSSH", "PRIVATE", "KEY-----"].join(" ")}\n${"A".repeat(120)}\n${["-----END", "OPENSSH", "PRIVATE", "KEY-----"].join(" ")}\n`;
    const resolver = new EnvSecretResolver({
      PROBE_FORGE_SECRET_GIT_SSH: JSON.stringify({ kind: "ssh_private_key", privateKey }),
    });
    await expect(resolver.resolve("secret://git/ssh")).resolves.toEqual({
      kind: "ssh_private_key",
      privateKey,
    });
  });
});

describe("redactGitError", () => {
  it("removes URL credentials, authorization values, and token-like fields", () => {
    const redacted = redactGitError(
      [
        "fatal: unable to access 'https://user:password@github.com/org/repo.git'",
        "Authorization: Basic dXNlcjpwYXNzd29yZA==",
        "password=fixture-password token=fixture-token",
        "path=/home/user/private",
      ].join("\n"),
    );
    expect(redacted).not.toContain("user:password");
    expect(redacted).not.toContain("github.com/org/repo.git");
    expect(redacted).not.toContain("dXNlcjpwYXNzd29yZA==");
    expect(redacted).not.toContain("fixture-password");
    expect(redacted).not.toContain("fixture-token");
    expect(redacted).toContain("<redacted>");
  });
});
