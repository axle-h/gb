# gb — Game Boy Emulator / Pokémon Red LLM Agent

A Game Boy (DMG **and CGB**) emulator written in Rust, repurposed as a platform for an LLM agent to play Pokémon Red entirely via text — no images required.

The emulator is accurate enough to pass hardware-compatibility test ROMs (Blargg's cpu_instrs and dmg_sound — **including both combined suite ROMs** — instr_timing; dmg-acid2 **and cgb-acid2** PPU tests). It has full CPU, PPU (graphics), audio, timer, DMA, interrupt, and joypad emulation.

**Real MBC support** lives in `src/mbc.rs` — `RomOnly`, MBC1, MBC2, MBC3, MBC5 and HuC1, dispatched from `CartType`, with the MBC3 real-time clock in `src/rtc.rs`. It replaced one hardcoded pseudo-mapper (MBC1's register layout with MBC3's width) that served every cartridge and silently dropped `0x6000-0x7FFF`. Mappers `gb` cannot emulate now fail with a typed `LoadError` rather than running as something else. **27 of mooneye's 28 MBC test ROMs pass** (`--features hwtests`); the one skip is MBC1 multicart, which is not implemented. See Phase D of `docs/compatibility/10-implementation-plan.md`.

⚠️ **The RTC's time source is injectable and anything replayable must pin it** (`MMU::set_rtc_time_source`) — the default is the host clock, so an RTC cartridge under a fixture-driven test would fail only sometimes. Nothing committed has an RTC: `pokered.gbc` is `0x13`, MBC3 with *no* timer.

⚠️ **Every mapper resolves its bank register differently and the differences are not decoration.** MBC1 remaps a zero selection *then* wraps, so a wrap can reach bank 0; MBC3 wraps *then* remaps, so it never can; MBC2/MBC5/HuC1 have their own rules again. Same two operations, opposite order, different answer — and it is what makes blargg's combined `dmg_sound.gb` terminate. `src/mbc.rs`'s module docs have the table.

**Game Boy Color is supported** — `GameBoy::cgb(cart)` beside `GameBoy::dmg(cart)`, with VRAM/WRAM
banking, CGB palette RAM, BG map attributes, OAM-index sprite priority, KEY1 double speed and
HDMA/GDMA. A DMG-only cartridge run on a CGB gets **compatibility mode**, including the boot ROM's
title-derived palette — which is why `GameBoy::cgb(POKERED)` comes out red-tinted exactly as
Pokémon Red does on real Game Boy Color hardware. The SDL UI still boots a DMG; colour is reachable
through the API and the tests. See `docs/compatibility/10-implementation-plan.md` Phase B.

The project has been extended with a Pokémon Red-specific layer that reads game state directly from emulator RAM (using symbols extracted from the [pokered](https://github.com/pret/pokered) disassembly) and drives the game via synthesised joypad input. The goal is to expose a complete text interface over MCP so an LLM agent can play through the entire game.

## Architecture

```
src/
├── main.rs              — entry point: `gb` (SDL UI) or `gb serve` (web), dispatched from cli.rs
├── cli.rs               — hand-rolled arg parsing; `parse` is unit-testable without a process
├── host.rs              — headless emulator host: GameBoy + PokemonAgent + video encoder on one thread
├── run/                 — the run directory (`web` feature): checkpoint, resume, transcript
│   ├── mod.rs           — $GB_RUN_DIR/<run-id>/: meta.json, state.gbst, sram.bin; atomic writes
│   └── transcript.rs    — transcript.jsonl writer thread + the /api/history backlog reader
├── web/                 — the axum server (`web` feature); read-only, five endpoints
│   ├── published.rs     — the only interface between the emulator thread and HTTP
│   ├── video.rs         — 8×8 block-diff video codec + the reference decoder
│   ├── assets.rs        — the SPA: `web/dist` embedded, or read from disk under GB_WEB_DEV=1
│   └── badges.rs        — /api/badges.png: the eight badges, decoded from the cartridge
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
├── schedule.rs          — event schedule: when each peripheral next does something (Phase C)
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
    ├── badge_gfx.rs      — the badge sprites, decoded from the trainer card's ROM graphics
    ├── delay.rs          — DelayContext: cycle-accurate waits between agent steps
    ├── text.rs           — PokemonTextReader: reads on-screen text from VRAM
    ├── integration_tests/ — agent end-to-end tests, tiered (see Tests below)
    ├── data/             — saved emulator state snapshots (.bin) used by tests
    └── roms.rs           — embeds pokered/pokered.gbc as a compile-time byte slice
```

## Key tech choices

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust | Performance; the emulator must run faster than real-time |
| UI | SDL2 (`sdl2` crate) | Lightweight, easy audio queue + video surface |
| Audio resampling | `src/audio/blip/` (no dependency) | Rust port of blargg's Blip_Buffer. Band-limited *step* synthesis rather than sinc resampling: the APU reports amplitude transitions and they are written straight into a buffer already at the output rate. 8 output samples of latency, no FFT, no crates. |
| Serialisation | `bincode` + `lz4_flex` | Fast snapshot/restore for save states used in tests |
| Symbol codegen | `build.rs` + `pokered/pokered.sym` | Parses the pokered symbol map at build time and generates typed `DmgPointer` constants for every RAM/ROM address |
| pokered submodule | `pokered/` (git submodule) | Source of truth for ROM binary (`pokered.gbc`), symbol map, and game data |

## pokered submodule

`pokered/` is a git submodule pointing at https://github.com/pret/pokered.

`build.rs` parses `pokered/pokered.sym` and generates `$OUT_DIR/pokered_symbols.rs`, which is included by `src/pokemon/symbols.rs`. This gives every RAM/ROM symbol a typed `DmgPointer` constant (e.g. `pokered_symbols::wPlayerName`, `pokered_symbols::wPartyDataStart`).

After cloning, initialise the submodule:
```
git submodule update --init --recursive
```

The submodule must be compiled to produce `pokered/pokered.gbc` (the ROM). Follow the pokered build instructions (`make` inside `pokered/`).

⚠️ **pokered needs rgbds ≥ 1.0.0 and `rgbdscheck.asm` `fail`s the assembly below that** — it is a
hard error, not a warning, so an older rgbds does not produce a wrong ROM, it produces none. The
container pins 1.0.3 (`ARG RGBDS_VERSION`), which is what upstream's own `INSTALL.md` and CI name.
The symbol *names* are upstream's to change: the 2026-08 bump renamed pokered's "hidden object" and
"missable object" vocabulary to **hidden event** and **toggleable object**
(`HiddenEventMaps`, `HiddenEventPointers`, `wToggleableObjectFlags`), which is a compile error here
rather than a silent one, because `build.rs` only emits constants for symbols that exist.

## Agent / Policy system

`PokemonAgent` is a frame-by-frame driver. Each call to `agent.step(&mut gb)` advances the emulator by `AGENT_RESOLUTION` (20 ms of emulated time) and dispatches actions based on the current `GameMode`:

- **Overworld** — computes available `OverworldAction`s from the `MetaTileMap` (warps, connections, grass, sprites), asks the `Policy` to pick one, then synthesises the D-pad/button sequence to execute it.
- **Battle** — waits for the battle menu, asks the `Policy` to pick a `BattleAction`, navigates the menus.
- **TextBox / Script / NamingScreen** — advances through dialogue, reads text, emits `AgentEvent::TextBox`.

`Policy` is a non-blocking trait — every method returns `Option<_>` so the game loop keeps ticking while the policy waits (e.g. for an async LLM response):

```rust
fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction>;
fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;
fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>>;
```

Current policy implementations: `RandomPolicy`, `ConsolePolicy` (stdin, used in the SDL2 debug UI), `DeterministicPolicy` (scripted steps, used in tests), `LlmPolicy` (`llm` feature).

⚠️ **`PokemonAgent::poll_policy` is the single seam every decision point goes through**, and it is
not just a tidy-up: it resets the clock W9's stuck-run watchdog reads. Call `policy.service_tools`
directly from a new poll site and the watchdog will believe the run has been wedged since that
moment, forever.

**The watchdog** (`Policy::{stuck_timeout, pick_unstick}`, `GB_STUCK_TIMEOUT_SECS`, default 300
*emulated* seconds; `0` is off) covers the one failure mode nothing else does: the agent reaching no
decision point at all, so the policy is never consulted and cannot notice. It raises a
`DecisionKind::Stuck` turn whose only terminal tools are `press_buttons` and `wait`. Two ⚠️s, both
learned the hard way in the design and both documented in the code:

- **It is asked on every tick of the jam, not once.** A tool batch is only serviced inside
  `agent.update`, so a one-shot notification would hang any turn that wanted to read first.
- **It must not reset the clock it reads**, or the jam clears the instant it is noticed and the turn
  is never polled again.

Every firing emits `AgentEvent::WatchdogFired` — to the model, the UI, the transcript and stdout. In
a healthy run it never fires: ordinary play's longest silence is ~6 s of game time (measured by
`mechanics::ordinary_play_stays_far_inside_the_stuck_timeout`) against a 300 s default.

## Common commands

### Build & run

```bash
# Build. ⚠️ The browser UI is built first and separately — see "The web UI" below.
cargo build --release

# Run SDL2 debug UI (loads pokemon-red.sav, starts ConsolePolicy on stdin)
cargo run --release

# Serve the web UI and play headlessly — no window, no SDL, no API key. Open http://localhost:8080.
# Streams the screen as 8x8 block deltas over SSE (~19 kbit/s) plus a 10 Hz status/event feed.
cargo run --release -- serve --policy random --port 8080

# Let an LLM play it. OPENAI_API_KEY and GB_MODEL are the only required settings; OPENAI_BASE_URL
# points at any OpenAI-compatible endpoint. `gb serve` defaults to --policy llm.
OPENAI_API_KEY=sk-… GB_MODEL=… cargo run --release -- serve

# ⚠️ `gb serve` RESUMES by default (W7): the newest run under $GB_RUN_DIR (default ./runs) whose
# state.gbst loads is continued in place, notes and all. `--new-run` starts the game over in a
# directory of its own and leaves the old one alone. Ctrl-C and SIGTERM both checkpoint on the way
# out — killing the process with SIGKILL loses up to a minute.
cargo run --release -- serve --new-run

# The container build: no window system at all.
cargo build --release --no-default-features --features llm
```

`default = ["sdl", "web", "llm"]`. **`web` and `llm` are on by default deliberately** — the video
codec, the late-joiner ordering, the emulator host, the SSE parser, the turn-cancellation contract
and the mock-server playthrough are all default-tier tests, and behind an opt-in feature a plain
`cargo test --release` would silently skip every one of them. `web` costs 58 crates on top of the 119
a default build already pulls, and `llm` (ureq + its rustls stack) a further 14.

**The LLM configuration is all environment variables**, never flags, because the API key has to be
one: `OPENAI_BASE_URL`, `OPENAI_API_KEY`, `GB_MODEL`, `GB_CONTEXT_LIMIT`, `GB_TEMPERATURE`,
`GB_MAX_TOOL_STEPS`, `GB_STUCK_TIMEOUT_SECS`, `GB_PORT`, `GB_RUN_DIR` and `GB_STATUS_HZ`. `gb --help` lists them, and
`cli::tests::the_usage_names_every_flag_and_variable` makes sure it keeps doing so; `src/llm/config.rs`
documents them (`GB_PORT` is read in `cli.rs`, `GB_RUN_DIR` and `GB_STATUS_HZ` in `web/mod.rs`, since
all three apply to `--policy random` too).

⚠️ **The status heartbeat is sent on change, not on a timer.** It is sampled at `GB_STATUS_HZ` (2 Hz)
and published only when it says something the last one did not, with a 2 s keepalive so an idle run
still proves it is alive and `curl -N /api/events` still ticks. At the original 10 Hz unconditional it
measured **49.7 kbit/s per viewer** — six times the idle video feed, with nine of ten payloads
byte-identical to the one before; it is now 5.2. Two consequences: `StatusSnapshot` compares with
`says_the_same_as`, which excludes the clocks and `frame_seq` (a derived `PartialEq` would never match
and the suppression would silently never fire), and `/api/events` **opens with the latest heartbeat**
— `Published::join_events`, subscribe-then-read, the same handshake as the video keyframe — or a page
opened during a quiet stretch shows an empty panel.

**A run keeps everything it needs in one directory** (`$GB_RUN_DIR/<run-id>/`): `meta.json`,
`state.gbst`, `sram.bin`, `transcript.jsonl`, and the model's own `memories/` and `todo.json`. That
directory is the whole of a run's state — copy it and the run moves with it.

⚠️ **The emulator never pauses while the model thinks, and must not be made to.** A tool batch is
answered by `Policy::service_tools`, which only runs when `gb.run` advances the agent — so any pause
spanning an LLM tool call deadlocks the run. A `GB_PAUSE_WHILE_THINKING` flag was built in W4 and
removed the same day; `HostConfig` in `src/host.rs` carries the ⚠️.

### The web UI

`web/` is a Vite + React + TypeScript SPA — screen, status panel, conversation, and a TypeScript port
of the video decoder in `web/src/video.ts`. The badge strip is the game's own art: `src/pokemon/badge_gfx.rs`
decodes the trainer card's badge tiles out of the ROM and `/api/badges.png` serves them as one sheet,
so no graphics are committed. `pnpm run build` produces `web/dist`, which `rust-embed`
bakes into the binary, so **the SPA build comes first**:

```bash
cd web && pnpm install && pnpm run build   # → web/dist
cargo build --release                      # embeds it
```

**The package manager is pnpm**, pinned by `packageManager` in `web/package.json` and activated by
corepack — no pnpm version is named in the Dockerfile, so there is nothing there to drift from it.

⚠️ **`web/dist` must exist for the crate to compile at all** — `rust-embed`'s derive fails if the
folder is missing, which is why `web/dist/.gitkeep` is committed and why `vite build` (which empties
`dist`) copies it back from `web/public/`. A checkout that has never run `pnpm run build` compiles and
serves a page saying which two commands to run, so a missing UI is never a mystery.

⚠️ **`web/pnpm-workspace.yaml` sets a `minimumReleaseAge` cooldown, and it is checked on *every*
install — `--frozen-lockfile` included, not just `pnpm update`.** A lockfile pinning anything younger
than the window fails with `ERR_PNPM_NO_MATURE_MATCHING_VERSION`, so the lockfile has to be
*generated* under the same number the builds enforce; raise the window without regenerating
`pnpm-lock.yaml` and what breaks is the container build, not the dev loop. The file documents why it
is 3 days rather than the usual 7 and when that can change.

Two dev loops:

```bash
# Hot reload: Vite on :5173 proxies /api to a `gb serve` on :8080.
cargo run --release -- serve --policy random --port 8080 &
cd web && pnpm run dev

# Or: skip the cargo rebuild after an SPA build by reading web/dist from disk.
GB_WEB_DEV=1 cargo run --release -- serve --policy random
```

### The container

`Dockerfile` builds the whole thing from a bare checkout in four stages — **146 MB, of which the
binary is 6.9 MB** — and `docker-compose.yml` is the ops shape of a run (named volume, restart
policy, 30 s stop grace period).

```bash
docker build -t gb .
docker run -d -p 8080:8080 -v gb-runs:/runs -e OPENAI_API_KEY=sk-… -e GB_MODEL=… gb
OPENAI_API_KEY=sk-… GB_MODEL=… docker compose up -d --build     # the same, with the volume named
docker compose run --rm --service-ports gb gb serve --policy random   # no API key, no spend
```

⚠️ **The cartridge is stage 1, not an input.** Two of this crate's compile-time inputs are generated
and neither is in git: `pokered/pokered.gbc` (`include_bytes!`) and `pokered/pokered.sym`
(`build.rs`). Stage 1 builds **rgbds 1.0.3 from source** (pinned by `ARG` + sha256; from source, not
the prebuilt tarball, so the image also builds on arm64) and then `make pokered.gbc` — and ends by
checking the result against upstream's own `pokered/roms.sha1`. **That check is load-bearing**: all
91 committed fixtures and every generated symbol are pinned to those exact bytes, so a ROM that
merely assembles is a different game and would fail somewhere deep in the agent instead of at the
build. Stage 2 is `pnpm run build` for the same class of reason (`rust-embed` needs `web/dist`).

⚠️ **`.dockerignore` must exclude the host's pokered artifacts with `**`.** `pokered/*.o` leaves
`pokered/gfx/pics_red.o` in the context, and a stale object file from a *newer* rgbds stops the build
dead (`Unsupported object file … expected revision 12, got 13`). None of what it excludes is tracked;
every one is a `make` output.

⚠️ **`CMD` is exec form so `gb` is PID 1 and receives SIGTERM itself** — that signal is what
checkpoints the run. A shell in between means `docker stop` loses everything since the last periodic
checkpoint. `gb serve` **resumes** the newest run under `/runs` by default, so `docker restart` picks
up where it was; `--new-run` starts the game over.

### Tests

**Always use `--release`.** The integration tests emulate every frame and are unusably slow in debug
mode. Agent/policy debugging goes to stdout, so add `--nocapture` when you care about it.

The crate has **no lib target** — everything lives in the `gb` binary, so it is `--bin gb`, never
`--lib`.

`src/pokemon/integration_tests/` is tiered by how much **game time** a test emulates, which is what it
costs: the emulator core runs at **~91× realtime** on Pokémon Red and the agent costs **~35%** on
top, giving **~50×** end to end (measured 2026-08-06 on a Ryzen 9 7900X by `bench_core_throughput`
and `bench_emulation_throughput` respectively), so wall clock ≈ emulated-minutes ÷ 48.

Those numbers are **post-Phase-C**: the core was 29× and the agent-inclusive figure 24× before
`docs/compatibility/10-implementation-plan.md`'s C1–C5, which made it 3.1× faster (ledger #13). The
agent's share grew from ~16% to ~35% for the obvious reason — it did not get slower, the emulator
under it got faster — so **it is now worth profiling and it was not before.**

⚠️ **Do not trust a single benchmark reading on this machine.** It has fast and slow states ~15%
apart — the same unmodified binary has measured `cpu_instrs` at 43.5× and 53.2× twenty minutes
apart. Compare only adjacent paired runs of the two builds, **alternate which one runs first**, and
report both orders. `docs/compatibility/compare.sh` does exactly that; see
`docs/compatibility/10-implementation-plan.md` §2.5.

**`perf` works, and needs no `sudo`** (`perf_event_paranoid` is 2). Build with
`RUSTFLAGS="-C debuginfo=2"` into a scratch `CARGO_TARGET_DIR`, then drive the benchmark with
`BENCH_FRAMES=40000 BENCH_ONLY=pokemon` so there is enough wall clock to sample and only one
workload in the profile. ⚠️ Watch for sampling skid: a hot instruction is often paying for the
*load* feeding it, not for itself — §2.5 has a worked example that cost an hour.

```bash
# Default tier: all unit tests + agent mechanics + two navigation smoke tests + the web/host/llm tier.
# ~7s, 1143 tests.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, each seeded from a committed snapshot.
# ~58s, 119 tests (measured 2026-08-06, after Phase C; it was ~131s before).
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The one leg that costs more game time than the whole leg chain combined: the Safari dex sweep,
# 171s of wall clock for ~190 min of emulated game time (21 paid ¥500 trips chasing 4.3%-slot
# species; it was 381s before Phase C). Split out because it, alone, set the leg tier's wall clock
# at six minutes — with libtest printing nothing until it finished, so there was no way to see what
# was still running. ⚠️ `very-slow-tests` does not imply `slow-tests`, and the test's module is
# behind that gate, so pass **both** features or this matches zero tests.
cargo test --release --features slow-tests,very-slow-tests --bin gb -- can_sweep_the_safari_zone

# The whole game from a fresh save, ~5 min of wall clock (was ~11 min before Phase C).
cargo test --release --features full-playthrough full_playthrough

# A single test with output (file module included in the path).
cargo test --release --bin gb -- pokemon::integration_tests::mechanics::test_debouncing --exact --nocapture

# Agent throughput (emulator + agent.step). `--exact` needs the full module path, or this
# matches zero tests.
cargo test --release --bin gb -- \
  pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture

# Emulator core alone — no agent, no policy, no observation. Three workloads. This is the
# number Phase C of docs/compatibility/10-implementation-plan.md is scored against.
# Behind the `bench` feature, so benchmarks never pad the ignored-test count.
cargo test --release --features bench --bin gb -- \
  game_boy::tests::bench_core_throughput --exact --nocapture
```

**A test that is `#[ignore]`d should be blocked, not merely slow or not-a-test.** Everything else
goes behind a Cargo feature, so the ignored list stays a readable backlog:

| Feature | Holds |
|---|---|
| `slow-tests` / `very-slow-tests` / `full-playthrough` | Tiering by emulated game time |
| `diagnostics` | `probe_*`, `dump_fixture_states`, `capture_golden_input` — tools that print a report rather than assert. They keep `#[ignore]` on top of the gate because their pass/fail is not a signal: two legitimately end by exhausting their cycle budget *after* printing what was asked for |
| `bench` | `bench_core_throughput`, `bench_emulation_throughput` |
| `hwtests` | The mooneye MBC suite and its ROMs. 22 MB raw, committed lz4-compressed (149 KB) and decompressed in memory by the fixture, so a default build carries none of it |

With every tier feature on and the tool features off, the ignored list is **18 blocked emulator
tests** and nothing else — 9 `oam_bug` and 9 `mem_timing`/`halt_bug`. (It was 19 until D1 fixed the
combined `dmg_sound` suite ROM.) Each names its blocker: a plan task ID, or why it will not be
fixed. Keep it that way.

```bash
# The diagnostics and probes
cargo test --release --features diagnostics,slow-tests --bin gb -- probe_ --ignored --nocapture
```

### ⚠️ Run `full_playthrough` after every major work item, and always before pushing

```bash
cargo test --release --features full-playthrough full_playthrough   # ~5 min
```

**This is not optional and the leg tier is not a substitute for it.** The leg tests each start from a
committed fixture, so they prove the legs *individually*; only `full_playthrough` proves they still
**compose**, and the two come apart in ways nothing else catches:

- **A leg test can be green for a reason the mainline does not give it.** `run_leg` keeps stepping
  after the queue empties until the effect lands, so a leg whose `Interact` pops before its
  conversation still passes — while `complete_game_steps` walks straight on without the item. That is
  exactly how the Poké Flute broke. `run_leg` now prints a ⚠️ when its post-exhaustion wait is long;
  **treat that warning as a failure in waiting.**
- **A fixture pins a party and a bag; the mainline earns them.** A leg seeded with 20 Hyper Potions
  says nothing about whether the run that reaches it can afford them.
- **Anything that changes frame timing re-rolls the RNG stream** (see `with_original_battle_timing`),
  and only a full run crosses every route that stream feeds.

Because it is opt-in and slow, it rotted once already: it sat broken while its own doc comment, this
file and the plan all claimed it played to all 8 badges. When it fails it now reports how far it got
(`completed 488/516 policy steps (94%)`) and drops a save state + screenshot in
`target/test-artifacts/`; `playthrough::probe_resume_playthrough` replays from there in seconds
instead of re-running the 20 minutes up to the stall. **If you cannot make it pass, say so explicitly
in the hand-off — do not leave a doc comment claiming it works.**

**Fixtures are committed inputs.** Each leg test snapshots its end state for the next leg, but the
write is a no-op unless `--features regen-fixtures` is on — otherwise every run silently changes the
next run's inputs, and a leg "fails" only because an earlier one re-saved its fixture. To regenerate
after a deliberate change, run the affected legs **in chain order**:

```bash
cargo test --release --features slow-tests,regen-fixtures --bin gb -- can_clear_ss_anne --exact
```

Failure artifacts (a save state + screenshot at the point of a stall or timeout) land in
`target/test-artifacts/`, not the repo root.

### Test ROM compatibility tests

The `src/roms/` directory contains standard GB test ROMs. These are exercised by unit tests and do not require the pokered submodule.

`cgb-acid2` is the Game Boy Color one, and unlike the blargg audio suites it **ships its own
reference image**, so nothing in it was promoted from `gb`'s own output. Its README pins the 5-bit
to 8-bit colour expansion as `(c << 3) | (c >> 2)` — the plain widening, **not** a colour-correction
curve — which is what `LcdColor::from_rgb555` implements. Adopting gambatte's `gbcToRgb32`
correction instead would break the comparison.

```bash
cargo test --release --bin gb -- game_boy::tests::ppu     # dmg-acid2, cgb-acid2, Pokémon Red in colour
```

### Audio / resampler tests

`src/audio/blip/tests.rs` checks the resampler two independent ways, and they fail differently.

**Golden vectors** are bit-exact comparisons against the original C++ (Blip_Buffer ships no test
suite of its own, only interactive SDL demos). The fixtures in `src/audio/data/blip_*.bin` are
produced by linking the vendored library in `tools/blip-golden/`. Regenerate after a *deliberate*
change to the algorithm or its parameters:

```bash
# 1. only if the realistic-signal input needs refreshing (writes src/audio/data/apu_capture_in.bin)
cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored
# 2. always — reads apu_capture_in.bin, writes the other src/audio/data/blip_*.bin
tools/blip-golden/build.sh
```

The goldens are pinned to `GOLDEN_TREBLE_DB` in the test module, deliberately *not* to
`blip::DEFAULT_TREBLE_DB` — tone is a taste knob, so changing what the emulator ships does not
invalidate the port's correctness fixtures.

**Invariants** need no C++ toolchain and are the real regression net: every phase's taps summing to
`kernel_unit`, a step depositing exactly its own amplitude of DC, zero sample-count drift over ten
emulated minutes, no aliasing on a 15 kHz square, and surviving a minute of emulation with no audio
consumer at all (which is what the headless integration tests do).

**Fast-forward.** The number keys `1`–`5` in the SDL UI scale emulation speed, and `render.rs`
mirrors that into `Audio::set_emulation_speed` so the resampler scales its *source clock* to match.
Without it a sped-up emulator simply produces audio faster than the device drains it and the queue
backs up. The speed is derived from `cycle_duration`, not from the key pressed, so it tracks what the
emulator actually targets — `REALTIME_CYCLE_DURATION / 5` truncates to 190 ns, which is 5.016×.

There is deliberately **no WAV "ear check"** any more. It rendered a few seconds to
`target/test-artifacts/` for a listen, which is a listening aid rather than an assertion; the
invariant tests above are the real regression net. It was removed along with `src/audio/wav.rs`
rather than left in the ignored list looking like a test.

## Save state format

Save states use a **labelled, sectioned container** — see the module documentation at the top of
`src/savestate/mod.rs`, which is the authoritative reference:

```
"GBST" | u16 container_version | lz4 { [label\0][u32 len][payload] }
```

An unknown section is skipped and a missing one is not an error, so the format tolerates change in
both directions. **Adding a section is free. Adding a field means appending it as an extra value
within its section and bumping that section's version** — neither churns fixtures. Never reorder or
retype an already-shipped value without bumping the section version: bincode is positional and has
no schema migration.

Phase B is the worked example of why this format was built. CGB support doubled VRAM, quadrupled
work RAM and added a whole register block — and cost **zero fixture regeneration**: the `wram` and
`ppu` sections kept their shipped first value (bank 0, or banks 0 and 1) and **appended** the new
banks as a second value, and everything genuinely new went into the reserved `cgb` section. If you
find yourself about to write a legacy struct, check first whether you can re-cut the boundary
instead.

The **91** committed fixture snapshots live in `src/pokemon/data/*.bin` and are `include_bytes!`'d at
compile time. `every_committed_fixture_decodes` in the default test tier fails in seconds if a
layout change breaks them. `pokemon-red.sav` is raw SRAM, not a save state — it is loaded/written at
runtime by the SDL2 UI.

## Important notes

- The ROM (`pokered/pokered.gbc`) is embedded at compile time via `include_bytes!` in `src/pokemon/roms.rs`. The submodule must be built before `cargo build` will succeed.
- `AGENT_RESOLUTION` (20 ms) is a tuned constant — too long and the player overshoots on the overworld, too short and the game state doesn't settle between frames.
- `DelayContext` post-script delay is 2500 ms — tuned to cover the worst-case pre-battle animation gap observed in practice.
- The SDL2 UI renders at 4× scale (640×576) and targets 60 fps with a 600-frame rolling FPS window.
- **`Audio` and `PPU` exclude derived state from `PartialEq`** — the resampler output and the
  cached mix (`mixed`/`levels`/`mix_dirty`), and the frame buffer plus the per-scanline sprite list
  respectively. None of it is serialised, so none of it may take part in equality, or
  `game_boy::tests::save_and_load_state` would compare restored state against state that was never
  saved. `Schedule` is derived the same way and is not serialised at all — only the clock it is
  built from (`MMU::now`, the `sched` section) is.
- Adding a field to `Audio` (or any other serialised type) **is** safe now — append it to the
  relevant section and bump that section's version. The old "nothing may be added to `Audio`" rule
  died with the sectioned save-state format. The output sample rate is still applied by
  `Audio::set_output_sample_rate` from the UI rather than stored, so a caller that loads a save
  state must re-apply it (see the `F9` handler in `render.rs`).
- ⚠️ **`PPU::draw_pixels_to` and the three DMA transfer loops are `#[inline(never)]`/`#[cold]` on
  purpose.** `MMU::update` runs once per CPU instruction, and letting those inline into it grew it
  60% (3052 → 4893 bytes) and cost several percent of core throughput to instruction-cache pressure
  alone. If you touch them, check with `nm -S --size-sort -C target/release/deps/gb-*` that
  `MMU::update` is still around 3-4 KB (Phase C left it at 3764 bytes). `Serial::complete_transfer`
  and the APU's `mix()` fell to the same rule — see ledger #13.
- `src/audio/blip/` is a translation of LGPL 2.1+ code (blargg's Blip_Buffer 0.4.0). The original C++
  and its licence live in `tools/blip-golden/vendor/`. The repo has no top-level `LICENSE`; if one is
  ever added, this is the constraint to check.