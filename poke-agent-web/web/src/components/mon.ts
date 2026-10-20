// What a party slot and its sheet both draw.

export function fillClass(hp: number, max: number): string {
  if (hp === 0) return 'fainted';
  if (hp * 4 <= max) return 'low';
  if (hp * 2 <= max) return 'hurt';
  return '';
}

/** The game's own three-letter abbreviations, which is what a player expects to see. */
export function shortStatus(status: string): string {
  return { Paralyzed: 'PAR', Asleep: 'SLP', Poisoned: 'PSN', Burned: 'BRN', Frozen: 'FRZ' }[status] ?? status;
}
