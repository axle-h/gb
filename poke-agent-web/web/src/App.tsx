import { useEffect, useMemo, useState } from 'react';
import type { RunStatus, UsageView } from './api';
import { BattleScriptPanel } from './components/BattleScriptPanel';
import { Conversation } from './components/Conversation';
import { Leaderboard } from './components/Leaderboard';
import { PlanPanel } from './components/PlanPanel';
import { Screen, describeRemaining } from './components/Screen';
import { SoundButton } from './components/SoundButton';
import { StatusPanel } from './components/StatusPanel';
import { useEventStream } from './useEventStream';
import { useWakeLock } from './useWakeLock';

/** The phone's tabs; Plan and Script are offered only while they have content. */
type PaneTab = 'log' | 'status' | 'plan' | 'script';

export function App() {
  const { status, entries, connection, usage, run, plan, battleScript, speed } = useEventStream();
  // Holds the screen on while the page is visible, so a phone watching does not lock; see the hook.
  useWakeLock();
  // Which pane a phone shows; inert from 640px up, where the stylesheet ignores it.
  const [tab, setTab] = useState<PaneTab>('log');
  // A selection whose pane has emptied falls back to the log; a new run clears plan and script
  // together. Every LLM run has a script from its first turn, the default; other policies never do.
  const scripted = battleScript?.source != null;
  const chosen: PaneTab = tab === 'script' && !scripted ? 'log' : tab;
  const pane: PaneTab = chosen === 'plan' && plan.length === 0 ? 'log' : chosen;
  // The leaderboard's cue that it is stale.
  const wins = useMemo(
    () => entries.filter((entry) => entry.type === 'agent' && entry.kind === 'hall_of_fame').length,
    [entries],
  );

  // `null` under every policy that is not an LLM.
  const player = status?.model ?? null;

  // A background tab's title is its whole UI, so it names who is playing.
  useEffect(() => {
    document.title = describeTitle(player, status?.policy ?? null);
  }, [player, status?.policy]);

  return (
    <div className="app">
      <header>
        {/* Identity and run figures are two groups so the narrow layout can fold the figures away. */}
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
        {/* Shown only once a turn has reported figures, marked `~` when estimated. */}
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
        {/* Kept on phones, like the trophy; the media query drops the gauge and the links. */}
        <SoundButton />
        {/* On a phone the links move to the foot of the Trainer tab. */}
        <Links />
        <span className={`pill ${connection}`}>
          {connection === 'live' ? status?.game?.mode ?? 'connected' : connection}
        </span>
      </header>

      <main className={`tab-${pane}`}>
        <section className="left">
          <Screen pausedUntil={run.state === 'throttled' ? run.until_ms : null} />
          {/* Phone-only tabs. The buttons key off `pane`, not `tab`, so a fallback moves the highlight. */}
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
            {scripted && (
              <button className={pane === 'script' ? 'on' : ''} onClick={() => setTab('script')}>
                Script
              </button>
            )}
          </nav>
          <StatusPanel status={status} speed={speed} />
          {/* Phone-only: what the header gave up, under the trainer card. */}
          <div className="about">
            {usage && (
              <span className="dim" title={describeUsage(usage)}>
                context {Math.round(occupancy(usage))}%{usage.estimated ? '~' : ''} ·{' '}
                {compactTokens(usage.prompt_tokens + usage.completion_tokens)} spent
              </span>
            )}
            <span className="spacer" />
            <Links />
          </div>
          <PlanPanel plan={plan} />
          <BattleScriptPanel script={battleScript} alwaysOpen={pane === 'script'} />
        </section>
        <section className="right">
          <Conversation entries={entries} visible={pane === 'log'} />
        </section>
      </main>
    </div>
  );
}

/** The repo and its author; the face is `web/public/mugshot.png`. */
function Links() {
  return (
    <span className="links">
      <a href="https://github.com/axle-h/gb" title="axle-h/gb on GitHub" aria-label="GitHub">
        <svg viewBox="0 0 16 16" width="20" height="20" aria-hidden="true">
          <path
            fill="currentColor"
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
      </a>
      <a href="https://ax-h.com" title="Alex Haslehurst" aria-label="ax-h.com">
        <img className="face" src="/mugshot.png" alt="Alex Haslehurst" width="24" height="24" />
      </a>
    </span>
  );
}

function occupancy(usage: UsageView): number {
  return Math.min(100, (100 * usage.context_tokens) / Math.max(1, usage.context_limit));
}

/** 128 400 tokens is `128k`. */
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

/** The tab title: the full `GB_MODEL`, or how the game is played under a policy with no model. */
function describeTitle(player: string | null, policy: string | null): string {
  if (player) return `${player} plays Pokémon Red`;
  switch (policy) {
    case 'random':
      return 'Randomly playing Pokémon Red';
    case 'console':
      return 'Playing Pokémon Red by hand';
    case 'scripted':
      return 'Scripted playthrough of Pokémon Red';
    // An unknown policy is a build newer than this page.
    default:
      return 'Pokémon Red';
  }
}

/** What the run is doing, in the fewest words that distinguish the cases. */
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
      return `endpoint refusing · paused for ${describeRemaining(run.until_ms - Date.now())}`;
    case 'error':
      return run.message;
  }
}
