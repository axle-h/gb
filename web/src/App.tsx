import type { RunStatus, UsageView } from './api';
import { Conversation } from './components/Conversation';
import { NewRunButton } from './components/NewRunButton';
import { Screen } from './components/Screen';
import { StatusPanel } from './components/StatusPanel';
import { useEventStream } from './useEventStream';

export function App() {
  const { status, entries, connection, usage, run } = useEventStream();

  return (
    <div className="app">
      <header>
        <span className="title">Pokémon Red</span>
        <span className="dim">·</span>
        <span className="policy">{status?.policy ?? '…'}</span>
        <span className={`run ${run.state}`} title={describeRun(run)}>
          {describeRun(run)}
        </span>
        <span className="spacer" />
        {/* W6's gauge. It appears only once a turn has reported figures, rather than as a
            placeholder zero, and it says when the numbers are our own estimate rather than the
            endpoint's — a guess presented as a measurement is worse than no number. */}
        {usage && (
          <span className="context" title={describeUsage(usage)}>
            <span className="gauge">
              <span className="fill" style={{ width: `${occupancy(usage)}%` }} />
            </span>
            context {Math.round(occupancy(usage))}%{usage.estimated ? '~' : ''}
            <span className="dim"> · {compactTokens(usage.prompt_tokens + usage.completion_tokens)} spent</span>
          </span>
        )}
        <span className={`pill ${connection}`}>
          {connection === 'live' ? status?.game?.mode ?? 'connected' : connection}
        </span>
        <NewRunButton />
      </header>

      <main>
        <section className="left">
          <Screen />
          <StatusPanel status={status} />
        </section>
        <section className="right">
          <Conversation entries={entries} />
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
  const source = usage.estimated ? 'estimated — the endpoint reports no usage' : 'reported by the endpoint';
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
    case 'error':
      return run.message;
  }
}
