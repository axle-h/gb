import { memo, useLayoutEffect, useRef, useState } from 'react';
import type { Entry } from '../api';
import { lastModelSide } from '../useEventStream';

/** Within this many pixels of the bottom counts as following the stream. */
const PIN_SLACK = 24;

/** The log; `memo` so the status heartbeat never re-renders it. */
export const Conversation = memo(function Conversation({
  entries,
  visible = true,
}: {
  entries: Entry[];
  /** False while a phone shows another tab; see the pin effect. */
  visible?: boolean;
}) {
  const list = useRef<HTMLDivElement>(null);
  // The live thought scrolls in its own capped box, so its tail is followed separately.
  const thought = useRef<HTMLSpanElement>(null);
  const [thoughtPinned, setThoughtPinned] = useState(true);
  // Pinned to the bottom unless the viewer has scrolled up.
  const [pinned, setPinned] = useState(true);
  const [showAllRaw, setShowAllRaw] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  // Finished thoughts the viewer reopened; separate from `expanded`, which shows wire JSON.
  const [thoughts, setThoughts] = useState<Set<number>>(() => new Set());

  // `visible` is a dependency, not a condition: a hidden pane has no height, so the pin is re-applied
  // on return, while on a desk `visible` can be stale-false with the pane on screen.
  useLayoutEffect(() => {
    const element = list.current;
    if (pinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, pinned, showAllRaw, expanded, thoughts, visible]);

  // The live thought follows its own tail, and re-pins when the next thought starts.
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
          // A thought is live exactly while it is the last row the model wrote, the same rule as `fold`.
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
              {/* A row with no `at` gets a blank of the same width, so the column does not jump. */}
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
                  {entry.type === 'tool' && <span className="disclose">{open ? '▾' : '▸'}</span>}
                </button>
                {entry.count > 1 && <span className="repeat">×{entry.count}</span>}
                {/* A tool row opens onto what was asked and answered; its wire JSON is behind `raw`. */}
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

/** `HH:MM:SS` in the viewer's timezone; the full stamp is the `title`. */
function shortTime(at: number | undefined): string {
  if (at === undefined) return '';
  return new Date(at).toLocaleTimeString(undefined, { hour12: false });
}

function longTime(at: number | undefined): string | undefined {
  return at === undefined ? undefined : new Date(at).toLocaleString();
}

interface Rendered {
  gutter: string;
  body: string;
  title: string;
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
      // Shown while it happens and summarised once over, so it does not bury the reply.
      return {
        gutter: 'think',
        body: unfolded ? entry.text : summarise(entry.text),
        title: `turn ${entry.turn}: the model's own reasoning`,
        modifier: unfolded ? 'unfolded' : '',
      };
    case 'tool':
      // A sentence, not the wire call; the arguments are one click away.
      return {
        gutter: 'tool',
        body: describeTool(entry),
        title: `turn ${entry.turn}: ${entry.name}`,
        // A rejected call reads as refused before its result arrives.
        modifier: entry.ok === false || entry.kind === 'rejected' ? 'refused' : entry.kind,
      };
    case 'decision':
      // The model's own sentence leads when it wrote one.
      return {
        gutter: '→',
        body: entry.narration ?? entry.summary,
        title: `turn ${entry.turn}: ${entry.summary}`,
        modifier: '',
      };
    case 'cancelled':
      return { gutter: 'dropped', body: entry.reason, title: `turn ${entry.turn}`, modifier: '' };
    case 'compacted': {
      // A compaction changes what the model knows without it acting, so it gets a row.
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

/** One tool call as a sentence; an unlisted tool falls back to its name and compact arguments. */
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
    // `todo_add` still appears in older transcripts.
    case 'todo_add':
    case 'todo_set': {
      const item = text('text');
      if (item) return args.id === undefined ? `Planned: ${item}` : `Revised plan item ${args.id}: ${item}`;
      // No text with an id is the delete overload older transcripts carry.
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
    // The size, not the script, which would take over the log; the source is one click away.
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
    // Written to be read by a person, so the message is the row.
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

/** An opened tool row. The image comes from a small server ring, so an old one 404s and is hidden. */
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

function pretty(json: string): string {
  const trimmed = json.trim();
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

/** A finished thought collapses to its word count: how long it deliberated. */
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

// The gutter names the part of the game talking, not the `AgentEvent` variant, which is the `title`.
// The `text_box` arm is unreachable while `UNLOGGED` drops that kind.
function category(kind: string): string {
  if (kind.startsWith('battle')) return 'battle';
  if (kind === 'text_box') return 'text';
  // The end of the game gets the badge strip's trophy.
  if (kind === 'hall_of_fame') return '🏆';
  return kind.includes('overworld') ? 'overworld' : kind.replace(/_/g, ' ');
}
