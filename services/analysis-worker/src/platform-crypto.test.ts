import { describe, expect, it } from "bun:test";
import { decryptSecret, loadSettingsKey, SETTINGS_KEY_ENV } from "./platform-crypto";

// Shared with the Rust unit test in `services/api/src/settings_secrets.rs`.
const VECTOR_KEY = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const VECTOR_ENVELOPE = "v1.BQUFBQUFBQUFBQUF.koPoSrSwGVySEDhd8Qj6wHowbWCHsTCmejbxTN2OTMcYZmNhQFSL";
const VECTOR_PLAINTEXT = "probe-forge-test-vector";

describe("platform secret decryption", () => {
  it("decrypts the cross-language test vector", () => {
    const key = loadSettingsKey({ [SETTINGS_KEY_ENV]: VECTOR_KEY });
    expect(key).not.toBeNull();
    expect(decryptSecret(key as Buffer, VECTOR_ENVELOPE)).toBe(VECTOR_PLAINTEXT);
  });

  it("returns null without a key and rejects tampered envelopes", () => {
    expect(loadSettingsKey({})).toBeNull();
    const key = Buffer.alloc(32, 9);
    expect(() => decryptSecret(key, VECTOR_ENVELOPE)).toThrow();
    expect(() => decryptSecret(key, "garbage")).toThrow();
  });

  it("rejects a wrong-length key", () => {
    expect(() => loadSettingsKey({ [SETTINGS_KEY_ENV]: "AAAA" })).toThrow();
  });
});
