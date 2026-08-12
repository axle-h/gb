import { useCallback, useEffect, useState } from 'react';
import type { Completion } from '../api';

/** What the overlay asks for. The server clamps anything larger. */
const LIMIT = 10;

/**
 * The runs that have finished the game, behind a trophy in the header.
 *
 * An overlay rather than a panel because the page has no room for one: the left column is the screen
 * and the party, the right is the log, and a leaderboard is something you look at occasionally
 * rather than watch. It is also the only thing on the page that is not driven by the event stream —
 * completions are rare, so it is fetched when opened and refetched when the log says another run has
 * just won.
 */
export function Leaderboard({ wins }: { wins: number }) {
  const [open, setOpen] = useState(false);
  const [rows, setRows] = useState<Completion[] | null>(null);
  const [failed, setFailed] = useState(false);

  const load = useCallback(() => {
    setFailed(false);
    fetch(`/api/leaderboard?limit=${LIMIT}`)
      .then((response) => (response.ok ? response.json() : Promise.reject(response.status)))
      .then(setRows)
      .catch(() => setFailed(true));
  }, []);

  // On open, and again whenever a `hall_of_fame` entry arrives — which is the only thing that can
  // change the answer, and it comes down the stream the log is already reading.
  useEffect(() => {
    if (open) load();
  }, [open, wins, load]);

  useEffect(() => {
    if (!open) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    window.addEventListener('keydown', escape);
    return () => window.removeEventListener('keydown', escape);
  }, [open]);

  return (
    <>
      <button className="trophy" onClick={() => setOpen(true)} title="runs that have finished the game">
        🏆
      </button>
      {open && (
        <div className="overlay" onClick={() => setOpen(false)}>
          {/* The panel swallows the click the backdrop closes on, so selecting a run id does not
              dismiss the thing it is in. */}
          <div className="leaderboard" onClick={(event) => event.stopPropagation()}>
            <header>
              <span className="title">🏆 Hall of Fame</span>
              <span className="spacer" />
              <button className="close" onClick={() => setOpen(false)} title="close (Esc)">
                ✕
              </button>
            </header>
            {failed && <p className="dim">the leaderboard could not be read</p>}
            {!failed && rows === null && <p className="dim">reading the ledger…</p>}
            {!failed && rows?.length === 0 && <p className="dim">nobody has finished the game yet</p>}
            {!failed && rows !== null && rows.length > 0 && <Table rows={rows} />}
          </div>
        </div>
      )}
    </>
  );
}

function Table({ rows }: { rows: Completion[] }) {
  return (
    <div className="scroller">
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>finished</th>
            <th className="right">in game</th>
            <th className="right">real</th>
            <th className="right">turns</th>
            <th className="right">tokens</th>
            <th>decided by</th>
            <th className="right">gb</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={`${row.archive}`}>
              <td className="dim">{index + 1}</td>
              <td title={`run ${row.run_id}, archived as ${row.archive}`}>{day(row.completed_at)}</td>
              <td className="right" title={row.playtime_maxed ? 'the game’s clock stopped at 255:59:59' : row.playtime}>
                {row.playtime_maxed ? '255:59:59+' : row.playtime}
              </td>
              <td className="right" title={`${row.resumes} resume${row.resumes === 1 ? '' : 's'}`}>
                {duration(row.wall_ms)}
              </td>
              <td className="right">{row.turns.toLocaleString()}</td>
              <td className="right" title={tokenDetail(row)}>
                {tokens(row.prompt_tokens + row.completion_tokens)}
                {row.tokens_estimated ? '~' : ''}
              </td>
              <td title={`${row.badges} badges · ${row.pokedex_owned} caught`}>{row.model ?? row.policy}</td>
              <td className="right dim">{row.app_version}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** `2026-08-12T14:30:00Z` → `08-12`. The year is the same for everything on a ten-row board. */
function day(iso: string): string {
  return iso.slice(5, 10);
}

/** Wall clock as `6h12m`, or `12m` under an hour. A finished run is never seconds long. */
function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  const hours = Math.floor(minutes / 60);
  return hours > 0 ? `${hours}h${`${minutes % 60}`.padStart(2, '0')}m` : `${minutes}m`;
}

/** The same compaction the header's context gauge uses: 128 400 → `128k`. */
function tokens(total: number): string {
  if (total === 0) return '—';
  if (total < 10_000) return `${total}`;
  if (total < 1_000_000) return `${Math.round(total / 1000)}k`;
  return `${(total / 1_000_000).toFixed(1)}M`;
}

function tokenDetail(row: Completion): string {
  const source = row.tokens_estimated ? 'estimated — the endpoint reported no usage' : 'reported by the endpoint';
  return [
    `${row.prompt_tokens.toLocaleString()} prompt + ${row.completion_tokens.toLocaleString()} completion`,
    `over ${row.completions.toLocaleString()} completions (${source})`,
    row.watchdog_firings > 0 ? `the watchdog fired ${row.watchdog_firings} time(s)` : null,
  ]
    .filter(Boolean)
    .join('\n');
}
