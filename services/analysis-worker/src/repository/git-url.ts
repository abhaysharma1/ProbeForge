import { isIP } from "node:net";
import { isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { RepositoryIngestError } from "./errors";
import { isInternalHostname, isPrivateOrReservedAddress, normalizeHostname } from "./net";
import { containsControlCharacter } from "./text";

export type GitProvider = "bitbucket" | "generic" | "github" | "gitlab";
export type GitScheme = "file" | "https" | "ssh";

export interface GitTarget {
  readonly url: string;
  readonly provider: GitProvider;
  readonly hostname: string;
  readonly scheme: GitScheme;
  readonly localPath: string | null;
}

export interface GitUrlOptions {
  readonly allowedHosts: readonly string[];
  readonly allowLocalFixtures?: boolean;
  readonly localFixtureRoot?: string | null;
}

const PROVIDER_HOSTS: Readonly<Record<string, GitProvider>> = {
  "bitbucket.org": "bitbucket",
  "github.com": "github",
  "gitlab.com": "gitlab",
};

const MAX_URL_LENGTH = 2_048;

export function providerForHost(hostname: string): GitProvider {
  return PROVIDER_HOSTS[normalizeHostname(hostname)] ?? "generic";
}

function invalidUrl(message: string): RepositoryIngestError {
  return new RepositoryIngestError("invalid_url", message);
}

function decodePath(rawPath: string): string {
  let decoded: string;
  try {
    decoded = decodeURIComponent(rawPath);
  } catch {
    throw invalidUrl("Git repository path is not valid URL encoding");
  }
  if (decoded.length === 0 || decoded === "/") {
    throw invalidUrl("Git repository URL must include a repository path");
  }
  if (decoded.includes("\\")) {
    throw invalidUrl("Git repository path must not contain backslashes");
  }
  if (containsControlCharacter(decoded)) {
    throw invalidUrl("Git repository path contains control characters");
  }
  const segments = decoded.slice(1).split("/");
  if (!decoded.startsWith("/")) {
    throw invalidUrl("Git repository path must be absolute");
  }
  for (const segment of segments) {
    if (segment.length === 0 || segment === "." || segment === "..") {
      throw invalidUrl("Git repository path contains an unsafe segment");
    }
  }
  return decoded;
}

function assertPortAllowed(scheme: "https" | "ssh", port: string): void {
  const allowed = scheme === "https" ? "443" : "22";
  if (port.length > 0 && port !== allowed) {
    throw new RepositoryIngestError(
      "unsupported_scheme",
      `Only the default ${scheme.toUpperCase()} port is allowed`,
    );
  }
}

function assertAllowedHost(hostname: string, allowedHosts: readonly string[]): void {
  const normalized = normalizeHostname(hostname);
  if (normalized.length === 0 || isInternalHostname(normalized)) {
    throw new RepositoryIngestError("unsupported_git_host", "Git host is not allowed");
  }
  if (isIP(normalized) !== 0 && isPrivateOrReservedAddress(normalized)) {
    throw new RepositoryIngestError("unsupported_git_host", "Git host is not public");
  }
  const allowed = new Set(allowedHosts.map(normalizeHostname));
  if (!allowed.has(normalized)) {
    throw new RepositoryIngestError("unsupported_git_host", "Git host is not allowlisted");
  }
}

function validateLocalTarget(url: URL, options: GitUrlOptions): GitTarget {
  if (options.allowLocalFixtures !== true) {
    throw new RepositoryIngestError(
      "unsupported_scheme",
      "Local fixture repositories are disabled for this worker",
    );
  }
  if (url.username.length > 0 || url.password.length > 0) {
    throw new RepositoryIngestError("credentials_in_url", "Git URL must not embed credentials");
  }
  const hostname = normalizeHostname(url.hostname);
  if (hostname.length > 0 && hostname !== "localhost") {
    throw new RepositoryIngestError(
      "unsupported_git_host",
      "Local fixture URLs must not use a remote host",
    );
  }
  let localPath: string;
  try {
    localPath = fileURLToPath(url);
  } catch (error) {
    throw new RepositoryIngestError("invalid_url", "Local fixture URL is not a valid file URL", {
      cause: error,
    });
  }
  if (!isAbsolute(localPath) || containsControlCharacter(localPath)) {
    throw invalidUrl("Local fixture path must be absolute");
  }
  if (options.localFixtureRoot !== undefined && options.localFixtureRoot !== null) {
    const root = resolve(options.localFixtureRoot);
    const path = resolve(localPath);
    const rel = relative(root, path);
    if (rel.startsWith("..") || isAbsolute(rel)) {
      throw new RepositoryIngestError(
        "unsafe_path",
        "Local fixture path escapes the configured fixture root",
      );
    }
  }
  return {
    url: url.href,
    provider: "generic",
    hostname: "local-fixture",
    scheme: "file",
    localPath,
  };
}

/**
 * Validates supported remote Git URLs without performing DNS or Git operations.
 * Only HTTPS and SSH are accepted; file URLs are allowed solely for explicit
 * local-fixture mode. Embedded credentials are always rejected.
 */
export function validateGitUrl(rawUrl: string, options: GitUrlOptions): GitTarget {
  if (rawUrl.length === 0 || rawUrl.length > MAX_URL_LENGTH) {
    throw invalidUrl("Git repository URL is empty or too long");
  }
  if (containsControlCharacter(rawUrl)) {
    throw invalidUrl("Git repository URL contains control characters");
  }

  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch (error) {
    throw new RepositoryIngestError("invalid_url", "Git repository URL is invalid", {
      cause: error,
    });
  }

  if (url.protocol === "file:") {
    return validateLocalTarget(url, options);
  }
  if (url.protocol !== "https:" && url.protocol !== "ssh:") {
    throw new RepositoryIngestError(
      "unsupported_scheme",
      "Only HTTPS, SSH, and explicit local fixture Git URLs are supported",
    );
  }
  const scheme = url.protocol === "https:" ? "https" : "ssh";
  if (url.password.length > 0) {
    throw new RepositoryIngestError("credentials_in_url", "Git URL must not embed credentials");
  }
  if (scheme === "https" && url.username.length > 0) {
    throw new RepositoryIngestError("credentials_in_url", "Git URL must not embed credentials");
  }
  if (scheme === "ssh" && url.username.length > 0 && url.username !== "git") {
    throw new RepositoryIngestError("credentials_in_url", "SSH URL username must be 'git'");
  }
  if (url.search.length > 0 || url.hash.length > 0) {
    throw invalidUrl("Git repository URL must not contain a query or fragment");
  }
  assertPortAllowed(scheme, url.port);
  const hostname = normalizeHostname(url.hostname);
  assertAllowedHost(hostname, options.allowedHosts);
  decodePath(url.pathname);

  return {
    url: url.href,
    provider: providerForHost(hostname),
    hostname,
    scheme,
    localPath: null,
  };
}

export function isCommitSha(value: string): boolean {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(value);
}

/**
 * Git refs are passed as arguments, never through a shell. This validation also
 * rejects option-like values and revision expressions such as `main~1`.
 */
export function assertReferenceSafe(reference: string): void {
  if (reference.length === 0 || reference.length > 200) {
    throw new RepositoryIngestError("unsafe_reference", "Source reference is empty or too long");
  }
  if (reference !== reference.trim()) {
    throw new RepositoryIngestError(
      "unsafe_reference",
      "Source reference must not have outer whitespace",
    );
  }
  if (!/^[A-Za-z0-9._/-]+$/.test(reference)) {
    throw new RepositoryIngestError(
      "unsafe_reference",
      "Source reference contains unsupported characters",
    );
  }
  if (reference.startsWith("-") || reference.startsWith("/") || reference.endsWith("/")) {
    throw new RepositoryIngestError("unsafe_reference", "Source reference has an unsafe boundary");
  }
  if (reference.includes("..") || reference.includes("//") || reference.includes("@{")) {
    throw new RepositoryIngestError(
      "unsafe_reference",
      "Source reference contains an unsafe sequence",
    );
  }
  if (reference.endsWith(".lock")) {
    throw new RepositoryIngestError("unsafe_reference", "Source reference must not end with .lock");
  }
  for (const segment of reference.split("/")) {
    if (segment.length === 0 || segment === "." || segment === "..") {
      throw new RepositoryIngestError(
        "unsafe_reference",
        "Source reference contains an unsafe segment",
      );
    }
  }
  if (
    reference.startsWith("refs/") &&
    !reference.startsWith("refs/heads/") &&
    !reference.startsWith("refs/tags/")
  ) {
    throw new RepositoryIngestError("unsafe_reference", "Only branch and tag refs are supported");
  }
}
