import { describe, expect, it } from "bun:test";

/**
 * Phase 1 pins the Pi SDK and proves it loads under the chosen Bun runtime.
 * The constrained tool registry and planning session arrive in Phase 7; no
 * repository access or model calls are made in this phase.
 *
 * The SDK pulls in provider modules on first import, so allow a longer timeout
 * than Bun's 5s default.
 */
describe("Pi SDK pin", () => {
  it("loads the pinned agent runtime", async () => {
    const sdk = await import("@earendil-works/pi-agent-core");
    expect(Object.keys(sdk).length).toBeGreaterThan(0);
  }, 30_000);
});
