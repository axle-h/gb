// The wire types, mirroring `src/web/published.rs`. Hand-written rather than generated: there are
// four of them, they are the contract between the two halves, and a generator would be more
// machinery than the thing it generates.

/**
 * One gym badge. All eight are always sent, in the order of the game's own badge bits — which is the
 * order of the sprites in `/api/badges.png`, so the array index is also the sprite index.
 */
export interface BadgeView {
  name: string;
  earned: boolean;
}

/** `observe::StatusView` — what the status panel renders. */
export interface GameView {
  map: string;
  position: { x: number; y: number };
  mode: string;
  badges: BadgeView[];
  money: number;
  /** `HH:MM:SS` of in-game play time. */
  playtime: string;
  /** `[nickname-or-species, hp, max_hp]` per party slot. */
  party_hp: [string, number, number][];
  in_battle: boolean;
}

/** `published::StatusSnapshot` — the 10 Hz heartbeat. */
export interface Status {
  wall_ms: number;
  emulated_ms: number;
  target_speed: number;
  /** `"random"` or `"llm"`. */
  policy: string;
  /** Which arm of the agent's state machine is driving — the field that says why a run looks stuck. */
  agent_state: string;
  frame_seq: number;
  /** `null` mid-transition: a heartbeat that says it could not read the game beats one that stops. */
  game: GameView | null;
}

/** One `data:` line of `/api/events`. `Status` is flattened into the event by serde, not nested. */
export type UiEvent =
  | ({ seq: number; type: 'status' } & Status)
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string };

/** Everything the conversation pane renders. W4 adds the LLM's own messages to this union. */
export type Entry =
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string };

export type Connection = 'connecting' | 'live' | 'reconnecting';
