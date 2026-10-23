import { describe, expect, it } from "bun:test";
import { RepositoryIngestError, type RepositoryIngestErrorCode } from "./errors";
import { assertRemoteHostSafe, isInternalHostname, isPrivateOrReservedAddress } from "./net";

async function expectCode(
  promise: Promise<unknown>,
  code: RepositoryIngestErrorCode,
): Promise<void> {
  try {
    await promise;
    throw new Error("expected an error");
  } catch (error) {
    expect(error).toBeInstanceOf(RepositoryIngestError);
    expect((error as RepositoryIngestError).code).toBe(code);
  }
}

describe("address policy", () => {
  it("rejects loopback, private, link-local, reserved, and metadata addresses", () => {
    for (const address of [
      "0.0.0.0",
      "10.0.0.1",
      "100.64.0.1",
      "127.0.0.1",
      "169.254.169.254",
      "172.16.0.1",
      "192.0.2.1",
      "192.168.1.1",
      "198.18.0.1",
      "224.0.0.1",
      "::",
      "::1",
      "fc00::1",
      "fd00::1",
      "fe80::1",
      "ff02::1",
      "2001:db8::1",
      "2001::1",
      "2001:10::1",
      "2002:7f00:1::1",
      "64:ff9b::7f00:1",
      "::ffff:127.0.0.1",
      "::ffff:7f00:1",
      "0:0:0:0:0:ffff:7f00:1",
      "::ffff:192.168.1.1",
      "::127.0.0.1",
      "::7f00:1",
    ]) {
      expect(isPrivateOrReservedAddress(address)).toBe(true);
    }
  });

  it("accepts public IPv4 and IPv6 addresses", () => {
    for (const address of ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111", "2001:4860:4860::8888"]) {
      expect(isPrivateOrReservedAddress(address)).toBe(false);
    }
  });

  it("recognizes internal hostnames", () => {
    for (const hostname of [
      "localhost",
      "api.localhost",
      "metadata.google.internal",
      "service.internal",
      "router.local",
      "printer.home.arpa",
    ]) {
      expect(isInternalHostname(hostname)).toBe(true);
    }
    expect(isInternalHostname("github.com")).toBe(false);
  });
});

describe("assertRemoteHostSafe", () => {
  it("rejects literal private and internal hosts without DNS", async () => {
    await expectCode(assertRemoteHostSafe("127.0.0.1"), "unsupported_git_host");
    await expectCode(assertRemoteHostSafe("localhost"), "unsupported_git_host");
  });

  it("rejects a hostname if any resolved address is private", async () => {
    await expectCode(
      assertRemoteHostSafe("github.com", async () => ["140.82.112.3", "10.0.0.5"]),
      "unsupported_git_host",
    );
  });

  it("accepts a hostname when all resolved addresses are public", async () => {
    await expect(
      assertRemoteHostSafe("github.com", async () => ["140.82.112.3", "2606:50c0::1"]),
    ).resolves.toBeUndefined();
  });

  it("fails closed when DNS resolution fails or returns nothing", async () => {
    await expectCode(
      assertRemoteHostSafe("github.com", async () => {
        throw new Error("dns failure");
      }),
      "dns_resolution_failed",
    );
    await expectCode(
      assertRemoteHostSafe("github.com", async () => []),
      "dns_resolution_failed",
    );
  });
});
