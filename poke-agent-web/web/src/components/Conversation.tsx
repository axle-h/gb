import { memo, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import type { Entry } from '../api';
import { lastModelSide } from '../useEventStream';
import {
  type ToolEntry,
  category,
  compact,
  describeTool,
  isAction,
  isIssue,
  pretty,
  summarise,
  toolChip,
} from './describe';

/** Within this many pixels of the bottom counts as following the stream. */
const PIN_SLACK = 24;

/** A card shows this many of its game lines until asked for the rest. */
const GAME_LINES = 4;

export type LogMode = 'story' | 'everything';

type TurnEntry = Extract<Entry, { type: 'turn' }>;

/** One turn and everything that happened until the next, or the rows before the first turn. */
interface Card {
  kind: 'card';
  key: number;
  turn?: TurnEntry;
  /** The rest of a turn a moment interrupted, drawn without the turn's header. */
  continued: boolean;
  rows: Entry[];
}

interface Banner {
  kind: 'moment';
  key: number;
  entry: Extract<Entry, { type: 'moment' }>;
}

/** A moment closes the card it lands in and the turn carries on below it, so the order is kept. */
function group(entries: Entry[]): (Card | Banner)[] {
  const out: (Card | Banner)[] = [];
  let card: Card | null = null;
  for (const entry of entries) {
    if (entry.type === 'turn') {
      card = { kind: 'card', key: entry.seq, turn: entry, continued: false, rows: [] };
      out.push(card);
    } else if (entry.type === 'moment') {
      out.push({ kind: 'moment', key: entry.seq, entry });
      card = null;
    } else {
      if (!card) {
        const previous = [...out].reverse().find((item): item is Card => item.kind === 'card');
        card = { kind: 'card', key: entry.seq, turn: previous?.turn, continued: previous?.turn !== undefined, rows: [] };
        out.push(card);
      }
      card.rows.push(entry);
    }
  }
  return out;
}

/** The log; `memo` so the status heartbeat never re-renders it. */
export const Conversation = memo(function Conversation({
  entries,
  visible = true,
  mode,
  raw,
}: {
  entries: Entry[];
  /** False while a phone shows another tab; see the pin effect. */
  visible?: boolean;
  mode: LogMode;
  /** Every card shows the wire JSON it was made from. */
  raw: boolean;
}) {
  const list = useRef<HTMLDivElement>(null);
  // The live thought scrolls in its own capped box, so its tail is followed separately.
  const thought = useRef<HTMLDivElement>(null);
  const [thoughtPinned, setThoughtPinned] = useState(true);
  // Pinned to the bottom unless the viewer has scrolled up.
  const [pinned, setPinned] = useState(true);
  // Rows the viewer opened, by `seq`: a tool's detail, a finished thought, a card's earlier game lines.
  const [opened, setOpened] = useState<Set<number>>(() => new Set());

  // `visible` is a dependency, not a condition: a hidden pane has no height, so the pin is re-applied
  // on return, while on a desk `visible` can be stale-false with the pane on screen.
  useLayoutEffect(() => {
    const element = list.current;
    if (pinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, pinned, raw, mode, opened, visible]);

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
    setOpened((current) => {
      const next = new Set(current);
      if (!next.delete(seq)) next.add(seq);
      return next;
    });

  const items = group(entries);
  const lastCard = items[items.length - 1];

  // A run with no model has no turns to tell apart, so it has no Story.
  const shown: LogMode = hasTurns(entries) ? mode : 'everything';
  const view: View = { mode: shown, raw, opened, toggle, liveSeq, thought, onThoughtScroll };

  return (
    <div className="conversation">
      <div className="conversation-list" ref={list} onScroll={onScroll}>
      {entries.length === 0 && <p className="dim">waiting for the agent…</p>}
      {items.map((item) =>
        item.kind === 'moment' ? (
          <MomentBanner key={item.key} entry={item.entry} />
        ) : (
          <TurnCard key={item.key} card={item} view={view} last={item === lastCard} />
        ),
      )}
      </div>
      {!pinned && (
        <button className="jump" onClick={() => setPinned(true)}>
          ↓ follow
        </button>
      )}
    </div>
  );
});

export function hasTurns(entries: Entry[]): boolean {
  return entries.some((entry) => entry.type === 'turn');
}

/** Consecutive calls that read the same and ended the same are one line with a count. */
function runs(tools: ToolEntry[], turnKind: string | undefined): { tool: ToolEntry; count: number }[] {
  const out: { tool: ToolEntry; count: number; key: string }[] = [];
  for (const tool of tools) {
    const key = `${describeTool(tool, turnKind)}|${tool.ok}|${tool.kind}`;
    const last = out[out.length - 1];
    if (last && last.key === key) last.count += 1;
    else out.push({ tool, count: 1, key });
  }
  return out;
}

interface View {
  mode: LogMode;
  raw: boolean;
  opened: Set<number>;
  toggle: (seq: number) => void;
  liveSeq: number | null;
  thought: RefObject<HTMLDivElement | null>;
  onThoughtScroll: () => void;
}

function TurnCard({ card, view, last }: { card: Card; view: View; last: boolean }) {
  const story = view.mode === 'story';
  const turnKind = card.turn?.kind;
  const tools = card.rows.filter((row): row is ToolEntry => row.type === 'tool');
  const actions = tools.filter(isAction);
  const issues = tools.filter(isIssue);
  const lookups = tools.filter((tool) => !isAction(tool) && !isIssue(tool));
  const decisions = card.rows.filter((row) => row.type === 'decision');
  const thoughts = card.rows.filter((row) => row.type === 'reasoning');
  const replies = card.rows.filter((row) => row.type === 'assistant' && row.text.trim() !== '');
  const cancelled = card.rows.filter((row) => row.type === 'cancelled');
  const notices = card.rows.filter((row) => row.type === 'notice' || row.type === 'compacted');
  const game = card.rows.filter((row) => row.type === 'agent');
  // The model's own words lead; the agent's summary stands in where it wrote none.
  const said = [...new Set(decisions.flatMap((row) => (row.type === 'decision' && row.narration ? [row.narration] : [])))];
  const summaries = said.length > 0 ? [] : [...new Set(decisions.map((row) => (row.type === 'decision' ? row.summary : '')))];
  const live = thoughts.find((row) => row.seq === view.liveSeq);
  // A turn thinks between every tool step; the finished thoughts open and close as one.
  const finished = thoughts.filter((row) => row !== live);
  const thinkingOpen = finished.length > 0 && view.opened.has(finished[0].seq);
  const loudNotices = notices.filter((row) => row.type === 'notice' && (row.level === 'warn' || row.level === 'error'));

  const hasModel = card.turn !== undefined && !card.continued;
  // Rows before any turn: under a policy with no model that is the whole run, so its story is the game.
  const loose = card.turn === undefined;
  if (story && !hasModel && issues.length === 0 && loudNotices.length === 0 && !(loose && game.length > 0)) {
    return null;
  }

  const at = card.rows[0]?.at ?? card.turn?.at;
  const shownGame = view.opened.has(card.key) ? game : game.slice(-GAME_LINES);
  const hidden = game.length - shownGame.length;

  return (
    <article className={`card${card.continued ? ' continued' : ''}${live ? ' live' : ''}`}>
      {hasModel && card.turn && (
        <header className="card-head" title={`${card.turn.kind} decision`}>
          <span className="turn-no">#{card.turn.turn}</span>
          {!card.turn.headline.startsWith(card.turn.kind) && (
            <span className={`kind ${card.turn.kind}`}>{card.turn.kind.replace(/_/g, ' ')}</span>
          )}
          <span className={`headline ${card.turn.kind}`}>{card.turn.headline}</span>
          <time dateTime={at ? new Date(at).toISOString() : undefined} title={longTime(at)}>
            {shortTime(at)}
          </time>
        </header>
      )}

      {said.map((text) => (
        <p className="narration" key={text}>
          {text}
        </p>
      ))}
      {summaries.map((text) => (
        <p className="summary" key={text} title="the agent's account; the model wrote none">
          {text}
        </p>
      ))}

      {last && hasModel && decisions.length === 0 && cancelled.length === 0 && !live && (
        <p className="pending dim">deciding…</p>
      )}

      {live && (
        <div className="thought live" ref={view.thought} onScroll={view.onThoughtScroll}>
          {live.text || '…'}
        </div>
      )}

      {runs(actions, turnKind).map(({ tool, count }) => (
        <ToolLine key={tool.seq} tool={tool} count={count} turnKind={turnKind} view={view} className="action" prefix="→ " />
      ))}

      {issues.map((tool) => (
        <ToolLine key={tool.seq} tool={tool} count={1} turnKind={turnKind} view={view} className="issue" prefix="⚑ " />
      ))}

      {cancelled.map((row) => (
        <p className="cancelled dim" key={row.seq}>
          dropped: {row.type === 'cancelled' ? row.reason : ''}
        </p>
      ))}

      {loudNotices.map((row) => (
        <p className={`notice ${row.type === 'notice' ? row.level : ''}`} key={row.seq}>
          {row.type === 'notice' ? row.message : ''}
        </p>
      ))}

      {!story && (lookups.length > 0 || finished.length > 0) && (
        <div className="chips">
          {finished.length > 0 && (
            <button
              className={`chip thinking${thinkingOpen ? ' on' : ''}`}
              onClick={() => view.toggle(finished[0].seq)}
              title="What the model thought on its way to this turn"
            >
              💭 {summarise(finished.map((row) => (row.type === 'reasoning' ? row.text : '')).join(' ')).replace(/^thought for /, '')}
            </button>
          )}
          {lookups.map((tool) => (
            <button
              key={tool.seq}
              className={`chip${tool.ok === false ? ' refused' : ''}${view.opened.has(tool.seq) ? ' on' : ''}`}
              onClick={() => view.toggle(tool.seq)}
              title={describeTool(tool, turnKind)}
            >
              {toolChip(tool)}
            </button>
          ))}
        </div>
      )}

      {!story &&
        thinkingOpen &&
        finished.map((row) => (
            <div className="thought" key={row.seq}>
              {row.type === 'reasoning' ? row.text : ''}
            </div>
          ))}
      {!story &&
        lookups.filter((tool) => view.opened.has(tool.seq)).map((tool) => (
          <ToolDetail key={tool.seq} entry={tool} title={describeTool(tool, turnKind)} />
        ))}

      {!story &&
        replies.map((row) => (
          <p className="reply" key={row.seq}>
            {row.type === 'assistant' ? row.text : ''}
          </p>
        ))}

      {!story &&
        notices
          .filter((row) => !loudNotices.includes(row))
          .map((row) => (
            <p className="notice quiet" key={row.seq}>
              {row.type === 'compacted'
                ? `context compacted ${row.before.toLocaleString()} → ${row.after.toLocaleString()} tokens${row.summarised ? ' (summarised)' : ''}`
                : row.type === 'notice'
                  ? row.message
                  : ''}
            </p>
          ))}

      {(!story || loose) && game.length > 0 && (
        <ul className="game">
          {hidden > 0 && (
            <li>
              <button className="more" onClick={() => view.toggle(card.key)}>
                {hidden} earlier line{hidden === 1 ? '' : 's'}
              </button>
            </li>
          )}
          {shownGame.map((row) =>
            row.type === 'agent' ? (
              <li key={row.seq} className={`line ${category(row.kind).replace(/\s/g, '-')} ${row.kind}`} title={row.kind}>
                {row.text}
                {row.count > 1 && <span className="repeat">×{row.count}</span>}
              </li>
            ) : null,
          )}
          {hidden === 0 && game.length > GAME_LINES && (
            <li>
              <button className="more" onClick={() => view.toggle(card.key)}>
                fewer
              </button>
            </li>
          )}
        </ul>
      )}

      {view.raw && (
        <pre className="raw">{card.rows.map((row) => JSON.stringify(row.raw)).join('\n')}</pre>
      )}
    </article>
  );
}

function ToolLine({
  tool,
  count,
  turnKind,
  view,
  className,
  prefix,
}: {
  tool: ToolEntry;
  count: number;
  turnKind: string | undefined;
  view: View;
  className: string;
  prefix: string;
}) {
  const open = view.opened.has(tool.seq);
  const refused = tool.ok === false || tool.kind === 'rejected';
  const sentence = describeTool(tool, turnKind);
  return (
    <>
      <button
        className={`tool-line ${className}${refused ? ' refused' : ''}`}
        onClick={() => view.toggle(tool.seq)}
        title="What was asked and what came back"
      >
        {prefix}
        {sentence}
        {count > 1 && <span className="repeat">×{count}</span>}
        {refused && <span className="tag">refused</span>}
        <span className="disclose">{open ? '▾' : '▸'}</span>
      </button>
      {open && <ToolDetail entry={tool} />}
    </>
  );
}

/** A moment is the run's story told in one line, with the screen it was taken on. */
function MomentBanner({ entry }: { entry: Extract<Entry, { type: 'moment' }> }) {
  const [screen, setScreen] = useState(true);
  return (
    <div className={`moment ${entry.kind}`}>
      {entry.n !== undefined && screen && (
        <img
          src={`/api/moments/${entry.n}/screen.png`}
          alt=""
          width={80}
          height={72}
          loading="lazy"
          onError={() => setScreen(false)}
        />
      )}
      <span className="moment-icon">{momentIcon(entry.kind)}</span>
      <span className="moment-title">{capitalise(entry.title)}</span>
      <time className="dim" title={longTime(entry.at)}>
        {shortTime(entry.at)}
      </time>
    </div>
  );
}

export function momentIcon(kind: string): string {
  switch (kind) {
    case 'badge':
      return '🏅';
    case 'boss_battle':
      return '⚔️';
    case 'evolution':
      return '✨';
    case 'new_species':
      return '📖';
    case 'lull':
      return '⏳';
    default:
      return '📸';
  }
}

export function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** `HH:MM` in the viewer's timezone; the full stamp is the `title`. */
function shortTime(at: number | undefined): string {
  if (at === undefined) return '';
  return new Date(at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
}

function longTime(at: number | undefined): string | undefined {
  return at === undefined ? undefined : new Date(at).toLocaleString();
}

/** An opened tool row. The image comes from a small server ring, so an old one 404s and is hidden. */
function ToolDetail({ entry, title }: { entry: ToolEntry; title?: string }) {
  const args = compact(entry.arguments);
  return (
    <div className="tool-detail">
      {title && <span className="tool-title">{title}</span>}
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
