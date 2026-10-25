import type { SQL } from "bun";
import type { PlanningLimits } from "./agent/limits";
import { decryptSecret, loadSettingsKey } from "./platform-crypto";

/**
 * Reads the admin-configured planning settings and decrypts the provider key.
 * The plaintext key is returned only to the caller that builds the runtime; it
 * is never logged, persisted, or included in planning metadata.
 */
export interface PlatformPlanningSettings {
  readonly provider: string | null;
  readonly model: string | null;
  readonly baseUrl: string | null;
  readonly contextWindow: number | null;
  readonly maxOutputTokens: number | null;
  readonly limitsOverride: Record<string, unknown>;
  readonly apiKey: string | null;
}

interface Row {
  readonly planning_provider: string | null;
  readonly planning_model: string | null;
  readonly planning_base_url: string | null;
  readonly planning_context_window: number | null;
  readonly planning_max_output_tokens: number | null;
  readonly planning_limits: Record<string, unknown>;
  readonly planning_api_key_ciphertext: string | null;
}

export async function loadPlatformPlanning(sql: SQL): Promise<PlatformPlanningSettings> {
  const rows = await sql<Row[]>`
    SELECT planning_provider, planning_model, planning_base_url,
           planning_context_window, planning_max_output_tokens, planning_limits,
           planning_api_key_ciphertext
    FROM platform_settings
    WHERE id = true
  `;
  const row = rows[0];
  const ciphertext = row?.planning_api_key_ciphertext ?? null;
  let apiKey: string | null = null;
  if (ciphertext !== null) {
    const key = loadSettingsKey();
    if (key !== null) {
      try {
        apiKey = decryptSecret(key, ciphertext);
      } catch {
        apiKey = null;
      }
    }
  }
  return {
    provider: row?.planning_provider ?? null,
    model: row?.planning_model ?? null,
    baseUrl: row?.planning_base_url ?? null,
    contextWindow: row?.planning_context_window ?? null,
    maxOutputTokens: row?.planning_max_output_tokens ?? null,
    limitsOverride: row?.planning_limits ?? {},
    apiKey,
  };
}

/** Overlay stored limit values onto the server defaults, ignoring junk. */
export function mergePlanningLimits(
  base: PlanningLimits,
  override: Record<string, unknown>,
): PlanningLimits {
  const merged: Record<string, number> = { ...base } as Record<string, number>;
  for (const [key, value] of Object.entries(override)) {
    if (key in merged && typeof value === "number" && Number.isFinite(value) && value >= 0) {
      merged[key] = value;
    }
  }
  return merged as unknown as PlanningLimits;
}
