import { useEffect, useRef, useState } from 'react';
import { STALE_MS } from './api';
import type {
  BattleScriptView,
  Connection,
  Entry,
  EntryBody,
  RunStatus,
  Status,
  TodoView,
  UiEvent,
  UsageView,
} from './api';

/** How many rows the log keeps; the transcript is the record. */
const MAX_ENTRIES = 500;

/** Retry cadence once the browser has given up on its own reconnect. */
const RETRY_MS = 1000;

// One SSE connection for the run, rebuilt when `EventSource` gives up or after `STALE_MS` without a
// heartbeat, since a dropped network raises no error. `onOpen` fires on every open and the caller
// reloads `/api/history`; `resync` forces a reload for a hidden tab whose socket never died.
export function subscribe(
  url: string,
  onMessage: (data: string) => void,
  onConnection: (connection: Connection) => void,
  onOpen: () => void,
): { close: () => void; resync: () => void } {
  let source: EventSource | null = null;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let watchdog: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;

  const rebuild = (delay: number) => {
    clearTimeout(watchdog);
    onConnection('reconnecting');
    source?.close();
    source = null;
    retry = setTimeout(open, delay);
  };

  const alive = () => {
    onConnection('live');
    clearTimeout(watchdog);
    watchdog = setTimeout(() => rebuild(0), STALE_MS);
  };

  const open = () => {
    if (stopped) return;
    source = new EventSource(url);
    source.onopen = () => {
      // Before any message of this connection, or the reset discards its opening heartbeat and plan.
      onOpen();
      alive();
    };
    source.onmessage = (message) => {
      alive();
      onMessage(message.data);
    };
    source.onerror = () => {
      onConnection('reconnecting');
      // A `CONNECTING` source is the browser's own retry; the watchdog stays armed in case it stalls.
      if (source?.readyState === EventSource.CLOSED) rebuild(RETRY_MS);
    };
  };
  open();

  return {
    close: () => {
      stopped = true;
      clearTimeout(retry);
      clearTimeout(watchdog);
      source?.close();
    },
    resync: () => {
      if (!stopped) rebuild(0);
    },
  };
}

export interface EventStream {
  status: Status | null;
  entries: Entry[];
  connection: Connection;
  usage: UsageView | null;
  /** Smoothed from the difference between heartbeats, never `emulated_ms / wall_ms`; `null` until measured. */
  speed: number | null;
  /** From the transition event; the heartbeat's copy is the late-joiner path. */
  run: RunStatus;
  /** State, not a log row: each event replaces the whole list. */
  plan: TodoView[];
  /** State, not a log row, like `plan`; `null` until a script is set. */
  battleScript: BattleScriptView | null;
}

/** What a row says, ignoring bookkeeping; rows with equal signatures collapse into one with a count. */
function signature(entry: Entry): string {
  // `at` differs on every event, so leaving it in would stop rows ever comparing equal.
  const { seq: _seq, raw: _raw, count: _count, at: _at, ...body } = entry;
  return JSON.stringify(body);
}

/** Append a row, or bump an identical row above it; the row keeps its first `seq`, its React key. */
function push(entries: Entry[], body: EntryBody, event: UiEvent): Entry[] {
  const entry: Entry = { ...body, seq: event.seq, raw: event, count: 1, at: event.at };
  const last = entries[entries.length - 1];
  if (last && signature(last) === signature(entry)) {
    return [...entries.slice(0, -1), { ...last, count: last.count + 1 }];
  }
  return [...entries, entry];
}

/** Dropped from the page only; the model and the transcript keep them, so filter here, never at the publish. */
const UNLOGGED = new Set(['text_box', 'overworld_interaction_completed']);

/** Rows the model produced, as against the game; the next of these closes a streaming block. */
const MODEL_SIDE = new Set(['reasoning', 'assistant', 'tool', 'decision', 'turn', 'cancelled', 'compacted']);

/** How far back a result looks for its call; the agent narrates in between, so it is never the last row. */
const RESULT_LOOKBACK = 40;

// Attach a tool's answer to its call by `id`, never by position or name: a message's calls come
// back as a batch. A result whose call has scrolled off is dropped.
function attachResult(
  entries: Entry[],
  event: Extract<UiEvent, { type: 'tool_result' }>,
): Entry[] {
  const floor = Math.max(0, entries.length - RESULT_LOOKBACK);
  for (let index = entries.length - 1; index >= floor; index -= 1) {
    const row = entries[index];
    if (row.type !== 'tool' || row.id !== event.id || row.result !== undefined) continue;
    const grown: Entry = {
      ...row,
      result: event.content,
      ok: event.ok,
      ...(event.image ? { imageSeq: event.seq } : {}),
    };
    return [...entries.slice(0, index), grown, ...entries.slice(index + 1)];
  }
  return entries;
}

export function lastModelSide(entries: Entry[]): number {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    if (MODEL_SIDE.has(entries[index].type)) return index;
  }
  return -1;
}

// Fold one event into the log. A streamed fragment grows the last row the model wrote, however many
// game lines arrived since: the emulator narrates while the model thinks, and grouping on `turn`
// alone would weld the thoughts either side of a tool call together.
export function fold(entries: Entry[], event: UiEvent): Entry[] {
  if (event.type === 'assistant_delta' || event.type === 'assistant_reasoning') {
    const type = event.type === 'assistant_delta' ? 'assistant' : 'reasoning';
    const index = lastModelSide(entries);
    const open = index < 0 ? undefined : entries[index];
    if (open?.type === type && open.turn === event.turn) {
      const grown = { ...open, text: open.text + event.text };
      return [...entries.slice(0, index), grown, ...entries.slice(index + 1)];
    }
    // Not through `push`: a growing row must never be collapsed. It keeps its first fragment's `at`.
    return [
      ...entries,
      { seq: event.seq, type, turn: event.turn, text: event.text, raw: event, count: 1, at: event.at },
    ];
  }
  switch (event.type) {
    case 'status':
    case 'run_status':
    case 'plan':
    case 'battle_script':
      return entries; // handled separately — none of these may re-render the log
    case 'turn_started':
      return push(entries, { type: 'turn', turn: event.turn, kind: event.kind, headline: event.headline }, event);
    case 'tool_call':
      // Not through `push`: collapsing a row still waiting for its result would misattach the answer.
      return [
        ...entries,
        {
          seq: event.seq,
          type: 'tool',
          turn: event.turn,
          id: event.id,
          kind: event.kind,
          name: event.name,
          arguments: event.arguments,
          raw: event,
          count: 1,
          at: event.at,
        },
      ];
    case 'tool_result':
      return attachResult(entries, event);
    case 'decision':
      return push(
        entries,
        {
          type: 'decision',
          turn: event.turn,
          summary: event.summary,
          ...(event.narration ? { narration: event.narration } : {}),
        },
        event,
      );
    case 'turn_cancelled':
      return push(entries, { type: 'cancelled', turn: event.turn, reason: event.reason }, event);
    case 'compacted':
      return push(
        entries,
        {
          type: 'compacted',
          before: event.before,
          after: event.after,
          images_evicted: event.images_evicted,
          summarised: event.summarised,
        },
        event,
      );
    case 'agent':
      if (UNLOGGED.has(event.kind)) return entries;
      return push(entries, { type: 'agent', kind: event.kind, text: event.text }, event);
    case 'notice':
      return push(entries, { type: 'notice', level: event.level, message: event.message }, event);
  }
}

/** `/api/events`; status and entries are separate state so the 10 Hz heartbeat never re-renders the log. */
export function useEventStream(): EventStream {
  const [status, setStatus] = useState<Status | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [connection, setConnection] = useState<Connection>('connecting');
  const [usage, setUsage] = useState<UsageView | null>(null);
  const [run, setRun] = useState<RunStatus>({ state: 'booting' });
  const [plan, setPlan] = useState<TodoView[]>([]);
  const [battleScript, setBattleScript] = useState<BattleScriptView | null>(null);
  const [speed, setSpeed] = useState<number | null>(null);
  const anchor = useRef<{ wall: number; emulated: number } | null>(null);
  // Batched per animation frame: a streaming reply is one event per token.
  const pending = useRef<UiEvent[]>([]);
  const frame = useRef<number | undefined>(undefined);

  useEffect(() => {
    // Subscribe first, backfill second, and backfill on every open (see `subscribe`); `generation`
    // discards a fetch started by an earlier connection.
    let generation = 0;
    const backfill = (started: number) => {
      fetch('/api/history')
        .then((response) => (response.ok ? response.json() : []))
        .then((backlog: UiEvent[]) => {
          if (started !== generation || backlog.length === 0) return;
          // The last decision's usage, a running total; a newer one from the stream wins.
          for (let index = backlog.length - 1; index >= 0; index -= 1) {
            const event = backlog[index];
            if (event.type !== 'decision' || !event.usage) continue;
            const usage = event.usage;
            setUsage((current) => current ?? usage);
            break;
          }
          // The last plan and script in the backlog, unless the stream has delivered newer ones.
          const planned = backlog.filter((event) => event.type === 'plan');
          const latest = planned[planned.length - 1];
          if (latest?.type === 'plan') setPlan((live) => (live.length > 0 ? live : latest.items));
          const scripts = backlog.filter((event) => event.type === 'battle_script');
          const script = scripts[scripts.length - 1];
          if (script?.type === 'battle_script') {
            const view: BattleScriptView = {
              source: script.source,
              armed: script.armed,
              is_default: script.is_default,
              last_failure: script.last_failure,
            };
            setBattleScript((live) => live ?? view);
          }
          const older = backlog.reduce(fold, [] as Entry[]);
          setEntries((live) => {
            // Live rows win; the transcript fills in only what came before them.
            const oldest = live.length > 0 ? live[0].seq : Number.MAX_SAFE_INTEGER;
            return [...older.filter((entry) => entry.seq < oldest), ...live].slice(-MAX_ENTRIES);
          });
        })
        .catch(() => {
          // No transcript: the live stream is the whole page.
        });
    };

    const flush = () => {
      frame.current = undefined;
      const arrived = pending.current;
      if (arrived.length === 0) return;
      pending.current = [];
      setEntries((previous) => arrived.reduce(fold, previous).slice(-MAX_ENTRIES));
    };

    /** A connection has (re)opened: forget what the last one delivered and reload the transcript. */
    const reload = () => {
      generation += 1;
      pending.current = [];
      if (frame.current !== undefined) cancelAnimationFrame(frame.current);
      frame.current = undefined;
      setEntries([]);
      setPlan([]);
      setBattleScript(null);
      setUsage(null);
      // A speed window must not span the gap.
      anchor.current = null;
      setSpeed(null);
      backfill(generation);
    };

    const stream = subscribe(
      '/api/events',
      (data) => {
        const event = JSON.parse(data) as UiEvent;
        if (event.type === 'status') {
          const { seq: _seq, type: _type, ...rest } = event;
          setStatus(rest);
          sampleSpeed(rest, anchor, setSpeed);
          setRun(rest.run);
          return;
        }
        if (event.type === 'run_status') {
          setRun(event.status);
          return;
        }
        if (event.type === 'plan') {
          setPlan(event.items);
          return;
        }
        if (event.type === 'battle_script') {
          setBattleScript({
            source: event.source,
            armed: event.armed,
            is_default: event.is_default,
            last_failure: event.last_failure,
          });
          return;
        }
        if (event.type === 'decision' && event.usage) setUsage(event.usage);
        pending.current.push(event);
        // A background tab gets no animation frames, so the queue is capped too.
        if (pending.current.length > MAX_ENTRIES) pending.current.splice(0, 1);
        frame.current ??= requestAnimationFrame(flush);
      },
      setConnection,
      reload,
    );

    // A tab back from the background resyncs: its queue may have overflowed on a healthy socket and
    // the watchdog timer was throttled. A short absence is not a gap.
    let hiddenAt: number | null = null;
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') {
        hiddenAt = Date.now();
        return;
      }
      const away = hiddenAt === null ? 0 : Date.now() - hiddenAt;
      hiddenAt = null;
      if (away > STALE_MS) stream.resync();
    };
    // A bfcache restore returns a page whose `EventSource` and timers were frozen.
    const onPageShow = (event: PageTransitionEvent) => {
      if (event.persisted) stream.resync();
    };
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('pageshow', onPageShow);

    return () => {
      generation += 1;
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('pageshow', onPageShow);
      stream.close();
    };
  }, []);

  useEffect(() => () => cancelAnimationFrame(frame.current ?? 0), []);

  return { status, entries, connection, usage, run, plan, battleScript, speed };
}

/** The shortest heartbeat span measured; shorter is dominated by the host's own cadence. */
const SPEED_WINDOW_MS = 500;

const SPEED_SMOOTHING = 0.3;

// Fold one heartbeat into the running speed. A park freezes both counters, so no window closes and
// the last reading holds; do not special-case it.
function sampleSpeed(
  status: Status,
  anchor: { current: { wall: number; emulated: number } | null },
  setSpeed: (next: (current: number | null) => number | null) => void,
): void {
  const previous = anchor.current;
  if (!previous) {
    anchor.current = { wall: status.wall_ms, emulated: status.emulated_ms };
    return;
  }
  const dw = status.wall_ms - previous.wall;
  const de = status.emulated_ms - previous.emulated;
  // Both counters reset on a new run, so either going backwards restarts measurement.
  if (dw < 0 || de < 0) {
    anchor.current = { wall: status.wall_ms, emulated: status.emulated_ms };
    setSpeed(() => null);
    return;
  }
  if (dw < SPEED_WINDOW_MS) return;
  anchor.current = { wall: status.wall_ms, emulated: status.emulated_ms };
  const sample = de / dw;
  setSpeed((current) =>
    current === null ? sample : current + (sample - current) * SPEED_SMOOTHING,
  );
}
