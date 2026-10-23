import { RepositoryIngestError } from "./errors";
import { containsHeaderControl, stripUnsafeControls } from "./text";

export interface HttpBasicCredential {
  readonly kind: "http_basic";
  readonly username: string;
  readonly password: string;
}

export interface SshPrivateKeyCredential {
  readonly kind: "ssh_private_key";
  readonly privateKey: string;
}

export type GitCredential = HttpBasicCredential | SshPrivateKeyCredential;

export interface SecretResolver {
  resolve(reference: string): Promise<GitCredential | null>;
}

const CREDENTIAL_REF_RE = /^secret:\/\/[a-z0-9][a-z0-9/_.-]{0,190}$/;

export function assertCredentialRefSafe(reference: string): void {
  if (!CREDENTIAL_REF_RE.test(reference)) {
    throw new RepositoryIngestError(
      "invalid_credential_ref",
      "Credential reference must use the secret:// scheme",
    );
  }
}

export class EnvSecretResolver implements SecretResolver {
  readonly #env: Readonly<Record<string, string | undefined>>;

  constructor(env: Readonly<Record<string, string | undefined>> = process.env) {
    this.#env = env;
  }

  async resolve(reference: string): Promise<GitCredential | null> {
    assertCredentialRefSafe(reference);
    const key = secretEnvironmentKey(reference);
    const raw = this.#env[key];
    if (raw === undefined || raw.length === 0) return null;
    if (raw.length > 131_072) {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Resolved credential is too large",
      );
    }

    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch (error) {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Resolved credential is invalid",
        {
          cause: error,
        },
      );
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Resolved credential is invalid",
      );
    }
    const candidate = parsed as Record<string, unknown>;
    if (candidate.kind === "ssh_private_key") {
      if (candidate.passphrase !== undefined) {
        throw new RepositoryIngestError(
          "unsupported_credential_kind",
          "Passphrase-protected SSH keys are not supported in this phase",
        );
      }
      const privateKey = candidate.privateKey;
      if (
        typeof privateKey !== "string" ||
        privateKey.length < 100 ||
        privateKey.length > 65_536 ||
        privateKey.includes("\0") ||
        !privateKey.includes("-----BEGIN ") ||
        !privateKey.includes("PRIVATE KEY-----")
      ) {
        throw new RepositoryIngestError(
          "unsupported_credential_kind",
          "Resolved SSH credential is invalid",
        );
      }
      return { kind: "ssh_private_key", privateKey };
    }
    if (candidate.kind !== "http_basic") {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Only HTTP basic and unencrypted SSH key Git credentials are supported in this phase",
      );
    }
    const username = candidate.username;
    const password = candidate.password;
    if (
      typeof username !== "string" ||
      typeof password !== "string" ||
      username.length === 0 ||
      username.length > 256 ||
      password.length === 0 ||
      password.length > 4_096
    ) {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Resolved credential is invalid",
      );
    }
    if (
      username.includes(":") ||
      containsHeaderControl(username) ||
      containsHeaderControl(password)
    ) {
      throw new RepositoryIngestError(
        "unsupported_credential_kind",
        "Resolved credential is invalid",
      );
    }
    return { kind: "http_basic", username, password };
  }
}

export function secretEnvironmentKey(reference: string): string {
  assertCredentialRefSafe(reference);
  const name = reference
    .slice("secret://".length)
    .toUpperCase()
    .replace(/[^A-Z0-9]/g, "_");
  return `PROBE_FORGE_SECRET_${name}`;
}

export function basicAuthorizationValue(credential: HttpBasicCredential): string {
  return `Basic ${Buffer.from(`${credential.username}:${credential.password}`, "utf8").toString("base64")}`;
}

/**
 * Git can echo remote URLs and HTTP headers in errors. Redact obvious credential
 * forms before the message reaches a log, audit event, or CLI response.
 */
export function redactGitError(input: string): string {
  return stripUnsafeControls(input)
    .replace(/\b(?:https?|ssh|file):\/\/[^\s'"]+/gi, "<redacted-url>")
    .replace(/\b[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:[^\s'"]+/g, "<redacted-url>")
    .replace(/\bAuthorization:\s*[^\r\n]+/gi, "Authorization: <redacted>")
    .replace(/\bBasic\s+[A-Za-z0-9+/=]+/g, "Basic <redacted>")
    .replace(/\b(token|password|secret|passwd)=([^\s&]+)/gi, "$1=<redacted>")
    .replace(/\bGIT_CONFIG_VALUE_\d+=\S+/g, "GIT_CONFIG_VALUE_<redacted>=<redacted>")
    .slice(0, 2_000);
}
