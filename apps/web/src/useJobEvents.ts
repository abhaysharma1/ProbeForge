import type { JobEvent, RunEvent } from "@probe-forge/contracts";
import { useEffect, useRef } from "react";

import { jobEventsUrl, runEventsUrl } from "./api";
import { JOB_EVENT_TYPES, parseJobEvent, parseRunEvent, RUN_EVENT_TYPES } from "./jobEvents";

/** Close a stream after this many consecutive transport failures. */
const MAX_CONSECUTIVE_ERRORS = 5;

/** Bound the number of streams a workspace keeps open at once. */
const MAX_CONCURRENT_STREAMS = 8;

interface StreamEntry {
  readonly source: EventSource;
  errors: number;
}

interface UseStreamsOptions<E> {
  /** Entity ids worth streaming; the caller only passes statuses that can still move. */
  readonly ids: readonly string[];
  readonly enabled: boolean;
  /** Event names to register; `EventSource` dispatches named events per type. */
  readonly eventTypes: readonly string[];
  readonly urlForId: (id: string) => string;
  readonly parseEvent: (data: string) => E | null;
  readonly onEvent: (id: string, event: E) => void;
  /** Called once when a stream gives up after repeated failures. */
  readonly onStreamError: (id: string) => void;
}

/**
 * Keep one `EventSource` per active id, opening streams only while the entity
 * can still change and closing them again when it leaves that set. The browser
 * replays missed events from `Last-Event-ID` on reconnect, so the client keeps
 * no cursor of its own; PostgreSQL stays the source of truth and the caller
 * re-reads the entity list whenever an event implies a status change.
 */
function useEventStreams<E>({
  ids,
  enabled,
  eventTypes,
  urlForId,
  parseEvent,
  onEvent,
  onStreamError,
}: UseStreamsOptions<E>): void {
  const sources = useRef(new Map<string, StreamEntry>());
  // Held in a ref so callback identity changes never reopen a live stream.
  const callbacks = useRef({ eventTypes, urlForId, parseEvent, onEvent, onStreamError });
  callbacks.current = { eventTypes, urlForId, parseEvent, onEvent, onStreamError };

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

    const wanted = new Set(ids);
    for (const [id, entry] of open) {
      if (!wanted.has(id)) {
        entry.source.close();
        open.delete(id);
      }
    }

    for (const id of ids) {
      if (open.has(id) || open.size >= MAX_CONCURRENT_STREAMS) {
        continue;
      }

      const source = new EventSource(callbacks.current.urlForId(id));
      const entry: StreamEntry = { source, errors: 0 };
      open.set(id, entry);

      const handle = (message: MessageEvent<string>) => {
        entry.errors = 0;
        const event = callbacks.current.parseEvent(message.data);
        if (event !== null) {
          callbacks.current.onEvent(id, event);
        }
      };

      for (const type of callbacks.current.eventTypes) {
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
          open.delete(id);
          callbacks.current.onStreamError(id);
        }
      };
    }
  }, [enabled, ids]);

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

export interface UseJobEventsOptions {
  /** Jobs worth streaming; the caller only passes statuses that can still move. */
  readonly jobIds: readonly string[];
  readonly enabled: boolean;
  readonly onEvent: (jobId: string, event: JobEvent) => void;
  /** Called once when a stream gives up after repeated failures. */
  readonly onStreamError: (jobId: string) => void;
}

export function useJobEvents({
  jobIds,
  enabled,
  onEvent,
  onStreamError,
}: UseJobEventsOptions): void {
  useEventStreams<JobEvent>({
    ids: jobIds,
    enabled,
    eventTypes: JOB_EVENT_TYPES,
    urlForId: jobEventsUrl,
    parseEvent: parseJobEvent,
    onEvent,
    onStreamError,
  });
}

export interface UseRunEventsOptions {
  /** Runs worth streaming; the caller only passes statuses that can still move. */
  readonly runIds: readonly string[];
  readonly enabled: boolean;
  readonly onEvent: (runId: string, event: RunEvent) => void;
  /** Called once when a stream gives up after repeated failures. */
  readonly onStreamError: (runId: string) => void;
}

export function useRunEvents({
  runIds,
  enabled,
  onEvent,
  onStreamError,
}: UseRunEventsOptions): void {
  useEventStreams<RunEvent>({
    ids: runIds,
    enabled,
    eventTypes: RUN_EVENT_TYPES,
    urlForId: runEventsUrl,
    parseEvent: parseRunEvent,
    onEvent,
    onStreamError,
  });
}
