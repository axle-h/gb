import { useEffect, useRef, useState } from 'react';
import type { Connection, Entry, Status, UiEvent } from './api';

/** How long the conversation keeps. A run is hours long; the DOM is not the transcript (W7 is). */
const MAX_ENTRIES = 500;

/** After the browser gives up on its own retry, try again on this cadence rather than never. */
const RETRY_MS = 1000;

/**
 * One SSE connection that stays up for the length of the run.
 *
 * `EventSource` reconnects by itself while it can, but it gives up permanently on some errors —
 * notably the server not being there yet, which is exactly what happens when the page is open while
 * `gb serve` restarts. So on `CLOSED` we rebuild it. Shared by both streams; the video one is not a
 * hook because its data must never reach React state.
 */
export function subscribe(
  url: string,
  onMessage: (data: string) => void,
  onConnection: (connection: Connection) => void,
): () => void {
  let source: EventSource | null = null;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;

  const open = () => {
    if (stopped) return;
    source = new EventSource(url);
    source.onopen = () => onConnection('live');
    source.onmessage = (message) => {
      onConnection('live');
      onMessage(message.data);
    };
    source.onerror = () => {
      onConnection('reconnecting');
      if (source?.readyState === EventSource.CLOSED) {
        source.close();
        source = null;
        retry = setTimeout(open, RETRY_MS);
      }
    };
  };
  open();

  return () => {
    stopped = true;
    clearTimeout(retry);
    source?.close();
  };
}

export interface EventStream {
  status: Status | null;
  entries: Entry[];
  connection: Connection;
}

/**
 * `/api/events`: the 10 Hz status heartbeat and everything the agent (and, from W4, the model) says.
 *
 * Status and entries are separate state so a heartbeat re-renders the status panel without touching
 * the conversation — at 10 Hz, re-rendering a 500-line log ten times a second is the one performance
 * mistake this page can make.
 */
export function useEventStream(): EventStream {
  const [status, setStatus] = useState<Status | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [connection, setConnection] = useState<Connection>('connecting');
  // Batched between animation frames: a burst of dialogue is many events in one tick, and each one
  // would otherwise be its own render.
  const pending = useRef<Entry[]>([]);
  const frame = useRef<number | undefined>(undefined);

  useEffect(() => {
    const flush = () => {
      frame.current = undefined;
      const arrived = pending.current;
      if (arrived.length === 0) return;
      pending.current = [];
      setEntries((previous) => [...previous, ...arrived].slice(-MAX_ENTRIES));
    };

    return subscribe(
      '/api/events',
      (data) => {
        const event = JSON.parse(data) as UiEvent;
        if (event.type === 'status') {
          const { seq: _seq, type: _type, ...rest } = event;
          setStatus(rest);
          return;
        }
        pending.current.push(event);
        // A backgrounded tab gets no animation frames, and a livestream is left in one for hours —
        // so the queue is capped as well as the list it flushes into.
        if (pending.current.length > MAX_ENTRIES) pending.current.splice(0, 1);
        frame.current ??= requestAnimationFrame(flush);
      },
      setConnection,
    );
  }, []);

  useEffect(() => () => cancelAnimationFrame(frame.current ?? 0), []);

  return { status, entries, connection };
}
