// The wire types, mirroring `src/web/published.rs` by hand.

/** All eight are sent in badge-bit order, so the index is the `/api/badges.png` sprite index. */
export interface BadgeView {
  name: string;
  earned: boolean;
}

/** One party slot; the sprite is the separately cached `/api/pokemon/{dex}/front.png`. */
export interface PartyMonView {
  nickname: string;
  dex: number;
  level: number;
  hp: number;
  max_hp: number;
  status: string;
}

export interface GameView {
  /** The name on the save, which can differ from `Status.model` after a restart under another model. */
  trainer: string;
  trainer_id: number;
  map: string;
  position: { x: number; y: number };
  mode: string;
  badges: BadgeView[];
  money: number;
  playtime: string;
  party: PartyMonView[];
  in_battle: boolean;
}

export type RunStatus =
  | { state: 'booting' }
  | { state: 'playing' }
  | { state: 'awaiting_llm'; kind: string }
  | { state: 'streaming' }
  | { state: 'running_tool'; name: string }
  | { state: 'compacting' }
  | { state: 'rate_limited'; retry_in_ms: number }
  /** Parked until `until_ms`: absolute Unix ms, since late joiners replay it; the countdown is derived here. */
  | { state: 'throttled'; until_ms: number; message: string }
  | { state: 'error'; message: string };

export interface Status {
  wall_ms: number;
  /** Not a speed over `wall_ms`: the host drops overrun time. See `EventStream.speed`. */
  emulated_ms: number;
  /** Emulated time over the run's whole life, across processes; the panel's "played" figure. */
  run_emulated_ms: number;
  dropped_ms: number;
  target_speed: number;
  policy: string;
  /** `GB_MODEL`, or `null` under any policy that is not an LLM. */
  model: string | null;
  agent_state: string;
  frame_seq: number;
  game: GameView | null;
  run: RunStatus;
}

export interface TodoView {
  id: number;
  text: string;
  done: boolean;
}

/** The battle script. A failed one keeps its source but is disarmed, so show `armed` and `source` apart. */
export interface BattleScriptView {
  source: string | null;
  armed: boolean;
  /** `battle_script::DEFAULT`, otherwise indistinguishable from a written script never armed. */
  is_default: boolean;
  last_failure: string | null;
}

export interface UsageView {
  context_tokens: number;
  context_limit: number;
  prompt_tokens: number;
  completion_tokens: number;
  completions: number;
  estimated: boolean;
}

/** Publish time in Unix ms; optional because older transcripts replayed by `/api/history` lack it. */
export type At = { at?: number };

/** One `data:` line of `/api/events`; `Status` is flattened into the event, not nested. */
export type UiEvent = At &
  (
  | ({ seq: number; type: 'status' } & Status)
  | { seq: number; type: 'agent'; kind: string; text: string }
  | { seq: number; type: 'notice'; level: string; message: string }
  | { seq: number; type: 'turn_started'; turn: number; kind: string; headline: string }
  | { seq: number; type: 'assistant_delta'; turn: number; text: string }
  | { seq: number; type: 'assistant_reasoning'; turn: number; text: string }
  /** Pairs with its `tool_result` by `id`, never by position: a message's calls are answered as a batch. */
  | {
      seq: number;
      type: 'tool_call';
      turn: number;
      id: string;
      kind: string;
      name: string;
      arguments: string;
    }
  /** `image` is a flag: fetch `/api/tool-image/{seq}/image.png`, which 404s once out of the server's ring. */
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
  /** `narration` is the model's own `summary` argument; null when omitted or on a forced wait. */
  | {
      seq: number;
      type: 'decision';
      turn: number;
      summary: string;
      narration: string | null;
      usage: UsageView | null;
    }
  | { seq: number; type: 'turn_cancelled'; turn: number; reason: string }
  | { seq: number; type: 'run_status'; status: RunStatus }
  | { seq: number; type: 'plan'; items: TodoView[] }
  | ({ seq: number; type: 'battle_script' } & BattleScriptView)
  | {
      seq: number;
      type: 'compacted';
      before: number;
      after: number;
      images_evicted: number;
      summarised: boolean;
    }
  );

/** A log row's content; `assistant` and `reasoning` are a turn's deltas, folded by `useEventStream`. */
export type EntryBody =
  | { type: 'agent'; kind: string; text: string }
  | { type: 'notice'; level: string; message: string }
  | { type: 'turn'; turn: number; kind: string; headline: string }
  | { type: 'assistant'; turn: number; text: string }
  | { type: 'reasoning'; turn: number; text: string }
  /** A tool call and, once it arrives, its result: one row, never collapse-matched (see `signature`). */
  | {
      type: 'tool';
      turn: number;
      id: string;
      kind: string;
      name: string;
      arguments: string;
      result?: string;
      ok?: boolean;
      imageSeq?: number;
    }
  | { type: 'decision'; turn: number; summary: string; narration?: string }
  | { type: 'cancelled'; turn: number; reason: string }
  | { type: 'compacted'; before: number; after: number; images_evicted: number; summarised: boolean };

/** One row of the log; `raw` is the wire event as it arrived, shown when the row is expanded. */
export type Entry = EntryBody & {
  seq: number;
  raw: UiEvent;
  count: number;
  /** When the row's first event was published. */
  at?: number;
};

export type Connection = 'connecting' | 'live' | 'reconnecting';

// How long a live stream may be silent before it is rebuilt, since a dropped network raises no error:
// 4x the server's `KEEP_ALIVE`, which both routes send unconditionally.
export const STALE_MS = 8000;

/** One finished run from `/api/leaderboard`, which arrives already ranked. */
export interface Completion {
  archive: string;
  run_id: string;
  teams: number;
  completed_at: string;
  started_at: string;
  app_version: string;
  policy: string;
  model: string | null;

  /** The cartridge's play clock in seconds, the ranking key. */
  playtime_seconds: number;
  playtime: string;
  playtime_maxed: boolean;
  emulated_ms: number;
  wall_ms: number;

  turns: number;
  completions: number;
  prompt_tokens: number;
  completion_tokens: number;
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
