# gb

A Game Boy emulator written in Rust, repurposed as a platform for an LLM to play Pokémon Red —
entirely through text, with no screenshots required.

The emulator half is a real one: DMG and Game Boy Color, full CPU, PPU, APU, timer, DMA, interrupt
and joypad emulation, accurate enough to pass the standard hardware-compatibility test ROMs. The
other half reads the game's own memory — party, bag, map, battle state, on-screen text — using
symbols lifted from the [pokered](https://github.com/pret/pokered) disassembly, and drives the game
by synthesising joypad input. What the model sees is a description of where it is and what it can
do; what it sends back is an action, not a button press.

It runs headless, serves its own web UI, and keeps a run going across restarts.

```
docker run -d -p 8080:8080 -v gb-runs:/runs \
  -e OPENAI_API_KEY=sk-… -e GB_MODEL=gpt-5 ghcr.io/axle-h/gb:latest
```

Then open <http://localhost:8080> and watch it play.

## What works

**The emulator.** Blargg's `cpu_instrs` and `dmg_sound` (including both combined suite ROMs) and
`instr_timing`; the `dmg-acid2` and `cgb-acid2` PPU tests. Six memory bank controllers — `RomOnly`,
MBC1, MBC2, MBC3, MBC5, HuC1 — with the MBC3 real-time clock; 27 of mooneye's 28 MBC test ROMs pass,
the exception being MBC1 multicart, which is not implemented. A mapper `gb` cannot emulate fails
with a typed error rather than quietly running as something else.

**Game Boy Color**, as a first-class model rather than a coat of paint: VRAM/WRAM banking, palette
RAM, BG map attributes, OAM-index sprite priority, KEY1 double speed, HDMA/GDMA. A DMG-only
cartridge on a CGB gets compatibility mode including the boot ROM's title-derived palette, which is
why Pokémon Red comes out red-tinted here exactly as it does on real hardware.

**The game.** The agent layer can play Pokémon Red from a fresh save to all eight badges and beyond;
`full_playthrough` is a test that does exactly that in about five minutes of wall clock, because the
emulator runs at roughly 50× real time with the agent on top of it.

**The LLM layer** drives that same agent over any OpenAI-compatible API. Its end-to-end tests run
against a mock server — a whole playthrough's worth of turns, cancellation and compaction included —
so what is proven here is the plumbing rather than any particular model's ability to actually finish
the game.

## Quick start

The container is the shortest path and needs nothing installed — see above. To build it yourself:

```shell
git clone --recursive https://github.com/axle-h/gb.git && cd gb
```

You need a Rust toolchain, [rgbds](https://rgbds.gbdev.io) ≥ 1.0.0 to assemble the cartridge, Node
with pnpm for the browser UI, and SDL2 if you want the desktop window.

```shell
# 1. The cartridge. `pokered/pokered.gbc` is embedded into the binary at compile time and
#    `pokered/pokered.sym` is parsed by build.rs, and neither is in git.
make -C pokered pokered.gbc

# 2. The browser UI. `web/dist` is baked into the binary, so this comes before cargo.
cd web && pnpm install && pnpm run build && cd ..

# 3. The binary.
cargo build --release
```

Then pick a way to run it:

```shell
# The web UI, played at random — no API key, no spend. http://localhost:8080
cargo run --release -- serve --policy random

# The web UI, played by a model.
OPENAI_API_KEY=sk-… GB_MODEL=gpt-5 cargo run --release -- serve

# The SDL desktop window, with the game driven from your keyboard and the policy from stdin.
cargo run --release
```

`gb serve` **resumes** by default: the newest run under `$GB_RUN_DIR` (`./runs`) is continued in
place, notes and all. `--new-run` starts the game over in a directory of its own — or, on something
already running, opening `/reset-game` does the same thing without a restart (see below).

## How the model plays

`PokemonAgent` advances the emulator 20 ms at a time and works out, from the game's memory, what
kind of decision is on the table — an overworld move, a battle turn, a nickname, a menu. It then
asks a `Policy`. The trait is non-blocking: every method returns `Option`, so the emulator keeps
running while the model thinks, which is the property everything else here is built around.

```rust
fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction>;
fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;
fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>>;
```

`LlmPolicy` is the interesting implementation. It turns each decision into a conversation turn with
a tool catalogue scoped to that kind of decision — read tools that inspect the party, the bag, the
map or the screen, and one terminal tool that commits to the action. The model keeps its own
notes: a `memories/` directory and a TODO list it edits itself, rendered back into the system prompt
each turn, which is how a playthrough spanning thousands of turns keeps a thread. History is
compacted when it grows: images are evicted first, then older turns are summarised.

Anything the model does not decide, the agent handles: dialogue is advanced, menus are navigated,
paths across the map are computed from a graph of all 248 maps built out of the ROM's own headers.

A **watchdog** covers the one failure nothing else can see — the agent reaching no decision point at
all, so the policy is never consulted and cannot notice it is stuck. After
`GB_STUCK_TIMEOUT_SECS` of emulated silence (300 by default; ordinary play's longest gap is about
six seconds) the model is asked for a nudge, and every firing is reported to the UI, the transcript
and stdout.

The other policies are `RandomPolicy`, `ConsolePolicy` (stdin, for the desktop UI) and
`DeterministicPolicy` (scripted, used by the tests — including the full playthrough).

## The run directory

Everything a run needs is one directory, `$GB_RUN_DIR/<run-id>/`:

| | |
|---|---|
| `meta.json` | run id, model, when it started |
| `state.gbst` | the save state — the emulator, exactly as it was |
| `sram.bin` | the cartridge's battery-backed save |
| `transcript.jsonl` | every event, appended; what `/api/history` replays into a page that just loaded |
| `memories/`, `todo.json` | the model's own notes |

Copy that directory and the run moves with it. `gb` checkpoints periodically and on the way out —
Ctrl-C and SIGTERM both — so a restart, a rollout or a reboot resumes rather than starts over.

Beside the runs is `$GB_RUN_DIR/hall-of-fame/`: a copy of every run that has finished the game, and
an append-only `ledger.jsonl` of one line each. See below.

## The web UI

`web/` is a Vite + React + TypeScript SPA, embedded into the binary by `rust-embed` and served by
the same process that runs the emulator. Seven read-only endpoints and two that are not:

| | |
|---|---|
| `/api/events` | SSE: status heartbeat, published on change, plus agent events as they happen |
| `/api/video` | binary: a keyframe, then 8×8 block deltas, deflated per connection — about 21 kbit/s |
| `/api/history?since=` | the transcript backlog, so a page that just loaded is not empty |
| `/api/leaderboard?limit=` | the runs that have finished the game, fastest first |
| `/api/badges.png` | the eight gym badges, decoded from the cartridge's own trainer-card graphics |
| `/api/pokemon/{dex}/front.png` | one Pokémon's battle sprite, decompressed from the cartridge |
| `/favicon.png` | the overworld Poké Ball, ditto |
| `/api/healthz` | liveness |
| `/reset-game` | start the game over, in place — HTTP Basic, off unless `GB_ADMIN_TOKEN` is set |
| `POST /api/new-run` | the same thing for a script, with an `X-GB-Token` header |

The screen is streamed as block deltas rather than as images because it is a 160×144 screen that
mostly does not change; the decoder is a TypeScript port of the encoder, in `web/src/video.ts`.

**No graphics are committed to this repo.** The badges, the party sprites and the favicon are all
read out of the ROM at run time. The Pokémon sprites are the interesting ones: Gen 1 pics are
compressed, so `src/pokemon/mon_gfx.rs` is a port of pokered's `UncompressSpriteData` — a bitstream
of two 1bpp planes, run-length-encoded zeros and an XOR delta between the planes. All 151 are checked
byte-for-byte against upstream's own build output.

`/api/video` is the one endpoint that is not SSE, and `src/web/video/bench.rs` is why. Measured
against four minutes of real play, the SSE version cost **565 kbit/s**; the "19" this file used to
claim was an idle screen, and an idle screen costs nothing at all. Three changes took that to **21**:
two bits per pixel against the stream's own palette rather than a per-block sub-palette, one deflate
stream across the whole connection rather than one per message (worth 5×, because a Game Boy screen
is repeated 8×8 tiles and a shared window sees every repeat), and dropping base64 — which costs 33%
before compression but 69–113% *after* it.
For comparison, the same footage through x264 is 45 kbit/s losslessly and 25 at a quality that
visibly mangles pixel art, so a real video codec was measured and rejected rather than assumed away.

### When a run finishes the game

A win is one byte: `wNumHoFTeams`, which pokered increments on the **first frame** of the Hall of Fame
ceremony — before the party parade, the credits, and the game's own save-and-soft-reset back to the
title screen. That is the moment of victory, with the winning party still in memory, so that is where
the record is taken.

What happens then, in order: the run is checkpointed, copied whole into
`$GB_RUN_DIR/hall-of-fame/<date>-<run-id>/` — save state, SRAM, the model's notes and the run's entire
transcript, gzipped — one line describing it is appended to `hall-of-fame/ledger.jsonl`, and the next
run starts automatically. Nothing is deleted: the finished run directory is left exactly where it was
and is still resumable.

The ledger row is the run's whole story in numbers: how long it took by the cartridge's own clock and
by ours, tokens spent, turns taken, which policy and model decided them, which version of `gb` played
it, how many times it was resumed, and what it finished with. `/api/leaderboard` reads it back and the
🏆 in the page's header shows the top ten, **fastest by in-game time** — the one figure that survives
a resume without any bookkeeping, because it lives in the save file.

### Starting a new run without a restart

Open **`/reset-game`** and the browser asks for a password — any user name, `GB_ADMIN_TOKEN` as the
password. The current run is checkpointed and left complete on disk, and the game starts again in a
fresh run directory: no restart, no downtime, and every open page follows on its own.

The page itself has no button and nothing links to that URL. A `WWW-Authenticate` challenge is the
browser's own dialog, so the SPA holds no token and needs no prompt; and a GET that resets the game
should not be reachable by a prefetch, a crawler or a middle-click. It is a URL you type.

For a script, the same thing with a header:

```shell
curl -X POST -H "X-GB-Token: $GB_ADMIN_TOKEN" https://your-host/api/new-run
# → {"run_id":"run-20260811-142233"}
```

Both are **off unless `GB_ADMIN_TOKEN` is set**, and both 404 rather than 403 when it is not — the
server is on the public internet and an endpoint that resets the game should not advertise itself to
whoever scans for it. Nothing is deleted: the old directory is a complete run and can be resumed by
pointing a process back at it.

For a hot-reload loop, run `gb serve` on :8080 and `pnpm run dev` in `web/`, which proxies `/api` to
it. `GB_WEB_DEV=1` reads `web/dist` from disk instead of the embedded copy, which skips the cargo
rebuild after an SPA build.

## Configuration

All environment variables, never flags — the API key has to be one, so the rest followed it.
`gb --help` lists them and `src/llm/config.rs` documents them.

| | |
|---|---|
| `OPENAI_API_KEY` | required for `--policy llm` |
| `GB_MODEL` | required for `--policy llm` |
| `OPENAI_BASE_URL` | any OpenAI-compatible endpoint |
| `GB_CONTEXT_LIMIT` | tokens of history before the turn loop compacts |
| `GB_TEMPERATURE`, `GB_MAX_TOOL_STEPS` | the turn loop's shape |
| `GB_STUCK_TIMEOUT_SECS` | the watchdog; `0` turns it off |
| `GB_RUN_DIR` | where runs live (default `./runs`) |
| `GB_PORT`, `GB_STATUS_HZ` | the server |
| `GB_ADMIN_TOKEN` | enables `/reset-game` and `POST /api/new-run`; unset means both 404 |

## Deployment

The `Dockerfile` builds everything from a bare checkout in four stages — rgbds and the cartridge,
then the SPA, then the crate, then a 146 MB image of which the binary is 6.9 MB. `ghcr.io/axle-h/gb`
is published by CI on every push to main, after a smoke test that proves the image actually serves
and emulates.

```shell
OPENAI_API_KEY=sk-… GB_MODEL=… docker compose up -d
docker compose run --rm --service-ports gb gb serve --policy random   # no API key, no spend
```

`k8s/` has manifests for k3s — one namespace, one pod, a volume for the run directory, and TLS
terminated outside by traefik and cert-manager. See [`k8s/README.md`](k8s/README.md).

## Architecture

```
src/
├── main.rs              — entry point: `gb` (SDL UI) or `gb serve` (web), dispatched from cli.rs
├── cli.rs               — hand-rolled arg parsing; `parse` is unit-testable without a process
├── host.rs              — headless emulator host: GameBoy + PokemonAgent + video encoder on one thread
├── run/                 — the run directory (`web` feature): checkpoint, resume, transcript
│   ├── mod.rs           — $GB_RUN_DIR/<run-id>/: meta.json, state.gbst, sram.bin; atomic writes
│   ├── transcript.rs    — transcript.jsonl writer thread + the /api/history backlog reader
│   └── hall_of_fame.rs  — a finished run: the archive, the ledger, and the leaderboard read back
├── web/                 — the axum server (`web` feature); read-only but for the reset
│   ├── published.rs     — the only interface between the emulator thread and HTTP
│   ├── video.rs         — 8×8 block-diff video codec + the reference decoder
│   │   └── bench.rs     — what the stream costs, and every alternative it was chosen over
│   ├── assets.rs        — the SPA: `web/dist` embedded, or read from disk under GB_WEB_DEV=1
│   ├── badges.rs        — /api/badges.png: the eight badges, decoded from the cartridge
│   ├── leaderboard.rs   — /api/leaderboard: the runs that have finished the game
│   └── sprites.rs       — /api/pokemon/{dex}/front.png and /favicon.png, ditto
├── llm/                 — the LLM client and turn loop (`llm` feature)
│   ├── config.rs        — the environment block: OPENAI_*, GB_MODEL, GB_MAX_TOOL_STEPS, …
│   ├── protocol.rs      — OpenAI wire types + the SSE accumulator (no HTTP; pure and testable)
│   ├── client.rs        — `ChatEndpoint` + `OpenAiClient` over ureq, and the retry policy
│   ├── tools.rs         — the tool catalogue, scoped per decision kind; ids; servicing
│   ├── prompt.rs        — the system prompt and the per-turn situation
│   ├── screenshot.rs    — one published frame as a PNG data URL, encoded on the worker thread
│   ├── accounting.rs    — tokens reported vs tokens estimated, and the calibration between them
│   ├── notes.rs         — the model's memory files and TODO list, rendered into the system prompt
│   ├── compaction.rs    — image eviction + summarising compaction, as pure functions over the history
│   └── worker.rs        — the turn loop: stream → tool batch → terminal call, with cancellation
├── game_boy.rs          — top-level GameBoy struct (run loop, save/restore)
├── core.rs              — CPU + MMU wiring
├── opcode.rs            — full SM83 instruction set
├── mmu.rs               — memory map, bank switching, the absolute clock (`now`)
├── mbc.rs               — memory bank controllers (RomOnly/MBC1/2/3/5/HuC1), dispatched from CartType
├── schedule.rs          — event schedule: when each peripheral next does something
├── ppu.rs               — pixel processing unit (LCD rendering)
├── model.rs             — Model (Dmg/Cgb) + ColorMode (Dmg/CgbCompat/Cgb)
├── cgb_palette.rs       — CGB palette RAM (BCPS/BCPD, OCPS/OCPD)
├── boot_palette/        — the CGB boot ROM's DMG-compatibility palette tables
├── hdma.rs              — CGB VRAM DMA (GDMA + HBlank-paced HDMA)
├── audio/               — APU (4-channel Game Boy audio)
│   └── blip/            — band-limited synthesis + resampling to the sink's rate (Blip_Buffer port)
├── sdl/                 — SDL2 UI: renders LCD at 4× scale, drives audio, keyboard input
│   └── render.rs        — main render loop; instantiates GameBoy + PokemonAgent
├── roms/                — bundled test ROMs (cpu_instrs, dmg-acid2, cgb-acid2, etc.)
└── pokemon/             — Pokémon Red layer (everything below)
    ├── mod.rs            — PokemonApi / PokemonApiTrait / GameState
    ├── agent.rs          — PokemonAgent: drives joypad each frame, emits AgentEvents
    ├── policy.rs         — Policy trait + impls (Random, Console/stdin, Deterministic)
    ├── llm_policy.rs     — LlmPolicy: kind-keyed turns, cancellation, tool servicing (`llm` feature)
    ├── actions.rs        — OverworldAction (walk to tile, warp, talk to sprite)
    ├── encoding.rs       — reads/writes Pokémon data structures from MMU
    ├── symbols.rs        — DmgPointer / DmgBank types + include of generated symbols
    ├── world_graph.rs    — graph of all 248 maps built from ROM headers at runtime
    ├── tile_map.rs       — MetaTileMap: abstracts the current map into typed tiles
    ├── map.rs            — Map enum (all 248 maps)
    ├── battle.rs         — battle state reader + BattleAction
    ├── rom_gfx.rs        — reading ROM graphics: bank windowing, 2bpp tiles, the Poké Ball
    ├── badge_gfx.rs      — the badge sprites, decoded from the trainer card's ROM graphics
    ├── mon_gfx.rs        — the front pics, a port of pokered's `UncompressSpriteData`
    ├── delay.rs          — DelayContext: cycle-accurate waits between agent steps
    ├── text.rs           — PokemonTextReader: reads on-screen text from VRAM
    ├── integration_tests/ — agent end-to-end tests, tiered by emulated game time
    ├── data/             — saved emulator state snapshots (.bin) used by tests
    └── roms.rs           — embeds pokered/pokered.gbc as a compile-time byte slice
```

Cargo features are `default = ["sdl", "web", "llm"]`. `web` and `llm` are on by default
deliberately: the video codec, the emulator host, the SSE parser, the turn-cancellation contract and
a mock-server playthrough are all default-tier tests, and behind an opt-in feature a plain
`cargo test` would silently skip every one of them. The container build is
`--no-default-features --features llm`, which drops the SDL2 link dependency entirely.

### Key choices

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust | The emulator must run far faster than real time |
| Desktop UI | SDL2 (`sdl2` crate) | Lightweight, easy audio queue + video surface |
| Audio resampling | `src/audio/blip/`, no dependency | A Rust port of blargg's Blip_Buffer. Band-limited *step* synthesis rather than sinc resampling: the APU reports amplitude transitions and they are written straight into a buffer already at the output rate. 8 output samples of latency, no FFT, no crates |
| Serialisation | `bincode` + `lz4_flex` | Fast snapshot/restore for the save states the tests are built on |
| Save state format | labelled sections | `"GBST" \| version \| lz4 { [label][len][payload] }`. Unknown sections are skipped and missing ones are not errors, so adding one is free — CGB support doubled VRAM and quadrupled WRAM at the cost of zero fixture regeneration |
| Symbol codegen | `build.rs` + `pokered/pokered.sym` | Every RAM/ROM symbol becomes a typed `DmgPointer` constant, so an address that moves upstream is a compile error |
| pokered | a git submodule | The source of truth for the ROM, the symbol map and the game's data tables |
| LLM transport | `ureq` + hand-rolled SSE | The wire types and the stream accumulator are pure and testable without HTTP |
| Video transport | chunked binary + `flate2` | Not a WebSocket: nothing is bidirectional, and a plain response needs no upgrade, no ping/pong and no second reconnection story. The compression is the protocol rather than a `Content-Encoding`, so no proxy can decide to buffer and re-encode it |

## Tests

```shell
cargo test --release                                              # ~7 s, the default tier
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests
cargo test --release --features full-playthrough full_playthrough # the whole game, ~5 min
```

Always `--release`: these tests emulate every frame and are unusably slow otherwise. The suite is
tiered by how much *game time* a test costs, since that is the only thing that matters to its wall
clock. `CLAUDE.md` has the full map of the tiers, the fixture chain and the benchmarking setup.

## A note on licences

There is no top-level `LICENSE` here yet. If one is added, the constraint to check first is
`src/audio/blip/`: it is a translation of blargg's Blip_Buffer 0.4.0, which is LGPL 2.1+. The
original C++ and its licence are vendored under `tools/blip-golden/`.

The ROM is not distributed and cannot be — `pokered/` is a submodule of the disassembly project, and
the cartridge is assembled locally from it.
