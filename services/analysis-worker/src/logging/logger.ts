// Structured logging for the analysis worker.
//
// This mirrors `crates/observability`: one implementation governs levels,
// correlation, and redaction so a Rust service line and a worker line read the
// same way in a log aggregator. Logging is dependency-free by design (see
// AGENTS.md): no package is added for it.
//
// Contract (the same one the Rust services follow):
//   * levels: error > warn > info > debug > trace, selected by
//     PROBE_FORGE_LOG_LEVEL (default info). Hot paths stay at `debug`.
//   * every line carries `service` and a stable `event` name in snake_case, plus
//     whatever correlation fields the caller passes (jobId, projectId, runId,
//     workerId). Fields are camelCase to match the JSON APIs.
//   * values are redacted before they are emitted. Never pass headers, tokens,
//     cookies, request/response bodies, repository file contents, or secrets.

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

export type LogFormat = "json" | "text";

export const REDACTED = "[redacted]";

const LEVEL_RANK: Record<LogLevel, number> = {
  trace: 10,
  debug: 20,
  info: 30,
  warn: 40,
  error: 50,
};

/** Object keys whose values are replaced wholesale, matched case-insensitively. */
const SENSITIVE_KEYS: ReadonlySet<string> = new Set([
  "authorization",
  "cookie",
  "set-cookie",
  "proxy-authorization",
  "password",
  "passwd",
  "secret",
  "client_secret",
  "clientsecret",
  "api_key",
  "apikey",
  "access_token",
  "accesstoken",
  "refresh_token",
  "refreshtoken",
  "private_key",
  "privatekey",
  "token",
]);

export interface LogFields {
  readonly [key: string]: unknown;
}

export interface LogRecord extends LogFields {
  readonly level: LogLevel;
  readonly time: string;
  readonly service: string;
  readonly event: string;
}

export interface Logger {
  error(event: string, fields?: LogFields): void;
  warn(event: string, fields?: LogFields): void;
  info(event: string, fields?: LogFields): void;
  debug(event: string, fields?: LogFields): void;
  trace(event: string, fields?: LogFields): void;
  /** A logger that adds `fields` to every record it emits. */
  child(fields: LogFields): Logger;
}

export interface LoggerOptions {
  readonly service: string;
  readonly level?: LogLevel;
  readonly format?: LogFormat;
  /** Destination for a rendered line. Defaults to `console.log`. */
  readonly sink?: (line: string) => void;
  /** Clock override for deterministic tests. */
  readonly now?: () => Date;
}

/** Parse a level name, returning `null` for anything unrecognized. */
export function parseLogLevel(value: string | undefined): LogLevel | null {
  if (value === undefined) return null;
  const normalized = value.trim().toLowerCase();
  return normalized === "trace" ||
    normalized === "debug" ||
    normalized === "info" ||
    normalized === "warn" ||
    normalized === "error"
    ? normalized
    : null;
}

/** Parse a log format, returning `null` for anything unrecognized. */
export function parseLogFormat(value: string | undefined): LogFormat | null {
  if (value === undefined) return null;
  const normalized = value.trim().toLowerCase();
  return normalized === "json" || normalized === "text" ? normalized : null;
}

/** Redact credential markers embedded in free text. */
export function redactText(input: string): string {
  let output = input;
  for (const marker of ["bearer ", "basic ", "authorization: ", "cookie: "]) {
    output = redactMarker(output, marker);
  }
  return redactJwts(output);
}

function redactMarker(input: string, marker: string): string {
  const lower = input.toLowerCase();
  let output = "";
  let cursor = 0;
  for (;;) {
    const relative = lower.indexOf(marker, cursor);
    if (relative === -1) break;
    const start = relative;
    const after = start + marker.length;
    output += input.slice(cursor, after);
    let tokenStart = after;
    while (tokenStart < input.length && (input[tokenStart] === " " || input[tokenStart] === "\t")) {
      tokenStart += 1;
    }
    let tokenEnd = tokenStart;
    while (tokenEnd < input.length) {
      const character = input[tokenEnd];
      if (
        character === undefined ||
        character === " " ||
        character === "\t" ||
        character === "\n" ||
        character === "\r" ||
        character === '"' ||
        character === ","
      ) {
        break;
      }
      tokenEnd += 1;
    }
    if (tokenEnd > tokenStart) {
      output += REDACTED;
    }
    cursor = tokenEnd;
  }
  output += input.slice(cursor);
  return output;
}

function redactJwts(input: string): string {
  let output = "";
  let index = 0;
  while (index < input.length) {
    const character = input[index];
    const isBoundary =
      character === " " || character === "\n" || character === "\r" || character === '"';
    if (isBoundary) {
      output += character;
      index += 1;
      continue;
    }
    let end = index;
    while (end < input.length) {
      const next = input[end];
      if (next === undefined || next === " " || next === "\n" || next === "\r" || next === '"') {
        break;
      }
      end += 1;
    }
    const token = input.slice(index, end);
    const dots = token.split(".").length - 1;
    output += token.startsWith("eyJ") && dots === 2 ? REDACTED : token;
    index = end;
  }
  return output;
}

/** Recursively redact sensitive keys and credential markers in a value. */
export function redactValue(value: unknown): unknown {
  if (typeof value === "string") return redactText(value);
  if (Array.isArray(value)) return value.map(redactValue);
  if (value !== null && typeof value === "object") {
    const output: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      output[key] = SENSITIVE_KEYS.has(key.toLowerCase()) ? REDACTED : redactValue(item);
    }
    return output;
  }
  return value;
}

function renderText(record: LogRecord): string {
  const reserved = new Set(["level", "time", "service", "event"]);
  const fields = Object.entries(record)
    .filter(([key]) => !reserved.has(key))
    .map(([key, value]) => `${key}=${formatField(value)}`)
    .join(" ");
  const suffix = fields.length > 0 ? ` ${fields}` : "";
  return `${record.time} ${record.level.toUpperCase()} ${record.service} ${record.event}${suffix}`;
}

function formatField(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}

/**
 * Create a structured logger. `format` defaults to `json` so the worker's output
 * is machine-parseable by default; `text` is available for local reading.
 */
export function createLogger(options: LoggerOptions): Logger {
  const level = options.level ?? "info";
  const format = options.format ?? "json";
  const sink = options.sink ?? ((line: string) => console.log(line));
  const now = options.now ?? (() => new Date());
  const threshold = LEVEL_RANK[level];

  function emit(event: string, recordLevel: LogLevel, output: LogFields): void {
    if (LEVEL_RANK[recordLevel] < threshold) return;
    const record = redactValue({
      ...output,
      level: recordLevel,
      time: now().toISOString(),
      service: options.service,
      event,
    }) as LogRecord;
    sink(format === "json" ? JSON.stringify(record) : renderText(record));
  }

  function make(bound: LogFields): Logger {
    return {
      error: (event, fields) => emit(event, "error", { ...bound, ...fields }),
      warn: (event, fields) => emit(event, "warn", { ...bound, ...fields }),
      info: (event, fields) => emit(event, "info", { ...bound, ...fields }),
      debug: (event, fields) => emit(event, "debug", { ...bound, ...fields }),
      trace: (event, fields) => emit(event, "trace", { ...bound, ...fields }),
      child: (fields) => make({ ...bound, ...fields }),
    };
  }

  return make({});
}
