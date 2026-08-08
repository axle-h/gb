import { memo, useLayoutEffect, useRef, useState } from 'react';
import type { Entry } from '../api';

/** Within this many pixels of the bottom counts as following the stream. */
const PIN_SLACK = 24;

/**
 * The running commentary: agent events now, the model's own messages and tool calls from W4.
 *
 * `memo` because the status heartbeat arrives at 10 Hz and re-rendering a 500-entry log with it
 * would be the one performance mistake this page can make. `entries` only changes when something is
 * actually said.
 */
export const Conversation = memo(function Conversation({ entries }: { entries: Entry[] }) {
  const list = useRef<HTMLDivElement>(null);
  // Pinned to the bottom unless the viewer has scrolled up to read something — in which case the
  // stream must not yank them back down.
  const [pinned, setPinned] = useState(true);

  useLayoutEffect(() => {
    const element = list.current;
    if (pinned && element) element.scrollTop = element.scrollHeight;
  }, [entries, pinned]);

  const onScroll = () => {
    const element = list.current;
    if (element) setPinned(element.scrollTop + element.clientHeight >= element.scrollHeight - PIN_SLACK);
  };

  return (
    <div className="conversation">
      <div className="conversation-list" ref={list} onScroll={onScroll}>
        {entries.length === 0 && <p className="dim">waiting for the agent…</p>}
        {entries.map((entry) => (
          <div key={entry.seq} className={`entry ${entry.type} ${entry.type === 'agent' ? entry.kind : entry.level}`}>
            <span className="chip" title={entry.type === 'agent' ? entry.kind : entry.level}>
              {entry.type === 'agent' ? category(entry.kind) : entry.level}
            </span>
            <span className="body">{entry.type === 'agent' ? entry.text : entry.message}</span>
          </div>
        ))}
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
 * The gutter says which part of the game is talking, not which `AgentEvent` variant it was:
 * `overworld_action_completed` and `started_overworld_action` differ by a glyph the line itself
 * already carries (→ ✓ ✗ 📖), and spelling both out in full costs a quarter of the width. The exact
 * variant is the `title`.
 */
function category(kind: string): string {
  if (kind.startsWith('battle')) return 'battle';
  if (kind === 'text_box') return 'text';
  return kind.includes('overworld') ? 'overworld' : kind.replace(/_/g, ' ');
}
