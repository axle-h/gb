# The test suite

Read before running anything but `cargo test --release`, before regenerating a fixture, and
before adding a test. Always `--release`, and the crate is `--bin gb`, never `--lib`. Tests are
tiered by how much game time they emulate, because that is what they cost.

## Commands

```bash
# Default tier: unit tests, agent mechanics, stalls, two navigation smoke tests, web/host/llm.
# ~1500 tests, ~40 s on a warm build.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, each seeded from a committed snapshot.
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The Safari dex sweep: 381 s for ~190 min of game time, more than the rest of the chain.
cargo test --release --features slow-tests,very-slow-tests --bin gb -- can_sweep_the_safari_zone

# The whole game to 8 badges from a fresh save, ~7 min. Run it after every major work item and
# before pushing; nothing else proves the legs compose.
cargo test --release --features full-playthrough full_playthrough

# The same run carried on to the credits, ~26 min.
cargo test --release --features hall-of-fame --bin gb -- hall_of_fame

# The stall hunt: 40 min of game time under RandomPolicy from each of 14 starting states, in parallel.
cargo test --release --features soak-tests --bin gb -- soak --nocapture

# One test with output. The file module is part of the path.
cargo test --release --bin gb -- pokemon::integration_tests::mechanics::test_debouncing --exact --nocapture

# PPU comparisons: dmg-acid2, cgb-acid2, Pokémon Red in colour.
cargo test --release --bin gb -- game_boy::tests::ppu

# Probes (`diagnostics`, all #[ignore]d: they print a report rather than assert).
#   probe_map_images   writes the PNGs read_map sends to target/map-renders/. Look before touching
#                      the palette, the labels or the tile lookup; non-blank is not the same as right.
#   probe_turn_requests writes each decision kind's first request to target/turn-requests/, the only
#                      way to see what the model is actually sent.
#   probe_grind_sites  ranks every encounter block by exp per knockout and per step. Argue about a
#                      grind site from it, never from memory.
#   probe_stall_actions prints a save's map, money, party, bag and every reachable action. Defaults to
#                      the last test_stall_state.bin; GB_PROBE_STATE picks another. First thing to
#                      reach for on a stalled leg: it tells "the route is wrong" from "there is no route".
cargo test --release --features diagnostics --bin gb -- llm::map_image::tests::probe_map_images --exact --ignored --nocapture
cargo test --release --features diagnostics --bin gb -- llm::prompt::tests::probe_turn_requests --exact --ignored --nocapture
cargo test --release --features diagnostics --bin gb -- pokemon::wild::tests::probe_grind_sites --exact --ignored --nocapture
GB_PROBE_STATE=src/pokemon/data/post-articuno.bin \
  cargo test --release --features diagnostics --bin gb -- probe_stall_actions --ignored --nocapture

# Throughput: the agent on top of the emulator, then the core alone.
cargo test --release --features bench --bin gb -- pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture
cargo test --release --features bench --bin gb -- game_boy::tests::bench_core_throughput --exact --nocapture

# What each stream costs. The kbit/s figures in the README come from these and nowhere else.
cargo test --release --features bench --bin gb -- video::bench --nocapture
cargo test --release --features bench --bin gb -- web::audio::bench --nocapture
```

## Tiers and features

| Feature | Holds |
|---|---|
| `slow-tests`, `very-slow-tests`, `full-playthrough`, `hall-of-fame` | tiering by emulated game time |
| `diagnostics` | `probe_*`, `dump_*`, `capture_golden_input`: tools that print rather than assert. They keep `#[ignore]` on top of the gate because their pass/fail is not a signal |
| `bench` | the two throughput benches and `web::{video,audio}::bench` |
| `soak-tests` | `integration_tests::soak`, gated as a module so it never appears in the ignored list |
| `regen-fixtures` | lets a leg test overwrite the snapshot the next leg reads |

A test that is `#[ignore]`d should be blocked, not merely slow; everything else goes behind a
feature. With every feature on, the ignored list is exactly 18 blocked emulator tests (9 `oam_bug`,
9 `mem_timing`/`halt_bug`), each naming its blocker. Keep it that way. Failure artifacts (a save
state and a screenshot at the stall) land in `target/test-artifacts/`.

## Why `full_playthrough` is not optional

Leg tests prove the legs individually; only `full_playthrough` proves they compose, and they come
apart in ways nothing else catches. A leg can pass for a reason the mainline does not give it
(`run_leg` keeps stepping after the queue empties until the effect lands; treat its long-wait
warning as a failure in waiting). A fixture pins a party and a bag the mainline has to earn.
Anything that changes frame timing re-rolls the RNG stream for every route after it. It rotted once
while its own doc comment claimed it worked. When it fails it reports how far it got and drops
artifacts, and `playthrough::probe_resume_playthrough` replays from there in seconds. If you cannot
make it pass, say so in the hand-off.

## Fixtures

- Every leg snapshots its end state for the next leg, and the write is a no-op without
  `--features regen-fixtures`; otherwise every run silently rewrites the next run's inputs.
  Regenerate in chain order:
  `cargo test --release --features slow-tests,regen-fixtures --bin gb -- can_clear_ss_anne --exact`.
- `at-cerulean.bin` is the root every leg fixture descends from, and
  `early_game::regen_at_cerulean_fixture` (`regen-fixtures`) re-cuts it from a fresh save. A
  `PartyRef` that does not resolve waits for ever rather than failing, so a party change on the
  mainline shows up as a row of legs going red at once.
- Cut a fixture where the mainline stands (a leg that opens with `enter(X)` needs a root saved
  inside the previous building, not in the street) and where the party is healed:
  `Interact(NURSE)` used to pop before the heal landed, and a root came out with Water Gun on 6 of
  25 PP.
- A fixture's name says where it is cut; a leg that walks further than its name is two tests and
  two fixtures (`vr1f-strength`, `vr2f-ladder`). Some retired roots (`post-cascade`,
  `at-mansion-blizzard`, `post-volcano-lone`, `at-saffron-post-silph`) are still on disk but no
  test reads them.
- A grind leg's game-time budget is sized to encounters, not walking. An `enter_at` naming the
  wrong landing only fails from a cold fixture, because the mainline re-routes over a world graph
  the leg's fresh agent has never observed.

## Soak and stalls

- `soak` runs `RandomPolicy` from each entry in `STATES` and fails when
  `PokemonAgent::since_last_policy_poll` passes `GB_SOAK_LIMIT_SECS` (default the watchdog's 300),
  the same value the deployed watchdog reads. A random walker diffuses rather than explores, so the
  budget buys starting points, and a state earns its place by what it makes reachable (a bicycle,
  a Safari counter, a bag with a TM), not by badges.
- It forces the cartridge's deployment options (medium text, animations on, battle style SHIFT) by
  writing them, because every fixture past the fresh save carries fast text, and SHIFT's
  "change POKéMON?" prompt is a screen no other test ever sees.
- It is seeded (`GB_SOAK_SEED`, default 1) and must stay so: seed 1 stays green, vary the seed to
  hunt, `GB_SOAK_MINUTES` to go deeper. Run at 120–150 s to find near-misses; below ~150 finds
  legitimate silences (a WRAP chain measured 124 s). The comments in `soak.rs` carry the numbers.
- Every jam it finds is promoted to `integration_tests::stalls` in the default tier: the state at
  the moment the agent went quiet, replayed against a fresh agent in about two seconds.
  `stalls::probe_stall_artifacts` (`GB_STALL_DIR`) is the bulk form. A jam that lived in the
  agent's own state does not survive the trip, so watch a new case go red before committing it.
  Artifacts are named per state and per seed.
- Nearly everything it finds is a closed loop under A. The rules that cover the class are on their
  constants in `agent.rs` and summarised in [pokemon-agent](pokemon-agent.md). Each is a
  frame-timing change, so `full_playthrough` is the only thing that can price one.

## Turns the game takes back

`LlmPolicy` cancels a turn when the agent asks a different kind of question. Measured on a deployed
run that is one turn in 2430: the agent presses nothing while a turn is in flight, so a battle is
the next turn rather than an interruption of this one. `SlowPolicy` in
`integration_tests::interruption` is the guard and has to key turns exactly as `LlmPolicy` does.
`Worker::give_up` also publishes `turn_cancelled`, so count by `reason`.

## Benchmarking and the goldens

- This machine has fast and slow states ~15% apart. Compare only adjacent paired runs, alternate
  which build runs first, and report both orders. `perf` works without sudo: build with
  `RUSTFLAGS="-C debuginfo=2"` into a scratch target dir and drive with
  `BENCH_FRAMES=40000 BENCH_ONLY=pokemon`. Watch for sampling skid.
- `cgb-acid2` ships its own reference image and pins the 5-to-8-bit expansion as
  `(c << 3) | (c >> 2)`, which `LcdColor::from_rgb555` implements. gambatte's colour correction
  would break the comparison.
- `src/audio/blip/tests.rs` checks the resampler against bit-exact golden vectors from the vendored
  C++ (`tools/blip-golden/build.sh`, after `capture_golden_input` if the input needs refreshing)
  and against invariants that need no toolchain. The goldens pin `GOLDEN_TREBLE_DB`, not
  `DEFAULT_TREBLE_DB`, so a taste change does not invalidate them.
