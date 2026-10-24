import { createDecipheriv } from "node:crypto";

/**
 * Platform secret decryption. This mirrors the Rust envelope in
 * `services/api/src/settings_secrets.rs`: AES-256-GCM with a 12-byte nonce and
 * the tag appended to the ciphertext, encoded as `v1.<nonce>.<ciphertext+tag>`
 * in base64. The master key comes from `PROBE_FORGE_SETTINGS_KEY`.
 */
export const SETTINGS_KEY_ENV = "PROBE_FORGE_SETTINGS_KEY";

const VERSION = "v1";
const NONCE_BYTES = 12;
const TAG_BYTES = 16;
const KEY_BYTES = 32;

export function loadSettingsKey(
  env: Record<string, string | undefined> = process.env,
): Buffer | null {
  const raw = env[SETTINGS_KEY_ENV]?.trim();
  if (raw === undefined || raw.length === 0) return null;
  const key = Buffer.from(raw, "base64");
  if (key.length !== KEY_BYTES) {
    throw new Error(`${SETTINGS_KEY_ENV} must be base64-encoded 32 bytes`);
  }
  return key;
}

export function decryptSecret(key: Buffer, envelope: string): string {
  if (key.length !== KEY_BYTES) {
    throw new Error("settings key must be 32 bytes");
  }
  const parts = envelope.split(".");
  if (parts.length !== 3 || parts[0] !== VERSION) {
    throw new Error("invalid secret envelope");
  }
  const nonce = Buffer.from(parts[1] ?? "", "base64");
  const payload = Buffer.from(parts[2] ?? "", "base64");
  if (nonce.length !== NONCE_BYTES || payload.length < TAG_BYTES) {
    throw new Error("invalid secret envelope");
  }
  const ciphertext = payload.subarray(0, payload.length - TAG_BYTES);
  const tag = payload.subarray(payload.length - TAG_BYTES);
  const decipher = createDecipheriv("aes-256-gcm", key, nonce);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString("utf8");
}
