# gb — Game Boy Emulator / Pokémon Red LLM Agent

A Game Boy (DMG) emulator written in Rust, repurposed as a platform for an LLM agent to play Pokémon Red entirely via text — no images required.

The emulator is accurate enough to pass hardware-compatibility test ROMs (Blargg's cpu_instrs, dmg_sound 1–8 and 11, instr_timing; dmg-acid2 PPU test). `dmg_sound` 09/10/12 are **not** passing — they are `#[ignore]`d in `src/game_boy.rs`, and their reference images in `src/roms/mod.rs` are placeholders. Task **A16** covers fixing them. It has full CPU, PPU (graphics), audio, timer, DMA, interrupt, and joypad emulation.

The project has been extended with a Pokémon Red-specific layer that reads game state directly from emulator RAM (using symbols extracted from the [pokered](https://github.com/pret/pokered) disassembly) and drives the game via synthesised joypad input. The goal is to expose a complete text interface over MCP so an LLM agent can play through the entire game.

## Architecture

```
src/
├── main.rs              — entry point, starts SDL2 UI
├── game_boy.rs          — top-level GameBoy struct (run loop, save/restore)
├── core.rs              — CPU + MMU wiring
├── opcode.rs            — full SM83 instruction set
├── mmu.rs               — memory map, bank switching
├── ppu.rs               — pixel processing unit (LCD rendering)
├── audio/               — APU (4-channel Game Boy audio)
│   └── blip/            — band-limited synthesis + resampling to the sink's rate (Blip_Buffer port)
├── sdl/                 — SDL2 UI: renders LCD at 4× scale, drives audio, keyboard input
│   └── render.rs        — main render loop; instantiates GameBoy + PokemonAgent
├── roms/                — bundled test ROMs (cpu_instrs, dmg-acid2, etc.)
└── pokemon/             — Pokémon Red layer (everything below)
    ├── mod.rs            — PokemonApi / PokemonApiTrait / GameState
    ├── agent.rs          — PokemonAgent: drives joypad each frame, emits AgentEvents
    ├── policy.rs         — Policy trait + impls (Random, Console/stdin, Deterministic)
    ├── actions.rs        — OverworldAction (walk to tile, warp, talk to sprite)
    ├── encoding.rs       — reads/writes Pokémon data structures from MMU
    ├── symbols.rs        — DmgPointer / DmgBank types + include of generated symbols
    ├── world_graph.rs    — graph of all 248 maps built from ROM headers at runtime
    ├── tile_map.rs       — MetaTileMap: abstracts the current map into typed tiles
    ├── map.rs            — Map enum (all 248 maps)
    ├── battle.rs         — battle state reader + BattleAction
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

Current policy implementations: `RandomPolicy`, `ConsolePolicy` (stdin, used in the SDL2 debug UI), `DeterministicPolicy` (scripted steps, used in tests).

## Common commands

### Build & run

```bash
# Build
cargo build --release

# Run SDL2 debug UI (loads pokemon-red.sav, starts ConsolePolicy on stdin)
cargo run --release
```

### Tests

**Always use `--release`.** The integration tests emulate every frame and are unusably slow in debug
mode. Agent/policy debugging goes to stdout, so add `--nocapture` when you care about it.

The crate has **no lib target** — everything lives in the `gb` binary, so it is `--bin gb`, never
`--lib`.

`src/pokemon/integration_tests/` is tiered by how much **game time** a test emulates, which is what it
costs: the emulator core runs at **~34× realtime** on Pokémon Red and the agent costs **~16%** on
top, giving **~28×** end to end (measured 2026-08-05 on a Ryzen 9 7900X by `bench_core_throughput`
and `bench_emulation_throughput` respectively), so wall clock ≈ emulated-minutes ÷ 28.

```bash
# Default tier: all unit tests + agent mechanics + two navigation smoke tests. ~22s, 800+ tests.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, each seeded from a committed snapshot.
# ~80s, 115 tests — every one of them ≤65s (measured 2026-08-05).
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The one leg that costs more game time than the whole leg chain combined: the Safari dex sweep,
# 381s of wall clock for ~190 min of emulated game time (21 paid ¥500 trips chasing 4.3%-slot
# species). Split out because it, alone, set the leg tier's wall clock at six minutes — with libtest
# printing nothing until it finished, so there was no way to see what was still running.
cargo test --release --features very-slow-tests --bin gb -- can_sweep_the_safari_zone --exact

# The whole game from a fresh save, ~20 min of wall clock.
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

With every tier feature on and the tool features off, the ignored list is **21 blocked emulator
tests** and nothing else — 9 `oam_bug`, 9 `mem_timing`/`halt_bug`, 3 `dmg_sound` wave. Each names
its blocker: a plan task ID, or why it will not be fixed. Keep it that way.

```bash
# The diagnostics and probes
cargo test --release --features diagnostics,slow-tests --bin gb -- probe_ --ignored --nocapture
```

### ⚠️ Run `full_playthrough` after every major work item, and always before pushing

```bash
cargo test --release --features full-playthrough full_playthrough   # ~20 min
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

The **91** committed fixture snapshots live in `src/pokemon/data/*.bin` and are `include_bytes!`'d at
compile time. `every_committed_fixture_decodes` in the default test tier fails in seconds if a
layout change breaks them. `pokemon-red.sav` is raw SRAM, not a save state — it is loaded/written at
runtime by the SDL2 UI.

## Important notes

- The ROM (`pokered/pokered.gbc`) is embedded at compile time via `include_bytes!` in `src/pokemon/roms.rs`. The submodule must be built before `cargo build` will succeed.
- `AGENT_RESOLUTION` (20 ms) is a tuned constant — too long and the player overshoots on the overworld, too short and the game state doesn't settle between frames.
- `DelayContext` post-script delay is 2500 ms — tuned to cover the worst-case pre-battle animation gap observed in practice.
- The SDL2 UI renders at 4× scale (640×576) and targets 60 fps with a 600-frame rolling FPS window.
- **`Audio` and `PPU` exclude derived state from `PartialEq`** — the resampler output, and the
  frame buffer plus the per-scanline sprite list respectively. None of it is serialised, so none of
  it may take part in equality, or `game_boy::tests::save_and_load_state` would compare restored
  state against state that was never saved.
- Adding a field to `Audio` (or any other serialised type) **is** safe now — append it to the
  relevant section and bump that section's version. The old "nothing may be added to `Audio`" rule
  died with the sectioned save-state format. The output sample rate is still applied by
  `Audio::set_output_sample_rate` from the UI rather than stored, so a caller that loads a save
  state must re-apply it (see the `F9` handler in `render.rs`).
- `src/audio/blip/` is a translation of LGPL 2.1+ code (blargg's Blip_Buffer 0.4.0). The original C++
  and its licence live in `tools/blip-golden/vendor/`. The repo has no top-level `LICENSE`; if one is
  ever added, this is the constraint to check.