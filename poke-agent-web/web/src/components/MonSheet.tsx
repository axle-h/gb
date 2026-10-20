import { useEffect } from 'react';
import type { PartyMonView } from '../api';
import { fillClass, shortStatus } from './mon';

/** One party member in full: a bottom sheet on a phone, a card in the middle of a desk. */
export function MonSheet({ mon, active, onClose }: { mon: PartyMonView; active: boolean; onClose: () => void }) {
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', escape);
    return () => window.removeEventListener('keydown', escape);
  }, [onClose]);

  const renamed = mon.nickname.toUpperCase() !== mon.species.toUpperCase();
  const span = mon.exp_next === null ? 0 : mon.exp_next - mon.exp_level;
  const progress = span <= 0 ? 100 : (100 * (mon.exp - mon.exp_level)) / span;

  return (
    <div className="overlay sheet-backdrop" onClick={onClose}>
      {/* Swallow the click the backdrop closes on. */}
      <div className="sheet" role="dialog" aria-label={mon.nickname} onClick={(event) => event.stopPropagation()}>
        <div className="sheet-grip" aria-hidden="true" />
        <div className="sheet-head">
          <img className="sprite" src={`/api/pokemon/${mon.dex}/front.png`} alt="" width={96} height={96} />
          <div className="who">
            <span className="name">{mon.nickname}</span>
            <span className="dim">
              {renamed ? `${mon.species} · ` : ''}№{`${mon.dex}`.padStart(3, '0')}
              {active ? ' · in battle' : ''}
            </span>
            <span className="types">
              {mon.types.map((type) => (
                <TypeChip key={type} type={type} />
              ))}
            </span>
          </div>
          <span className="level">L{mon.level}</span>
          <button className="close" onClick={onClose} title="close (Esc)" aria-label="close">
            ✕
          </button>
        </div>

        <div className="sheet-row">
          <span className="label">HP</span>
          <span className="bar">
            <span
              className={`fill ${fillClass(mon.hp, mon.max_hp)}`}
              style={{ width: `${mon.max_hp === 0 ? 0 : (mon.hp / mon.max_hp) * 100}%` }}
            />
          </span>
          <span className="value">
            {mon.status && <span className="ailment">{shortStatus(mon.status)} </span>}
            {mon.hp}/{mon.max_hp}
          </span>
        </div>
        <div className="sheet-row">
          <span className="label">EXP</span>
          <span className="bar exp">
            <span className="fill" style={{ width: `${Math.max(0, Math.min(100, progress))}%` }} />
          </span>
          <span className="value dim">
            {mon.exp_next === null ? 'max' : `${(mon.exp_next - mon.exp).toLocaleString()} to L${mon.level + 1}`}
          </span>
        </div>

        <dl className="stats">
          <div>
            <dt>ATK</dt>
            <dd>{mon.attack}</dd>
          </div>
          <div>
            <dt>DEF</dt>
            <dd>{mon.defense}</dd>
          </div>
          <div>
            <dt>SPD</dt>
            <dd>{mon.speed}</dd>
          </div>
          <div>
            <dt>SPC</dt>
            <dd>{mon.special}</dd>
          </div>
        </dl>

        <ol className="moves">
          {mon.moves.map((move, slot) => (
            <li key={slot} className={move.pp === 0 ? 'spent' : ''}>
              <span className="move-name">{move.name}</span>
              <TypeChip type={move.move_type} />
              <span className="pp" title="power points left">
                {move.pp}/{move.max_pp}
              </span>
              <span className="move-meta dim" title="power · accuracy">
                {move.power === null ? 'status' : `${move.power} pow`} · {move.accuracy}%
              </span>
            </li>
          ))}
          {mon.moves.length === 0 && <li className="dim">no moves</li>}
        </ol>
      </div>
    </div>
  );
}

export function TypeChip({ type }: { type: string }) {
  return <span className={`type-chip t-${type.toLowerCase()}`}>{type}</span>;
}
