import { useCallback, useEffect, useState } from 'react';
import type { Completion } from '../api';

/** What the overlay asks for. The server clamps anything larger. */
const LIMIT = 10;

/** Finished runs behind the header's trophy, fetched on open and whenever another run wins. */
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

  // On open, and whenever a `hall_of_fame` entry arrives, the only thing that changes the answer.
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
          {/* Swallow the click the backdrop closes on. */}
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
            <th className="num">time</th>
            <th className="agent">agent</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={`${row.archive}`}>
              <td className="dim">{index + 1}</td>
              <td title={runDetail(row)}>{day(row.completed_at)}</td>
              <td className="num" title={row.playtime_maxed ? 'the game’s clock stopped at 255:59:59' : row.playtime}>
                {row.playtime_maxed ? '255:59:59+' : row.playtime}
              </td>
              <td className="agent" title={tokenDetail(row)}>
                {agent(row)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Who decided the turns, with the cost as text on the name, since a scripted run has none. */
function agent(row: Completion): string {
  const total = row.prompt_tokens + row.completion_tokens;
  const cost = [
    row.turns > 0 ? `${row.turns.toLocaleString()} turns` : null,
    total > 0 ? `${tokens(total)}${row.tokens_estimated ? '~' : ''} tokens` : null,
  ].filter(Boolean);
  const name = row.model ?? row.policy;
  return cost.length > 0 ? `${name} (${cost.join(', ')})` : name;
}

/** The completion date in the viewer's locale; the ledger is UTC. */
function day(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? iso : at.toLocaleDateString();
}

function runDetail(row: Completion): string {
  const at = new Date(row.completed_at);
  return [
    Number.isNaN(at.getTime()) ? row.completed_at : at.toLocaleString(),
    `run ${row.run_id}, archived as ${row.archive}`,
    `${row.badges} badges · ${row.pokedex_owned} caught`,
    `${row.resumes} resume${row.resumes === 1 ? '' : 's'} · gb ${row.app_version}`,
  ].join('\n');
}

/** The same compaction the header's context gauge uses: 128 400 → `128k`. */
function tokens(total: number): string {
  if (total < 10_000) return `${total}`;
  if (total < 1_000_000) return `${Math.round(total / 1000)}k`;
  return `${(total / 1_000_000).toFixed(1)}M`;
}

function tokenDetail(row: Completion): string {
  if (row.prompt_tokens + row.completion_tokens === 0) return `decided by ${row.policy}, which asks no endpoint`;
  const source = row.tokens_estimated ? 'estimated — the endpoint reported no usage' : 'reported by the endpoint';
  return [
    `${row.prompt_tokens.toLocaleString()} prompt + ${row.completion_tokens.toLocaleString()} completion`,
    `over ${row.completions.toLocaleString()} completions (${source})`,
    row.watchdog_firings > 0 ? `the watchdog fired ${row.watchdog_firings} time(s)` : null,
  ]
    .filter(Boolean)
    .join('\n');
}
