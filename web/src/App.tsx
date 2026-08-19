import { useEffect, useMemo, useState } from 'react';
import type { RunStatus, UsageView } from './api';
import { Conversation } from './components/Conversation';
import { Leaderboard } from './components/Leaderboard';
import { PlanPanel } from './components/PlanPanel';
import { Screen, describeRemaining } from './components/Screen';
import { StatusPanel } from './components/StatusPanel';
import { useEventStream } from './useEventStream';

/** The phone's three tabs: the log, the trainer card and party, the model's plan. */
type PaneTab = 'log' | 'status' | 'plan';

export function App() {
  const { status, entries, connection, usage, run, plan, speed } = useEventStream();
  // Which pane a phone is showing. From 640px up the stylesheet hides the tab bar and ignores the
  // tab classes, so this state is inert on a desk. The log is the default because it is the thing
  // the page is for.
  const [tab, setTab] = useState<PaneTab>('log');
  // The Plan tab is only offered while there is a plan (PlanPanel renders nothing without one, and
  // `--policy random` never has one), so a selection that outlives its pane falls back to the log
  // rather than to an empty column.
  const pane: PaneTab = tab === 'plan' && plan.length === 0 ? 'log' : tab;
  // The leaderboard's only cue that it is stale. A win is rare enough that this counter changes at
  // most once per run, and the log is already carrying the event that says so.
  const wins = useMemo(
    () => entries.filter((entry) => entry.type === 'agent' && entry.kind === 'hall_of_fame').length,
    [entries],
  );

  // Who is playing. `null` under `--policy random`, where the run has no model and saying it has one
  // would be a small lie — the policy name beside it is the honest answer there.
  const player = status?.model ?? null;

  // ⚠️ **The tab is a third audience, and it is the one that is read while the page is not.** A
  // livestream lives in a background tab for hours, where the title is the whole of the UI, so it
  // says who is playing rather than what the site is called — and under a policy with nobody to
  // name, how it is being played. `index.html` still ships a title for the moment before the first
  // heartbeat lands.
  useEffect(() => {
    document.title = describeTitle(player, status?.policy ?? null);
  }, [player, status?.policy]);

  return (
    <div className="app">
      <header>
        {/* Two groups: who is playing, and what the run is doing. The split is what the narrow
            layout hangs off — the identity stays and the figures fold away, rather than every item
            in a flat row wrapping into a paragraph of chips. */}
        <span className="who">
          <span className="title">Pokémon Red</span>
          <span className="dim">played by</span>
          <span className="policy" title={player ? `GB_MODEL=${player}` : `--policy ${status?.policy ?? ''}`}>
            {player ?? status?.policy ?? '…'}
          </span>
        </span>
        <span className="dim sep">·</span>
        <span className={`run ${run.state}`} title={describeRun(run)}>
          {describeRun(run)}
        </span>
        <span className="spacer" />
        {/* W6's gauge. It appears only once a turn has reported figures, rather than as a
            placeholder zero, and it says when the numbers are our own estimate rather than the
            endpoint's: a guess presented as a measurement is worse than no number. */}
        {usage && (
          <span className="context" title={describeUsage(usage)}>
            <span className="gauge">
              <span className="fill" style={{ width: `${occupancy(usage)}%` }} />
            </span>
            context {Math.round(occupancy(usage))}%{usage.estimated ? '~' : ''}
            <span className="dim spent"> · {compactTokens(usage.prompt_tokens + usage.completion_tokens)} spent</span>
          </span>
        )}
        <Leaderboard wins={wins} />
        <span className={`pill ${connection}`}>
          {connection === 'live' ? status?.game?.mode ?? 'connected' : connection}
        </span>
      </header>

      <main className={`tab-${pane}`}>
        <section className="left">
          <Screen pausedUntil={run.state === 'throttled' ? run.until_ms : null} />
          {/* Phone-only (hidden from 640px up): the three panes that share what height the screen
              leaves become tabs, one scrollable pane at a time. The screen itself stays above them,
              because it is the one thing every tab wants. The buttons key off `pane` rather than
              `tab` so the Plan fallback above also moves the highlight. */}
          <nav className="pane-tabs">
            <button className={pane === 'log' ? 'on' : ''} onClick={() => setTab('log')}>
              Log
            </button>
            <button className={pane === 'status' ? 'on' : ''} onClick={() => setTab('status')}>
              Trainer
            </button>
            {plan.length > 0 && (
              <button className={pane === 'plan' ? 'on' : ''} onClick={() => setTab('plan')}>
                Plan
              </button>
            )}
          </nav>
          <StatusPanel status={status} speed={speed} />
          {/* Under the game rather than beside the conversation: it changes a few times an hour and
              is read at a glance, where the log is read as it scrolls. */}
          <PlanPanel plan={plan} />
        </section>
        <section className="right">
          <Conversation entries={entries} visible={pane === 'log'} />
        </section>
      </main>
    </div>
  );
}

function occupancy(usage: UsageView): number {
  return Math.min(100, (100 * usage.context_tokens) / Math.max(1, usage.context_limit));
}

/** The gutter is narrow and a run is long: 128 400 tokens is `128k`. */
function compactTokens(tokens: number): string {
  if (tokens < 10_000) return `${tokens}`;
  if (tokens < 1_000_000) return `${Math.round(tokens / 1000)}k`;
  return `${(tokens / 1_000_000).toFixed(1)}M`;
}

function describeUsage(usage: UsageView): string {
  const source = usage.estimated ? 'estimated, since the endpoint reports no usage' : 'reported by the endpoint';
  return [
    `${usage.context_tokens.toLocaleString()} of ${usage.context_limit.toLocaleString()} tokens in context`,
    `${usage.prompt_tokens.toLocaleString()} prompt + ${usage.completion_tokens.toLocaleString()} completion`,
    `over ${usage.completions.toLocaleString()} completions (${source})`,
  ].join('\n');
}

/**
 * What the run is doing, in the fewest words that still distinguish the cases. `playing` is the
 * quiet one and reads as such; everything else is something a viewer might want to act on.
 */
/**
 * The tab, which is a sentence rather than a label — see the `useEffect` that sets it.
 *
 * A model plays under its **full** `GB_MODEL`, not the seven characters the trainer card had to be
 * shortened to: the tab has room, and `gemma-3-12b` and `gemma-3-27b` are the same `GEMMA3` on the
 * card. Every other policy has no model to name, so it says how the game is being played instead —
 * `Pokémon Red` alone reads as a fan site rather than as a run of one.
 */
function describeTitle(player: string | null, policy: string | null): string {
  if (player) return `${player} plays Pokémon Red`;
  switch (policy) {
    case 'random':
      return 'Randomly playing Pokémon Red';
    case 'console':
      return 'Playing Pokémon Red by hand';
    case 'scripted':
      return 'Scripted playthrough of Pokémon Red';
    // `llm` with no model cannot happen (the model is what the policy is built from), and an
    // unknown policy is a build newer than this page. Both want the plain name.
    default:
      return 'Pokémon Red';
  }
}

function describeRun(run: RunStatus): string {
  switch (run.state) {
    case 'booting':
      return 'booting';
    case 'playing':
      return 'playing';
    case 'awaiting_llm':
      return `thinking · ${run.kind}`;
    case 'streaming':
      return 'replying';
    case 'running_tool':
      return `tool · ${run.name}`;
    case 'compacting':
      return 'compacting context';
    case 'rate_limited':
      return `rate limited · retrying in ${Math.round(run.retry_in_ms / 100) / 10}s`;
    case 'throttled':
      return `quota spent · paused for ${describeRemaining(run.until_ms - Date.now())}`;
    case 'error':
      return run.message;
  }
}
