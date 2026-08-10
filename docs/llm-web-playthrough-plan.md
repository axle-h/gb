# LLM Playthrough over the Web — Implementation Plan

An `LlmPolicy` that delegates every decision to an LLM over an OpenAI-compatible API, and a
browser UI — served from the same process — that lets anyone with the URL watch it play.

**Status:** **W0–W4 complete** (W0–W3 2026-08-07, W4 2026-08-08). The seams are in, `gb serve` plays
the game headlessly and streams it to a browser over SSE, the React SPA renders it, and
`--policy llm` hands every overworld and battle decision to an OpenAI-compatible endpoint and streams
the conversation back to the page. W5 onward is unbuilt. Each completed task below carries what
actually shipped, including the places the plan was wrong.

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

⚠️ **The emulator keeps running while the LLM thinks, always.** In the overworld the player stands
still and in battle the game waits at the menu, so this is usually harmless — but a `press_buttons`
decision made against a screenshot taken 4 seconds ago can land on a different screen.

An earlier draft of this section offered `GB_PAUSE_WHILE_THINKING` to skip `gb.run` while a turn was
in flight. **It was built in W4 and removed the same day, and it is not coming back.** Two reasons,
either of which is sufficient:

- **A live picture is the product.** A frozen screen with a "Thinking" pill is a worse watch than a
  slightly stale one, under every circumstance anyone could name — so the flag's *on* position is
  never the one you want, which makes it not a trade-off but dead weight.
- ⚠️ **It could deadlock the run.** A tool batch is answered by `Policy::service_tools`, which only
  runs when `gb.run` advances the agent. Any pause spanning a tool round trip hangs the run on the
  first `read_map`. W4's implementation dodged this by pausing only while a completion was
  *streaming* — a subtlety that existed purely to keep a feature nobody wanted from breaking things.

The staleness it was meant to address is handled where the decision lands instead: a `choose_action`
id is re-resolved against a **freshly recomputed** `actions()` (§7.4), and W5's `press_buttons` goes
through `queue_manual_input`, which is applied within one agent tick of the poll that receives it.
`src/host.rs` carries a ⚠️ above `HostConfig` so the next person to reach for a pause finds the
argument before they write it.

### 2.2 Threading contract

| Channel | Type | Direction |
|---|---|---|
| `TurnRequest` | `std::sync::mpsc::Sender` | policy (emulator thread) → worker |
| `TurnOutcome` | `std::sync::mpsc::Receiver`, polled with `try_recv()` | worker → policy |
| `ToolCall` | `std::sync::mpsc::Sender` | worker → policy |
| `ToolResult` | `std::sync::mpsc::Receiver`, **blocking `recv()`** | policy → worker |
| `UiEvent` | `tokio::sync::broadcast::Sender` (capacity 1024) + an `RwLock` holding the latest heartbeat | emulator + worker → SSE clients |
| `VideoMsg` | `tokio::sync::broadcast::Sender` (capacity 64) | emulator → SSE clients |
| `RunStatus` | `RwLock<RunStatus>` + a `UiEvent` on each transition (**W6**) | emulator + worker → SSE clients |
| Latest frame | `RwLock<Arc<FrameSnapshot>>` | emulator → worker (screenshots) + late joiners |

The only tokio types are the two broadcast senders, and they are used purely as the sync→async
bridge: **`broadcast::Sender::send` is synchronous and non-blocking**, callable from a plain thread
with no runtime handle and no `block_on`. Everything between the emulator, the policy and the worker
is `std::sync::mpsc`.

⚠️ **The status heartbeat is sent on change, not on a timer.** At the original 10 Hz unconditional it
measured **49.7 kbit/s per viewer** against ~8 for the idle video feed, with nine of ten payloads
identical to the one before. It is now sampled at `GB_STATUS_HZ` (2 Hz) and published only when
`StatusSnapshot::says_the_same_as` says it differs, with a 2 s keepalive for liveness: 5.2 kbit/s
measured. The consequence is that `/api/events` has to **open with the latest heartbeat** — one
shared cell, not a buffer per client — or a page opened during a quiet stretch waits for something to
move. Same subscribe-then-read handshake as the video keyframe, and for the same reason.

⚠️ **There is no `watch<Status>`, and W6 did not add one.** A `watch` would give a late joiner the
current value, which is the whole reason it was in this table — but the 10 Hz `StatusSnapshot` is
already going to every client, so carrying `RunStatus` on it costs nothing and needs no second
channel. What is left is an `RwLock` the heartbeat reads and a `UiEvent` on each transition, so a
viewer sees a change at once and a joiner is at worst 100 ms behind.

`broadcast` drops the oldest message for a slow client rather than blocking the producer — correct
for video, and for `UiEvent` the client recovers via `/api/history`.

---

## 3. Phase W0 — Seams in the core

**No web, no LLM.** Everything here is a small, independently mergeable change to existing code that
the later phases depend on.

### W0.1 — SDL behind a cargo feature ✅ **done**

```toml
[features]
default = ["sdl", "web"]                                     # ← `web` joined the default in W1
sdl  = ["dep:sdl2", "dep:fontdue"]
web  = ["dep:serde", "dep:serde_json",
        "dep:tokio", "dep:tokio-stream", "dep:axum", "dep:base64"]
# W3 adds: rust-embed, tower-http. W4 adds:
llm  = ["web", "dep:ureq"]
```

**`web` shipped in W0 with only `serde`/`serde_json`.** The server crates were declared where they
were first used, in W1 — adding `tokio`, `axum` and the rest in W0 would have been ~200 crates that
nothing compiles against and no acceptance test could exercise. What W0 needed from `web` was the
`Serialize` derives on [W0.5](#w05--the-observation-facade-done)'s views.

**W1 then put `web` in `default`**, which W0 deliberately did not do. That reason expired the moment
`src/host.rs` and `src/web/` landed: the video codec, the late-joiner ordering and the host's
publishing are all default-tier tests, and behind an opt-in feature a plain `cargo test --release`
would silently skip every one of them — which is precisely how `full_playthrough` rotted once
already. The measured cost is **57 extra crates** on a build that already pulls 119, and a
server-only build still drops SDL entirely (`--no-default-features --features web`). Reverting is a
one-word change if it ever stops being worth it.

`tokio` is needed only for `axum` and for the `broadcast`/`watch` bridge types (§2.2) — the emulator
and the LLM worker are plain `std::thread`s and neither is ever inside a runtime. `ureq` is a
blocking client with no async runtime of its own; `reqwest::blocking` would spin up a second hidden
tokio runtime for no benefit.

`mod sdl;` in `src/main.rs` becomes `#[cfg(feature = "sdl")]`.

Only `sdl2` and `fontdue` are SDL-only (`src/sdl/font.rs`, `src/sdl/render.rs`) — **verified
2026-08-07**. `itertools` is *not*: it is used by `src/ppu.rs`, `src/pokemon/mod.rs` and
`src/pokemon/map_header.rs`, so it stays unconditional. An earlier survey claimed otherwise.

**Acceptance — met.** `cargo build --release` links `libSDL2-2.0.so.0` exactly as before;
`cargo build --release --no-default-features` succeeds and `ldd` shows no SDL2 at all. `mod sdl` and
`run_ui` are both `#[cfg(feature = "sdl")]`, with a `#[cfg(not(...))]` `run_ui` that reports the
missing feature.

⚠️ A no-SDL build emits ~780 warnings against the default build's ~313: most of the emulator's
inspection surface (`PPU`, palettes, joypad helpers) has the UI as its only caller, so dropping it
makes them dead code. Nothing is broken; do not "fix" it by deleting the accessors.

### W0.2 — CLI dispatch ✅ **done**

`main.rs` grows a hand-rolled arg parse over `std::env::args` — no `clap`.

```
gb                          → SDL UI, ConsolePolicy               (today's behaviour, unchanged)
gb serve [--port 8080]      → web UI + LlmPolicy
gb serve --policy random    → web UI, RandomPolicy (no API key needed; the video-pipeline harness)
gb --help                   → usage, exit 0
```

Lives in `src/cli.rs` so `parse` is unit-testable without spawning a process; `main` is dispatch and
exit codes only. Every rejection prints the specific complaint followed by the full usage, and exits
non-zero.

Keyed on **`sdl`**, not `llm`: without the feature there is no UI to run, so a bare `gb` says so and
prints usage. `gb serve` is keyed on `web` the same way. **W1 filled the command in** — it now
starts the server; `--policy llm` reports that it arrives with W4 rather than starting a server that
would sit at a decision point forever with nothing to answer it.

### W0.3 — `Policy::on_event` ✅ **done**

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

~~⚠️ Some sites push into a local `new_events: Vec<AgentEvent>` and drain later. Route those through
`event()` too, or `on_event` silently misses them.~~ **Checked — this was already the case.** The
drain at the end of `update()` calls `self.event(x)` for each, so every path converges on the one
function and `on_event` cannot miss a class of event. `policy_sees_every_event_and_gets_a_tool_poll`
pins it by asserting an `OverworldActionCompleted` arrives, which is emitted *only* via `new_events`.

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

### W0.4 — Manual input queue on the agent ✅ **done**

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

Handled at the top of `update()`, immediately after the `AGENT_RESOLUTION` gate — ahead of
`drive_post_champion_cutscene` *and* ahead of the `game_mode()` "Not in game" check, so a title screen
or a continue prompt, where the state machine cannot help and a raw button can, is still reachable.

- The queue is capped at **16** buttons so a confused model cannot run away with the game.
- `queue_manual_input` sets `state = Idle` on entry (and drops `backup_state`) — otherwise a queued
  press mid-`OverworldMovement` corrupts the route and the walk resumes into a wall. Nothing else is
  needed when the queue drains: the state machine simply resumes from that `Idle`.

**⚠️ The cadence is three ticks per press, not one — measured, and the plan's original "one tick
each" was wrong.** A press is held for **2** agent ticks and then released for **1**:

- **The hold is 2 because 1 does not work.** 20 ms is longer than a frame, so one tick looks
  sufficient; it is not. Driving START in a standing Pallet Town at 16 successive tick alignments, a
  1-tick hold opens the menu at 11 of them and does *nothing* at the other 5 — pokered does not sample
  the pad on every frame in the overworld. A 2-tick hold lands at all 16. `probe_manual_input_hold_length`
  (`--features diagnostics`) prints the grid; the failure it prevents is the worst one this feature
  has, since a dropped press is indistinguishable to the LLM from a button the game ignored on purpose.
- **The released tick separates repeats.** A button held straight through is one continuous press and
  pokered drives menus off *newly* pressed bits, so without the gap "A, A" arrives as a single A.

`manual_input_pending()` counts the in-flight press as well as the queued ones, so zero means the
state machine is back in charge.

Tests (default tier, `mechanics.rs`): `manual_input_presses_a_button_the_agent_never_would` (START
from a quiet overworld — the one thing that takes `GameMode` to `TextBox` with nobody touching the
pad; note `on_screen_text` cannot corroborate it, as the overworld has not loaded `vFont`),
`manual_input_holds_then_releases_each_press` (the cadence, tick by tick), `manual_input_queue_is_capped`.

### W0.5 — The observation facade ✅ **done**

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
policy holds at a poll — which is what lets W0.5b service tool calls without any round trip. In
practice each takes only the part of the triple it needs (`party(&GameState)`,
`screen_text(&PokemonApi)`, `world_graph(&WorldGraph)`), which is what makes them individually
testable.

**Shipped detail worth knowing before W5 writes the tool schemas.**

- **The views are the contract, not `GameState`.** `GameState` is the agent's working set —
  `raw_tile_ids`, tile-pair collision tables, spinner maps, BFS inputs — most of which is noise in a
  context window, and every rename inside it would silently change the schema the model was prompted
  against. The `*View` structs are hand-cut so `GameState` stays free to move.
- **Ordering is pinned.** `warp_targets` and `connection_targets` are `HashSet`s, so `warps`,
  `connections`, `actions` and the world-graph `nodes` are all sorted before they leave. Without it
  two reads of an unchanged map come back in different orders, which reads to a model as the world
  having moved. `map_view_is_well_formed_stable_and_fully_documented` asserts a second read is equal.
- **`MAP_LEGEND` travels with every grid**, and a test asserts the legend explains every character
  the grid actually uses across three maps — so a new symbol in `impl Display for MetaTileMap` fails
  a test instead of appearing undocumented in a context window.
- **`screen_text` answers `None` in the overworld** and that is correct, not a failure: no dialogue
  font is loaded there, so there is nothing in VRAM to decode. Anything asserting on menu text has to
  use `game_mode` instead — this cost time in W0.4.
- `Serialize` is `#[cfg_attr(feature = "web", ...)]`, applied by a small `view!` macro so the
  attribute cannot be forgotten on a new struct. `observation_views_serialise_to_json` proves the
  derive actually lands, which a compile alone would not.

### W0.5b — `Policy::service_tools` ✅ **done**

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

Four of the five already had a `GameState` in hand. **`NamingPokemon` did not**, so it reads one — but
with `if let Ok(...)` rather than `?`: the naming screen is the least ordinary place in the game to
ask for a full `GameState`, and adding the tool seam must not invent a failure mode for the policies
that ignore it.

Both assumptions the seam rested on hold: the signature is object-safe under `Box<dyn Policy>`, and
`self.policy.service_tools(&state, api, &self.world_graph)` borrow-checks at every site — the same
disjoint-field shape `pick_overworld_action` has always used.

⚠️ The observed state is computed **once** per poll and every queued tool call is answered from it,
so a turn never sees a torn view — `read_party` and `read_map` in the same assistant message are
guaranteed consistent with each other.

**Acceptance for W0 — met.** `cargo test --release` green (1033 tests); **`cargo test --release
--features full-playthrough full_playthrough` green** — W0.3, W0.4 and W0.5b all touch `agent.rs`,
and per `CLAUDE.md` the leg tier is not a substitute for the full run.

Tests added, all in the default tier: `src/cli.rs`'s own module (parsing, including that every
rejection carries the usage); `manual_input_*` ×3 (W0.4);
`policy_sees_every_event_and_gets_a_tool_poll` (W0.3 + W0.5b, via a `RecordingPolicy` that delegates
every decision and records what it was asked); `observation_views_describe_the_snapshot`,
`map_view_is_well_formed_stable_and_fully_documented`, `battle_view_describes_a_live_battle`,
`world_graph_view_reports_only_visited_maps` and `observation_views_serialise_to_json` (W0.5).
`TestFixture::with_policy` was added so a test can supply its own policy and still get the whole
harness — the options pin, the stall detector, the cycle budget.

---

## 4. Phase W1 — Headless host + server skeleton ✅ **done**

### W1.1 — `src/host.rs` ✅ **done**

Lift the pacing loop out of `render.rs` into an `EmulatorHost` that owns `GameBoy`, `PokemonAgent`
and `MapMetadataCache` and runs on its own thread. The pacing algorithm transplants unchanged:
accumulate wall-clock into `since_last_update`, drain it in `cycle_duration` steps, credit
`ahead_by_cycles` for `gb.run`'s instruction-boundary overshoot (`render.rs:231-247`).

`render.rs` is **not** refactored to use it. It keeps its own loop — it has F1–F12 debug affordances
tangled into the same event pump, and untangling them buys nothing. The duplication is ~30 lines and
the SDL path stays exactly as it is today.

**Shipped detail.**

- **`tick()` is public and `run()` is a loop around it.** The loop is otherwise untestable — a test
  would be racing a thread — and the two host tests drive `tick` directly instead.
- **`EmulatorHost::spawn` takes a policy *factory*, `Box<dyn FnOnce() -> Box<dyn Policy> + Send>`,
  not a policy.** `Policy` is not declared `Send`, and `PokemonAgent` holds a `Box<dyn Policy>`, so
  a built host cannot cross a thread boundary. Adding `Send` to the trait would constrain every
  implementation for one call site; building the policy on the thread that will own it costs one
  closure. Construction still reports on the *calling* thread through a one-shot channel, so a
  starting state that will not load is a clean error before anything is listening.
- **Two things `render.rs` does not do, both needed by a process meant to run for hours.**
  `IDLE_SLEEP` is 1 ms rather than `sleep(0)`, so an idle server does not spin a core; and
  `MAX_CATCHUP` caps one iteration's make-up at 250 ms, so a container throttled for a few seconds
  comes back and carries on rather than emulating the backlog flat out — which on a livestream looks
  exactly like the game fast-forwarding for no reason.
- **The host starts from `pokemon::data::START_OF_GAME`** (`start-of-game-state.bin`, the fixture
  `full_playthrough` plays from — `mod data` became `pub mod data` for it). A fresh boot lands on the
  title screen and no policy can get past that, so the harness would have nothing to show. **W7
  replaces this with a run directory.** The state is a DMG save, so the host builds a DMG `GameBoy`
  to match.
- **Status is `Option`al by design.** `game_state()` can legitimately fail mid-transition; a
  heartbeat that says "no game state" is far easier to diagnose than one that stops arriving.

### W1.2 — Server skeleton ✅ **done**

`src/web/mod.rs` with axum on `0.0.0.0:<port>`, an index page, `/api/healthz`, and `/api/events` SSE
emitting the 10 Hz status snapshot plus `AgentEvent`s as they happen.

`src/web/published.rs` holds the buffers, and it is the **whole** interface between the two sides:
the web layer can reach `Published` and nothing else, which is what makes §1.1's "strictly view-only"
structural rather than a matter of not exposing a POST route. There is no channel back into the
emulator to expose.

**Two deviations from §2.2, both deliberate.**

1. **No `watch<Status>`.** Status rides the `UiEvent` broadcast as `UiEventBody::Status`. A second
   channel exists only to hand a joiner the current value instantly, and at 10 Hz the wait it saves
   is under 100 ms. W6, where `Status` becomes a real enum with transitions worth latching, is where
   to revisit it — building it now would be unused machinery.
2. **`GB_PORT` is not read yet.** `--port` is the only source. The env vars in §7.1 land as a block
   in W4 with the rest of the config; one of them read early would be the odd one out.

`tokio-stream` joined the dependency list. `BroadcastStream` is the sync→async bridge in stream form
and, more to the point, the only thing that surfaces `Lagged` — see W2.

**Acceptance — met.** `gb serve --policy random --port 8099`, then `curl -N
localhost:8099/api/events` shows status frames ticking with the map changing (`RedsHouse2F` →
`RedsHouse1F`) and `emulated_ms` tracking `wall_ms` at 1.0×. `the_host_publishes_a_moving_game_state`
asserts the same thing without a socket.

---

## 5. Phase W2 — Video pipeline ✅ **done**

`src/web/video.rs`. The wire format below shipped as specified with **one correction**, marked ⚠️
under §5.1 — the original palette rule quietly desynchronised every late joiner.

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
- **Mode 1 (raw):** 64 palette indices.
- **Mode 2 (packed) — added during W2, not in the original spec:** a 4-entry sub-palette of global
  indices in first-appearance order, then 2 bits per pixel into it, low bits first. A flat 20 bytes.
  The encoder picks whichever of the three is smallest per block.
- **Palette exhaustion:** if a frame would push past 256 entries, the encoder emits a keyframe with a
  fresh palette instead. Pokémon Red in CGB-compat mode uses a few dozen colours on screen; this is
  a safety valve, not a normal path.
- **RGB888, not RGB565.** DMG's greys are `FF/AA/55/00`; `0xAA` does not survive a round trip through
  5-bit. The palette is small enough that the extra byte per entry is free.

⚠️ **The palette rule as first written was wrong, and its failure is silent.** "Entries appended to
the persistent palette" is right for a *delta*; for a **keyframe it has to be a replacement**, and
the keyframe has to carry the encoder's **entire** palette rather than only the colours its own
blocks need. Otherwise §5.2's handshake — a keyframe encoded on demand for a late joiner — leaves
that joiner's palette a *subset*, in a different order, from the encoder's, and the first delta that
references an index the keyframe did not list paints the wrong colour. Nothing errors; a corner of
the screen is simply wrong forever. `a_keyframe_catches_a_fresh_decoder_up_exactly` and
`palette_exhaustion_forces_a_keyframe` are what pin it.

A consequence worth knowing: `VideoEncoder::keyframe()` is **pure**. It describes the state the
encoder is already in, advancing nothing, so the emulator thread publishes one beside every delta and
a late joiner picks it up without racing the encoder. Producing one costs ~5 KB of RLE at 30 fps,
which is nothing, and it removes the on-demand-encoding race the plan would otherwise have needed.

The encoder also tracks what the **decoder** will hold rather than what the frame contained
(`last_sent` stores palette-resolved colours). That matters only on the lossy path — a frame with
more than 256 distinct colours, which Pokémon Red never produces and which falls back to the nearest
palette entry rather than failing — but without it a block approximated once would read as unchanged
forever after.

Base64 the whole thing into one SSE `data:` line. A keep-alive comment every 2 s stops proxies
closing an idle connection.

⚠️ **The two-mode estimate was wrong, and the reason is why mode 2 exists.** "Game Boy 8×8 blocks are
extremely run-friendly … ~15 bytes RLE each" does not survive contact with the game: measured over
10 s of continuous outdoor walking under `--policy random` at 30 fps, RLE blocks averaged **39.7
bytes**. The runs are ~3.5 pixels long, not ~10 — a tile is 8 pixels wide and detailed ones (grass,
trees, interior clutter) change colour every two or three, so a scrolling screen is close to the
worst case for run-length coding at 1:1. That put the stream at **1.1 Mbit/s**, 3.7× the budget.

Mode 2 was added in response, and the *same 10 s of content* re-encoded through all three modes:

| | RLE + raw (as specced) | with mode 2 |
|---|---|---|
| Continuous outdoor walking | 1117 kbit/s | **536 kbit/s** — 52 % less |
| Static screen (text box, nobody moving) | ~11 kbit/s | **~8 kbit/s** |
| Blocks choosing RLE | 26 242 @ 39.7 B | 4 643 @ 6.4 B — only the flat ones are left |
| Blocks choosing packed | — | 26 858 @ 23.0 B |
| Blocks choosing raw | 5 259 @ 67 B | **0** |
| Keyframe | ~6 KB | ~4 KB |

**Raw is now dead on DMG and stays anyway.** Every block that chose it had ≤4 distinct indices, so
packing beats it everywhere; it is the fallback for a block with five or more colours, which is a CGB
concern rather than a Pokémon Red one. Keeping it costs one `match` arm and removes a cliff.

536 kbit/s is still above the 300 the plan budgeted, and that is the honest number for the worst
case. It matters less than it looks: under `--policy llm` the screen is static for most of every turn
while the model thinks, and a static screen costs nothing at all. The next lever, if one is ever
needed, is not another block mode but a **tile cache** — the hardware draws 8×8 tiles and a scroll
re-sends the same handful of them at every offset — and that is a much larger design.

### 5.2 Cadence and late joiners ✅ **done**

The encoder is called from the emulator loop on a **wall-clock** timer, at 30 fps, independent of
emulated frame rate (so fast-forward does not multiply bandwidth).

Late joiners must not miss a delta between reading the current frame and subscribing:

1. `broadcast::subscribe()` **first**.
2. Take the published keyframe, note its `seq`.
3. Send it, then forward messages from the receiver, **discarding any with `seq <= keyframe_seq`**.

`Published::join_video` does 1 and 2 in that order, and `Published::publish_video` **stores the
keyframe before broadcasting the delta**. That second ordering is the load-bearing half and it is
easy to get backwards: broadcast-first leaves a window in which a joiner subscribes, reads the
*previous* keyframe, and never sees the delta that followed it. Storing first makes the worst case a
delta the joiner already has, which `seq` filters out. `late_joiner_never_misses_a_delta` loops over
the size of that window rather than testing one interleaving.

**Two changes from the plan.**

- **No 5-second keyframe timer.** Keyframes go into the stream only on a palette reset, which is the
  one case that is semantically a reset. A client that falls out of the 64-message ring buffer is
  handled better: `BroadcastStream` reports `Lagged`, and `/api/video` answers it by sending the
  latest keyframe in place. Re-syncing is invisible to the viewer, where a periodic keyframe costs
  every viewer 6 KB every 5 s to insure against something that mostly does not happen.
- **`seq` is `u64` in `VideoMessage`, `u16` on the wire.** The discard rule in step 3 is a
  comparison, and a comparison across a `u16` wrap is wrong — that is ~36 minutes into a run, which
  is exactly the kind of bug that survives every test and appears in production.

### 5.3 Decoder ✅ **done**

TypeScript decoder mirroring the encoder, writing into an `ImageData` backing a 160×144 `<canvas>`,
scaled up with CSS and `image-rendering: pixelated`. Only changed blocks are written; `putImageData`
once per message. (Shipped as plain JS in the dev page — no build step until W3 introduces Vite.)

**Testing.** A Rust reference decoder in `src/web/video/tests.rs` round-trips 120 frames of the agent
actually playing under `RandomPolicy` and asserts pixel-exact reconstruction after every message.
Synthetic frames would have exercised none of what makes this codec cheap and would not catch a
regression on a sprite edge. Around it: the keyframe-catch-up invariant, a forced palette reset, both
block modes (`a_noisy_block_falls_back_to_raw_mode` also asserts a *real* frame still prefers RLE, or
the "cheap" claim above is wrong), and a corruption sweep that truncates a valid message at every
length and requires an error rather than a panic at each.

**Acceptance — met.** `web/dev/video.html` (plain canvas + two `EventSource`s, no build step) renders
the game under `--policy random`; the SSE capture was independently decoded and rendered to PNG to
confirm it is a real Pokémon Red screen and not merely self-consistent. This page was discarded in W3
— it was `include_str!`'d at `src/web/mod.rs`'s `DEV_PAGE`, so deleting it was a compile error rather
than a dead route.

---

## 6. Phase W3 — The SPA ✅ **done**

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

**Shipped detail.**

- **The layout is the mock, minus what does not exist yet.** The header's model and token accounting
  is W4's; a placeholder number there would be worse than the gap, so it carries the policy name and
  the game mode instead. The party list has no levels because `StatusView.party_hp` does not carry
  them — the full party is a W5 read tool, and widening the 10 Hz heartbeat for one decoration is
  the wrong trade. The playtime clock **was** added (`observe::status` now takes `&PokemonApi` for
  three byte reads), because "how long has this run been going" is the one number a livestream
  viewer actually wants.
- **The `AgentEvent` variant is not the gutter label.** `overworld_action_completed` and
  `started_overworld_action` differ by a glyph the line itself already carries (`→ ✓ ✗ 📖`), and
  spelling both out cost a quarter of the pane's width. The gutter says which part of the game is
  talking (`overworld` / `battle` / `text`); the variant is the `title` attribute.
- **No `tower-http`.** `GB_WEB_DEV=1` reads the same paths from disk through 20 lines in
  `src/web/assets.rs`, sharing one sanitiser with the embedded path. `ServeDir` would have been a
  reasonable call, but the sanitiser has to exist for the URL→embed-key mapping anyway, and a
  whitelist of path shapes (`Component::Normal` only) is a smaller thing to be sure of than a
  dependency that also does ranges, precompression and directory listings.
- **The `.gitkeep` needs help.** `vite build` empties `dist` first, so the committed marker whose
  entire job is to make `web/dist` exist at compile time was deleted by the first build — and
  `rust-embed` fails to **compile**, loudly but confusingly, when it is missing. `web/public/.gitkeep`
  is copied back into `dist` on every build, which keeps the committed file in place instead of
  leaving a deletion in `git status` after each build.
- **`index.html` is not shipped as a stub.** A checkout that has never run `npm run build` compiles
  (the `.gitkeep` is enough) and serves a page saying exactly which two commands to run. A committed
  stub would have been overwritten by every build, which is churn for a worse message.
- **The screen is scaled by an `outline`, not a `border`.** Under `box-sizing: border-box` a 1 px
  border leaves the canvas 638 px wide in a 640 px column — 3.99 device pixels per Game Boy pixel,
  and a few visibly narrow rows. An outline does not take part in layout, so the scale stays exactly
  4×.
- **The badge strip is the ROM's own art.** `src/pokemon/badge_gfx.rs` decodes
  `GymLeaderFaceAndBadgeTileGraphics` — the blob the trainer card uses, 8 tiles per gym (a 2×2 gym
  leader face, then its 2×2 badge) — and `/api/badges.png` serves the eight badges as one 128×16
  sheet the UI slices by `background-position`. Nothing is committed: it is read from the same
  cartridge bytes the emulator boots, at run time.
  - ⚠️ **The tone ramp is inverted on the way out.** On the trainer card a badge is dark line art on
    a *white* background. Dropped on this page it would be either a bright white chip or — if the
    background were simply made transparent — black-on-black. Inverting gives light line art on a
    dark page, and the first attempt was still too dark at 2×: the visible levels are now
    `#8B94A2 / #C6CCD6 / #FFFFFF`, checked on the page rather than in a 4× preview.
  - **`StatusView.badge_count` became `badges: Vec<BadgeView>`** — a name and an `earned` flag for
    all eight, in bit order, which is also the sheet's sprite order. A count cannot drive the strip:
    gyms can be beaten out of order, so "the first N" is wrong. Sending all eight with their names
    also keeps the client from carrying its own copy of the badge list.
  - **Earned and unearned are the same sprite at different opacity** (1.0 against 0.16), so the
    sheet needs no second variant and the transition is one CSS property.
- **A viewer joining mid-run starts with an empty conversation.** The event stream is live-only;
  `/api/history?since=` is W7's, and until then a late joiner sees the status panel populate
  immediately and the log fill from the next thing that happens.

**Acceptance — met.** `gb serve --policy random --port 8099`, driven headlessly through CDP: the
screen renders, the status panel tracks `OaksLab (5, 3)` / the badge strip / ¥ / the playtime clock, and the
conversation streams and auto-scrolls. Killing `gb` mid-session puts the pill and the screen overlay
into `reconnecting…`; restarting it repaints the canvas and resumes the log with no reload. Both
asset paths were exercised: an empty `web/dist` compiles and serves the not-built page, and
`GB_WEB_DEV=1` serves the SPA from disk out of a binary that embeds nothing. The TypeScript decoder
was run under node against the live stream and its output written to a PNG — the four DMG shades and
a real Pokémon Red screen, not merely self-consistent bytes.

---

## 7. Phase W4 — LLM client and the turn lifecycle ✅ **done**

**What shipped.** `src/llm/` — `config.rs` (the §7.1 environment block), `protocol.rs` (the wire
types and the SSE accumulator), `client.rs` (`ChatEndpoint` + `OpenAiClient` over `ureq`, and the
retry policy), `tools.rs` (the catalogue, the per-kind scoping, the ids and the servicing),
`prompt.rs` (the system prompt and the turn situation), `worker.rs` (the turn loop) — plus
`src/pokemon/llm_policy.rs`. `gb serve --policy llm` is live; the SPA renders turns, streamed prose,
tool calls, decisions and context occupancy.

**Acceptance — met.** Against a mock endpoint on loopback, `gb serve --policy llm` played from the
start of the game through Oak's script, took a starter, and fought the rival — asking one turn per
decision point, streaming its prose into the browser, and answering `read_map` from the live game.
`the_llm_plays_from_a_fixture` (default tier, 0.08 s) pins the same path in CI: a real socket, a real
`text/event-stream`, `OpenAiClient` parsing it, and the emulator executing what came out.

**Where the plan was wrong, or silent.**

1. ⚠️ **`GB_PAUSE_WHILE_THINKING` was built, then deleted — see §2.1 for the argument.** It was
   specified as "skip `gb.run` while a turn is in flight", which **deadlocks**: a tool batch is
   answered at the policy poll, and the policy is only polled when `gb.run` advances the agent, so
   the first `read_map` of the run hangs it forever. The shipped version dodged that by pausing only
   while a completion was *streaming* — and then the whole feature came out, because freezing the
   livestream is never the trade anyone wants and a knob whose on position is a footgun is worse than
   no knob. `src/host.rs` keeps a ⚠️ above `HostConfig` so the next person to reach for a pause finds
   the reasoning first.

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
| `GB_RUN_DIR` | `./runs` | W7 |

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

## 8. Phase W5 — The full tool surface ✅ **done**

**What shipped.** `screenshot` (`src/llm/screenshot.rs`, encoded on the worker thread and carried in
the multi-part content form `src/llm/protocol.rs` now models), `press_buttons`, `use_field_move`, and
the three menu decision kinds — `Nickname`, `MartPurchase`, `ForgetMove` — with `set_nickname`,
`buy_item` and `forget_move`. `DecisionKind` is now exactly the agent's five policy poll sites.

**Acceptance — met.** The mock-server test asks for `read_map` **and** `screenshot` in one assistant
message, and asserts the PNG that came back through the real socket decodes at the right size; the
policy tests drive all three menu prompts, including the forget prompt pre-empting a battle turn;
`mechanics::a_policy_can_ask_for_a_raw_press_and_the_agent_delivers_it` closes the `press_buttons`
loop through the real agent. Default tier 1112 green, `full_playthrough` green.

**Where the plan was wrong, or silent.**

1. ⚠️ **An image cannot ride on the `tool` message that answered the call.** §8 says "base64 in an
   `image_url` content part", which reads as "as the tool result". OpenAI allows an array of content
   parts on a tool message but only *text* parts in it, and several compatible endpoints reject an
   `image_url` there outright. The shipped shape answers the call with a sentence and appends the
   picture as a **user** message immediately after every tool result — never interleaved, because a
   `user` message between an assistant's `tool_calls` and their answers is rejected too.
2. ⚠️ **`observed_kind` cannot be inferred from the `GameState` alone any more.** W4 read the kind off
   `state.battle.is_some()`, which is fine for two kinds and wrong for five: a naming screen, a mart
   menu and the forget prompt all look like an ordinary overworld or battle state. Getting it wrong is
   not a wasted round trip but an **infinite loop** — every read batch cancelled, every turn restarted,
   for as long as the prompt is open. The fix is `LlmPolicy::site`, the kind of the last `pick_*`,
   which is exact for every poll of a decision point after the first.
3. ⚠️ **`service_tools` now snapshots unconditionally, and W4's "only when nothing is pending" guard
   was wrong.** Any such guard has to predict whether *this* poll is the first of a new decision
   point, and it cannot — the site is only known once the `pick_*` after it runs. Two cases broke it:
   a battle interrupting an overworld turn built its menu from the overworld state it replaced, and a
   mart opening mid-turn rendered a stock list read before the player reached the shop. The price of
   being right is a `GameState` clone and one VRAM text decode per poll, and `LlmPolicy` only ever
   runs at **1× real time** — it is the livestream's policy — so that is fifty of each per wall-clock
   second against an emulator that is otherwise idle 95% of the time.
4. **`use_field_move` is one tool with a `move` discriminator, and covers a chosen subset of
   `FieldMove`.** `Fish`, `UseItemPc`, `UsePcBox`, `SellToMart`, `RedeemPrize`, `UsePartyScript` and
   `UseElevator` are left out: their arguments are internal types a model cannot name from anything it
   is shown, and none is on the path to the Hall of Fame. Surf is left out for the opposite reason —
   the agent mounts it by itself the moment a route steps onto water.
5. **A field move is stashed, not returned.** `pick_overworld_action` decides it and `pick_field_move`
   — which runs *before* it, on the next tick — hands it over. `pick_field_move` still touches nothing:
   not `pending`, not `waiting`, not `site`.
6. **`press_buttons` needed a *pull*, not a push.** The plan has "the host calls `queue_manual_input`",
   but the host does not own the policy — the agent does. `Policy::take_manual_input` (default: an
   empty `Vec`, which does not allocate) is drained at the top of `PokemonAgent::update`, immediately
   before `drive_manual_input`.
7. **`ItemId` and `BagItem` had to become publicly reachable** (`pokemon::{item, bag}` are now `pub
   mod`), because the tool layer parses them and lives outside `pokemon`. Names are matched
   case-insensitively with non-alphanumerics ignored, so `"HM01 Cut"`, `"hm01_cut"` and `"Hm01Cut"`
   are the same item — nothing the model is shown spells them any other way, so rejecting two of the
   three would be a rejection it could not learn its way out of.
8. **The mart's stock is not in `GameState`** and had to join `ApiSnapshot`, which is the only thing
   holding a `PokemonApi` at the moment a turn is built.

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

**What shipped for that:** the picture goes out with `detail: "low"`, which is a flat 85 tokens on
OpenAI against roughly a thousand for `"high"` — the screen is four shades and 8×8 tiles, so there is
no detail the expensive tier could find. It is upscaled 3× to 480×432 first, the largest whole
multiple that still fits inside the 512×512 box a low-detail image is fitted to, so the endpoint
resizes nothing. `Message::approximate_tokens` charges an image at that flat rate rather than by the
length of its data URL: a 3 KB PNG is four thousand base64 characters, and estimating it as prose
would overstate the context fifty-fold and trip the trim on a history that is nowhere near full.
`Message::has_image` is there for §9's eviction.

---

## 9. Phase W6 — Tokens, status, compaction ✅ **done**

**What shipped.** `src/llm/accounting.rs` (the token ledger and the estimator's calibration),
`src/llm/compaction.rs` (both stages, as pure functions over `Vec<Message>`), `RunStatus` in
`src/web/published.rs`, and the worker changes that drive all three. The SPA gained a run-status
label, a context gauge with the run's cumulative spend, and a `compacted` line in the conversation.

**Acceptance — met.** `llm::compaction::tests::*` and `llm::accounting::tests::*` cover the surgery
and the arithmetic; `llm_policy::tests::a_full_context_is_summarised_and_the_next_turn_carries_the_summary`
drives a real compaction through the real worker and asserts the next turn opens on the summary;
`the_run_status_follows_the_turn_and_settles_back_to_playing` pins the state machine. Verified live
against a mock endpoint that reports honest `usage`: the gauge climbed turn by turn, a compaction
fired at 72% (**5 829 → 1 439 tokens, summarised**), and the run carried on for sixty more turns on
the compacted history. Default tier **1128** green, `full_playthrough` green (266 s).

### Token accounting
`Accounting` folds each response's `usage` in, with `Usage::estimate` as the fallback, and publishes
`UsageView` — occupancy, the run's cumulative prompt and completion totals, the number of completions
billed, and whether the figures are reported or estimated — on every `Decision` event.

### Status
`RunStatus`, exactly as the enum below, and it rides **both** the transition event
(`UiEventBody::Run`, sent only when the value changes) and every 10 Hz `StatusSnapshot`. That is the
answer to W1's deferred "no `watch<Status>`": the event is instant, the heartbeat is what a late
joiner reads, and neither needs W7's history endpoint.

```rust
enum RunStatus {
    Booting,                              // until the emulator runs its first cycle
    Playing,                              // agent driving, no decision pending
    AwaitingLlm { kind: &'static str },   // request sent, nothing back yet
    Streaming,                            // tokens arriving
    RunningTool { name: String },
    Compacting,
    RateLimited { retry_in_ms: u64 },
    Error { message: String },
}
```

### Compaction

Two stages, cheapest first, both triggered when occupancy crosses **70%** of `GB_CONTEXT_LIMIT`
(`worker::COMPACT_ABOVE`):

1. **Image eviction.** Every screenshot but the two most recent becomes its caption plus
   `[screenshot removed to save context]`. Often enough on its own, and it costs nothing.
2. **Summarising compaction.** One extra completion writes the story so far; everything except the
   system prompt and the last **8** messages is replaced by it. Skipped when it cannot help
   (`compaction::worth_summarising`).

If neither gets the history under the line, `Worker::trim_history` — W4's stopgap, now the last
resort — drops whole turns from the front. A `UiEvent::Compacted { before, after, images_evicted,
summarised }` reports what happened, and `Status::Compacting` shows it happening.

**Where the plan was wrong, or silent.**

1. ⚠️ **"70% full" has to mean the same thing before and after a message is removed, and it did not.**
   The reported figure counts the request that was *sent*; our own estimator counts the history as it
   *stands*, and the two differ by tens of percent. Deciding stage 2 on the estimate after deciding
   stage 1 on the report means a history the endpoint calls 90 k, whose estimate is 40 k, drops three
   screenshots to 39 k and never gets summarised. Every reported figure now calibrates the estimator
   (`Accounting::calibration`, clamped to 0.25–8×) and every decision is taken on that one scale.
2. ⚠️ **The summarisation request must carry no `tools` key *and* no `parallel_tool_calls`.** OpenAI
   rejects the latter outright when the former is absent — "only allowed when 'tools' are specified" —
   so `ChatRequest::parallel_tool_calls` became an `Option<bool>` that is omitted for this one request.
   A 400 here is worse than a 400 anywhere else: it is the request that exists to stop the *next* one
   failing.
3. ⚠️ **Eviction creates a cut point where there was none.** A picture is a `user` message that is not
   a turn boundary (W5); turning it into text makes it look exactly like one, in the middle of a turn.
   `compaction::is_turn_start` therefore excludes an evicted picture as well as a real one, and it is
   now the single definition used by the trim, the summary's tail cut and `pop_if_user`.
4. ⚠️ **The summary sits exactly where "drop the oldest turn" looks first.** Without
   `compaction::is_summary`, the last-resort trim eats the summary one turn after paying a completion
   for it — throwing away everything it stands for to save fifty tokens.
5. ⚠️ **A context limit smaller than the system prompt would summarise on every turn, forever**, each
   one making the history one message *longer*. `worth_summarising` requires more in the history than
   the system prompt, a summary and the tail that would be kept.
6. ⚠️ **Compaction is deliberately not cancellable**, unlike every other blocking point in the worker.
   Abandoning it leaves the history over the limit and makes the *next* request the one that fails.
   The cost is one completion's latency on a game that is not waiting for anything — the emulator runs
   throughout, as it does during any turn.
7. **The turn that filled the window survives the compaction**, because it is the most recent one and
   the tail is what stage 2 keeps. That is not a bug and not worth special-casing: the next turn's
   trim drops it once it is no longer the newest.
8. **`Playing` is set by the emulator loop exactly once**, on the first tick that emulates a cycle.
   Setting it every tick — the obvious reading — stamps on `AwaitingLlm` fifty times a second. After
   that the worker owns every transition, and an `Error` is left standing until the next turn starts,
   because a status that flicks straight back to `Playing` is one nobody ever sees.
9. **The summary's deltas are not published as `AssistantDelta`.** A thousand words of bookkeeping in
   the conversation pane reads as the model talking to itself.

## 10. Phase W6b — Memory and TODO ✅ **done**

Files on disk, in the run directory, so they survive both compaction and process restart.

```
$GB_RUN_DIR/<run-id>/
    memories/<slug>.md      # one note per file: frontmatter name + freeform body
    todo.json               # [{ id, text, done }]
```

**What shipped.** `src/llm/notes.rs` and four tools — `memory_write`, `memory_read`, `todo_add`,
`todo_complete` — offered on every decision kind and answered on the **worker thread**, like
`screenshot` and for the same reason: none of it needs the emulator, so a round trip through
`service_tools` would be a round trip for a file write. `memory_write` slugifies the name, caps the
body (8 KB) and the count (64); the TODO list caps at 64 items of 200 characters.

The **index** (names + first line of each) and the **entire TODO list** are re-rendered into the
system prompt every turn by `prompt::system_message`, so the model always knows what it knows without
spending a tool call; `memory_read` fetches a full body on demand.

This is the mechanism by which a run keeps long-horizon intent across compactions: "beat Brock" is a
TODO item, not something in the last 8 messages.

**Where the plan was wrong, or silent.**

1. ⚠️ **A note written in the same message as the terminal call was being thrown away.** §7.3's rule
   is that a message mixing reads with a terminal call ends the turn and the reads are *not run* — a
   read's answer is worthless once the turn is over. A note is not a question, it is a **side effect
   the model asked for**, and "remember this, and go north" is the most natural sentence in the
   world. Dropping half of it silently loses exactly the intent §10 exists to keep. Found within
   thirty seconds of pointing a mock at it: the very first turn wrote a TODO and it never appeared.
2. ⚠️ **A name from a model is not a filename.** `memory_write { name: "../../etc/passwd" }` is one
   tool call away at all times. `notes::slugify` reduces to `[a-z0-9-]`, and the test asserts on the
   *directory listing* rather than on the function, because that is the thing that must hold.
3. **The notes go in the system message, not a user one.** Index 0 is the one message compaction
   never touches (§9) — putting them anywhere else would mean the memory feature stops working
   exactly when it starts mattering.
4. **`Notes::open(None)` is a first-class mode**, not a test seam: everything works, nothing is kept.
   That is what the worker's own tests run against, and it means no calling code has to branch on
   whether there is a run directory.
5. **The TODO cap drops a *done* item to make room** rather than refusing. A run that finished sixty
   things an hour ago should not be unable to plan the sixty-first.

---

## 11. Phase W7 — Persistence and resume ✅ **done**

```
$GB_RUN_DIR/<run-id>/
    meta.json           # run id, model, started-at, last-checkpoint-at, emulated ms, resume history
    state.gbst          # GameBoy::save_state() — what a resume actually loads
    sram.bin            # dump_sram(), as an ordinary .sav for anything else that reads one
    transcript.jsonl    # one JSON object per UiEvent, append-only
    memories/  todo.json
```

**What shipped.** `src/run/mod.rs` (the directory, `meta.json`, atomic checkpoints, resume
discovery), `src/run/transcript.rs` (the writer thread and the backlog reader), the host's periodic
and shutdown checkpoints, `GET /api/history?since=`, `--new-run`, `GB_RUN_DIR`, and the SPA's
backfill on mount.

- **Checkpoint** every 60 s and on clean shutdown — **SIGTERM as well as Ctrl-C**, because `docker
  stop` sends the former and a container that handled only the latter would lose up to a minute of
  play on every deploy. The transcript is appended continuously, not checkpointed.
- **Resume** on startup: the newest directory holding a *loadable* `state.gbst` wins, unless
  `--new-run`. `GameBoy::load_state` applies to a clone, which is what makes it usable as the
  validity test — a corrupt checkpoint falls through to the next candidate and then to a fresh run,
  and says so, rather than refusing to start.
- **Backlog:** `GET /api/history?since=<seq>` returns the transcript from a sequence number, capped
  at the most recent 2 000 events. The SPA attaches to `/api/events` **first** and calls this second
  — the same subscribe-then-backfill ordering as the video path (§5.2).
- Transcript rotation at 256 MB; the LLM's own message history is capped by compaction, not by this.

**Acceptance — met.** `run::tests::*` cover the directory, the resume, the corrupt-checkpoint
fallthrough and the rename; `transcript::tests::*` cover the writer, the append-on-restart and the
backlog cap; `host::tests::a_checkpointed_run_resumes_where_it_stopped` plays, checkpoints, and
brings a second host up in the same place. Verified for real against a mock endpoint: a run played
for 90 s, took SIGTERM, wrote its checkpoint, and a second process printed `resuming run
run-…` and came up at 4:47 of in-game play time with its notes intact.

**Where the plan was wrong, or silent.**

1. ⚠️ **`UiEvent::seq` restarted at zero in the second process**, which quietly breaks the two things
   the sequence number is for: `?since=` selects across both ranges, and the browser keys its
   entries by it, so a resumed run renders duplicate keys. `Published::resuming` continues the
   numbering from `transcript::last_seq`. Found by reading the file after a restart, not by a test —
   which is why the test exists now.
2. ⚠️ **The status heartbeat is excluded from the transcript.** "One JSON object per `UiEvent`" taken
   literally is ten a second of a message whose whole purpose is to be current: 36 000 lines and
   ~14 MB an hour, drowning the conversation and making `/api/history` a replay of yesterday's clock.
   A viewer gets a fresh heartbeat within 100 ms of connecting.
3. ⚠️ **Every file is written by rename.** A SIGTERM landing inside a `state.gbst` write leaves a
   truncated file, and the *next* start is the one that fails — by which time the good copy is gone.
4. **A resume continues the run in place** rather than creating a new directory that points at the
   old one. A run resumed nightly is one directory, not thirty; `meta.json` keeps the list of resume
   times.
5. **`sram.bin` is an artifact, not part of the resume.** The save state already carries the
   cartridge RAM (`mmu`'s `ram_banks`), so a resume needs `state.gbst` alone; the `.sav` is there for
   anything else that wants to read the file.
6. **The first periodic checkpoint is one interval away, not immediate.** Otherwise a process that
   crash-loops rewrites its own save every few seconds.
7. **The run directory is resolved before the policy is built**, so a missing `OPENAI_API_KEY` is an
   error before a directory exists for a run that cannot start.

⚠️ **`Audio::set_output_sample_rate` is not serialised** and must be re-applied after every
`load_state` (`render.rs:154-159`). Irrelevant while audio is deferred, but the resume path is where
it will bite when §12 lands — `EmulatorHost::new` now carries the comment.

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
(`render.rs:224-229`). This section used to warn that `GB_PAUSE_WHILE_THINKING` would produce
audible gaps and was mutually exclusive with audio; that flag no longer exists (§2.1), and the
emulator now never stops while a run is in progress — which is exactly the property a continuous
audio stream needs.

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
| `web::video::tests::roundtrip_recorded_frames` ✅ | default | Pixel-exact reconstruction over 120 frames of real play |
| `web::video::tests::a_keyframe_catches_a_fresh_decoder_up_exactly` ✅ | default | §5.1's ⚠️ — a keyframe leaves a joiner holding the encoder's exact palette, and both stay in step across the deltas after it |
| `web::video::tests::palette_exhaustion_forces_a_keyframe` ✅ | default | The safety valve, and that a decoder joining *at* the reset lands in the same place |
| `web::video::tests::every_block_mode_is_chosen_when_it_is_the_smallest` ✅ | default | All three modes, and that a real four-shade frame splits RLE/packed and never needs raw |
| `web::video::tests::a_frame_that_overflows_the_palette_degrades_without_desynchronising` ✅ | default | The length byte does not wrap at the 255 cap, and a lossy frame does not re-emit forever |
| `web::video::tests::a_corrupt_message_is_an_error_not_a_panic` ✅ | default | Truncation at every length, a bad version, an unsent palette index |
| `web::published::tests::late_joiner_never_misses_a_delta` ✅ | default | §5.2's subscribe-then-keyframe ordering, looped over the size of the race window |
| `web::published::tests::events_are_numbered_from_zero_and_reach_a_subscriber` ✅ | default | `UiEvent` sequencing, which W7's `/api/history?since=` replays from |
| `host::tests::the_host_publishes_a_moving_game_state` ✅ | default | W1's acceptance without a socket: heartbeats arrive, carry a game state, and the player moves |
| `host::tests::the_host_publishes_decodable_video` ✅ | default | What the host publishes decodes back to the emulator's own frame buffer, and the frame snapshot and keyframe describe the same moment |
| `web::assets::tests::sanitise_keeps_asset_paths_and_rejects_everything_else` ✅ | default | The traversal defence from both directions — `../`, absolute paths and a `.` component are rejected, hashed asset paths survive |
| `web::assets::tests::index_is_always_a_page` ✅ | default | `/` is HTML and `no-cache` whether or not `npm run build` ran before `cargo build`, and takes the right branch of the two |
| `web::assets::tests::a_missing_asset_is_a_404_rather_than_the_index` ✅ | default | No SPA fallback: there is no client-side router, so an unknown path is a genuine 404 |
| `pokemon::badge_gfx::tests::eight_distinct_badges_come_out_of_the_rom` ✅ | default | The ROM offsets — being one tile out still yields a plausible 16×16 sprite, of half a gym leader's face |
| `pokemon::badge_gfx::tests::the_quadrants_are_four_consecutive_tiles` ✅ | default | The 2×2 assembly, re-read tile by tile without trusting the decoder |
| `pokemon::badge::tests::badges_are_declared_in_bit_order` ✅ | default | `Badge::ORDER` is bit order — the sprite sheet is indexed by it, and lighting badge 3 for bit 4 would look plausible |
| `web::badges::tests::the_sheet_is_eight_distinct_badges_side_by_side` ✅ | default | Sheet geometry (what `background-position` slices), a transparent background, and eight different sprites |
| `mechanics::the_badge_strip_reports_which_badges_not_only_how_many` ✅ | default | Against the post-Earth-Badge fixture, where the answer is known — a mapping that returns `false` eight times passes on a fresh save |
| `llm::protocol::tests::parses_fragmented_tool_call_arguments` ✅ | default | §7.1's ⚠️ — arguments split across SSE chunks reassemble, split mid-key and mid-string |
| `llm::protocol::tests::parallel_tool_calls_are_kept_apart_even_interleaved` ✅ | default | Two calls interleaved by `index`, with a fragmented *name* as well as fragmented arguments |
| `llm::protocol::tests::a_minimal_endpoint_still_yields_a_usable_call` ✅ | default | §17 risk 3 — no `index`, no id, no `usage`: still a call with an id, and the estimator stands in |
| `llm::protocol::tests::indexless_calls_split_on_a_new_id` ✅ | default | …and two indexless calls are not concatenated into one |
| `llm::protocol::tests::{non_data_lines_are_ignored, a_mid_stream_error_frame_is_an_error, a_corrupt_chunk_is_an_error, read_stream_stops_the_moment_a_turn_is_cancelled}` ✅ | default | Keep-alives, an error inside a 200, truncation, and the per-line cancel point |
| `llm::protocol::tests::the_request_serialises_to_the_documented_shape` ✅ | default | `stream_options`, `parallel_tool_calls`, and no `null` where a key should be absent |
| `llm::client::tests::*` ✅ | default | Backoff doubles and caps; 429 retried **and reported**; 400 not retried; cancellation beats backing off; a persistent fault gives up |
| `llm::config::tests::*` ✅ | default | §7.1's block: the two required variables name themselves, a trailing slash does not double up |
| `llm::tools::tests::terminal_tools_are_scoped_per_kind` ✅ | default | §7.5 — a battle turn's `tools` array omits `choose_action`, and vice versa, and the contract matches the array |
| `llm::tools::tests::every_schema_is_a_well_formed_object` ✅ | default | A malformed schema is a 400 on the first turn of a run |
| `llm::tools::tests::a_terminal_tool_from_the_wrong_kind_is_rejected_with_the_right_one` ✅ | default | It is a message to the model, not a dead turn |
| `llm::tools::tests::a_battle_id_ignores_the_volatile_parts` ✅ | default | §7.4 — a battle id survives the PP that `BattleAction`'s `Display` carries |
| `llm::prompt::tests::the_contract_names_every_tool_the_turn_is_actually_sent` ✅ | default | §7.5's second/third lines of defence cannot drift from the first |
| `llm_policy::tests::one_decision_point_is_one_turn_and_its_answer_is_executed` ✅ | default | The re-issue guard: fifty polls a second, one turn, and the action lands |
| `llm_policy::tests::a_kind_change_cancels_the_turn_in_flight` ✅ | default | §7.2 — an overworld turn dies when a battle starts, and the battle decision is what lands |
| `llm_policy::tests::a_cancelled_batch_leaves_the_history_well_formed` ✅ | default | §7.3's one-step rollback: no `tool_call` left without a result |
| `llm_policy::tests::a_parallel_read_batch_is_answered_from_one_observation` ✅ | default | All-or-nothing, from one `GameState`, in one poll |
| `llm_policy::tests::a_reply_with_no_tool_call_is_nudged_once_then_forced_to_wait` ✅ | default | §7.5's fallback, and the marker event that makes it a visible rate |
| `llm_policy::tests::field_move_polls_do_not_cancel_the_overworld_turn` ✅ | default | `pick_field_move` shares the `Overworld` kind and never pre-empts |
| `llm_policy::tests::an_unresolvable_id_is_explained_on_the_next_turn` ✅ | default | §7.4's ⚠️ — a stale id is a message, not a panic and not a silent no-op |
| `integration_tests::llm::the_llm_plays_from_a_fixture` ✅ | default | A **mock OpenAI server** (axum, in-process, real socket, real SSE with fragmented arguments) serves a scripted tool-call sequence; the agent executes it from a committed fixture |
| `cli::tests::gb_port_is_the_default_and_the_flag_overrides_it` ✅ | default | §7.1's `GB_PORT`, and that a nonsense one refuses to start |
| `llm::protocol::tests::an_image_rides_on_a_user_message_in_the_multi_part_form` ✅ | default | §8's ⚠️ — the shape that goes on the wire, that it round-trips, and that an ordinary message's content stays a bare string |
| `llm::protocol::tests::an_image_is_estimated_by_the_flat_rate_rather_than_by_its_length` ✅ | default | A 40 kB data URL must not be charged as 40 kB of prose |
| `llm::screenshot::tests::a_frame_becomes_a_data_url_holding_the_same_picture` ✅ | default | The prefix an `image_url` needs, and that the 3× upscale puts the frame's own pixels where it claims |
| `llm::tools::tests::terminal_tools_are_scoped_per_kind` ✅ | default | Extended to all five kinds: the three menu prompts offer their one tool and `wait`, and never `choose_action` |
| `llm::tools::tests::a_field_move_call_parses_into_the_move_it_names` ✅ | default | Every `use_field_move` argument shape, and the sentence each malformed one earns |
| `llm::tools::tests::a_field_move_is_resolved_against_the_party_and_the_bag_it_needs` ✅ | default | `cut` checks the tile in front; a move nobody knows, an empty slot and an item not held are all messages |
| `llm::tools::tests::a_party_field_moves_index_is_computed_from_the_moves_it_knows` ✅ | default | ⚠️ The field-move box is in move-slot order, so index 0 is right only for an HM slave |
| `llm::tools::tests::names_are_matched_the_way_a_model_spells_them` ✅ | default | `"HM01 Cut"` = `"hm01_cut"` = `Hm01Cut`; `"a potion of healing"` is still not an item |
| `llm::tools::tests::press_buttons_parses_a_sequence_and_refuses_what_is_not_one` ✅ | default | A bad button is a rejection, not a silently shorter queue; the agent's own cap is the cap |
| `llm::tools::tests::omitting_the_argument_is_an_answer_for_the_three_menu_prompts` ✅ | default | No nickname / no purchase / no move forgotten are real answers, not malformed calls |
| `llm::tools::tests::a_screenshot_is_classified_apart_from_the_other_reads` ✅ | default | It never reaches the emulator thread, and is still offered as a read |
| `llm_policy::tests::a_field_move_decision_is_collected_by_the_next_field_move_poll` ✅ | default | Both halves of the stash: the overworld poll answers `None`, the next field-move poll hands it over once |
| `llm_policy::tests::an_impossible_field_move_is_explained_rather_than_attempted` ✅ | default | Nothing reaches the agent, and the model is told why on its next turn |
| `llm_policy::tests::press_buttons_leaves_the_presses_for_the_agent_to_collect` ✅ | default | The policy half of the escape hatch, and that a collected press is not queued twice |
| `llm_policy::tests::the_menu_prompts_are_their_own_turns_and_can_use_read_tools` ✅ | default | §8's ⚠️ 2 — a read *during* a naming screen is answered, not cancelled into a restart loop |
| `llm_policy::tests::a_mart_turn_answers_with_a_purchase` ✅ | default | The stock comes from `ApiSnapshot`; nothing in `GameState` has it |
| `llm_policy::tests::a_forget_prompt_pre_empts_the_battle_turn_it_interrupts` ✅ | default | §7.2's ⚠️ — the battle turn is cancelled, and the four known moves are the menu |
| `llm_policy::tests::a_forget_slot_the_pokemon_does_not_have_declines_instead_of_hanging` ✅ | default | A cursor sent to a fifth move slot never arrives |
| `mechanics::a_policy_can_ask_for_a_raw_press_and_the_agent_delivers_it` ✅ | default | W5's pull seam end to end: the START menu opens without the test touching `queue_manual_input` |
| `llm::compaction::tests::eviction_keeps_the_two_most_recent_pictures_and_costs_nothing_else` ✅ | default | Stage 1: which pictures go, that the caption stays, that a plain string is left behind, and that it saves what it claims |
| `llm::compaction::tests::an_evicted_picture_is_never_a_cut_point` ✅ | default | §9's ⚠️ 3 — eviction must not turn a mid-turn message into a legal boundary |
| `llm::compaction::tests::a_summary_replaces_the_middle_and_leaves_a_well_formed_history` ✅ | default | Stage 2, and the invariant the endpoint enforces with a 400: every surviving `tool` message still has its call |
| `llm::compaction::tests::the_tail_starts_at_a_turn_and_never_in_the_middle_of_one` ✅ | default | Looped over every tail length, including the ones shorter than one turn |
| `llm::compaction::tests::summary_restates_turn_contract` ✅ | default | §9's ⚠️ — the contract survives a compaction, in the same words the turn contract uses |
| `llm::compaction::tests::a_summary_is_recognisable_and_a_short_history_is_not_worth_one` ✅ | default | §9's ⚠️ 4 and 5 — the trim can spot the summary, and a history too short to gain from one is left alone |
| `llm::compaction::tests::a_summary_request_asks_for_prose_and_offers_no_tools` ✅ | default | §9's ⚠️ 2 — no `tools`, and therefore no `parallel_tool_calls` |
| `llm::compaction::tests::degenerate_histories_survive_a_compaction` ✅ | default | No system prompt, nothing at all, a history shorter than the tail |
| `llm::accounting::tests::the_estimator_is_calibrated_against_what_the_endpoint_reported` ✅ | default | §9's ⚠️ 1 — one scale, before and after a message is removed |
| `llm::accounting::tests::{totals_accumulate…, an_endpoint_that_reports_nothing…, an_absurd_ratio_is_clamped}` ✅ | default | The bill against the gauge; the W4 degradation path; a lying endpoint |
| `web::published::tests::a_status_is_broadcast_on_transition_and_only_on_transition` ✅ | default | The silence matters: `set_status` is called from loops running at 50 Hz |
| `web::published::tests::a_run_status_serialises_flat_with_a_state_discriminator` ✅ | default | The wire shape `api.ts` is hand-written against |
| `llm_policy::tests::the_run_status_follows_the_turn_and_settles_back_to_playing` ✅ | default | §9's state machine end to end, including that it comes back to `Playing` |
| `llm_policy::tests::a_full_context_is_summarised_and_the_next_turn_carries_the_summary` ✅ | default | A real compaction through the real worker: summarised, cheaper, well-formed, and the contract still in the history |
| `run::tests::a_fresh_run_becomes_a_resumable_one` ✅ | default | §11 — the directory, `meta.json`, and a checkpoint that comes back |
| `run::tests::a_corrupt_checkpoint_falls_through_to_a_fresh_run` ✅ | default | §11's rule: an unloadable state is not a reason to refuse to start, and a good one beside it still wins |
| `run::tests::{new_run_starts_beside_the_old_one…, a_checkpoint_is_written_by_rename, run_ids_do_not_collide_within_a_second}` ✅ | default | `--new-run` leaves the old run untouched; the `.tmp`-then-rename; two runs in one second |
| `run::tests::the_clock_agrees_with_a_calendar` ✅ | default | The twelve lines of date arithmetic that replace a `chrono` dependency — a leap day, and a century that is not one |
| `run::transcript::tests::the_story_is_written_and_the_heartbeats_are_not` ✅ | default | §11's ⚠️ 2, and the sequence number a restart continues from |
| `run::transcript::tests::a_second_process_appends_rather_than_starting_again` ✅ | default | The transcript is the one thing in the directory that is not a snapshot |
| `run::transcript::tests::the_backlog_is_capped_at_the_most_recent_events` ✅ | default | A month-old run must not make a page load allocate the file |
| `host::tests::a_checkpointed_run_resumes_where_it_stopped` ✅ | default | W7's acceptance without a restart: a second host, given only the directory, comes up in the same place |
| `host::tests::a_heartbeat_that_says_nothing_new_is_not_sent` ✅ | default | Every heartbeat sent says something the one before did not — sampled far faster than anything can change, so every suppression is exercised |
| `host::tests::an_idle_run_still_sends_a_keepalive` ✅ | default | …and a game that is not moving still proves it is alive |
| `published::tests::a_heartbeat_is_the_same_as_another_when_only_the_clock_has_moved` ✅ | default | ⚠️ A derived `PartialEq` would never match and the suppression would silently never fire |
| `published::tests::a_joiner_is_handed_the_last_heartbeat_rather_than_an_empty_panel` ✅ | default | The other half of send-on-change, and that it is one shared cell rather than a buffer per client |
| `cli::tests::the_usage_names_every_flag_and_variable` ✅ | default | ⚠️ `--new-run` shipped without ever appearing in `--help`; for a tool discovered through `--help` that is a flag that does not exist |
| `published::tests::events_are_numbered_from_zero…` ✅ (extended) | default | §11's ⚠️ 1 — a resumed process continues the numbering |
| `llm::notes::tests::notes_survive_the_process_that_wrote_them` ✅ | default | §10 end to end: written, indexed into the system prompt, read back after a reopen |
| `llm::notes::tests::a_name_from_a_model_can_never_escape_the_directory` ✅ | default | §10's ⚠️ 2 — asserted on the directory listing, not on the function |
| `llm::notes::tests::{the_caps_hold_and_say_why, notes_without_a_directory_still_answer}` ✅ | default | The caps and their messages; `Notes::open(None)` as a first-class mode |
| `llm::tools::tests::terminal_tools_are_scoped_per_kind` ✅ (extended) | default | Every kind is offered the four note tools as well as the reads |
| `prompt::tests::the_contract_names_every_tool_the_turn_is_actually_sent` ✅ (extended) | default | The notes reach the system message, and the contract names them as non-terminal |
| `cli::tests::new_run_is_a_switch_and_not_a_setting` ✅ | default | It must not swallow the flag after it |
| `mechanics::manual_input_preempts_state_machine` ✅ | default | W0.4 — a queued press fires and resets to `Idle` |
| `mechanics::policy_receives_text_events` ✅ | default | W0.3 — `on_event` sees `TextBox` |
| Existing suite | all tiers | Unchanged |

`llm_policy::tests::idle_poll_is_allocation_free` was dropped rather than written: what it was
guarding — one decision point becoming fifty turns — is asserted directly by
`one_decision_point_is_one_turn_and_its_answer_is_executed`, and "allocation free" is not true of the
poll anyway (`service_tools` refreshes the `ApiSnapshot` before a turn starts) nor worth making true
at 50 Hz against a 90×-realtime emulator (W0.3b).

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
| **W1** ✅ | `EmulatorHost` thread · shared published state · axum skeleton · `/api/events` status SSE | ✅ `curl` shows status ticking, map changing |
| **W2** ✅ | Block-diff encoder (three modes) + JS decoder · `/api/video` | ✅ Round-trip tests; game visible in the dev page. 8 kbit/s idle, 536 kbit/s walking — see §5.1's ⚠️ |
| **W3** ✅ | Vite/React SPA · embedded via `rust-embed` · screen + status + conversation shell | ✅ Full UI under `--policy random`, verified headlessly including reconnect |
| **W4** ✅ | OpenAI client (streaming, tool calls) · `LlmPolicy` · kind-keyed turns + cancellation · overworld + battle decisions · the read tools | ✅ LLM plays from the start of the game against a mock endpoint; conversation, tool calls and decisions stream into the SPA |
| **W5** ✅ | The rest of the tool surface: `screenshot`, raw buttons, field moves, nickname/mart/forget | ✅ Mock-server test asks for a read and a screenshot in one message and checks the PNG that came back; `full_playthrough` green |
| **W6** ✅ | Token accounting · status broadcast · two-stage compaction | ✅ Compaction tests, plus a live run against a mock endpoint that compacted at 72% and carried on |
| **W6b** ✅ | Memory and TODO (§10): four tools, a note directory, a TODO list, both rendered into the system prompt every turn | ✅ Notes survive a compaction and a restart; verified live |
| **W7** ✅ | Run directory · checkpoint/resume · SIGTERM · transcript · `/api/history` · `--new-run` | ✅ Survives a restart mid-run: SIGTERM, checkpoint, second process resumes at 4:47 of play with its notes |
| **W8** | Multi-stage Dockerfile · no-SDL build · ops config | Image builds and runs |
| **W9** | Stuck-run watchdog — lenient, last resort, loud (§14) | Fires on a deliberately jammed agent |
| *(deferred)* | Audio streaming (§12) | — |

W0–W3 are independent of any LLM and are worth shipping on their own: they give a browser-watchable
emulator with the existing policies. W4 is where the actual subject of this plan begins.

**What W8 inherits.** W6, W6b and W7 all shipped, so a run now bounds its own context, keeps its own
notes, and survives the process being restarted. The container has one job left that is really its
own: `GB_RUN_DIR` wants to be a volume, and `docker stop`'s SIGTERM is already handled.

Still open, and neither is a phase:

- **The per-run token ceiling (§17's risk 4).** The accounting is there —
  `UsageView::{prompt_tokens, completion_tokens, completions}` — so it is a limit and a halting
  `RunStatus`.
- **Cancellation churn as a number (§17's risk 2b).** `TurnCancelled` is an event; nothing counts it.

`Worker::trim_history` survives as the last resort behind both compaction stages rather than as the
whole of the strategy, and `is_turn_start` moved to `compaction` where the rules about what may be
cut now live together.

**What W3 inherited.** `/` served `web/dev/video.html` via `DEV_PAGE` in `src/web/mod.rs`; it is now
`rust-embed` over `web/dist` (`src/web/assets.rs`) and the file is gone. Its JS decoder was ported to
`web/src/video.ts` rather than rewritten — it was the only version that had been checked against a
real stream, and the port was re-checked the same way.

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
   multiply it. Worth adding a per-run token ceiling that stops the run rather than a surprise bill.
   **W6's accounting is now there** — `UsageView` carries the cumulative prompt and completion totals
   and the number of completions billed — so what is left is a limit and a halting `RunStatus`.
5. **`CLAUDE.md` references deleted docs.** `docs/` was removed wholesale in `1aa9141`, but
   `CLAUDE.md` still points at `docs/compatibility/10-implementation-plan.md`,
   `docs/postgame-coverage-plan.md` and others in several places. Unrelated to this work, but it will
   confuse the next reader of this file — worth a separate cleanup commit.
