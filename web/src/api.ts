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
  /**
   * `GB_MODEL` — who is playing. `null` under any policy that is not an LLM, which is the same rule
   * the leaderboard's column follows: `"random"` is not a model name and must not be shown as one.
   */
  model: string | null;
  /** Which arm of the agent's state machine is driving — the field that says why a run looks stuck. */
  agent_state: string;
  frame_seq: number;
  /** `null` mid-transition: a heartbeat that says it could not read the game beats one that stops. */
  game: GameView | null;
  run: RunStatus;
}

/**
 * `published::TodoView` — one item on the model's plan.
 *
 * ⚠️ **The only thing on this page that is neither the game nor the conversation.** It is what the
 * run is *trying* to do, which a viewer cannot infer from watching either — and it is also the only
 * thing the model writes that survives a compaction or a restart, so the two audiences want the
 * same list for once.
 */
export interface TodoView {
  id: number;
  text: string;
  done: boolean;
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

/**
 * When an event was published, in Unix milliseconds.
 *
 * ⚠️ **Optional, and it has to stay optional.** `/api/history` replays a transcript written by
 * whatever build was running at the time, and the runs on disk predate this field — a required `at`
 * would be a type that lies about half the backlog. Everything that renders one treats its absence
 * as "no time known" rather than as the epoch.
 */
export type At = { at?: number };

/** One `data:` line of `/api/events`. `Status` is flattened into the event by serde, not nested. */
export type UiEvent = At &
  (
  | ({ seq: number; type: 'status' } & Status)
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string }
  // W4. Every one of these carries `turn`, and the client groups on it — a turn is one block, not
  // one block per token.
  | { seq: number; type: 'turn_started'; turn: number; kind: string; headline: string }
  | { seq: number; type: 'assistant_delta'; turn: number; text: string }
  /** The model's thinking, for endpoints that stream it apart from the reply (`reasoning_content`). */
  | { seq: number; type: 'assistant_reasoning'; turn: number; text: string }
  /**
   * `id` pairs this with the `tool_result` that answers it — a turn can call several tools in one
   * message and they are answered as a batch, so neither position nor arrival order will do it.
   * `kind` is what the call turned out to be: `read`, `todo`, `terminal`, `rejected`.
   */
  | {
      seq: number;
      type: 'tool_call';
      turn: number;
      id: string;
      kind: string;
      name: string;
      arguments: string;
    }
  /**
   * What that call answered. `content` is the text the *model* was sent, truncated server-side.
   *
   * ⚠️ `image` is a flag, not a picture: the PNG is fetched from `/api/tool-image/{seq}/image.png`
   * against **this event's** seq, and it is served out of a small ring — so a live viewer gets it
   * and a page replaying an old backlog gets a 404 and shows the text on its own.
   */
  | {
      seq: number;
      type: 'tool_result';
      turn: number;
      id: string;
      name: string;
      ok: boolean;
      content: string;
      image: boolean;
    }
  /**
   * `summary` is the agent's mechanical account of what it was told to do; `narration` is the
   * model's own sentence about why, from the `summary` argument every terminal tool carries. Null
   * when the model omitted it, and on the wait the loop forces when a turn will not decide.
   */
  | {
      seq: number;
      type: 'decision';
      turn: number;
      summary: string;
      narration: string | null;
      usage: UsageView | null;
    }
  | { seq: number; type: 'turn_cancelled'; turn: number; reason: string }
  // W6.
  | { seq: number; type: 'run_status'; status: RunStatus }
  /**
   * W6b. The model's plan, in full, whenever it changes — so the newest one wins and every earlier
   * one is discarded, including the ones `/api/history` replays into a page that just loaded.
   */
  | { seq: number; type: 'plan'; items: TodoView[] }
  | {
      seq: number;
      type: 'compacted';
      before: number;
      after: number;
      images_evicted: number;
      summarised: boolean;
    }
  );

/**
 * What the conversation pane renders, before the bookkeeping every row carries.
 *
 * `assistant` and `reasoning` are the entries that are not one event: the deltas of a turn are
 * folded into a single growing block by [`fold`](./useEventStream), because a hundred one-token rows
 * is not a reply.
 */
export type EntryBody =
  | { type: 'agent'; kind: string; text: string }
  | { type: 'notice'; level: string; message: string }
  | { type: 'turn'; turn: number; kind: string; headline: string }
  | { type: 'assistant'; turn: number; text: string }
  /**
   * One block of thinking. It has no "finished" flag of its own and does not need one: a thought
   * ends when anything else is said, so the row is live exactly while it is the last one in the log.
   */
  | { type: 'reasoning'; turn: number; text: string }
  /**
   * One tool call and, once it arrives, its answer — **one row, not two**. They are separate events
   * because the answer can be a round trip through the emulator away, but a call and its result are
   * one thing that happened, and splitting them would put "read the map" and what the map said at
   * opposite ends of whatever the agent narrated in between.
   *
   * ⚠️ A row that grows must never be collapse-matched; see `signature` in `useEventStream`.
   */
  | {
      type: 'tool';
      turn: number;
      id: string;
      kind: string;
      name: string;
      arguments: string;
      result?: string;
      ok?: boolean;
      /** The seq to fetch the picture from, when the answer had one. */
      imageSeq?: number;
    }
  | { type: 'decision'; turn: number; summary: string; narration?: string }
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
  /**
   * When the row's **first** event was published. A row that grew (a streamed reply) or repeated
   * (`×3`) is stamped when it started rather than when it stopped, which is what a reader watching
   * the log scroll past expects a time beside a line to mean.
   */
  at?: number;
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
