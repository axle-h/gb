import { Conversation } from './components/Conversation';
import { Screen } from './components/Screen';
import { StatusPanel } from './components/StatusPanel';
import { useEventStream } from './useEventStream';

export function App() {
  const { status, entries, connection } = useEventStream();

  return (
    <div className="app">
      <header>
        {/* W4 puts the model and the token/context accounting here — it is the one part of §6's
            mock that has nothing behind it yet, and inventing a placeholder number would be worse
            than the gap. */}
        <span className="title">Pokémon Red</span>
        <span className="dim">·</span>
        <span className="policy">{status?.policy ?? '…'}</span>
        <span className="spacer" />
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
