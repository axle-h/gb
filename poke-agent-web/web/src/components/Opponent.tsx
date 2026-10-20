import type { OpponentView } from '../api';
import { TypeChip } from './MonSheet';
import { fillClass, shortStatus } from './mon';

/** Who the player is fighting, under the screen for as long as the battle lasts. */
export function Opponent({ opponent }: { opponent: OpponentView }) {
  const share = opponent.max_hp === 0 ? 0 : (opponent.hp / opponent.max_hp) * 100;
  const whose = opponent.battle === 'Trainer' ? 'Foe' : 'Wild';
  return (
    <div className="opponent" aria-label={`fighting ${opponent.species}, level ${opponent.level}`}>
      {opponent.dex !== null ? (
        <img className="sprite" src={`/api/pokemon/${opponent.dex}/front.png`} alt="" width={40} height={40} />
      ) : (
        <span className="sprite ghost" aria-hidden="true">👻</span>
      )}
      <span className="who">
        <span className="dim">{whose}</span> {opponent.species}
        <span className="dim"> L{opponent.level}</span>
        {opponent.status && <span className="ailment"> {shortStatus(opponent.status)}</span>}
      </span>
      <span className="types">
        {opponent.types.map((type) => (
          <TypeChip key={type} type={type} />
        ))}
      </span>
      <span className="bar">
        <span className={`fill ${fillClass(opponent.hp, opponent.max_hp)}`} style={{ width: `${share}%` }} />
      </span>
      <span className="hp">
        {opponent.hp}/{opponent.max_hp}
      </span>
    </div>
  );
}
