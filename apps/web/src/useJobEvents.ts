import type { JobEvent } from "@probe-forge/contracts";
import { useEffect, useRef } from "react";

import { JOB_EVENT_TYPES, parseJobEvent } from "./jobEvents";

/** Close a stream after this many consecutive transport failures. */
const MAX_CONSECUTIVE_ERRORS = 5;

/** Bound the number of streams a workspace keeps open at once. */
const MAX_CONCURRENT_STREAMS = 8;

interface StreamEntry {
  readonly source: EventSource;
  errors: number;
}

export interface UseJobEventsOptions {
  /** Jobs worth streaming; the caller only passes statuses that can still move. */
  readonly jobIds: readonly string[];
  readonly enabled: boolean;
  readonly urlForJob: (jobId: string) => string;
  readonly onEvent: (jobId: string, event: JobEvent) => void;
  /** Called once when a stream gives up after repeated failures. */
  readonly onStreamError: (jobId: string) => void;
}

/**
 * Keep one `EventSource` per active job, opening streams only while a job can
 * still change and closing them again when it leaves that set. The browser
 * replays missed events from `Last-Event-ID` on reconnect, so the client keeps
 * no cursor of its own; PostgreSQL stays the source of truth and the caller
 * re-reads the job list whenever an event implies a status change.
 */
export function useJobEvents({
  jobIds,
  enabled,
  urlForJob,
  onEvent,
  onStreamError,
}: UseJobEventsOptions): void {
  const sources = useRef(new Map<string, StreamEntry>());
  // Held in a ref so callback identity changes never reopen a live stream.
  const callbacks = useRef({ urlForJob, onEvent, onStreamError });
  callbacks.current = { urlForJob, onEvent, onStreamError };

  useEffect(() => {
    const open = sources.current;
    const closeAll = () => {
      for (const entry of open.values()) {
        entry.source.close();
      }
      open.clear();
    };

    if (!enabled || typeof EventSource === "undefined") {
      closeAll();
      return;
    }

    const wanted = new Set(jobIds);
    for (const [jobId, entry] of open) {
      if (!wanted.has(jobId)) {
        entry.source.close();
        open.delete(jobId);
      }
    }

    for (const jobId of jobIds) {
      if (open.has(jobId) || open.size >= MAX_CONCURRENT_STREAMS) {
        continue;
      }

      const source = new EventSource(callbacks.current.urlForJob(jobId));
      const entry: StreamEntry = { source, errors: 0 };
      open.set(jobId, entry);

      const handle = (message: MessageEvent<string>) => {
        entry.errors = 0;
        const event = parseJobEvent(message.data);
        if (event !== null) {
          callbacks.current.onEvent(jobId, event);
        }
      };

      for (const type of JOB_EVENT_TYPES) {
        source.addEventListener(type, handle as EventListener);
      }
      // The unnamed `stream.error` event arrives on the default channel.
      source.onmessage = handle;
      source.onopen = () => {
        entry.errors = 0;
      };
      source.onerror = () => {
        entry.errors += 1;
        if (source.readyState === EventSource.CLOSED || entry.errors >= MAX_CONSECUTIVE_ERRORS) {
          source.close();
          open.delete(jobId);
          callbacks.current.onStreamError(jobId);
        }
      };
    }
  }, [enabled, jobIds]);

  useEffect(
    () => () => {
      for (const entry of sources.current.values()) {
        entry.source.close();
      }
      sources.current.clear();
    },
    [],
  );
}
