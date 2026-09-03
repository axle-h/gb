import { memo, useLayoutEffect, useRef, useState } from 'react';
import type { Entry } from '../api';
import { lastModelSide } from '../useEventStream';

/** Within this many pixels of the bottom counts as following the stream. */
const PIN_SLACK = 24;

/**
 * The running commentary: agent events now, the model's own messages and tool calls from W4.
 *
 * `memo` because the status heartbeat arrives several times a second and re-rendering a 500-entry
 * log with it would be the one performance mistake this page can make. `entries` only changes when
 * something is actually said.
 */
export const Conversation = memo(function Conversation({
  entries,
  visible = true,
}: {
  entries: Entry[];
  /** False while a phone is showing another tab — see the pin effect below for why it matters. */
  visible?: boolean;
}) {
  const list = useRef<HTMLDivElement>(null);
  // ⚠️ **The live thought scrolls inside its own box, and that box has to be followed separately.**
  // A thought is capped at a few lines so it cannot bury the log, which means the tokens arriving
  // are appended *below* the visible part of it: the pane pins itself to the bottom of the list and
  // the thought is still showing its first nine lines, frozen, for the whole minute the model
  // thinks. Measured on the deployed run mid-thought — 222px of text in a 117px box, `scrollTop` 0.
  const thought = useRef<HTMLSpanElement>(null);
  const [thoughtPinned, setThoughtPinned] = useState(true);
  // Pinned to the bottom unless the viewer has scrolled up to read something — in which case the
  // stream must not yank them back down.
  const [pinned, setPinned] = useState(true);
  // Every row shows its wire JSON. The per-row toggle below is the common case; this is for
  // watching the shape of a whole stretch of the stream at once.
  const [showAllRaw, setShowAllRaw] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  // Which finished thoughts the viewer has opened back up. Separate from `expanded` because the two
  // are different questions about the same row — "what did it think" and "what came down the wire".
  const [thoughts, setThoughts] = useState<Set<number>>(() => new Set());

  // ⚠️ `visible` is a dependency and not a condition. While a phone shows another tab this pane is
  // `display: none`, where `scrollHeight` is 0 and every pin applied is a no-op — so switching back
  // has to re-apply it, or the log reopens at the top. It must not gate the assignment itself: the
  // tab state is inert on a desk (the stylesheet ignores it from 640px up), so `visible` can be
  // stale-false there while the pane is plainly on screen and still needs following.
  useLayoutEffect(() => {
    const element = list.current;
    if (pinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, pinned, showAllRaw, expanded, thoughts, visible]);

  // The live thought follows its own tail, on the same terms as the list above it: pinned until the
  // viewer scrolls back through it, and re-pinned when the next thought starts (a new row means a
  // fresh element, so the ref changes and there is nothing left to have scrolled away from).
  const openBlock = lastModelSide(entries);
  const liveSeq = entries[openBlock]?.type === 'reasoning' ? entries[openBlock].seq : null;
  useLayoutEffect(() => setThoughtPinned(true), [liveSeq]);
  useLayoutEffect(() => {
    const element = thought.current;
    if (thoughtPinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, thoughtPinned]);

  const onScroll = () => {
    const element = list.current;
    if (element) setPinned(element.scrollTop + element.clientHeight >= element.scrollHeight - PIN_SLACK);
  };

  const onThoughtScroll = () => {
    const element = thought.current;
    if (element) setThoughtPinned(element.scrollTop + element.clientHeight >= element.scrollHeight - PIN_SLACK);
  };

  const toggle = (seq: number) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(seq)) next.add(seq);
      return next;
    });

  const toggleThought = (seq: number) =>
    setThoughts((current) => {
      const next = new Set(current);
      if (!next.delete(seq)) next.add(seq);
      return next;
    });

  return (
    <div className="conversation">
      <div className="conversation-bar">
        <button
          className={showAllRaw ? 'raw-toggle on' : 'raw-toggle'}
          onClick={() => setShowAllRaw(!showAllRaw)}
          title="Show the JSON behind every line, exactly as it arrived on /api/events"
        >
          raw
        </button>
      </div>
      <div className="conversation-list" ref={list} onScroll={onScroll}>
        {entries.length === 0 && <p className="dim">waiting for the agent…</p>}
        {entries.map((entry, index) => {
          // ⚠️ A thought is live exactly while it is the last row **the model wrote**: the worker
          // publishes one block per completion and closes it by saying something else, so "still
          // thinking" is a property of the log's shape rather than a flag on the event. The block
          // collapses on its own the moment the reply or the tool call lands, with no second event
          // to wait for — and it stays open while the game narrates underneath it, which it does
          // throughout, because the emulator never stops for the model. Same rule as `fold`'s, and
          // it has to be: one decides what the row *contains* and the other how it is drawn.
          const live = entry.type === 'reasoning' && index === openBlock;
          const unfolded = live || thoughts.has(entry.seq);
          const { gutter, body, title, modifier } = render(entry, unfolded);
          const open = showAllRaw || expanded.has(entry.seq);
          const thinking = entry.type === 'reasoning';
          return (
            <div
              key={entry.seq}
              className={`entry ${entry.type} ${modifier}${open ? ' open' : ''}${live ? ' live' : ''}`}
            >
              {/* A `<time>` rather than a span: the row is prose with a timestamp in front of it,
                  and the machine-readable value is the thing a reader hovering wants. A row from a
                  transcript written before the server stamped times has none, and gets a blank of
                  the same width so the column does not jump. */}
              <time className="at" dateTime={entry.at ? new Date(entry.at).toISOString() : undefined} title={longTime(entry.at)}>
                {shortTime(entry.at)}
              </time>
              <span className="chip" title={title}>
                {gutter}
              </span>
              <span className="body" ref={live ? thought : undefined} onScroll={live ? onThoughtScroll : undefined}>
                <button
                  className="line"
                  onClick={() => (thinking ? toggleThought(entry.seq) : toggle(entry.seq))}
                  title={
                    thinking
                      ? 'What the model thought on its way to this turn'
                      : entry.type === 'tool'
                        ? 'What was asked and what came back'
                        : 'Show the JSON this line was made from'
                  }
                >
                  {body}
                  {/* A tool row has something underneath it whether or not it has been opened, and
                      an affordance is cheaper than discovering that by clicking every row. */}
                  {entry.type === 'tool' && <span className="disclose">{open ? '▾' : '▸'}</span>}
                </button>
                {/* A repeat says nothing new, so it costs three characters rather than a row. */}
                {entry.count > 1 && <span className="repeat">×{entry.count}</span>}
                {/* ⚠️ A tool row opens onto what was *asked and answered*, not onto its own wire
                    event — that is the question a reader of this log actually has. The raw JSON is
                    still one toggle away, on the `raw` switch above, which is the operator's view
                    rather than the viewer's. */}
                {open && entry.type === 'tool' && <ToolDetail entry={entry} />}
                {open && (entry.type !== 'tool' || showAllRaw) && (
                  <pre className="raw">{JSON.stringify(entry.raw, null, 2)}</pre>
                )}
              </span>
            </div>
          );
        })}
      </div>
      {!pinned && (
        <button className="jump" onClick={() => setPinned(true)}>
          ↓ follow
        </button>
      )}
    </div>
  );
});

/**
 * The time column: `14:22:33`, in the **viewer's** timezone.
 *
 * Seconds and not milliseconds because the log is a narration rather than a trace — and no date,
 * because a page shows a few minutes of a run at a time and a date on every one of five hundred rows
 * is four fifths of the column saying today. The full stamp, date and all, is on the `title` for the
 * one case that wants it: a backfilled row from a run that has been going since yesterday.
 */
function shortTime(at: number | undefined): string {
  if (at === undefined) return '';
  return new Date(at).toLocaleTimeString(undefined, { hour12: false });
}

function longTime(at: number | undefined): string | undefined {
  return at === undefined ? undefined : new Date(at).toLocaleString();
}

interface Rendered {
  /** The left gutter: who is talking. */
  gutter: string;
  body: string;
  /** The full detail, on hover — the gutter is ten characters wide and the log is long. */
  title: string;
  /** A second class name for colour. */
  modifier: string;
}

/** `unfolded` is only consulted for a `reasoning` row: every other kind renders the same either way. */
function render(entry: Entry, unfolded: boolean): Rendered {
  switch (entry.type) {
    case 'agent':
      return { gutter: category(entry.kind), body: entry.text, title: entry.kind, modifier: entry.kind };
    case 'notice':
      return { gutter: entry.level, body: entry.message, title: entry.level, modifier: entry.level };
    case 'turn':
      return { gutter: `#${entry.turn}`, body: entry.headline, title: `${entry.kind} decision`, modifier: entry.kind };
    case 'assistant':
      return { gutter: 'model', body: entry.text, title: `turn ${entry.turn}`, modifier: '' };
    case 'reasoning':
      // Shown while it happens and summarised once it is over. A local reasoning model spends most
      // of a turn's output here — three quarters of it on a trivial overworld step — so leaving the
      // block open would bury the reply, the tool call and the decision under the thinking that led
      // to them. Watching it arrive is worth a great deal; re-reading it afterwards rarely is, so
      // that costs a click.
      return {
        gutter: 'think',
        body: unfolded ? entry.text : summarise(entry.text),
        title: `turn ${entry.turn}: the model's own reasoning`,
        modifier: unfolded ? 'unfolded' : '',
      };
    case 'tool':
      // ⚠️ **A sentence, not the wire call.** This row used to be `read_map {}` — the tool's own
      // identifier and an empty object — which is the log telling a reader what the *protocol* did
      // rather than what happened. The arguments are not lost: they are one click away, beside the
      // answer, which is where they are worth reading anyway.
      return {
        gutter: 'tool',
        body: describeTool(entry),
        title: `turn ${entry.turn}: ${entry.name}`,
        // A call the turn layer would not run reads as refused from the moment it appears, without
        // waiting for the result that says so — `kind` already carries the verdict.
        modifier: entry.ok === false || entry.kind === 'rejected' ? 'refused' : entry.kind,
      };
    case 'decision':
      // The model's own sentence when it wrote one, and the agent's account of what it was told to
      // do underneath it. Its `summary` argument is the only thing it says about a turn that
      // survives the turn, so it is also the only thing here worth leading with.
      return {
        gutter: '→',
        body: entry.narration ?? entry.summary,
        title: `turn ${entry.turn}: ${entry.summary}`,
        modifier: '',
      };
    case 'cancelled':
      return { gutter: 'dropped', body: entry.reason, title: `turn ${entry.turn}`, modifier: '' };
    case 'compacted': {
      // W6. Worth a line in the log: a compaction is the one thing that changes what the model
      // knows without the model doing anything, so a reply that suddenly forgets something should
      // have an entry above it explaining why.
      const how = [
        entry.summarised ? 'summarised' : null,
        entry.images_evicted > 0 ? `${entry.images_evicted} screenshot${entry.images_evicted === 1 ? '' : 's'} dropped` : null,
      ]
        .filter(Boolean)
        .join(', ');
      return {
        gutter: 'context',
        body: `compacted ${entry.before.toLocaleString()} → ${entry.after.toLocaleString()} tokens${how ? ` (${how})` : ''}`,
        title: 'the history was compacted to fit the context window',
        modifier: 'compacted',
      };
    }
  }
}

/**
 * One tool call as a sentence a person would say.
 *
 * ⚠️ **Named cases, with a fallback that is still readable.** The temptation is a generic
 * `verb(name) + arguments` renderer; the reason it is a table is that the interesting part differs
 * per tool — a route read is about *where*, a nickname about *what*, a wait about *how long* — and
 * a renderer that does not know that produces "use field move {move: cut, ...}", which is the wire
 * call with the punctuation moved around. Anything unlisted falls through to exactly that, which is
 * the right answer for a tool this table has not been taught yet.
 */
function describeTool(entry: Extract<Entry, { type: 'tool' }>): string {
  const args = parseArguments(entry.arguments);
  const text = (key: string): string | undefined => {
    const value = args[key];
    return typeof value === 'string' && value !== '' ? value : undefined;
  };
  switch (entry.name) {
    case 'read_map':
      return 'Read the map';
    case 'read_party':
      return 'Read the party';
    case 'read_bag':
      return 'Read the bag';
    case 'read_battle':
      return 'Read the battle';
    case 'read_route': {
      const to = text('to');
      return to ? `Asked the way to ${to}` : 'Asked the way';
    }
    case 'screenshot':
      return 'Looked at the screen';
    // `todo_add` is the tool's old name; transcripts written before `todo_set` still replay it.
    case 'todo_add':
    case 'todo_set': {
      const item = text('text');
      if (item) return args.id === undefined ? `Planned: ${item}` : `Revised plan item ${args.id}: ${item}`;
      // No text and an id is the delete overload `todo_delete` replaced. Still reachable, and every
      // transcript written before that tool existed is full of it.
      return args.id === undefined ? 'Added to the plan' : `Dropped plan item ${args.id}`;
    }
    case 'todo_delete':
      return args.id === undefined ? 'Dropped a plan item' : `Dropped plan item ${args.id}`;
    case 'todo_complete':
      return args.id === undefined ? 'Ticked something off' : `Ticked off plan item ${args.id}`;
    case 'get_battle_script_docs':
      return 'Read how to script a battle';
    case 'read_battle_script':
      return 'Read its battle script';
    // ⚠️ **The size, not the script.** `report_issue` inlines its message because that message is
    // written to be read by a person; a script is written to be read by the sandbox, runs to
    // dozens of lines, and would take over the log on the one turn it appears. The row already
    // opens onto its own arguments, so the source is one click away where it belongs.
    case 'set_battle_script': {
      const script = text('script');
      if (!script) return 'Stopped scripting its battles';
      const lines = script.trim().split('\n').length;
      return `Wrote a battle script (${lines} line${lines === 1 ? '' : 's'})`;
    }
    case 'choose_action':
    case 'choose_battle_action': {
      const id = text('id');
      return id ? `Chose ${id}` : 'Chose an action';
    }
    case 'use_field_move': {
      const move = text('move');
      return move ? `Used ${move}` : 'Used a field move';
    }
    case 'press_buttons': {
      const buttons = Array.isArray(args.buttons) ? args.buttons.join(', ') : undefined;
      return buttons ? `Pressed ${buttons}` : 'Pressed buttons';
    }
    // The model reporting that the *agent* is wrong. The message is the row rather than a label
    // above it: unlike every other tool here this one is written to be read by a person, and it is
    // filed to disk whether or not anyone opens the row.
    case 'report_issue': {
      const message = text('message');
      return message ? `Reported: ${message}` : 'Reported a problem with the agent';
    }
    case 'set_nickname': {
      const name = text('name');
      // Omitting the argument is the ordinary answer here, not a missing one.
      return name ? `Named it ${name}` : 'Kept the default name';
    }
    case 'buy_item': {
      const item = text('item');
      const quantity = typeof args.quantity === 'number' ? ` ×${args.quantity}` : '';
      return item ? `Bought ${item}${quantity}` : 'Bought nothing';
    }
    case 'forget_move':
      return args.slot === undefined ? 'Declined the new move' : `Forgot the move in slot ${args.slot}`;
    case 'wait':
      return args.ticks === undefined ? 'Waited' : `Waited ${args.ticks} ticks`;
    default: {
      const rest = compact(entry.arguments);
      return rest ? `${entry.name} ${rest}` : entry.name;
    }
  }
}

/** The model's arguments, or an empty object — a call whose JSON will not parse is one to show raw. */
function parseArguments(json: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(json.trim() || '{}');
    return typeof parsed === 'object' && parsed !== null ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/**
 * What a tool row reveals when it is opened: what was asked, what came back, and the picture if
 * there was one.
 *
 * ⚠️ **The picture is fetched, not carried.** `imageSeq` addresses it in a small server-side ring —
 * a map render is a couple of hundred kilobytes — so a page replaying an old backlog gets a 404 and
 * the caption on its own. `onError` hides the element rather than leaving a broken-image icon,
 * because "that was a while ago" is not an error worth drawing.
 */
function ToolDetail({ entry }: { entry: Extract<Entry, { type: 'tool' }> }) {
  const args = compact(entry.arguments);
  return (
    <div className="tool-detail">
      {args && (
        <>
          <span className="tool-label">asked</span>
          <pre className="raw">{pretty(entry.arguments)}</pre>
        </>
      )}
      {entry.result !== undefined && (
        <>
          <span className="tool-label">{entry.ok === false ? 'refused' : 'answered'}</span>
          <pre className="raw">{entry.result}</pre>
        </>
      )}
      {entry.result === undefined && <span className="tool-label pending">waiting for an answer…</span>}
      {entry.imageSeq !== undefined && (
        <img
          className="tool-image"
          src={`/api/tool-image/${entry.imageSeq}/image.png`}
          alt={`what ${entry.name} answered with`}
          loading="lazy"
          onError={(event) => {
            event.currentTarget.style.display = 'none';
          }}
        />
      )}
    </div>
  );
}

/** Arguments as the model sent them, re-indented — this one is read rather than skimmed. */
function pretty(json: string): string {
  const trimmed = json.trim();
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

/**
 * What a finished thought collapses to. A word count rather than the first sentence: a reasoning
 * model opens with "Okay, so the user wants me to" about as often as with anything worth quoting,
 * and the useful thing to know at a glance is how long it deliberated.
 */
function summarise(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') return 'thought about it';
  const words = trimmed.split(/\s+/).length;
  return `thought for ${words.toLocaleString()} word${words === 1 ? '' : 's'}`;
}

/** Arguments arrive as the model sent them, which may be pretty-printed across several lines. */
function compact(json: string): string {
  const trimmed = json.trim();
  if (trimmed === '' || trimmed === '{}') return '';
  try {
    return JSON.stringify(JSON.parse(trimmed));
  } catch {
    return trimmed;
  }
}

/**
 * The gutter says which part of the game is talking, not which `AgentEvent` variant it was:
 * `overworld_action_completed` and `started_overworld_action` differ by a glyph the line itself
 * already carries (→ ✓ ✗ 📖), and spelling both out in full costs a quarter of the width. The exact
 * variant is the `title`, and the whole event is one click away.
 *
 * The `text_box` arm (and `.entry.text_box` in the stylesheet) is not reachable while
 * `useEventStream`'s `UNLOGGED` drops that kind — kept because it is what the log looks like the
 * moment anyone puts the dialogue back, which is a filter, not a rewrite.
 */
function category(kind: string): string {
  if (kind.startsWith('battle')) return 'battle';
  if (kind === 'text_box') return 'text';
  // The end of the game gets the gutter the badge strip uses, because it is the one line in the
  // whole log that someone scrolling back is actually looking for.
  if (kind === 'hall_of_fame') return '🏆';
  return kind.includes('overworld') ? 'overworld' : kind.replace(/_/g, ' ');
}
