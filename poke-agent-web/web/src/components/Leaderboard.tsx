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

/**
 * Whoever decided the turns, and what that cost: `gpt-5.4-nano (1,204 turns, 1.5M tokens)`.
 *
 * ⚠️ The cost is *free text on the name* rather than two columns of its own, because a scripted run
 * has neither — `turns` and `tokens` are both zero there, and two permanently empty cells on every
 * row that did not pay an endpoint read as missing data rather than as a run that never spent
 * anything. A model's name with nothing after it says the same thing about a model that has only
 * just started.
 */
function agent(row: Completion): string {
  const total = row.prompt_tokens + row.completion_tokens;
  const cost = [
    row.turns > 0 ? `${row.turns.toLocaleString()} turns` : null,
    total > 0 ? `${tokens(total)}${row.tokens_estimated ? '~' : ''} tokens` : null,
  ].filter(Boolean);
  const name = row.model ?? row.policy;
  return cost.length > 0 ? `${name} (${cost.join(', ')})` : name;
}

/**
 * The completion date in the **viewer's** locale rather than in the ledger's ISO.
 *
 * The ledger is UTC and the reader is not, so `08-12` was ambiguous by up to a day at either end and
 * in an order half the world does not write dates in. `toLocaleDateString` with no options is the
 * short form each browser already agrees with its own operating system about; the exact instant,
 * with the time on it, is in the tooltip.
 */
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
