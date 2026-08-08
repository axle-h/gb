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

/** `published::UsageView` — context occupancy after a turn. W6 turns this into a real gauge. */
export interface UsageView {
  context_tokens: number;
  context_limit: number;
}

/** One `data:` line of `/api/events`. `Status` is flattened into the event by serde, not nested. */
export type UiEvent =
  | ({ seq: number; type: 'status' } & Status)
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string }
  // W4. Every one of these carries `turn`, and the client groups on it — a turn is one block, not
  // one block per token.
  | { seq: number; type: 'turn_started'; turn: number; kind: string; headline: string }
  | { seq: number; type: 'assistant_delta'; turn: number; text: string }
  | { seq: number; type: 'tool_call'; turn: number; name: string; arguments: string }
  | { seq: number; type: 'decision'; turn: number; summary: string; usage: UsageView | null }
  | { seq: number; type: 'turn_cancelled'; turn: number; reason: string };

/**
 * Everything the conversation pane renders.
 *
 * `assistant` is the one entry that is not one event: the deltas of a turn are folded into a single
 * growing block by [`fold`](./useEventStream), because a hundred one-token rows is not a reply.
 */
export type Entry =
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string }
  | { seq: number; type: 'turn'; turn: number; kind: string; headline: string }
  | { seq: number; type: 'assistant'; turn: number; text: string }
  | { seq: number; type: 'tool'; turn: number; name: string; arguments: string }
  | { seq: number; type: 'decision'; turn: number; summary: string }
  | { seq: number; type: 'cancelled'; turn: number; reason: string };

export type Connection = 'connecting' | 'live' | 'reconnecting';
