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

/**
 * The phone's tabs: the log, the trainer card and party, the model's plan, and the program deciding
 * its battles. The last two are conditional — see `pane` below.
 */
type PaneTab = 'log' | 'status' | 'plan' | 'script';

export function App() {
  const { status, entries, connection, usage, run, plan, battleScript, speed } = useEventStream();
  // A phone watching a livestream is an idle phone as far as the phone is concerned, and it dims and
  // then locks. Nothing else on the page notices, so this holds the screen on while it is visible.
  // No-op on a desk, on an insecure origin, and while the tab is in the background: see the hook.
  useWakeLock();
  // Which pane a phone is showing. From 640px up the stylesheet hides the tab bar and ignores the
  // tab classes, so this state is inert on a desk. The log is the default because it is the thing
  // the page is for.
  const [tab, setTab] = useState<PaneTab>('log');
  // The Plan and Script tabs are only offered while there is something in them — neither panel draws
  // anything otherwise, and only an LLM ever has either — so a selection that outlives its pane falls
  // back to the log rather than to an empty column. Both can vanish under a running page: a
  // `POST /api/new-run` clears the plan and the script together.
  //
  // ⚠️ **The Script tab is now offered from an LLM run's first turn**, because every run starts on
  // `battle_script::DEFAULT` rather than on nothing. That is the honest answer and not clutter: the
  // chip in the head says `default`, which is the live fact that this run's battles are costing it a
  // request each. A run under `random` or `deterministic` still publishes no script and gets no tab.
  const scripted = battleScript?.source != null;
  const chosen: PaneTab = tab === 'script' && !scripted ? 'log' : tab;
  const pane: PaneTab = chosen === 'plan' && plan.length === 0 ? 'log' : chosen;
  // The leaderboard's only cue that it is stale. A win is rare enough that this counter changes at
  // most once per run, and the log is already carrying the event that says so.
  const wins = useMemo(
    () => entries.filter((entry) => entry.type === 'agent' && entry.kind === 'hall_of_fame').length,
    [entries],
  );

  // Who is playing. `null` under every policy that is not an LLM — `random`, `scripted` — where the
  // run has no model and saying it has one would be a small lie; the policy name beside it is the
  // honest answer there.
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
        {/* Beside the trophy rather than on the screen, where it first went: over four shades of
            Game Boy it was a small grey glyph on whatever the game happened to be drawing, and it
            had to be dimmed further still not to sit on top of the picture. Here it is one control
            among several, at a size the rest of the row already established. Like the trophy it
            survives the phone layout — the media query drops the context gauge and the links
            because they are a desk's questions, and sound is not. */}
        <SoundButton />
        {/* Off the header on a phone and at the foot of the Trainer tab instead, with the context
            figure: neither is about the run, and the row has three lines' worth of things that are. */}
        <Links />
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
          {/* Under the game rather than beside the conversation: it changes a few times an hour and
              is read at a glance, where the log is read as it scrolls. */}
          <PlanPanel plan={plan} />
          {/* Under the plan, and closed: the plan is what the run is trying to do and moves every few
              turns, this is how it fights and moves a handful of times a playthrough. On a phone it
              is a tab of its own, where the pane it fills already answers the question the chevron
              asks. */}
          <BattleScriptPanel script={battleScript} alwaysOpen={pane === 'script'} />
        </section>
        <section className="right">
          <Conversation entries={entries} visible={pane === 'log'} />
        </section>
      </main>
    </div>
  );
}

/** The repo this is, and who made it. The face is `web/public/mugshot.png`, copied from ax-h.com. */
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
