# LLM Playthrough over the Web — Implementation Plan

An `LlmPolicy` that delegates every decision to an LLM over an OpenAI-compatible API, and a
browser UI — served from the same process — that lets anyone with the URL watch it play.

**Status:** planning. Nothing here is built yet.

---

## 1. What we are building

Today `PokemonAgent` drives Pokémon Red from a `Policy` (`src/pokemon/policy.rs:26`), and the only
interactive implementation is `ConsolePolicy`, which prints a numbered menu to stdout and reads a
choice from a background stdin thread. The SDL UI (`src/sdl/render.rs`) is the only way to see what
is happening.

We want:

1. **`LlmPolicy`** — same `Policy` seam, but the menu goes to an LLM over an OpenAI-compatible chat
   endpoint, and the choice comes back as a tool call.
2. **An escape hatch.** The derived actions are always offered *first*, but the LLM can also press
   raw buttons for one agent tick at a time, which lets non-determinism route around bugs and gaps in
   the agent's state machine.
3. **A web UI**, served by the emulator process itself: the game screen, the conversation streaming
   in, token usage, current status, and a live read-out of location / party / badges.
4. **Persistence** — a run survives a process restart, and a viewer joining mid-run gets the whole
   backlog before the live stream.

Explicitly **out of scope for now**: audio streaming (specced in §12, deferred), TLS (the platform
terminates it), authentication, horizontal scale. One container, one process, one emulator.

### 1.1 Decisions already taken

| Decision | Choice | Consequence |
|---|---|---|
| Server stack | `axum` + `tokio` for the **HTTP server only** | Emulator and LLM worker stay synchronous threads |
| LLM client | `ureq` (blocking, streams via `Read`) | No second runtime; the worker is a plain `std::thread` |
| Video transport | Hand-rolled 8×8 block-diff, base64 in SSE | No image codec, no ffmpeg, ~30–200 kbit/s |
| Audio | **Deferred** | §12 keeps the design; no work in phases W0–W8 |
| LLM protocol | OpenAI native tool calling | Screenshots return as image content in tool results |
| SPA delivery | Embedded at compile time (`rust-embed`) | Node is a build-stage dependency; dev mode serves from disk |
| SDL UI | Kept, behind a **cargo feature** | Server-only builds drop `libsdl2` entirely |
| Persistence | Run directory: save state + transcript + memories | Resume on restart, full backlog on join |
| Viewer controls | **Strictly view-only** | No pause, no takeover, no operator chat. No write endpoints at all |

---

## 2. Architecture

The emulator is `&mut`-single-threaded and lives on a run-loop stack. It stays that way. Everything
async talks to it through channels.

Three units. **Two of them are ordinary synchronous threads**; async exists only to serve HTTP.

```
   ┌──────────────── emulator thread (sync) ────────────────┐
   │  GameBoy · PokemonAgent · LlmPolicy · VideoEncoder      │
   │                                                        │
   │  loop {                                                │
   │      pace();                    // from render.rs      │
   │      let n = gb.run(min_cycles);                       │
   │      agent.update(&mut api, n)?;                       │
   │        └─ per poll: policy.service_tools(state, api…)  │
   │           then    : policy.pick_*(…) -> Option<_>      │
   │      publish_video();           // ≤30 fps             │
   │      publish_status();          // 10 Hz               │
   │      maybe_checkpoint();        // 60 s                │
   │  }                                                     │
   └──┬────────────────────────────────────────┬────────────┘
      │ std::sync::mpsc                        │ tokio::sync::{broadcast, watch}
      │ ToolCall ▲ ▼ ToolResult                │ (send() is sync — no runtime needed)
      │ TurnRequest ▼ ▲ TurnOutcome            │
      ▼                                        ▼
   ┌──── llm worker thread (sync) ────┐   ┌──── shared published state ────┐
   │  ureq ──► OpenAI-compatible API  │   │  RwLock<Arc<FrameSnapshot>>    │
   │  blocking; streams SSE via Read  │   │  broadcast<VideoMsg>           │
   │  memory/todo file I/O directly   │   │  broadcast<UiEvent>            │
   └──────────────────┬───────────────┘   │  watch<Status>                 │
                      └──────────────────►└────────────┬───────────────────┘
                                                       │ read-only
                      ┌────────────── tokio runtime ───▼───────────────────┐
                      │  axum   GET /            embedded SPA              │
                      │         GET /api/video   SSE — keyframe + deltas   │
                      │         GET /api/events  SSE — conversation/status │
                      │         GET /api/history JSON — transcript backlog │
                      │         GET /api/healthz                           │
                      └────────────────────────────────────────────────────┘
```

**The web layer never talks to the emulator.** It only reads published buffers. That is what makes
"strictly view-only" (§1.1) structural rather than a matter of not exposing a POST route.

### 2.1 Tools are serviced at the policy poll

An earlier draft had the worker send queries *back* into the emulator over a channel. That was
unnecessary. At the poll the policy already holds `&GameState` — party, bag, badges, tilemap,
sprites, the action list, battle state — plus `&WorldGraph`. Almost the entire read surface is
already in hand, and the rest is already published for the UI:

| Tool group | Serviced from | Emulator round trip? |
|---|---|---|
| party / bag / trainer / map / battle / world graph | `&GameState`, `&WorldGraph` at the poll | none — already there |
| `read_screen_text` | `api.on_screen_text(false)` at the poll | none — `&mut PokemonApi` is in scope |
| `screenshot` | `RwLock<Arc<FrameSnapshot>>`, already published for video | none |
| memory / todo | files, read and written by the worker thread | none — never touches the emulator |

So the worker emits tool calls, pushes them onto an `mpsc` and **blocks on the result channel**. The
next poll drains the queue, services every call against one consistent `GameState`, and sends the
results back. Worst-case latency is one agent tick — 20 ms of emulated time (W0.3b).

The blocking is the point: the worker thread is *supposed* to wait. It is one thread doing one
request at a time, and it has nothing else to do.

⚠️ **The emulator keeps running while the LLM thinks.** In the overworld the player stands still and
in battle the game waits at the menu, so this is usually harmless — but a `press_buttons` decision
made against a screenshot taken 4 seconds ago can land on a different screen. Config flag
`GB_PAUSE_WHILE_THINKING` (default `false`) skips `gb.run` while a turn is in flight, at the cost of
a frozen livestream. Default off: a frozen screen with a "Thinking" pill is a worse watch than a
slightly stale one.

### 2.2 Threading contract

| Channel | Type | Direction |
|---|---|---|
| `TurnRequest` | `std::sync::mpsc::Sender` | policy (emulator thread) → worker |
| `TurnOutcome` | `std::sync::mpsc::Receiver`, polled with `try_recv()` | worker → policy |
| `ToolCall` | `std::sync::mpsc::Sender` | worker → policy |
| `ToolResult` | `std::sync::mpsc::Receiver`, **blocking `recv()`** | policy → worker |
| `UiEvent` | `tokio::sync::broadcast::Sender` (capacity 1024) | emulator + worker → SSE clients |
| `VideoMsg` | `tokio::sync::broadcast::Sender` (capacity 64) | emulator → SSE clients |
| `Status` | `tokio::sync::watch::Sender` | emulator + worker → SSE clients |
| Latest frame | `RwLock<Arc<FrameSnapshot>>` | emulator → worker (screenshots) + late joiners |

The only tokio types are the three broadcast/watch senders, and they are used purely as the
sync→async bridge: **`broadcast::Sender::send` and `watch::Sender::send` are synchronous and
non-blocking**, callable from a plain thread with no runtime handle and no `block_on`. Everything
between the emulator, the policy and the worker is `std::sync::mpsc`.

`broadcast` drops the oldest message for a slow client rather than blocking the producer — correct
for video, and for `UiEvent` the client recovers via `/api/history`.

---

## 3. Phase W0 — Seams in the core

**No web, no LLM.** Everything here is a small, independently mergeable change to existing code that
the later phases depend on.

### W0.1 — SDL behind a cargo feature

```toml
[features]
default = ["sdl"]
sdl  = ["dep:sdl2", "dep:fontdue"]
web  = ["dep:tokio", "dep:axum", "dep:tower-http", "dep:rust-embed",
        "dep:serde", "dep:serde_json", "dep:base64"]
llm  = ["web", "dep:ureq"]
```

`tokio` is needed only for `axum` and for the `broadcast`/`watch` bridge types (§2.2) — the emulator
and the LLM worker are plain `std::thread`s and neither is ever inside a runtime. `ureq` is a
blocking client with no async runtime of its own; `reqwest::blocking` would spin up a second hidden
tokio runtime for no benefit.

`mod sdl;` in `src/main.rs` becomes `#[cfg(feature = "sdl")]`.

Only `sdl2` and `fontdue` are SDL-only (`src/sdl/font.rs`, `src/sdl/render.rs`) — **verified
2026-08-07**. `itertools` is *not*: it is used by `src/ppu.rs`, `src/pokemon/mod.rs` and
`src/pokemon/map_header.rs`, so it stays unconditional. An earlier survey claimed otherwise.

**Acceptance:** `cargo build --release` unchanged; `cargo build --release --no-default-features`
succeeds and links no SDL2.

### W0.2 — CLI dispatch

`main.rs` grows a hand-rolled arg parse over `std::env::args` — no `clap`.

```
gb                          → SDL UI, ConsolePolicy               (today's behaviour, unchanged)
gb serve [--port 8080]      → web UI + LlmPolicy
gb serve --policy random    → web UI, RandomPolicy (no API key needed; the video-pipeline harness)
```

With `--no-default-features --features llm`, bare `gb` prints usage and exits non-zero.

### W0.3 — `Policy::on_event`

The user-visible requirement: *text-reading events must reach the policy.*

```rust
// src/pokemon/policy.rs — added to `trait Policy`
/// Called for every event the agent emits, before it is buffered.
/// Default: ignore. `DeterministicPolicy` and `RandomPolicy` do not override it.
fn on_event(&mut self, _event: &AgentEvent) {}
```

`PokemonAgent::event` (`src/pokemon/agent.rs:433`) calls `self.policy.on_event(&event)` before
`push_back`. This delivers `TextBox { message }`, `OverworldActionAborted { reason }`,
`BattleStarted/Ended` and the rest to `LlmPolicy`, which turns them into conversation entries — the
abort reasons in particular are exactly the feedback the LLM needs to stop retrying a blocked route.

⚠️ Some sites push into a local `new_events: Vec<AgentEvent>` (`agent.rs:870`) and drain later. Route
those through `event()` too, or `on_event` silently misses them.

### W0.3b — Poll rate: considered, deliberately not changed

Worth recording, because it looks like a problem and isn't.

`DelayContext::tick` returns `true` **forever** once exhausted (`delay.rs:98-104`,
`delay_keeps_firing_after_completion`), and nothing re-arms it when a policy returns `None`. So
`AwaitingOverworldAction` (`agent.rs:918`) waits out its initial 500 ms/1000 ms and then polls **every
agent tick — 50 times per emulated second — until the policy answers**. Battle is the same shape
(`agent.rs:1334`). A 10-second LLM turn therefore triggers ~500 polls.

The policy call is nanoseconds (a `try_recv` on an empty channel, exactly as `ConsolePolicy` has
always done). The only real cost is that `observe_state(api)` — a full `PokemonApi::game_state()`:
party, bag, tilemap, sprites, dex, flags — runs *before* it on each of those ticks. At ~10–100 µs a
read that is 0.05–0.5% of one core, or ~10–25 ms of CPU across a 10-second turn. Under `gb serve`
the emulator targets 1× realtime, where the core has 50–90× headroom. It does not matter.

A `Policy::repoll_delay` hook to throttle it was designed and dropped. Three reasons:

1. **It buys nothing measurable** — see above, and `ConsolePolicy` has run this way for the life of
   the project against a decider far slower than any LLM.
2. **It would make tool servicing worse.** Tool batches are answered at the poll (§2.1), so a 200 ms
   backoff would add ~1 s per turn to a turn making 4–5 read calls. Polling every tick answers them
   within one agent tick.
3. **It is the only W0 change that would touch frame timing**, which per `CLAUDE.md` re-rolls the RNG
   stream and puts fixtures and `full_playthrough` at risk. Not doing it takes that risk to zero.

If profiling under a real run ever contradicts this, the hook is a ten-line change — but it needs a
measurement first, not an intuition.

### W0.4 — Manual input queue on the agent

The escape hatch. `OverworldAction` cannot express "press B once": the agent re-derives the action
mid-walk and matches it by `MetaTile` (`agent.rs:1140-1154`), so a synthetic action fails with
`NoRoute`. And the requirement is broader than the overworld — the LLM must be able to press buttons
while the agent believes it is mid-script, which is precisely when the policy is never consulted.

```rust
// src/pokemon/agent.rs
impl PokemonAgent {
    /// Queue raw button presses, one per agent tick, pre-empting the state machine.
    /// Clears the current state so the next tick re-derives actions from scratch.
    pub fn queue_manual_input(&mut self, buttons: impl IntoIterator<Item = JoypadButton>);
}
```

Handled at the top of `update()`, immediately after the `AGENT_RESOLUTION` gate and before
`drive_post_champion_cutscene`. Each tick: release everything, press the head of the queue, pop. On
an empty queue, `self.state = AgentState::Idle` once, then fall through to the normal machine.

- The queue is capped at **16** buttons so a confused model cannot run away with the game.
- `queue_manual_input` sets `state = Idle` on entry — otherwise a queued press mid-`OverworldMovement`
  corrupts the route and the walk resumes into a wall.

### W0.5 — The observation facade

One module, `src/pokemon/observe.rs`, holding every read the LLM tools need, each a free function
taking `&mut PokemonApi` (plus `&WorldGraph` where relevant) and returning a serialisable struct.
This is the *only* place tool reads live, so the tool layer in W5 is a thin dispatch.

| Function | Backed by |
|---|---|
| `trainer(...)` | `GameState` — name, rival, badges, money, playtime, dex owned/seen |
| `party(...)` | `state.pokemon` — species, nickname, level, HP, status, 4 moves + PP |
| `bag(...)` | `state.bag` + `money` |
| `map_view(...)` | `format!("{}", state.map)` + legend + sprites + warp targets + `actions()` |
| `screen_text(...)` | `api.on_screen_text(false)` |
| `world_graph(...)` | `WorldGraph::nodes` / `neighbors` — known maps only |
| `battle(...)` | `state.battle` + `policy::battle_options(state)` |
| `status(...)` | the cheap 10 Hz UI subset: map, position, badges, party HP, money, mode |

`MetaTileMap` already has `impl Display` (`tile_map.rs:984`) rendering an ASCII grid — `_` empty,
`O` obstacle, `X` water, `W` warp, `g` grass, `S` sprite, `P` player, etc. It drops identity (*which*
sprite, *which* warp), so `map_view` pairs the grid with the `actions()` list and the sprite table.

Every function here is pure over `(&GameState, &mut PokemonApi, &WorldGraph)` — the exact triple a
policy holds at a poll — which is what lets W0.5b service tool calls without any round trip.

### W0.5b — `Policy::service_tools`

```rust
// Policy trait, defaulted
/// Called at the top of every policy poll, before any `pick_*`.
/// Default: no-op. No existing policy is affected.
fn service_tools(&mut self, _state: &GameState, _api: &mut PokemonApi, _graph: &WorldGraph) {}
```

Five call sites in `agent.rs`, each immediately after the state for that decision point is observed
(`AwaitingOverworldAction`, battle `AwaitingPolicy`, `NamingPokemon`, the mart flow, the global
forget-move handler). `LlmPolicy` drains its pending tool-call queue here and replies; every other
policy ignores it.

⚠️ The observed state is computed **once** per poll and every queued tool call is answered from it,
so a turn never sees a torn view — `read_party` and `read_map` in the same assistant message are
guaranteed consistent with each other.

**Acceptance for W0:** `cargo test --release` green; **`cargo test --release --features
full-playthrough full_playthrough` green** — W0.3 and W0.4 both touch `agent.rs`, and per `CLAUDE.md`
the leg tier is not a substitute for the full run.

---

## 4. Phase W1 — Headless host + server skeleton

### W1.1 — `src/host.rs`

Lift the pacing loop out of `render.rs` into an `EmulatorHost` that owns `GameBoy`, `PokemonAgent`
and `MapMetadataCache` and runs on its own thread. The pacing algorithm transplants unchanged:
accumulate wall-clock into `since_last_update`, drain it in `cycle_duration` steps, credit
`ahead_by_cycles` for `gb.run`'s instruction-boundary overshoot (`render.rs:231-247`).

`render.rs` is **not** refactored to use it. It keeps its own loop — it has F1–F12 debug affordances
tangled into the same event pump, and untangling them buys nothing. The duplication is ~30 lines and
the SDL path stays exactly as it is today.

### W1.2 — Server skeleton

`src/web/mod.rs` with axum on `0.0.0.0:$GB_PORT` (default 8080), a placeholder index page,
`/api/healthz`, and `/api/events` SSE emitting the 10 Hz status snapshot. `gb serve --policy random`
plays the game and streams status; nothing is rendered yet.

**Acceptance:** `gb serve --policy random`, then `curl -N localhost:8080/api/events` shows status
frames ticking with a changing map/position.

---

## 5. Phase W2 — Video pipeline

### 5.1 Wire format

160×144, 8×8 blocks → 20×18 = **360 blocks**. The server keeps `last_sent: [LcdColor; 23040]` and a
persistent palette (`LcdColor → u8`, ≤256 entries) that carries across messages and resets on a
keyframe.

```
u8   version (=1)
u8   flags            bit0 = keyframe
u16  frame_seq        wrapping
u8   new_palette_len
[new_palette_len × u24 RGB]          entries appended to the persistent palette
u16  block_count
[block_count × {
    u16 block_index                  0..360, row-major
    u8  mode                         0 = RLE palette, 1 = raw palette (64 bytes)
    payload
}]
```

- **Mode 0 (RLE):** `[u8 run_len (1..=64)][u8 palette_index]` pairs until 64 pixels are covered.
  Game Boy 8×8 blocks are extremely run-friendly.
- **Mode 1 (raw):** 64 palette indices. The encoder picks whichever is smaller per block.
- **Palette exhaustion:** if a frame would push past 256 entries, the encoder emits a keyframe with a
  fresh palette instead. Pokémon Red in CGB-compat mode uses a few dozen colours on screen; this is
  a safety valve, not a normal path.
- **RGB888, not RGB565.** DMG's greys are `FF/AA/55/00`; `0xAA` does not survive a round trip through
  5-bit. The palette is small enough that the extra byte per entry is free.

Base64 the whole thing into one SSE `data:` line. Cost estimate: a static screen sends nothing
(heartbeat comment every 2 s to keep proxies from closing the connection); a walking animation
touches ~40–80 blocks at ~15 bytes RLE each ≈ 1 KB → 1.4 KB base64 → **well under 300 kbit/s at
20 fps**, and typically a small fraction of that.

### 5.2 Cadence and late joiners

The encoder is called from the emulator loop on a **wall-clock** timer, at most 30 fps, independent
of emulated frame rate (so fast-forward does not multiply bandwidth). Keyframe every 5 s or when the
palette resets.

Late joiners must not miss a delta between reading the current frame and subscribing:

1. `broadcast::subscribe()` **first**.
2. Read `RwLock<Arc<FrameSnapshot>>`, encode a keyframe from it, note its `frame_seq`.
3. Send the keyframe, then forward buffered deltas, **discarding any with `seq <= keyframe_seq`**.

### 5.3 Decoder

TypeScript decoder mirroring the encoder, writing into an `ImageData` backing a 160×144 `<canvas>`,
scaled up with CSS and `image-rendering: pixelated`. Only changed blocks are written; `putImageData`
once per message.

**Testing.** A Rust reference decoder in `src/web/video/tests.rs` round-trips a recorded sequence of
real frames (captured from a fixture playthrough) and asserts pixel-exact reconstruction after every
message, including across keyframes and a forced palette reset. The TS decoder is a direct port and
is checked by eye; the Rust test is the regression net.

**Acceptance:** a throwaway `web/dev/video.html` (plain canvas + `EventSource`, no build step) shows
the game playing under `--policy random`. This page is discarded in W3.

---

## 6. Phase W3 — The SPA

`web/` — Vite + React + TypeScript. Dependencies: `react`, `react-dom`, `vite`,
`@vitejs/plugin-react`, `typescript`. Nothing else — no UI kit, no state library, no router.

```
┌──────────────────────────────┬────────────────────────────────────┐
│  ┌────────────────────────┐  │  gpt-4.1 · 84,201 tok (42% ctx)    │
│  │                        │  │  ● Thinking…                       │
│  │      160×144 canvas    │  │ ─────────────────────────────────  │
│  │       CSS-scaled 4×    │  │  [assistant] I'm in Viridian Forest│
│  │                        │  │  ▸ read_map()                      │
│  └────────────────────────┘  │  [assistant] Heading for the exit  │
│                              │  ▸ choose_action("Warp → Route 2") │
│  Viridian Forest (12, 9)     │  ▸ screenshot()   [thumbnail]      │
│  ⬢⬢⬡⬡⬡⬡⬡⬡  ¥3,200  4:12:07  │  [agent] OverworldActionCompleted  │
│  ─────────────────────────── │  …                                 │
│  PIKACHU   L14  ███████░ 41  │                                    │
│  BULBASAUR L16  ██████████   │  ← streams, auto-scrolls,          │
│                              │    pinned to bottom unless         │
│                              │    the user scrolls up             │
└──────────────────────────────┴────────────────────────────────────┘
```

Three components and a hook: `<Screen>`, `<StatusPanel>`, `<Conversation>`, `useEventStream()`. Two
`EventSource` connections (`/api/video`, `/api/events`), both with reconnect-on-error.

Embedded with `rust-embed` over `web/dist`. `GB_WEB_DEV=1` serves from disk via `tower-http`'s
`ServeDir` instead, so `npm run dev` on :5173 with a proxy to :8080 gives hot reload.

⚠️ `rust-embed` at compile time means **`web/dist` must exist before `cargo build --features web`**.
Ship a committed placeholder `web/dist/.gitkeep` + an `index.html` stub so a clean checkout builds,
and document the real order (`npm ci && npm run build`, then `cargo build`) in the Dockerfile and
the README section this plan adds.

---

## 7. Phase W4 — LLM client and the turn lifecycle

### 7.1 Client — `src/llm/client.rs`

A minimal OpenAI-compatible `POST /chat/completions` client over `ureq`, with `stream: true`.
`ureq`'s `Response::into_reader()` hands back a plain `impl Read`, so SSE parsing is a
`BufReader::lines()` loop — no async, no runtime. Frames are parsed into:

- assistant content deltas → forwarded to `UiEvent::AssistantDelta` so the UI streams live;
- `tool_calls` deltas, accumulated by index (id / name arrive first, arguments arrive in fragments —
  **the arguments of one call arrive across many chunks and must be concatenated before parsing**);
- the final `usage` object.

⚠️ Not every OpenAI-compatible endpoint returns `usage` on a streamed response; several require
`stream_options: {"include_usage": true}`, and some return neither. Send the flag, and when usage is
absent fall back to a local estimate (chars / 3.7 for text, a fixed per-image cost) so the token
gauge degrades rather than freezes. Flag the estimate in the UI.

**Parallel tool calls need no special tool.** A single assistant message may carry an *array* of
`tool_calls` — `read_party` and `read_map` together, answered in one round trip. That is already how
the design works: §2.1 services a whole `ToolBatch` at one poll, from one observed `GameState`, so
batching is both the efficient path and the only path that guarantees the reads agree with each
other. Send `parallel_tool_calls: true` explicitly (it is the default, but be explicit), and prompt
for it — "request every read you need in one message".

⚠️ Some endpoints ignore the flag and emit one call per message anyway. Do not add a meta-tool to
work around it; the mitigation below removes most of the need. If a specific endpoint turns out to be
badly affected, the narrow fix is a single `read_state { fields: [...] }` tool returning several
sections at once, which is one call by construction.

**The larger win is not batching — it is not needing the calls at all.** The turn request already
carries a state summary (§7.3); make it rich enough that a typical turn needs *zero* read tools:
location, position, party with HP/PP, badges, money, the action menu, and current on-screen text.
Reserve tools for what does not fit or is rarely wanted — `screenshot`, `read_world_graph`, the full
bag, memory bodies. A turn that opens with what it needs beats a turn that fetches it efficiently.

Config, all environment variables, never exposed to the browser:

| Var | Default | Meaning |
|---|---|---|
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Any compatible endpoint |
| `OPENAI_API_KEY` | — | Required for `--policy llm` |
| `GB_MODEL` | — | Required |
| `GB_CONTEXT_LIMIT` | `128000` | Drives the compaction trigger |
| `GB_TEMPERATURE` | `1.0` | |
| `GB_MAX_TOOL_STEPS` | `12` | Non-terminal tool calls per turn before we force a decision |
| `GB_PORT` | `8080` | |
| `GB_RUN_DIR` | `./runs` | |
| `GB_PAUSE_WHILE_THINKING` | `false` | §2.1 |

Retry with exponential backoff on 429/5xx, surfacing `Status::RateLimited { retry_in }` to the UI.

### 7.2 `LlmPolicy` — `src/pokemon/llm_policy.rs`

Modelled on `ConsolePolicy` (`policy.rs:108-249`), which is the reference for a non-blocking
asynchronous policy: kick off on the first call, return `None` every tick, `try_recv` until the
answer lands.

```rust
pub enum DecisionKind { Overworld, Battle, Nickname, MartPurchase, ForgetMove }

pub struct LlmPolicy {
    turns:      mpsc::Sender<TurnRequest>,      // std::sync::mpsc throughout
    outcomes:   mpsc::Receiver<TurnOutcome>,
    tool_calls: mpsc::Receiver<ToolBatch>,      // drained in service_tools
    tool_out:   mpsc::Sender<ToolBatchResult>,
    pending:    Option<DecisionKind>,           // the kind the in-flight turn is answering
    field_move: Option<FieldMove>,   // stashed; consumed by the next pick_field_move
    manual:     Vec<JoypadButton>,   // drained by the host into queue_manual_input
    events:     Vec<AgentEvent>,     // from on_event, folded into the next turn's prompt
    waiting:    u16,                 // ticks remaining on a `wait` decision
}
```

**A turn is keyed by the decision kind it is answering, and only the matching poll may advance it.**

`service_tools` runs first (W0.5b): drain `tool_calls`, and

- `pending == Some(k)` where `k` is the kind about to be asked → service the batch against the live
  `GameState` / `PokemonApi` / `WorldGraph`, send `ToolBatchResult::Ok(results)`.
- otherwise → send `ToolBatchResult::Cancelled`. The tool is never executed.

Then the `pick_*` for kind `k`:

1. `pending == Some(k)` → `outcomes.try_recv()`. Empty → `None`. Outcome → resolve (§7.4).
2. `pending == Some(other)` → bump the turn generation (which cancels, §7.3), start a fresh turn for
   `k`, return `None`.
3. `pending == None` → build a `TurnRequest` (kind + rendered menu + compact state summary + any
   `AgentEvent`s since the last turn), `send`, set `pending = Some(k)`, return `None`.

This is what makes the interleaving safe. The agent asks for an overworld action, the model spends
eight seconds on it, and meanwhile a trainer spots the player: the very next poll is
`pick_battle_action`, the kind no longer matches, the stale turn dies and a battle turn starts. A
battle decision can never be applied to an overworld state, and no tokens are spent finishing a
completion that is already answering a dead question.

⚠️ **Guard against re-issuing.** `agent.update` polls the policy up to 50× per emulated second. The
`pending` field is the guard — without it, one decision point spawns fifty LLM turns. `ConsolePolicy`
has exactly this guard in its `*_menu_shown` flags (`policy.rs:113-117`). Both `service_tools` and
the `pick_*` path must be cheap no-ops when there is nothing to do, because both run at that rate.

⚠️ **`pick_field_move` shares the `Overworld` kind — it must never be a kind of its own.** It is
called on *every* idle overworld tick, immediately before `pick_overworld_action` (`agent.rs:944`).
Given its own kind, the two would cancel each other 50 times a second and no turn would ever
complete. It returns `self.field_move.take()` and nothing else; a field move is one possible
*outcome* of an overworld turn (§7.4), never a turn of its own.

⚠️ **`ForgetMove` legitimately pre-empts `Battle`.** The level-up forget prompt fires mid-battle via
the agent's global handler (`agent.rs:846-858`). Cancelling the battle turn to answer it is correct —
the prompt is the live question — and a fresh battle turn starts afterwards.

### 7.3 The turn loop (LLM worker)

A plain blocking loop on its own `std::thread`:

```
recv TurnRequest (blocking)
  ├─ append a user message: the situation + the menu + agent events since last turn
  ├─ loop up to GB_MAX_TOOL_STEPS:
  │     ├─ stream a completion  →  UiEvent::AssistantDelta…      [cancel point]
  │     ├─ no tool calls?  →  nudge once ("you must call a tool"), then force `wait`
  │     ├─ non-terminal calls → send ToolBatch, block on recv    [cancel point]
  │     │     ├─ Ok(results)   → append tool result messages, continue
  │     │     └─ Cancelled     → roll back one step, abandon the turn
  │     └─ terminal tool call  →  break
  ├─ if the step budget is exhausted without a terminal call → force `wait { ticks: 1 }`
  ├─ compaction check (§9)
  └─ send TurnOutcome
```

**Cancellation is a generation counter, checked at two points.** A shared
`Arc<AtomicU64>` holds the current turn id; the policy bumps it when the decision kind changes. The
worker compares it against its own id

- on every SSE line while streaming — bail out and drop the reader, which aborts the HTTP request;
- on `ToolBatchResult::Cancelled`, which is the same signal arriving through the tool channel.

No `select!`, no `CancellationToken`, no async. Those are the only two places a turn can be sitting,
so they are the only two places that need to look.

**Rollback is one step.** On cancellation the worker **drops the last assistant message** — the one
carrying the tool calls that were never serviced — and abandons the turn. The history is then
well-formed *by construction*: every remaining `tool_call` already has its matching result, so
nothing needs synthesising and the next request cannot 400.

⚠️ **Service a tool batch all-or-nothing.** One assistant message may carry several calls; they are
all answered from one observed state at one poll, so a partial batch cannot happen — and a partial
batch is exactly what would leave an orphaned `tool_call` behind. The all-or-nothing rule is what
makes single-step rollback sufficient. Unit test in §15.

The replacement turn then opens with the new situation ("a trainer battle started; here is the battle
menu"). What is lost is one unanswered reasoning step and the tokens spent on it — see §17 risk 2b.

### 7.4 Resolving a decision

| Terminal tool | Resolves to |
|---|---|
| `choose_action { id }` | An `OverworldAction` from a **freshly recomputed** `state.map.actions()` |
| `use_field_move { … }` | Stashed in `field_move`; returns `None` this tick, picked up next tick |
| `press_buttons { buttons }` | Pushed to `manual`; the host calls `queue_manual_input`; returns `None` |
| `wait { ticks }` | `waiting = ticks`; returns `None` for that many ticks |
| `choose_battle_action { id }` | A `BattleAction` from a fresh `battle_options(state)` |
| `set_nickname { name? }` | `Some(name)` / `Some(None)` |
| `buy_item { item?, quantity? }` | `Some(Some(BagItem))` / `Some(None)` |
| `forget_move { slot? }` | `Some(Some(slot))` / `Some(None)` |

⚠️ **`id` is never a list index.** `actions()` is `sort()`ed by `MetaTile` and can reorder between the
tick that rendered the menu and the tick the answer lands — `ConsolePolicy` learned this and matches
by `MetaTile` instead (`policy.rs:113-117, 164-172`). We go further and use a stable composite key,
`"{map}:{dest.x},{dest.y}:{tile_kind}"`, recomputed against a fresh `actions()` at resolution time.
An `id` with no match becomes a tool error fed straight back to the model — "that action is no longer
available, here is the current list" — rather than a panic or a silent no-op.

### 7.5 The turn contract

**Every turn ends with exactly one terminal tool call. Nothing else ends a turn** — not prose, not a
read tool, not silence. This is the single invariant the whole loop rests on, and a model that drifts
from it stalls the run, so it is enforced in four independent places rather than trusted to a prompt.

**1. Scope the `tools` array per decision kind.** The read tools are always present; the terminal
tools are only ever the ones valid for the kind being asked (§7.4). A battle turn is not *sent*
`choose_action`; an overworld turn is not sent `choose_battle_action`. The model cannot end a turn
the wrong way because the wrong way is not offered.

**2. Restate the contract in every turn request**, not only after a compaction. It is two lines of
tokens and it makes the rule the most recent instruction in the context every single time:

```
End this turn by calling exactly one of: choose_action, use_field_move, press_buttons, wait.
Read tools (read_map, read_party, …) do not end the turn — call as many as you need, in one
message, then finish with a terminal call.
```

**3. The system prompt carries it too**, and the system prompt is never compacted (§9). This is the
copy that survives everything.

**4. The compaction summary ends with a restatement** (§9), so the message immediately following a
compaction re-establishes it even if the summariser drops everything else.

**Fallback when it still happens.** A completion with no tool calls gets one nudge quoting the rule
verbatim; a second failure forces `wait { ticks: 1 }` and emits a `UiEvent` marking it, so a model
that cannot hold the contract shows up as a visible rate rather than a mysteriously idle game. Same
for exhausting `GB_MAX_TOOL_STEPS` (§7.3).

**Acceptance for W4:** the LLM plays from a committed fixture and the conversation streams into the
browser. Overworld and battle decisions only.

---

## 8. Phase W5 — The full tool surface

### Terminal tools
`choose_action`, `use_field_move`, `press_buttons`, `wait`, `choose_battle_action`, `set_nickname`,
`buy_item`, `forget_move` — as §7.4.

### Read tools (non-terminal, callable any number of times within a turn)

| Tool | Returns |
|---|---|
| `screenshot()` | The latest published `FrameSnapshot` (§2.1) PNG-encoded via the `image` crate, base64 in an `image_url` content part, plus a one-line caption. Encoded on the **worker** thread, not the emulator's |
| `read_screen_text()` | `api.on_screen_text(false)` — the decoded on-screen text |
| `read_map()` | ASCII grid + legend + sprite table + warp targets + the current action list |
| `read_party()` | Per slot: species, nickname, level, HP, status, 4 moves with PP |
| `read_bag()` | Items + quantities + money |
| `read_trainer()` | Badges, dex owned/seen, playtime, key flags (`can_use_cut`, `can_use_surf`, …) |
| `read_world_graph()` | Maps visited so far and how they connect — the route-planning surface |
| `memory_list()` `memory_read(name)` `memory_write(name, content)` `memory_delete(name)` | §10 |
| `todo_read()` `todo_write(items)` | §10 |

⚠️ **`PokemonTextReader::update` mashes A as a side effect** (`text.rs:28`). `read_screen_text` must
call `api.on_screen_text(false)` directly — going through the reader would advance dialogue the LLM
only wanted to look at.

⚠️ **Screenshots are the dominant token cost.** A 160×144 PNG is small, but every vision model bills
it as a few hundred to ~1000 tokens, and they accumulate in history. §9 evicts old images first.

---

## 9. Phase W6 — Tokens, status, compaction

### Token accounting
Track `prompt_tokens` / `completion_tokens` / cumulative totals from each response's `usage`, with
the §7.1 estimator as a fallback. The UI shows current context occupancy as a percentage of
`GB_CONTEXT_LIMIT` and a running total for the run.

### Status
Broadcast on every transition:

```rust
enum Status {
    Booting,
    Playing,                              // agent driving, no decision pending
    AwaitingLlm { kind: DecisionKind },   // request sent, nothing back yet
    Streaming,                            // tokens arriving
    RunningTool { name: String },
    Compacting,
    RateLimited { retry_in_ms: u64 },
    Error { message: String },
}
```

### Compaction

Two stages, cheapest first, both triggered when occupancy crosses **70%** of `GB_CONTEXT_LIMIT`:

1. **Image eviction.** Replace all but the two most recent screenshot content parts with the text
   `[screenshot removed to save context]`. Often enough on its own.
2. **Summarising compaction.** If still over threshold: one extra completion asking the model to
   write a "story so far" — where it is, what it has done, what it is trying to do next, what it has
   learned about the world. Replace everything except the system prompt and the last **8** messages
   with that summary as a single user message.

The system prompt is never compacted, and neither are the memory index or the TODO list — they are
re-rendered into the system prompt every turn (§10), so they survive compaction by construction.
That is the point of having them.

⚠️ **The summary message must end with a restatement of the turn contract** (§7.5). Compaction is
exactly where a long-running behavioural rule gets quietly dropped, and the failure mode — a model
that replies with prose and never calls a terminal tool — stalls the run rather than erroring. The
summarisation prompt appends it as fixed text rather than asking the model to carry it over.

Emit `Status::Compacting` and a `UiEvent::Compacted { before, after }` so a viewer sees it happen.

---

## 10. Phase W6b — Memory and TODO

Files on disk, in the run directory, so they survive both compaction and process restart.

```
$GB_RUN_DIR/<run-id>/
    memories/<slug>.md      # one note per file: frontmatter title + freeform body
    todo.json               # [{ id, text, done }]
```

`memory_write` slugifies the name, caps the body (8 KB) and the count (64 files). The **index**
(names + first line of each) and the **entire TODO list** are re-rendered into the system prompt every
turn, so the model always knows what it knows without spending a tool call; `memory_read` fetches a
full body on demand.

This is the mechanism by which a run keeps long-horizon intent across compactions: "beat Brock" is a
TODO item, not something in the last 8 messages.

---

## 11. Phase W7 — Persistence and resume

```
$GB_RUN_DIR/<run-id>/
    meta.json           # run id, model, started-at, last-checkpoint-at
    state.gbst          # GameBoy::save_state()
    sram.bin            # dump_sram()
    transcript.jsonl    # one JSON object per UiEvent, append-only
    memories/  todo.json
```

- **Checkpoint** every 60 s and on clean shutdown (SIGTERM handler): save state + SRAM + `meta.json`.
  The transcript is appended continuously, not checkpointed.
- **Resume** on startup: newest run directory wins unless `--new-run`. `GameBoy::load_state` is
  transactional (applies to a clone, `game_boy.rs:115`), so a corrupt checkpoint fails cleanly and
  we fall back to a fresh run rather than a half-loaded one.
- **Backlog:** `GET /api/history?since=<seq>` streams `transcript.jsonl` from a sequence number. The
  SPA calls it on mount, renders it, then attaches to `/api/events` — same subscribe-then-backfill
  ordering as the video path (§5.2), so nothing is lost or duplicated at the seam.
- Transcript rotation at 256 MB; the LLM's own message history is capped by compaction, not by this.

⚠️ **`Audio::set_output_sample_rate` is not serialised** and must be re-applied after every
`load_state` (`render.rs:154-159`). Irrelevant while audio is deferred, but the resume path is where
it will bite when §12 lands — leave a comment there now.

---

## 12. Deferred — audio

Kept for later, not built. The intended design, so the seam is not designed against:

`Audio::read_samples_f32` (`src/audio/mod.rs:107`) is the only consumer entry point and the SDL loop
is its only caller — detaching it is ~15 lines. `BlipStereo::read_interleaved_i16`
(`blip/mod.rs:229`) already exists and is unused. Plan: set the output rate to 24 kHz, drain i16
stereo on the emulator thread, ship raw PCM over a WebSocket into an `AudioWorklet` with a ~200 ms
jitter buffer — 768 kbit/s, no encoder. If that proves too fat, IMA-ADPCM is ~60 lines of Rust and
~40 of JS for 192 kbit/s.

⚠️ Under fast-forward, `set_emulation_speed` must track `cycle_duration` or the queue backs up
(`render.rs:224-229`). And `GB_PAUSE_WHILE_THINKING=true` would produce audible gaps — the two
features are mutually exclusive in practice.

---

## 13. Phase W8 — Container

```dockerfile
# stage 1 — SPA
FROM node:22-alpine AS web
WORKDIR /web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build                      # → /web/dist

# stage 2 — Rust, no SDL2 system library needed
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --no-default-features --features llm

# stage 3
FROM debian:bookworm-slim
COPY --from=build /src/target/release/gb /usr/local/bin/gb
VOLUME /runs
ENV GB_RUN_DIR=/runs GB_PORT=8080
EXPOSE 8080
CMD ["gb", "serve"]
```

The pokered submodule must be initialised and built before stage 2 — `pokered/pokered.gbc` is
`include_bytes!`'d at compile time (`src/pokemon/roms.rs`). Either commit the built ROM into the
build context or add a submodule build stage.

**Acceptance:** the image builds, `docker run -e OPENAI_API_KEY=… -e GB_MODEL=… -p 8080:8080` serves
a live playthrough, and the run survives `docker restart`.

---

## 14. Phase W9 — Stuck-run watchdog (last resort)

**Deliberately the last thing built, and deliberately lenient.** This is insurance for a multi-hour
run, not a mechanism the design leans on.

Two failure modes get confused here, and only one of them needs a watchdog:

| Symptom | Covered by | Watchdog? |
|---|---|---|
| The action the model wants isn't in the offered list — an uncovered gameplay mechanic | `press_buttons`, already on every normal turn (W5) | **No.** The agent is polling normally |
| Nothing asks the model anything at all — a jam in `RunningScript`, or `OverworldMovement` walking into a sprite forever | nothing today | **Yes.** This is the only case |

The second is narrow: it can only happen in agent states that never consult the policy. The
navigation bugs that used to cause it are believed fixed, so in a healthy run this never fires.

**Design.** The host tracks emulated time since the last decision point of any kind. After
`GB_STUCK_TIMEOUT_SECS` (default **300** — five emulated minutes; normal play never approaches it) it
raises a `DecisionKind::Stuck` turn: no action menu, the full read-tool set, and only `press_buttons`
and `wait` as terminal tools. `queue_manual_input` (W0.4) is the execution path, so nothing new is
needed on the agent side.

⚠️ **Every firing is a bug report.** Log it prominently to the transcript, the UI and stdout with the
agent's `state_debug()` at the moment it tripped. A watchdog that quietly rescues runs turns an
agent bug into an invisible ongoing cost; the point is to un-stick the run *and* leave evidence.

---

## 15. Testing

The repo's convention is that `#[ignore]` means *blocked*, and everything else goes behind a cargo
feature. New tests follow it.

| Test | Tier | Asserts |
|---|---|---|
| `video::tests::roundtrip_recorded_frames` | default | Pixel-exact reconstruction across deltas, keyframes and a forced palette reset |
| `video::tests::late_joiner_never_misses_a_delta` | default | The subscribe-then-keyframe ordering of §5.2 |
| `llm::client::tests::parses_fragmented_tool_call_arguments` | default | Arguments split across SSE chunks reassemble |
| `llm::compaction::tests::*` | default | Image eviction, summary replacement, system prompt survives |
| `llm_policy::tests::kind_change_cancels_pending_turn` | default | An overworld turn in flight is dropped when `pick_battle_action` is polled, and a battle turn replaces it |
| `llm_policy::tests::cancelled_turn_leaves_history_well_formed` | default | After a mid-turn cancel and one-step rollback, no `tool_call` is left without a result (§7.3) |
| `llm_policy::tests::tool_batch_is_all_or_nothing` | default | A batch is never partially serviced across two polls |
| `llm::tools::tests::terminal_tools_scoped_per_kind` | default | §7.5 — a battle turn's `tools` array omits `choose_action`, and vice versa |
| `llm::tools::tests::parallel_batch_shares_one_observation` | default | Two reads in one assistant message are answered from the same `GameState` |
| `llm::compaction::tests::summary_restates_turn_contract` | default | §9 — the contract survives a compaction |
| `llm_policy::tests::no_tool_call_nudges_then_forces_wait` | default | §7.5 fallback, and it emits the marker event |
| `llm_policy::tests::field_move_does_not_cancel_overworld_turn` | default | `pick_field_move` shares the `Overworld` kind and never pre-empts |
| `mechanics::manual_input_preempts_state_machine` | default | W0.4 — a queued press fires and resets to `Idle` |
| `mechanics::policy_receives_text_events` | default | W0.3 — `on_event` sees `TextBox` |
| `llm_policy::tests::idle_poll_is_allocation_free` | default | W0.3b — a poll with no pending tool batch and no outcome does no work |
| `llm_policy::plays_from_fixture` | `slow-tests,llm` | A **mock OpenAI server** (axum, in-process) serves a scripted tool-call sequence; the agent executes it from a committed fixture |
| Existing suite | all tiers | Unchanged |

**`full_playthrough` gates W0 and any later phase that touches `agent.rs` or `policy.rs`.** Per
`CLAUDE.md`: the leg tier proves the legs individually, only the full run proves they compose, and
anything that changes frame timing re-rolls the RNG stream. W0.4 inserts a branch at the top of
`update()` — that is exactly the kind of change that needs it.

The mock-server test is the important one: it is what stops the tool schema, the resolution logic and
the agent's expectations drifting apart, and it costs no API key and no network.

---

## 16. Phase summary

| Phase | Deliverable | Gate |
|---|---|---|
| **W0** | SDL feature-gated · CLI dispatch · `Policy::{on_event, service_tools}` · manual input queue · observation facade | `full_playthrough` |
| **W1** | `EmulatorHost` thread · shared published state · axum skeleton · `/api/events` status SSE | `curl` shows status ticking |
| **W2** | Block-diff encoder + TS decoder · `/api/video` | Round-trip test; game visible in a dev page |
| **W3** | Vite/React SPA · embedded via `rust-embed` · screen + status + conversation shell | Full UI under `--policy random` |
| **W4** | OpenAI client (streaming, tool calls) · `LlmPolicy` · kind-keyed turns + cancellation · overworld + battle decisions | LLM plays; conversation streams |
| **W5** | Full tool surface: screenshot, reads, raw buttons, field moves, nickname/mart/forget | Mock-server test |
| **W6** | Token accounting · status broadcast · two-stage compaction · memory + TODO | Compaction tests |
| **W7** | Run directory · checkpoint/resume · transcript backlog | Survives restart mid-run |
| **W8** | Multi-stage Dockerfile · no-SDL build · ops config | Image builds and runs |
| **W9** | Stuck-run watchdog — lenient, last resort, loud (§14) | Fires on a deliberately jammed agent |
| *(deferred)* | Audio streaming (§12) | — |

W0–W3 are independent of any LLM and are worth shipping on their own: they give a browser-watchable
emulator with the existing policies. W4 is where the actual subject of this plan begins.

---

## 17. Open risks

1. **The LLM will find the agent's sharp edges.** Where the offered action list is incomplete — an
   uncovered gameplay mechanic — the model reaches for `press_buttons`, so the manual-input path
   (W0.4) is load bearing rather than a curiosity. Budget real testing for it. The narrower case
   where the agent stops asking altogether is W9's, and only W9's.
2. **Turn latency vs. `AGENT_RESOLUTION`.** A turn takes seconds to tens of seconds while the agent
   polls every tick. Fine in the overworld and at a battle menu, where the game is waiting anyway.
   Not fine anywhere the game is on a timer — essentially nowhere in Pokémon Red, but the Safari
   Zone step counter and the Game Corner are worth checking.
2b. **Cancellation churn.** Each pre-empted turn is tokens paid for and discarded. Harmless when a
   battle interrupts an overworld decision; pathological if some state oscillates between two kinds.
   Count cancellations per run and surface the number in the UI — a rising rate is a bug signal.
3. **Endpoint compatibility.** "OpenAI-compatible" varies most in exactly the two places we depend
   on: streamed `usage`, and tool-call argument fragmentation. §7.1 hedges both.
4. **Cost.** Every decision point is a completion, and there are thousands per playthrough. Screenshots
   multiply it. Worth adding a per-run token ceiling that stops the run rather than a surprise bill —
   the accounting from W6 is already there, it just needs a limit and a `Status::Halted`.
5. **`CLAUDE.md` references deleted docs.** `docs/` was removed wholesale in `1aa9141`, but
   `CLAUDE.md` still points at `docs/compatibility/10-implementation-plan.md`,
   `docs/postgame-coverage-plan.md` and others in several places. Unrelated to this work, but it will
   confuse the next reader of this file — worth a separate cleanup commit.
