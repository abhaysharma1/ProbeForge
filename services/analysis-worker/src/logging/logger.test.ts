import { describe, expect, test } from "bun:test";

import {
  createLogger,
  parseLogFormat,
  parseLogLevel,
  REDACTED,
  redactText,
  redactValue,
} from "./logger";

function capture(): { lines: string[]; sink: (line: string) => void } {
  const lines: string[] = [];
  return { lines, sink: (line) => lines.push(line) };
}

describe("parseLogLevel / parseLogFormat", () => {
  test("accepts known values case-insensitively and rejects the rest", () => {
    expect(parseLogLevel("DEBUG")).toBe("debug");
    expect(parseLogLevel(" info ")).toBe("info");
    expect(parseLogLevel("verbose")).toBeNull();
    expect(parseLogLevel(undefined)).toBeNull();
    expect(parseLogFormat("JSON")).toBe("json");
    expect(parseLogFormat("text")).toBe("text");
    expect(parseLogFormat("xml")).toBeNull();
  });
});

describe("createLogger", () => {
  test("emits JSON records with level, service, and event", () => {
    const { lines, sink } = capture();
    const logger = createLogger({ service: "worker", sink, now: () => new Date(0) });
    logger.info("job.processed", { jobId: "abc" });

    expect(lines).toHaveLength(1);
    const record = JSON.parse(lines[0] ?? "{}");
    expect(record.level).toBe("info");
    expect(record.service).toBe("worker");
    expect(record.event).toBe("job.processed");
    expect(record.jobId).toBe("abc");
    expect(record.time).toBe("1970-01-01T00:00:00.000Z");
  });

  test("filters records below the configured level", () => {
    const { lines, sink } = capture();
    const logger = createLogger({ service: "worker", level: "warn", sink });
    logger.debug("ignored");
    logger.info("ignored");
    logger.warn("kept");
    logger.error("kept");
    expect(lines).toHaveLength(2);
  });

  test("child loggers merge bound fields into every record", () => {
    const { lines, sink } = capture();
    const logger = createLogger({ service: "worker", sink }).child({ workerId: "w1" });
    logger.info("loop.started", { intervalMs: 5000 });
    const record = JSON.parse(lines[0] ?? "{}");
    expect(record.workerId).toBe("w1");
    expect(record.intervalMs).toBe(5000);
    expect(record.event).toBe("loop.started");
  });

  test("redacts sensitive keys and inline markers before emitting", () => {
    const { lines, sink } = capture();
    const logger = createLogger({ service: "worker", sink });
    logger.warn("auth.failed", {
      authorization: "Bearer super-secret-token",
      nested: { clientSecret: "shh" },
      note: "sent Authorization: Bearer abc1234567890 to the target",
    });
    const record = JSON.parse(lines[0] ?? "{}");
    expect(record.authorization).toBe(REDACTED);
    expect(record.nested.clientSecret).toBe(REDACTED);
    expect(lines[0]).not.toContain("super-secret-token");
    expect(lines[0]).not.toContain("abc1234567890");
  });

  test("renders a readable text line when requested", () => {
    const { lines, sink } = capture();
    const logger = createLogger({
      service: "worker",
      format: "text",
      sink,
      now: () => new Date(0),
    });
    logger.info("job.processed", { jobId: "abc" });
    expect(lines[0]).toBe("1970-01-01T00:00:00.000Z INFO worker job.processed jobId=abc");
  });
});

describe("redaction helpers", () => {
  test("redacts JWT-shaped tokens", () => {
    const token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signature";
    const output = redactText(`${token} rest`);
    expect(output).toContain(REDACTED);
    expect(output).not.toContain(token);
  });

  test("recurses through arrays and objects", () => {
    const redacted = redactValue({
      list: ["token=abc", "keep"],
      nested: { apiKey: "secret-value", safe: "keep" },
    }) as Record<string, unknown>;
    expect(redacted.nested).toEqual({ apiKey: REDACTED, safe: "keep" });
    expect(JSON.stringify(redacted)).not.toContain("secret-value");
  });
});
