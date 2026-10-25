import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { RepositoryIngestError } from "./errors";

export type HostResolver = (hostname: string) => Promise<readonly string[]>;

const INTERNAL_HOST_SUFFIXES = [
  ".home.arpa",
  ".internal",
  ".lan",
  ".local",
  ".localdomain",
  ".localhost",
] as const;

const INTERNAL_HOSTNAMES = new Set([
  "instance-data",
  "localhost",
  "metadata",
  "metadata.google.internal",
]);

export function normalizeHostname(hostname: string): string {
  return hostname
    .trim()
    .toLowerCase()
    .replace(/^\[|\]$/g, "")
    .replace(/\.$/, "");
}

export function isInternalHostname(hostname: string): boolean {
  const normalized = normalizeHostname(hostname);
  if (normalized.length === 0) return true;
  if (INTERNAL_HOSTNAMES.has(normalized)) return true;
  return INTERNAL_HOST_SUFFIXES.some((suffix) => normalized.endsWith(suffix));
}

function parseIpv4(address: string): readonly [number, number, number, number] | null {
  const parts = address.split(".");
  if (parts.length !== 4) return null;
  const octets: number[] = [];
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const value = Number.parseInt(part, 10);
    if (value < 0 || value > 255) return null;
    octets.push(value);
  }
  return [octets[0] ?? 0, octets[1] ?? 0, octets[2] ?? 0, octets[3] ?? 0];
}

function isUnsafeIpv4(octets: readonly [number, number, number, number]): boolean {
  const [a, b, c] = octets;
  if (a === 0 || a === 10 || a === 127) return true;
  if (a === 100 && b >= 64 && b <= 127) return true;
  if (a === 169 && b === 254) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 0 && (c === 0 || c === 2)) return true;
  if (a === 192 && b === 88 && c === 99) return true;
  if (a === 192 && b === 168) return true;
  if (a === 198 && (b === 18 || b === 19)) return true;
  if (a === 198 && b === 51 && c === 100) return true;
  if (a === 203 && b === 0 && c === 113) return true;
  if (a >= 224) return true;
  return false;
}

function parseIpv6Groups(address: string): number[] | null {
  const normalized = address.toLowerCase().replace(/%.*$/, "");
  if (normalized.length === 0) return null;
  const halves = normalized.split("::");
  if (halves.length > 2) return null;

  const parseHalf = (half: string): number[] | null => {
    if (half.length === 0) return [];
    const tokens = half.split(":");
    const groups: number[] = [];
    for (let index = 0; index < tokens.length; index += 1) {
      const token = tokens[index];
      if (token === undefined || token.length === 0) return null;
      if (token.includes(".")) {
        if (index !== tokens.length - 1) return null;
        const octets = parseIpv4(token);
        if (octets === null) return null;
        groups.push((octets[0] << 8) | octets[1]);
        groups.push((octets[2] << 8) | octets[3]);
        continue;
      }
      if (!/^[0-9a-f]{1,4}$/.test(token)) return null;
      groups.push(Number.parseInt(token, 16));
    }
    return groups;
  };

  const left = parseHalf(halves[0] ?? "");
  const right = halves.length === 2 ? parseHalf(halves[1] ?? "") : [];
  if (left === null || right === null) return null;
  let groups: number[];
  if (halves.length === 2) {
    const missing = 8 - left.length - right.length;
    if (missing < 1) return null;
    groups = [...left, ...new Array<number>(missing).fill(0), ...right];
  } else {
    groups = left;
  }
  return groups.length === 8 ? groups : null;
}

function unsafeEmbeddedIpv4(groups: readonly number[], start: number): boolean {
  const first = groups[start];
  const second = groups[start + 1];
  if (first === undefined || second === undefined) return true;
  return isUnsafeIpv4([(first >> 8) & 0xff, first & 0xff, (second >> 8) & 0xff, second & 0xff]);
}

function isUnsafeIpv6(address: string): boolean {
  const groups = parseIpv6Groups(address);
  if (groups === null) return true;
  const g0 = groups[0] ?? 0;
  const g1 = groups[1] ?? 0;
  const g2 = groups[2] ?? 0;
  const g3 = groups[3] ?? 0;
  const g4 = groups[4] ?? 0;
  const g5 = groups[5] ?? 0;
  const g6 = groups[6] ?? 0;
  const g7 = groups[7] ?? 0;

  if (groups.every((group) => group === 0)) return true;
  if (
    g0 === 0 &&
    g1 === 0 &&
    g2 === 0 &&
    g3 === 0 &&
    g4 === 0 &&
    g5 === 0 &&
    g6 === 0 &&
    g7 === 1
  ) {
    return true;
  }
  // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) forms.
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0xffff) {
    return unsafeEmbeddedIpv4(groups, 6);
  }
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0) {
    return unsafeEmbeddedIpv4(groups, 6);
  }
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0xffff && g5 === 0) return true;

  if ((g0 & 0xfe00) === 0xfc00) return true; // unique local
  if ((g0 & 0xffc0) === 0xfe80 || (g0 & 0xffc0) === 0xfec0) return true; // link/site local
  if ((g0 & 0xff00) === 0xff00) return true; // multicast

  if (g0 === 0x2001 && g1 === 0x0db8) return true; // documentation
  if (g0 === 0x2001 && g1 === 0x0000) return true; // Teredo
  if (g0 === 0x2001 && ((g1 & 0xfff0) === 0x0010 || (g1 & 0xfff0) === 0x0020)) return true; // ORCHID
  if (g0 === 0x2001 && g1 === 0x0002) return true; // benchmarking
  if (g0 === 0x2002) return true; // 6to4
  if (g0 === 0x0064 && g1 === 0xff9b) return true; // NAT64
  if (g0 === 0x0100 && g1 === 0 && g2 === 0 && g3 === 0) return true; // discard-only
  return false;
}

export function isPrivateOrReservedAddress(address: string): boolean {
  const normalized = address.trim().replace(/^\[|\]$/g, "");
  const family = isIP(normalized);
  if (family === 4) {
    const octets = parseIpv4(normalized);
    return octets === null || isUnsafeIpv4(octets);
  }
  if (family === 6) return isUnsafeIpv6(normalized);
  return true;
}

/**
 * Synchronous public-host check for a hostname or IP literal. It rejects
 * internal names and private, loopback, link-local, reserved, or metadata
 * addresses. It performs no DNS resolution, so a public name that resolves to a
 * private address is not caught here (matching the best-effort stance of
 * `assertRemoteHostSafe`).
 */
export function isPublicHost(hostname: string): boolean {
  const normalized = normalizeHostname(hostname);
  if (normalized.length === 0) return false;
  if (isInternalHostname(normalized)) return false;
  if (isIP(normalized) !== 0) return !isPrivateOrReservedAddress(normalized);
  return true;
}

export const defaultHostResolver: HostResolver = async (hostname) => {
  const answers = await lookup(hostname, { all: true, verbatim: true });
  return answers.map((answer) => answer.address);
};

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error("DNS lookup timed out")), timeoutMs);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * Rejects hostnames that are loopback, private, link-local, reserved, or cloud
 * metadata endpoints. Every resolved address must be public. This check runs
 * before Git opens a connection; known-provider allowlisting and disabled HTTP
 * redirects reduce (but do not eliminate) DNS-rebinding risk for the Git CLI.
 */
export async function assertRemoteHostSafe(
  hostname: string,
  resolver: HostResolver = defaultHostResolver,
  timeoutMs = 5_000,
): Promise<void> {
  const normalized = normalizeHostname(hostname);
  if (isInternalHostname(normalized)) {
    throw new RepositoryIngestError("unsupported_git_host", "Git host is not public");
  }
  if (isIP(normalized) !== 0) {
    if (isPrivateOrReservedAddress(normalized)) {
      throw new RepositoryIngestError("unsupported_git_host", "Git host is not public");
    }
    return;
  }

  let addresses: readonly string[];
  try {
    addresses = await withTimeout(resolver(normalized), timeoutMs);
  } catch (error) {
    throw new RepositoryIngestError("dns_resolution_failed", "Could not resolve Git host", {
      cause: error,
      retryable: true,
    });
  }
  if (addresses.length === 0) {
    throw new RepositoryIngestError("dns_resolution_failed", "Could not resolve Git host", {
      retryable: true,
    });
  }
  for (const address of addresses) {
    if (isPrivateOrReservedAddress(address)) {
      throw new RepositoryIngestError("unsupported_git_host", "Git host is not public");
    }
  }
}
