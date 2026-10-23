import { describe, expect, it } from "bun:test";
import { BoundedProcessError, type BoundedProcessErrorCode, runBoundedProcess } from "./process";

async function expectCode(
  promise: Promise<unknown>,
  code: BoundedProcessErrorCode,
): Promise<BoundedProcessError> {
  try {
    await promise;
    throw new Error("expected an error");
  } catch (error) {
    expect(error).toBeInstanceOf(BoundedProcessError);
    const bounded = error as BoundedProcessError;
    expect(bounded.code).toBe(code);
    return bounded;
  }
}

describe("runBoundedProcess", () => {
  it("runs a fixed executable with arguments and captures bounded output", async () => {
    const result = await runBoundedProcess(
      process.execPath,
      ["-e", "process.stdout.write('stdout'); process.stderr.write('stderr')"],
      { timeoutMs: 5_000, maxOutputBytes: 1_024 },
    );
    expect(result.exitCode).toBe(0);
    expect(result.stdout.toString("utf8")).toBe("stdout");
    expect(result.stderr.toString("utf8")).toBe("stderr");
  });

  it("reports a non-zero exit without a shell", async () => {
    const error = await expectCode(
      runBoundedProcess(process.execPath, ["-e", "process.stdout.write('bad'); process.exit(3)"], {
        timeoutMs: 5_000,
        maxOutputBytes: 1_024,
      }),
      "process_failed",
    );
    expect(error.exitCode).toBe(3);
    expect(error.stdout.toString("utf8")).toBe("bad");
  });

  it("kills a process that exceeds the output limit", async () => {
    await expectCode(
      runBoundedProcess(
        process.execPath,
        ["-e", "process.stdout.write('x'.repeat(10000)); setTimeout(() => {}, 5000)"],
        { timeoutMs: 5_000, maxOutputBytes: 100 },
      ),
      "process_output_limit",
    );
  });

  it("kills a process that exceeds the timeout", async () => {
    await expectCode(
      runBoundedProcess(process.execPath, ["-e", "setTimeout(() => {}, 5000)"], {
        timeoutMs: 100,
        maxOutputBytes: 1_024,
      }),
      "process_timeout",
    );
  });

  it("kills a process on cancellation", async () => {
    const controller = new AbortController();
    setTimeout(() => controller.abort(), 20);
    await expectCode(
      runBoundedProcess(process.execPath, ["-e", "setTimeout(() => {}, 5000)"], {
        timeoutMs: 5_000,
        maxOutputBytes: 1_024,
        signal: controller.signal,
      }),
      "process_aborted",
    );
  });
});
