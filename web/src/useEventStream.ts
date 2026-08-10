import { useEffect, useRef, useState } from 'react';
import type { Connection, Entry, RunStatus, Status, UiEvent, UsageView } from './api';

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
  usage: UsageView | null;
  /** W6. From the transition event, which is instant; the heartbeat's copy is the late-joiner path. */
  run: RunStatus;
}

/**
 * Fold one event into the log.
 *
 * The only interesting case is `assistant_delta`: the worker publishes one event per fragment the
 * model emits, and the reply is the concatenation of all of them. Appending to the last entry when
 * it belongs to the same turn is what turns a stream of tokens back into a paragraph — and it has to
 * happen *here*, in the updater, rather than in the batch, because a flush can land in the middle of
 * a reply.
 */
function fold(entries: Entry[], event: UiEvent): Entry[] {
  if (event.type === 'assistant_delta') {
    const last = entries[entries.length - 1];
    if (last?.type === 'assistant' && last.turn === event.turn) {
      return [...entries.slice(0, -1), { ...last, text: last.text + event.text }];
    }
    return [...entries, { seq: event.seq, type: 'assistant', turn: event.turn, text: event.text }];
  }
  switch (event.type) {
    case 'status':
    case 'run_status':
      return entries; // handled separately — neither may re-render the log
    case 'turn_started':
      return [...entries, { seq: event.seq, type: 'turn', turn: event.turn, kind: event.kind, headline: event.headline }];
    case 'tool_call':
      return [...entries, { seq: event.seq, type: 'tool', turn: event.turn, name: event.name, arguments: event.arguments }];
    case 'decision':
      return [...entries, { seq: event.seq, type: 'decision', turn: event.turn, summary: event.summary }];
    case 'turn_cancelled':
      return [...entries, { seq: event.seq, type: 'cancelled', turn: event.turn, reason: event.reason }];
    case 'compacted':
      return [
        ...entries,
        {
          seq: event.seq,
          type: 'compacted',
          before: event.before,
          after: event.after,
          images_evicted: event.images_evicted,
          summarised: event.summarised,
        },
      ];
    default:
      return [...entries, event];
  }
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
  const [usage, setUsage] = useState<UsageView | null>(null);
  const [run, setRun] = useState<RunStatus>({ state: 'booting' });
  // Batched between animation frames: a burst of dialogue is many events in one tick, and a
  // streaming reply is one per token — each would otherwise be its own render.
  const pending = useRef<UiEvent[]>([]);
  const frame = useRef<number | undefined>(undefined);

  useEffect(() => {
    // **W7 / §11.** ⚠️ **Subscribe first, backfill second** — the same ordering the video path uses
    // (§5.2), and for the same reason: the other way round loses everything published between the
    // fetch returning and the stream attaching, and loses it invisibly.
    let abandoned = false;
    const backfill = () => {
      fetch('/api/history')
        .then((response) => (response.ok ? response.json() : []))
        .then((backlog: UiEvent[]) => {
          if (abandoned || backlog.length === 0) return;
          for (const event of backlog) {
            if (event.type === 'decision' && event.usage) setUsage((current) => current ?? event.usage!);
          }
          const older = backlog.reduce(fold, [] as Entry[]);
          setEntries((live) => {
            // Anything the stream has already delivered wins; the transcript only fills in what
            // happened before this page existed.
            const oldest = live.length > 0 ? live[0].seq : Number.MAX_SAFE_INTEGER;
            return [...older.filter((entry) => entry.seq < oldest), ...live].slice(-MAX_ENTRIES);
          });
        })
        .catch(() => {
          // No transcript, or a build with no run directory. The live stream is the whole of the
          // page either way, so there is nothing to report.
        });
    };

    const flush = () => {
      frame.current = undefined;
      const arrived = pending.current;
      if (arrived.length === 0) return;
      pending.current = [];
      setEntries((previous) => arrived.reduce(fold, previous).slice(-MAX_ENTRIES));
    };

    const unsubscribe = subscribe(
      '/api/events',
      (data) => {
        const event = JSON.parse(data) as UiEvent;
        if (event.type === 'status') {
          const { seq: _seq, type: _type, ...rest } = event;
          setStatus(rest);
          // The heartbeat carries the run status too, which is how a page opened mid-turn shows the
          // right thing without waiting for the next transition.
          setRun(rest.run);
          return;
        }
        if (event.type === 'run_status') {
          setRun(event.status);
          return;
        }
        if (event.type === 'decision' && event.usage) setUsage(event.usage);
        pending.current.push(event);
        // A backgrounded tab gets no animation frames, and a livestream is left in one for hours —
        // so the queue is capped as well as the list it flushes into.
        if (pending.current.length > MAX_ENTRIES) pending.current.splice(0, 1);
        frame.current ??= requestAnimationFrame(flush);
      },
      setConnection,
    );
    backfill();

    return () => {
      abandoned = true;
      unsubscribe();
    };
  }, []);

  useEffect(() => () => cancelAnimationFrame(frame.current ?? 0), []);

  return { status, entries, connection, usage, run };
}
