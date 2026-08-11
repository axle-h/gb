import { memo, useLayoutEffect, useRef, useState } from 'react';
import type { Entry } from '../api';

/** Within this many pixels of the bottom counts as following the stream. */
const PIN_SLACK = 24;

/**
 * The running commentary: agent events now, the model's own messages and tool calls from W4.
 *
 * `memo` because the status heartbeat arrives several times a second and re-rendering a 500-entry
 * log with it would be the one performance mistake this page can make. `entries` only changes when
 * something is actually said.
 */
export const Conversation = memo(function Conversation({ entries }: { entries: Entry[] }) {
  const list = useRef<HTMLDivElement>(null);
  // Pinned to the bottom unless the viewer has scrolled up to read something — in which case the
  // stream must not yank them back down.
  const [pinned, setPinned] = useState(true);
  // Every row shows its wire JSON. The per-row toggle below is the common case; this is for
  // watching the shape of a whole stretch of the stream at once.
  const [showAllRaw, setShowAllRaw] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());

  useLayoutEffect(() => {
    const element = list.current;
    if (pinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, pinned, showAllRaw, expanded]);

  const onScroll = () => {
    const element = list.current;
    if (element) setPinned(element.scrollTop + element.clientHeight >= element.scrollHeight - PIN_SLACK);
  };

  const toggle = (seq: number) =>
    setExpanded((current) => {
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
        {entries.map((entry) => {
          const { gutter, body, title, modifier } = render(entry);
          const open = showAllRaw || expanded.has(entry.seq);
          return (
            <div key={entry.seq} className={`entry ${entry.type} ${modifier}${open ? ' open' : ''}`}>
              <span className="chip" title={title}>
                {gutter}
              </span>
              <span className="body">
                <button className="line" onClick={() => toggle(entry.seq)} title="Show the JSON this line was made from">
                  {body}
                </button>
                {/* A repeat says nothing new, so it costs three characters rather than a row. */}
                {entry.count > 1 && <span className="repeat">×{entry.count}</span>}
                {open && <pre className="raw">{JSON.stringify(entry.raw, null, 2)}</pre>}
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

interface Rendered {
  /** The left gutter: who is talking. */
  gutter: string;
  body: string;
  /** The full detail, on hover — the gutter is ten characters wide and the log is long. */
  title: string;
  /** A second class name for colour. */
  modifier: string;
}

function render(entry: Entry): Rendered {
  switch (entry.type) {
    case 'agent':
      return { gutter: category(entry.kind), body: entry.text, title: entry.kind, modifier: entry.kind };
    case 'notice':
      return { gutter: entry.level, body: entry.message, title: entry.level, modifier: entry.level };
    case 'turn':
      return { gutter: `#${entry.turn}`, body: entry.headline, title: `${entry.kind} decision`, modifier: entry.kind };
    case 'assistant':
      return { gutter: 'model', body: entry.text, title: `turn ${entry.turn}`, modifier: '' };
    case 'tool':
      // Arguments are shown, not hidden: `choose_action {"id": "PalletTown:5,6:Warp"}` is the single
      // most informative line in the whole log, and `read_map {}` costs three characters.
      return {
        gutter: 'tool',
        body: `${entry.name} ${compact(entry.arguments)}`,
        title: `turn ${entry.turn}`,
        modifier: '',
      };
    case 'decision':
      return { gutter: '→', body: entry.summary, title: `turn ${entry.turn}`, modifier: '' };
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
 */
function category(kind: string): string {
  if (kind.startsWith('battle')) return 'battle';
  if (kind === 'text_box') return 'text';
  return kind.includes('overworld') ? 'overworld' : kind.replace(/_/g, ' ');
}
