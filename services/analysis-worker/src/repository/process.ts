import { type ChildProcess, spawn } from "node:child_process";

export type BoundedProcessErrorCode =
  | "process_aborted"
  | "process_failed"
  | "process_output_limit"
  | "process_spawn_failed"
  | "process_timeout";

export class BoundedProcessError extends Error {
  readonly code: BoundedProcessErrorCode;
  readonly stdout: Buffer;
  readonly stderr: Buffer;
  readonly exitCode: number | null;

  constructor(
    code: BoundedProcessErrorCode,
    message: string,
    options: {
      readonly stdout?: Buffer;
      readonly stderr?: Buffer;
      readonly exitCode?: number | null;
      readonly cause?: unknown;
    } = {},
  ) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = "BoundedProcessError";
    this.code = code;
    this.stdout = options.stdout ?? Buffer.alloc(0);
    this.stderr = options.stderr ?? Buffer.alloc(0);
    this.exitCode = options.exitCode ?? null;
  }
}

export interface BoundedProcessOptions {
  readonly cwd?: string;
  readonly env?: NodeJS.ProcessEnv;
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
  readonly signal?: AbortSignal;
}

export interface BoundedProcessResult {
  readonly stdout: Buffer;
  readonly stderr: Buffer;
  readonly exitCode: number;
}

/**
 * Runs a fixed executable with an argument array and no shell. Output and runtime
 * are bounded, and cancellation terminates the child process tree.
 */
export async function runBoundedProcess(
  executable: string,
  args: readonly string[],
  options: BoundedProcessOptions,
): Promise<BoundedProcessResult> {
  if (options.signal?.aborted) {
    throw new BoundedProcessError("process_aborted", "Process was cancelled before it started");
  }
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("timeoutMs must be a positive integer");
  }
  if (!Number.isSafeInteger(options.maxOutputBytes) || options.maxOutputBytes <= 0) {
    throw new Error("maxOutputBytes must be a positive integer");
  }

  return await new Promise<BoundedProcessResult>((resolve, reject) => {
    let child: ChildProcess;
    const stdoutChunks: Buffer[] = [];
    const stderrChunks: Buffer[] = [];
    let outputBytes = 0;
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const cleanup = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
    };

    const finish = (error: BoundedProcessError | null, result?: BoundedProcessResult): void => {
      if (settled) return;
      settled = true;
      cleanup();
      if (error !== null) {
        reject(error);
        return;
      }
      if (result !== undefined) resolve(result);
    };

    const outputError = (): BoundedProcessError =>
      new BoundedProcessError(
        "process_output_limit",
        `Process output exceeded ${options.maxOutputBytes} bytes`,
        {
          stdout: Buffer.concat(stdoutChunks),
          stderr: Buffer.concat(stderrChunks),
        },
      );

    const killTree = (): void => {
      if (child.pid === undefined) return;
      if (process.platform === "win32") {
        try {
          const killer = spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
            stdio: "ignore",
            windowsHide: true,
            shell: false,
          });
          killer.on("error", () => undefined);
        } catch {
          // Best effort. The direct child may still be killable.
        }
      } else {
        try {
          process.kill(-child.pid, "SIGKILL");
          return;
        } catch {
          // Fall through to the direct kill below.
        }
      }
      try {
        child.kill("SIGKILL");
      } catch {
        // The process may already have exited.
      }
    };

    const fail = (code: BoundedProcessErrorCode, message: string): void => {
      if (settled) return;
      const error = new BoundedProcessError(code, message, {
        stdout: Buffer.concat(stdoutChunks),
        stderr: Buffer.concat(stderrChunks),
      });
      killTree();
      finish(error);
    };

    const onAbort = (): void => {
      fail("process_aborted", "Process was cancelled");
    };

    try {
      child = spawn(executable, [...args], {
        ...(options.cwd === undefined ? {} : { cwd: options.cwd }),
        ...(options.env === undefined ? {} : { env: options.env }),
        detached: process.platform !== "win32",
        shell: false,
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
    } catch (error) {
      finish(
        new BoundedProcessError("process_spawn_failed", "Failed to start process", {
          cause: error,
        }),
      );
      return;
    }

    options.signal?.addEventListener("abort", onAbort, { once: true });

    timer = setTimeout(() => {
      fail("process_timeout", `Process exceeded the ${options.timeoutMs} ms timeout`);
    }, options.timeoutMs);

    const collect = (chunk: Buffer, target: Buffer[]): void => {
      if (settled) return;
      outputBytes += chunk.byteLength;
      if (outputBytes > options.maxOutputBytes) {
        finish(outputError());
        killTree();
        return;
      }
      target.push(chunk);
    };

    child.stdout?.on("data", (chunk: Buffer) => collect(chunk, stdoutChunks));
    child.stderr?.on("data", (chunk: Buffer) => collect(chunk, stderrChunks));
    child.on("error", (error) => {
      finish(
        new BoundedProcessError("process_spawn_failed", "Failed to start process", {
          cause: error,
        }),
      );
    });
    child.on("close", (code) => {
      if (settled) return;
      const stdout = Buffer.concat(stdoutChunks);
      const stderr = Buffer.concat(stderrChunks);
      if (code === 0) {
        finish(null, { stdout, stderr, exitCode: 0 });
        return;
      }
      finish(
        new BoundedProcessError("process_failed", `Process exited with code ${code ?? "unknown"}`, {
          stdout,
          stderr,
          exitCode: code,
        }),
      );
    });
  });
}
