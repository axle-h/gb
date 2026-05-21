# gb — Game Boy Emulator / Pokémon Red LLM Agent

A Game Boy (DMG) emulator written in Rust, repurposed as a platform for an LLM agent to play Pokémon Red entirely via text — no images required.

The emulator is accurate enough to pass hardware-compatibility test ROMs (Blargg's cpu_instrs, dmg_sound, instr_timing; dmg-acid2 PPU test). It has full CPU, PPU (graphics), audio, timer, DMA, interrupt, and joypad emulation.

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
    ├── integration_tests.rs — slow agent integration tests (run in release mode)
    ├── data/             — saved emulator state snapshots (.bin) used by tests
    └── roms.rs           — embeds pokered/pokered.gbc as a compile-time byte slice
```

## Key tech choices

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust | Performance; the emulator must run faster than real-time |
| UI | SDL2 (`sdl2` crate) | Lightweight, easy audio queue + video surface |
| Audio resampling | `rubato` | High-quality sinc resampler from GB native 1 048 576 Hz → 44 100 Hz |
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

**Integration tests run the emulator and are very slow in debug mode.** Always use `--release`. Agent/policy debugging goes to stdout so use `--nocapture`:

```bash
# Run all tests (release for reasonable speed)
cargo test --release

# Run a specific integration test with stdout visible
cargo test --release --package gb --bin gb -- pokemon::integration_tests::test_debouncing --exact --nocapture

# Run a specific integration test (another example)
cargo test --release --package gb --bin gb -- pokemon::integration_tests::test_ledge_jump_does_not_abort_overworld_movement --exact --nocapture

# Run unit tests only (fast, debug mode is fine)
cargo test --package gb --lib
```

### Test ROM compatibility tests

The `src/roms/` directory contains standard GB test ROMs. These are exercised by unit tests and do not require the pokered submodule.

## Save state format

The emulator state is serialised with `bincode` + lz4 compression. `GameBoy` implements `Encode`/`Decode`. Test fixture snapshots live in `src/pokemon/data/*.bin` and are `include_bytes!`'d at compile time. `pokemon-red.sav` is the SRAM save loaded/written at runtime by the SDL2 UI.

## Important notes

- The ROM (`pokered/pokered.gbc`) is embedded at compile time via `include_bytes!` in `src/pokemon/roms.rs`. The submodule must be built before `cargo build` will succeed.
- `AGENT_RESOLUTION` (20 ms) is a tuned constant — too long and the player overshoots on the overworld, too short and the game state doesn't settle between frames.
- `DelayContext` post-script delay is 2500 ms — tuned to cover the worst-case pre-battle animation gap observed in practice.
- The SDL2 UI renders at 4× scale (640×576) and targets 60 fps with a 600-frame rolling FPS window.