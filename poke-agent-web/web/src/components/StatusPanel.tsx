import type { CSSProperties } from 'react';
import type { Status } from '../api';

/** Where the player is, what they have, and how the party is holding up. */
export function StatusPanel({ status, speed }: { status: Status | null; speed: number | null }) {
  const game = status?.game ?? null;
  const badges = game?.badges ?? [];

  return (
    <div className="status">
      {/* The save's name, which can differ from the header's model after a resume. */}
      <div className="status-line">
        <span className="trainer">{game ? game.trainer : '—'}</span>
        <span className="dim">{game ? `ID №${idNo(game.trainer_id)}` : ''}</span>
      </div>

      <div className="status-line">
        <span className="place">{game ? game.map : '—'}</span>
        <span className="dim">{game ? `(${game.position.x}, ${game.position.y})` : ''}</span>
      </div>

      <div className="status-line">
        {/* One cartridge sprite sheet sliced by `background-position`; an unearned badge is dimmed. */}
        <span className="badges">
          {badges.map((badge, index) => (
            <span
              key={badge.name}
              className={badge.earned ? 'badge on' : 'badge'}
              // The slot, not a pixel offset: the stylesheet multiplies it by the responsive sprite size.
              style={{ '--slot': index } as CSSProperties}
              title={`${spaced(badge.name)}${badge.earned ? '' : ' (not earned)'}`}
            />
          ))}
        </span>
      </div>

      <div className="status-line">
        <span className="money">{game ? `¥${game.money.toLocaleString()}` : ''}</span>
        <span className="dim">{game?.playtime ?? ''}</span>
      </div>

      {/* `key` is the slot, since a party can hold two of one species. */}
      <ol className="party">
        {game?.party.map((mon, slot) => (
          <li key={slot}>
            <img className="sprite" src={`/api/pokemon/${mon.dex}/front.png`} alt="" loading="lazy" width={56} height={56} />
            <span className="mon" title={mon.nickname}>
              {mon.nickname}
            </span>
            <span className="meta">
              {mon.status && <span className="ailment">{shortStatus(mon.status)}</span>}
              <span className="level">L{mon.level}</span>
            </span>
            <span className="bar">
              <span
                className={`fill ${fillClass(mon.hp, mon.max_hp)}`}
                style={{ width: `${mon.max_hp === 0 ? 0 : (mon.hp / mon.max_hp) * 100}%` }}
              />
            </span>
            <span className="hp">
              {mon.hp}/{mon.max_hp}
            </span>
          </li>
        ))}
      </ol>

      {/* `agent_state` outgrows the column and is ellipsised, so the full string is the `title`.
          Each row is classed so the narrow layout can drop it. */}
      <dl className="detail">
        <dt>mode</dt>
        <dd>{game ? (game.in_battle ? `${game.mode} · battle` : game.mode) : '(no game state)'}</dd>
        <dt>agent</dt>
        <dd title={status?.agent_state ?? undefined}>{status?.agent_state ?? '—'}</dd>
        <dt className="speed">speed</dt>
        <dd className="speed">{status ? describeSpeed(status, speed) : '—'}</dd>
      </dl>
    </div>
  );
}

/** Speed now against target; `speed` is measured between heartbeats, never `emulated_ms / wall_ms`. */
function describeSpeed(status: Status, speed: number | null): string {
  const target = status.target_speed === 1 ? 'realtime' : `target ${status.target_speed}×`;
  // `null` until the first window closes.
  const achieved = speed === null ? '—' : `${speed.toFixed(2)}×`;
  const dropped =
    status.dropped_ms > 0 ? ` · ${formatDuration(status.dropped_ms)} dropped` : '';
  // `run_emulated_ms`, never `emulated_ms`, which restarts with the process.
  return `${achieved} ${target} · ${formatDuration(status.run_emulated_ms)} played${dropped}`;
}

/** Five digits with leading zeroes, as the cartridge prints it. */
function idNo(id: number): string {
  return `${id}`.padStart(5, '0');
}

/** `BoulderBadge` → `Boulder Badge`, for the tooltip. */
function spaced(name: string): string {
  return name.replace(/([a-z])([A-Z])/g, '$1 $2');
}

function fillClass(hp: number, max: number): string {
  if (hp === 0) return 'fainted';
  if (hp * 4 <= max) return 'low';
  if (hp * 2 <= max) return 'hurt';
  return '';
}

/** The game's own three-letter abbreviations, which is what a player expects to see. */
function shortStatus(status: string): string {
  return { Paralyzed: 'PAR', Asleep: 'SLP', Poisoned: 'PSN', Burned: 'BRN', Frozen: 'FRZ' }[status] ?? status;
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = `${Math.floor(total / 60) % 60}`.padStart(2, '0');
  const seconds = `${total % 60}`.padStart(2, '0');
  return hours > 0 ? `${hours}:${minutes}:${seconds}` : `${minutes}:${seconds}`;
}
