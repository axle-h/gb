import type { CSSProperties } from 'react';
import type { Status } from '../api';

/** Where the player is, what they have, and how the party is holding up. */
export function StatusPanel({ status }: { status: Status | null }) {
  const game = status?.game ?? null;
  const badges = game?.badges ?? [];

  return (
    <div className="status">
      {/* The trainer card's two facts, above the place, because they are the run's identity rather
          than its position. The header says which *model* is playing; this says what the save calls
          it, and the two come apart on a resume — a run started under one `GB_MODEL` keeps its name
          when the process comes back under another, since the game has already printed it in a
          dozen places. */}
      <div className="status-line">
        <span className="trainer">{game ? game.trainer : '—'}</span>
        <span className="dim">{game ? `ID №${idNo(game.trainer_id)}` : ''}</span>
      </div>

      <div className="status-line">
        <span className="place">{game ? game.map : '—'}</span>
        <span className="dim">{game ? `(${game.position.x}, ${game.position.y})` : ''}</span>
      </div>

      <div className="status-line">
        {/* The sprites come out of the cartridge itself (`/api/badges.png`), eight 16×16 badges in
            one sheet, drawn at 2× — so this is a strip of `background-position` offsets. An
            unearned badge is the same sprite at low opacity, which is why the sheet has no
            "locked" variant. */}
        <span className="badges">
          {badges.map((badge, index) => (
            <span
              key={badge.name}
              className={badge.earned ? 'badge on' : 'badge'}
              // ⚠️ **The slot, not the offset.** The offset is the slot times the sprite's on-screen
              // size, and that size is a *responsive* number — the strip is drawn at 24px on a phone
              // and 32 elsewhere. Computing the pixels here meant the stylesheet could shrink the
              // badges and the sheet would still be sliced at the old stride: eight badges each
              // showing a quarter of the next one along. The multiplication belongs beside the
              // number it multiplies.
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

      {/* Each sprite is `/api/pokemon/{dex}/front.png`, decompressed from the cartridge's own pic
          data. The heartbeat carries only the dex number; the endpoint is `immutable`, so the
          browser fetches a species once and never asks again. `key` is the slot rather than the
          species — a party can hold two of the same Pokémon. */}
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

      {/* `agent_state` is the single most useful field when a run looks stuck, so it is on screen
          rather than in a console somewhere. It names its destination now — `move→the warp to
          OaksLab@RedsHouse2F` rather than `move→Warp` — which is longer than this column, and
          `.detail dd` ellipsises rather than wraps, so the full string is on the `title` (the same
          reason each party nickname carries one). */}
      {/* Each row is classed so the narrow layout can drop the ones a phone has no room for. The
          speed line is the operator's — a livestream that has fallen behind real time is worth
          seeing on a desk, and is three quarters of this block's width on a handset. */}
      <dl className="detail">
        <dt>mode</dt>
        <dd>{game ? (game.in_battle ? `${game.mode} · battle` : game.mode) : '(no game state)'}</dd>
        <dt>agent</dt>
        <dd title={status?.agent_state ?? undefined}>{status?.agent_state ?? '—'}</dd>
        <dt className="speed">speed</dt>
        <dd className="speed">{status ? describeSpeed(status) : '—'}</dd>
      </dl>
    </div>
  );
}

/**
 * The speed the emulator is *achieving* against the speed it is targeting. A livestream targets 1.0×,
 * so anything below it is the host failing to keep up — which is worth being able to see.
 */
function describeSpeed(status: Status): string {
  const achieved = status.wall_ms > 0 ? status.emulated_ms / status.wall_ms : 0;
  const target = status.target_speed === 1 ? 'realtime' : `target ${status.target_speed}×`;
  return `${achieved.toFixed(2)}× ${target} · ${formatDuration(status.emulated_ms)} played`;
}

/**
 * The ID as the cartridge itself prints it: five digits wide, leading zeroes kept
 * (`PrintNumber`, `LEADING_ZEROES | 2, 5` on the status screen). A player recognises `00042`; `42`
 * is the same number and not the same thing.
 */
function idNo(id: number): string {
  return `${id}`.padStart(5, '0');
}

/** `BoulderBadge` → `Boulder Badge`, for the tooltip. The wire name is the game's flag name. */
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
