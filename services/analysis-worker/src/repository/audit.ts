import { appendFile, mkdir } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";
import type { GitProvider } from "./git-url";

export type AuditEventType =
  | "repository.clone.started"
  | "repository.ingest.failed"
  | "repository.reference.resolved"
  | "repository.snapshot.ready"
  | "repository.workspace.cleaned"
  | "repository.workspace.cleanup_failed";

export interface SnapshotAuditEvent {
  readonly event: AuditEventType;
  readonly at: string;
  readonly workspaceId: string;
  readonly provider?: GitProvider;
  readonly sourceReference?: string;
  readonly commitSha?: string;
  readonly fileCount?: number;
  readonly totalBytes?: number;
  readonly errorCode?: string;
}

export interface AuditSink {
  write(event: SnapshotAuditEvent): Promise<void>;
}

export class MemoryAuditSink implements AuditSink {
  readonly events: SnapshotAuditEvent[] = [];

  async write(event: SnapshotAuditEvent): Promise<void> {
    this.events.push(event);
  }
}

/**
 * Durable, append-only JSONL audit sink. Audit events deliberately omit remote
 * URLs, credential references, absolute paths, and repository content.
 */
export class JsonLineAuditSink implements AuditSink {
  readonly #path: string;

  constructor(path: string) {
    if (path.length === 0 || path.includes("\0")) {
      throw new Error("Audit log path is invalid");
    }
    this.#path = isAbsolute(path) ? path : resolve(path);
  }

  async write(event: SnapshotAuditEvent): Promise<void> {
    await mkdir(dirname(this.#path), { recursive: true, mode: 0o700 });
    await appendFile(this.#path, `${JSON.stringify(event)}\n`, { encoding: "utf8", mode: 0o600 });
  }
}
