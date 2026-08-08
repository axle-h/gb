import { Conversation } from './components/Conversation';
import { Screen } from './components/Screen';
import { StatusPanel } from './components/StatusPanel';
import { useEventStream } from './useEventStream';

export function App() {
  const { status, entries, connection, usage } = useEventStream();

  return (
    <div className="app">
      <header>
        <span className="title">Pokémon Red</span>
        <span className="dim">·</span>
        <span className="policy">{status?.policy ?? '…'}</span>
        <span className="spacer" />
        {/* Context occupancy after the last turn. W6 makes this a real gauge with cumulative totals
            and an estimated-vs-reported flag; until then it is the one number there is, and it
            appears only once a turn has reported one rather than as a placeholder zero. */}
        {usage && (
          <span className="context" title={`${usage.context_tokens} of ${usage.context_limit} tokens`}>
            context {Math.round((100 * usage.context_tokens) / Math.max(1, usage.context_limit))}%
          </span>
        )}
        <span className={`pill ${connection}`}>
          {connection === 'live' ? status?.game?.mode ?? 'connected' : connection}
        </span>
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
