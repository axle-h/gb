import { useEffect, useRef, useState } from 'react';
import type { Connection, Entry, EntryBody, RunStatus, Status, TodoView, UiEvent, UsageView } from './api';

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
  /**
   * W6b. The model's plan, as of the last time it changed.
   *
   * ⚠️ **State, not a log row.** It is a *replacement* every time — the server publishes the whole
   * list — so folding it into the conversation would print the entire plan again for every item the
   * model ever ticks off, and the page's job is to show what the plan *is*.
   */
  plan: TodoView[];
}

/**
 * What a row says, ignoring which event it came from and how many times it has been said. Two rows
 * with the same signature are the same line and are collapsed into one with a count.
 */
function signature(entry: Entry): string {
  // ⚠️ `at` is excluded for the reason `seq` is: it differs on every event by construction, so
  // leaving it in would mean no two rows ever compared equal and the collapsing below would silently
  // never fire again.
  const { seq: _seq, raw: _raw, count: _count, at: _at, ...body } = entry;
  return JSON.stringify(body);
}

/**
 * Append a row, or — if it repeats the one above it verbatim — bump that row's counter instead.
 *
 * Worth doing because the agent genuinely repeats itself: under `--policy random` the same battle
 * action comes up several times running, and the text reader emits doubled sentences ("Got away
 * safely! Got away safely!"). Three identical lines say no more than one line and a `×3`, and they
 * push the interesting ones off the top of a 500-row window.
 *
 * The row keeps its **first** `seq`, which is both its React key and what the backfill merge sorts
 * on — a key that changed as the count rose would remount the row on every repeat.
 */
function push(entries: Entry[], body: EntryBody, event: UiEvent): Entry[] {
  const entry: Entry = { ...body, seq: event.seq, raw: event, count: 1, at: event.at };
  const last = entries[entries.length - 1];
  if (last && signature(last) === signature(entry)) {
    return [...entries.slice(0, -1), { ...last, count: last.count + 1 }];
  }
  return [...entries, entry];
}

/**
 * Agent events that are said but not shown.
 *
 * ⚠️ **Dropped here rather than at the server**, and the distinction is the whole point: these still
 * go to the model — dialogue is most of what `### Since your last decision` is made of — and they
 * are still written to `transcript.jsonl`, so `/api/history` and the run's archived record keep
 * every line. Only the page is quiet. Filtering them out of the publish instead would have deleted
 * them from the record too.
 *
 * `text_box` is the screen's own dialogue, which the picture above the log is already showing, one
 * character at a time, in the game's own font. `overworld_interaction_completed` is the "✓ talked to
 * Mom" that the dialogue immediately after it says better.
 */
const UNLOGGED = new Set(['text_box', 'overworld_interaction_completed']);

/**
 * Rows the **model** produced, as against rows the game narrated.
 *
 * The distinction exists for exactly one reason: it is what closes a block of streaming text. See
 * [`fold`](#fold) — and note that a `turn` row is model-side, because a new turn certainly ends the
 * thought before it.
 */
const MODEL_SIDE = new Set(['reasoning', 'assistant', 'tool', 'decision', 'turn', 'cancelled', 'compacted']);

/**
 * The most recent row the model produced, or `-1`. The rows after it are the game talking.
 */
export function lastModelSide(entries: Entry[]): number {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    if (MODEL_SIDE.has(entries[index].type)) return index;
  }
  return -1;
}

/**
 * Fold one event into the log.
 *
 * The interesting cases are `assistant_delta` and `assistant_reasoning`: the worker publishes one
 * event per fragment the model emits, and the reply — or the thought — is the concatenation of all
 * of them. Appending to the row those fragments belong to is what turns a stream of tokens back into
 * a paragraph, and it has to happen *here*, in the updater, rather than in the batch, because a
 * flush can land in the middle of a reply.
 *
 * ⚠️ **A thought is closed by the next thing the *model* says, not by the turn ending and not by the
 * next event of any kind.** Two failures, one on each side of that line:
 *
 * - Grouping on `turn` alone welds two thoughts together. A turn that reads before it decides thinks
 *   once per completion, so the tool call and its result sit between two separate blocks.
 * - Closing on the next event *of any kind* shreds a single thought into a dozen. ⚠️ **The emulator
 *   never pauses while the model thinks** — that is the property the whole agent is built on — so
 *   the game narrates over the top of every thought it has: "→ heading for Mom", "✓ reached the warp
 *   to PalletTown". Each of those used to end the block, so a thought that took the model a minute
 *   arrived as five rows, four of them collapsed to `thought for 9 words`, and none of them the
 *   thing the viewer was watching.
 *
 * So the fragment is appended to the last row the *model* wrote, however many lines the game has said
 * since. The row keeps its place in the log, which is where it belongs: it started when it started.
 */
export function fold(entries: Entry[], event: UiEvent): Entry[] {
  if (event.type === 'assistant_delta' || event.type === 'assistant_reasoning') {
    const type = event.type === 'assistant_delta' ? 'assistant' : 'reasoning';
    const index = lastModelSide(entries);
    const open = index < 0 ? undefined : entries[index];
    if (open?.type === type && open.turn === event.turn) {
      const grown = { ...open, text: open.text + event.text };
      return [...entries.slice(0, index), grown, ...entries.slice(index + 1)];
    }
    // Not through `push`: a reply grows, so it must never be collapsed against the row above it.
    // The block keeps the `at` of its **first** fragment: a thought is timed from when the model
    // started thinking, which is the interesting number, not from the token that ended it.
    return [
      ...entries,
      { seq: event.seq, type, turn: event.turn, text: event.text, raw: event, count: 1, at: event.at },
    ];
  }
  switch (event.type) {
    case 'status':
    case 'run_status':
    case 'plan':
      return entries; // handled separately — none of these may re-render the log
    case 'turn_started':
      return push(entries, { type: 'turn', turn: event.turn, kind: event.kind, headline: event.headline }, event);
    case 'tool_call':
      return push(entries, { type: 'tool', turn: event.turn, name: event.name, arguments: event.arguments }, event);
    case 'decision':
      return push(entries, { type: 'decision', turn: event.turn, summary: event.summary }, event);
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
  const [plan, setPlan] = useState<TodoView[]>([]);
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
          // ⚠️ The **last** plan in the backlog, and only if the stream has not already delivered a
          // newer one — the same "live wins" rule the entries below use. Each event carries the
          // whole list, so replaying them in order and keeping the final one is the current plan.
          const planned = backlog.filter((event) => event.type === 'plan');
          const latest = planned[planned.length - 1];
          if (latest?.type === 'plan') setPlan((live) => (live.length > 0 ? live : latest.items));
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
        if (event.type === 'plan') {
          setPlan(event.items);
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

  return { status, entries, connection, usage, run, plan };
}
