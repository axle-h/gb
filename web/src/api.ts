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

/**
 * `observe::PartyMonView` — one party slot.
 *
 * ⚠️ **`dex` is a number and the sprite is a separate request.** `/api/pokemon/{dex}/front.png` is
 * `immutable`, so the browser fetches each species once and caches it for ever; putting the pixels
 * on a heartbeat that arrives several times a second would be thousands of times the traffic for
 * the same picture.
 */
export interface PartyMonView {
  nickname: string;
  /** National Pokédex number, 1–151. */
  dex: number;
  level: number;
  hp: number;
  max_hp: number;
  /** `''` when healthy; otherwise `Paralyzed`, `Asleep`, … */
  status: string;
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
  party: PartyMonView[];
  in_battle: boolean;
}

/**
 * `published::RunStatus` — what the run is doing (W6). Arrives two ways: as its own event the moment
 * it changes, and on every heartbeat so a late joiner is never more than 100 ms behind.
 */
export type RunStatus =
  | { state: 'booting' }
  | { state: 'playing' }
  | { state: 'awaiting_llm'; kind: string }
  | { state: 'streaming' }
  | { state: 'running_tool'; name: string }
  | { state: 'compacting' }
  | { state: 'rate_limited'; retry_in_ms: number }
  | { state: 'error'; message: string };

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
  run: RunStatus;
}

/** `published::UsageView` — context occupancy after a turn, and the run's bill so far. */
export interface UsageView {
  /** Prompt + completion of the most recent response: how full the window was, last time we knew. */
  context_tokens: number;
  context_limit: number;
  /** Cumulative over the run. */
  prompt_tokens: number;
  completion_tokens: number;
  completions: number;
  /** The endpoint reported no `usage` and these are our own character count. Say so, do not imply. */
  estimated: boolean;
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
  | { seq: number; type: 'turn_cancelled'; turn: number; reason: string }
  // W6.
  | { seq: number; type: 'run_status'; status: RunStatus }
  | {
      seq: number;
      type: 'compacted';
      before: number;
      after: number;
      images_evicted: number;
      summarised: boolean;
    };

/**
 * What the conversation pane renders, before the bookkeeping every row carries.
 *
 * `assistant` is the one entry that is not one event: the deltas of a turn are folded into a single
 * growing block by [`fold`](./useEventStream), because a hundred one-token rows is not a reply.
 */
export type EntryBody =
  | { type: 'agent'; kind: string; text: string }
  | { type: 'notice'; level: string; message: string }
  | { type: 'turn'; turn: number; kind: string; headline: string }
  | { type: 'assistant'; turn: number; text: string }
  | { type: 'tool'; turn: number; name: string; arguments: string }
  | { type: 'decision'; turn: number; summary: string }
  | { type: 'cancelled'; turn: number; reason: string }
  | { type: 'compacted'; before: number; after: number; images_evicted: number; summarised: boolean };

/**
 * One row of the log.
 *
 * ⚠️ **`raw` is the event exactly as it arrived**, and it is what the row reveals when it is
 * expanded. That is deliberate: the summary is prose the server wrote for people, and the thing
 * underneath it should be the real wire message rather than a second rendering of the same fields —
 * which is why nothing on the server had to grow a `detail` string. It costs one reference per row;
 * the objects were parsed either way.
 */
export type Entry = EntryBody & {
  seq: number;
  raw: UiEvent;
  /** How many identical rows this one stands for. `1` unless the agent repeated itself. */
  count: number;
};

export type Connection = 'connecting' | 'live' | 'reconnecting';

/**
 * `run::hall_of_fame::Completion` — one run that finished the game, as `/api/leaderboard` returns it.
 *
 * The rows arrive already ranked (fastest `playtime_seconds` first, a maxed clock last), so nothing
 * here re-sorts them.
 */
export interface Completion {
  /** The archive directory's name under `$GB_RUN_DIR/hall-of-fame/`. */
  archive: string;
  run_id: string;
  /** `wNumHoFTeams` after the increment: 1 is a first championship, 2 a second in the same save. */
  teams: number;
  completed_at: string;
  started_at: string;
  app_version: string;
  /** `Policy::name()` — `llm`, `random`, `console` or `scripted`. */
  policy: string;
  /** `GB_MODEL`, or `null` under any policy that is not an LLM. */
  model: string | null;

  /**
   * The cartridge's own play clock, in seconds — **the ranking key**. Emulated time from our side is
   * `emulated_ms`; these differ, and the game's own clock is the one a player would quote.
   */
  playtime_seconds: number;
  /** The same clock as `HH:MM:SS`. ⚠️ Never sort on this — see the Rust field. */
  playtime: string;
  /** The clock stopped at 255:59:59 and the real figure is unknown. */
  playtime_maxed: boolean;
  emulated_ms: number;
  /** Wall clock actually spent playing, summed over every process that played the run. */
  wall_ms: number;

  turns: number;
  completions: number;
  prompt_tokens: number;
  completion_tokens: number;
  /** The endpoint reported no usage and the token figures are our own estimate. Say so. */
  tokens_estimated: boolean;
  watchdog_firings: number;
  resumes: number;
  checkpoints: number;

  badges: number;
  pokedex_owned: number;
  pokedex_seen: number;
  money: number;
  party: { nickname: string; species: string; level: number }[];
}
