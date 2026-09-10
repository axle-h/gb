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

# C2's measurement: a stretch of route played through the deployed LlmPolicy against an in-process
# mock, with a god party and a battle script. Prints ms per turn and turns per game-minute. ~5 s.
cargo test --release --features godmode --bin gb -- godmode --nocapture

# C3's walk: 90 game-minutes of exhaustive exploration through the deployed LlmPolicy, from a
# *finished* save with a god party and every key item. Fails on any defect, writes the whole table
# either way, and then prints the ROM cross-check — every warp and object in the headers of the
# maps it entered that never once appeared as a row. ~95 s.
# ⚠️ 90 game-minutes is a smoke budget. A coverage run wants GB_COVERAGE_MINUTES=360 and
# GB_COVERAGE_PATIENCE high (patience, not the budget, is what has stopped every sweep), which
# costs 3-7 min of wall clock. ⚠️ And two runs of identical code differ by more than noise on this
# fixture — 38 maps and 30 maps on the same day — so a single pair of runs cannot A/B a change; see
# coverage-plan §6.2.
cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture

# ⭐ The same walk from every region, and the union of what they reach. One start cannot reach
# Kanto — see coverage-plan §2.1 and §6.1 — so `GB_COVERAGE_START` picks one of ten committed
# fixtures: eight finished games (`phase0` cerulean vermilion lavender celadon saffron fuchsia
# cinnabar) and two cut before the S.S. Anne sails (ssanne ssanneship). `all` walks each in turn and
# prints the union. ⚠️ `all` spends GB_COVERAGE_MINUTES **per region**, so the coverage budget above
# is an hour of wall clock rather than 5 min; for a measurement run the ten in parallel, one per
# directory, and union the walk-*.tsv files (the recipe is coverage-plan §6.1).
GB_COVERAGE_START=all cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture

# The stall hunt: 40 min of game time under RandomPolicy from each of 26 starting states, in
# parallel. ~39 s each, about 5.5 min of wall clock on 16 threads.
cargo test --release --features soak-tests --bin gb -- soak --nocapture

# Re-cut the twelve soak starting states that are taken off the mainline. Plays the eight-badge
# route once (~5.5 min) and writes src/pokemon/data/soak-*.bin.
cargo test --release --features full-playthrough,regen-fixtures --bin gb -- \
  pokemon::integration_tests::playthrough::regen_soak_checkpoints --exact --nocapture

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
#                      On a save cut mid-battle it prints the fight and its menu instead of the grid,
#                      which is how the battle_refusals fixtures were picked.
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
| `godmode` | C2's measured run. The machinery under it — `Intent`, `ScriptedBrain`, `cheats::Cheats`, `coverage::CoverageLog` — is all default tier; only the run that spends game time is gated |
| `coverage-tests` | C3's frontier walk, the one test that spends game time going *everywhere*. Same split: the oracle is default tier and only the walk is gated |

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

## Writing an assertion that polls

- ⚠️ **An `LlmRun` test has two deadlines and the *emulated* one is the trap.** The emulator keeps
  running while the worker thinks, so a mock round trip slowed by a loaded machine is paid for in
  **game** seconds — and `game_time` running out is a panic from `step_coarse`, not the readable
  assertion the test was written around. Size it minutes above what the test needs (`branch_points`
  uses six game-hours, `battle_refusals` fifteen game-minutes) and let the wall-clock deadline bind.
  Both are deadlines: a passing run spends neither. Found by running the default tier twelve times
  back to back, which lost a different test on two of the twelve; a single green run proves nothing
  about this class.
- ⚠️ **A Pokédex bit and a party count are never true on the same tick.** `_AddPartyMon` increments
  `wPartyCount` first and sets `wPokedexOwned` about eighty lines later
  (`engine/pokemon/add_mon.asm`), so a test that waits for the party to grow and then asserts the dex
  in the same sample is a race — one that fails roughly **one default-tier run in five**, on a
  different test each time, which reads as a flake rather than as a bug in the test. Wait for each
  separately (`branch_points::assert_owned`). The same shape applies to anything the cartridge writes
  in two steps.

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
- `split-cerulean.bin`, `split-celadon.bin` and `pocket-route14.bin` are not part of the chain
  either. They are `issues/turn-<id>/state.gbst` lifted straight out of the deployed run of
  2026-09-02, cut where the model was actually standing, and the property they carry is *which
  terrace* — so a fixture re-cut a tile away is testing a different map. `llm::tools::tests` and
  `llm::prompt::tests` read them.
- ⚠️ **`postgame-hidden-item.bin` was deleted on 2026-09-03** along with the H4 leg that produced it
  (hidden-item collection is gone from the crate — see
  [deployed-run-defects](deployed-run-defects.md) W10). The three H5 legs that read it now root on
  `postgame-itemfinder.bin`, its predecessor. A fixture whose producing leg is removed has to be
  re-pointed or deleted, never left in the list: `every_committed_fixture_decodes` would still load
  it and nothing would say it had stopped being reachable.
- ⚠️ **The four `branch-*.bin` are cut one decision *before* a choice that is exclusive per save**,
  by the `regen_*` tests in `integration_tests::branch_points` (`regen-fixtures`), each from a leg
  fixture the chain already produces — `start-of-game-state`, `mt-moon`, `postgame-lapras`,
  `postgame-bike-voucher`. Nothing reads them but that file, so they are free to re-cut; what they
  have to keep is the branch still being *open*, which each cutter asserts. ⚠️ **And none of them may
  be cut inside a trainer's line of sight**: a trainer walks up to the player on the map's own tick
  rather than on a step, so such a state restores straight into a battle and the arm's first turn is
  a battle turn instead of the menu the branch lives in. Mt Moon B2F's landing is inside `ROCKET1`'s,
  and the cutter fights him on the way past for exactly that reason.
- `vr3f-boulder-given-up.bin` is the same kind of thing and the newest: the coverage sweep of
  2026-09-10 dropped it where `VictoryRoad3F:3,5:PushBoulderOntoSwitch` gave up after 34 pushes
  without reaching its target, five times on one walk. It has **no producing leg** and is not in the
  chain, so it must not be re-cut; its property is *which puzzle, mid-solve*. See
  `docs/coverage-plan.md` step 1.1, which is the work it is evidence for.
- `route21-islands.bin` is the same kind of thing: the deployed run of 2026-09-03's own checkpoint,
  at Route 21 (7, 72) mid-crossing and mid-battle, read by
  `stalls::a_water_route_does_not_climb_out_onto_route_21s_islands`. Its property is *which map*, so
  it must not be re-cut somewhere tidier, and it is not in the leg chain.
- `vr3f-strength.bin` is the other **mid-leg** cut, by `endgame::cut_vr3f_fixture`: the 1F climb
  plus the 2F half of `victory_road_2f_3f_steps()`, stopping on VictoryRoad3F with Strength armed
  and its four boulders untouched. Same reason as `seafoam-b3f.bin` below — the boulder work lives
  in the last steps of a long leg — and the same rule: not read by the chain, so free to re-cut, but
  it must keep landing on 3F armed.
- `seafoam-b3f.bin` is a **mid-leg** cut, and the only other one: `cinnabar::cut_seafoam_b3f_fixture`
  runs `seafoam_articuno_steps()` truncated at the first `DropBoulderInHole` and saves there. The
  Articuno leg is 60 game-minutes end to end and its whole difficulty lives in its last two steps,
  so debugging the boulder puzzle through the leg meant a minutes-long round trip per idea; off this
  fixture it is ten seconds. It is not something the chain reads — `post-articuno.bin` still comes
  from the full leg — so it can be re-cut freely, but it must keep landing on B3F with Strength
  armed and all four boulders untouched, which the cutter asserts.
- ⭐ **The four battle fixtures are read twice over**, by `stalls` for the jam each was cut in and by
  `integration_tests/battle_refusals.rs` for the refusal each one *contains*:
  `stall-battle-key-item.bin` is a wild Oddish with five Poké Balls (a ball that fails, an item the
  bag runs out of), `stall-battle-key-item-trainer.bin` a trainer's Weedle with eight Great Balls (a
  run withheld, a ball a trainer blocks), `stall-safari-menu.bin` a Safari Rhyhorn (the game ending
  around a battle), and `postgame-sold.bin` stands in Viridian City with the old man **awake**.
  ⚠️ **A pre-Pokédex Viridian fixture is the wrong one and looks right**: `EVENT_GOT_POKEDEX` is what
  hides the sleepy old man in the road and shows the one who offers the tutorial. A mid-battle save
  is expensive to cut, so re-reading these is deliberate — but it means a re-cut has to keep *which
  battle* as well as which jam.
- `soak-*.bin` are **not** part of the chain: nothing reads one as the input to a route, so the
  rules above about cutting where the mainline stands and where the party is healed do not apply to
  them. They are re-cut wholesale by `regen_soak_checkpoints`, never by hand.
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
- It walks under `RandomPolicy::exploring`, not `seeded`: the last `EXPLORE_MEMORY` overworld action
  ids are kept and each repeat multiplies that action's weight by `EXPLORE_DECAY`. It forbids
  nothing (floors exist whose only exit is the one just used) and it leaves the battle draw uniform
  (the argument is on `exploring`); it is still fully seeded. `--policy random` is untouched.
- Half the starting states are **checkpoints cut off the mainline**, listed as
  `playthrough::SOAK_CHECKPOINTS` and written by `regen_soak_checkpoints` — the route already walks
  through Mt Moon, an unlit Rock Tunnel, Silph Co and the Cinnabar quiz, so a copy taken on the way
  past costs no hand-cut fixture. Each carries an `expect_map` asserted on the way in, and the
  regeneration fails naming any checkpoint whose map the route no longer crosses. The other half are
  hand-cut, and are the states a fuzzer cannot reach on its own in forty minutes.
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
- ⚠️ **`soak` cannot see anything about the agent's tick *rate*, and neither can any other test
  here.** Every one of them drives `TestFixture::step` — `gb.run(AGENT_RESOLUTION)` then one
  `agent.update` — so the agent is always handed exactly one tick's worth of emulated time. Both
  real drivers hand it however long their last loop iteration took, and the defect of 2026-09-03
  lived entirely in that gap. `TestFixture::step_coarse` is the way to test at a driver's cadence;
  `mechanics::a_corner_is_turned_at_a_coarse_host_tick` is the first case.
- Nearly everything it finds is a closed loop under A. The rules that cover the class are on their
  constants in `agent.rs` and summarised in [pokemon-agent](pokemon-agent.md). Each is a
  frame-timing change, so `full_playthrough` is the only thing that can price one.

## The LLM end-to-end harness

`integration_tests/llm_harness.rs` is the assembly every LLM test is a client of: a mock OpenAI
endpoint on a loopback port, a pluggable `Brain`, and `LlmRun` — worker, policy, agent, emulator and
a real run directory. Default tier; the whole of `llm.rs` runs in about two seconds.

- ⚠️ **A `Brain` is handed strings and nothing else, and the type enforces it.** No `GameState`, no
  `&mut GameBoy`, no fixture handle. If a brain cannot find what it needs in the rendered situation
  and the action menu, a real model cannot either — and that is a finding about `llm::prompt`, not a
  test to work around. Anything that adds a live-state field to `TurnRequest` throws the property
  away.
- ⚠️ **It drives `step_coarse`, not `step`** — see the ⚠️ under *Soak and stalls* below. It is the one
  test assembly that runs at a driver's cadence rather than the harness's.
- ⚠️ **The endpoint fragments tool-call arguments across several `data:` frames and interleaves
  parallel calls**, on purpose. That is the part of the wire format most likely to be got wrong.
  Inherited from the mock it replaces; do not simplify it away.
- The seven `Fault`s — a hard HTTP status, a dated 429, an undated 429, a timeout, malformed
  arguments, a truncated stream, an empty choice — each have a test asserting *what the run does
  next*. The dated 429 asserts the **cartridge clock** did not advance, which is what the
  leaderboard ranks on, so `LlmRun::tick` honours `throttled_until` exactly as `host.rs` does.
- `LlmRun::restart` checkpoints, drops the worker and the fixture, and rebuilds both from the run
  directory. It is the only thing that exercises `GB_RESTORE_HISTORY`, the re-minted system prompt
  and `prompt::RESUMED_NOTE` end to end.
- Backoff is `NO_BACKOFF` by default. The shipped policy sleeps 1+2+4+8 s between attempts and none
  of that is what a fault test is asserting.

## Coverage and cheats

- `coverage::CoverageLog` folds `AgentEvent`s into a verdict per action id, so it works under any
  driver. `TestFixture::with_coverage()` turns it on — **opt-in, because the fixture then owns the
  event stream**: the agent's buffer is drained rather than peeked and is capped at 100, so a test
  that opts in must read events from `fixture.coverage` and not from `agent.drain_events()`.
- ⭐ **A defect drops a save state and a screenshot where it happened**, into
  `target/test-artifacts/coverage/defect-<id>_state.bin`, taken by `TestFixture::observe_coverage`
  on the tick the verdict turns. ⚠️ **It cannot be taken at the end of the run.** Exploration is
  destructive and mostly one-shot — a sprite talked to is gone, an item picked up is gone — so by
  the time a walk ends the square that failed cannot be stood on again. A dropped state that turns
  out to be the only way to reach a case gets **committed**, exactly as a `soak` jam does:
  `data/seafoam-b3f-on-the-water-warp.bin` is the first, and
  `cinnabar::a_seafoam_warp_on_the_water_is_stepped_onto_rather_than_leant_on` is its two-tick test
  in the default tier. `target/` is swept, so copy one out before it is. Three more have been
  committed the same way since: `silph-elevator-warped-in.bin`
  (`saffron::an_elevator_door_you_warped_onto_is_stepped_onto_rather_than_leant_on`) and
  `celadon-mansion-pets-in-the-way.bin`
  (`celadon::a_route_a_wandering_pet_is_standing_on_is_waited_out_rather_than_disputed`), both from
  the sweep of 2026-09-10.
- ⭐ **`probe_button_at_state` is how a dropped state is argued from**, and every warp finding so far
  was misdiagnosed without it: `GB_PROBE_STATE=…_state.bin GB_PROBE_BUTTONS=down:60,up:40,down:120`
  prints the map, the position, the mode and `wMovementFlags` per 25 ticks, plus the map's raw tile
  ids. Reading the ROM and reasoning is what produced the wrong causes in
  [coverage-plan](coverage-plan.md) §7.2's items 8 and 13.
- ⚠️ **The `cheats` line says what was *shed* as well as what would not fit, and both matter.** Gen
  1's bag holds twenty *kinds* and a finished save arrives with fourteen to twenty used, so until
  2026-09-10 the walk was refused between one and nine of its key items, differently per start, and
  the refusal was reported rather than fatal — a sweep looking healthy while Rocket Hideout's four
  floors, the Game Corner prize room and every `Fish` row in the game were unreachable. Room is now
  made first (`debug_keep_only_items`), `every_coverage_start_can_be_handed_all_of_the_key_items`
  asserts it in the default tier, and the line prints the junk that went so a walk that turns out to
  have needed one of them can see which start dropped it.
- ⚠️ **The walk is not reproducible, and that is `step_coarse` rather than a bug.** `LlmRun` hands
  the agent however long the driver's last loop iteration took, with a worker thread and a real
  socket in that loop, so three runs from the same fixture gave 352, 355 and 360 ids (always 28
  maps) and two different defects. Read the totals as a measurement with a couple of per cent on
  them, and chase a defect from its save state rather than by re-running.
- ⚠️ **A repeat is the signal, not the first block.** Being stopped is how this game says almost
  everything, so `Textbox`/`Script` is `Blocked` and only becomes a defect past
  `coverage::REPEAT_IS_A_DEFECT`. Everything that says the agent could not execute a row it had
  already offered — `NoRoute`, `DidNotArrive`, `WrongMap`, `NoAdjacentGrass`, `Unknown`,
  `CastRefused`, `CastNeverFinished` — is a defect on the first one, and a watchdog firing always is.
- ⭐ **A `Silent` fails the walk too, as of 2026-09-09** (`Verdict::fails_the_walk`, and the list the
  test asserts on is `CoverageLog::failures`, not `defects`). A row chosen and never reported is the
  failure mode this tier exists to find; the two counts stay apart in `summary()` because a defect is
  a row the agent could not carry out and a silence is one it did and never spoke about.
  [coverage-plan](coverage-plan.md) step 1.
- `cheats::Cheats` is applied by the **driver between ticks**, never from a policy, which is what
  keeps a finding from a cheated run trustworthy: `LlmPolicy` is byte-identical to the deployed one
  and sees the result only through an ordinary `GameState`.
  `postgame::debug::play_path_contains_no_debug_ram_writes` still passes unchanged and must keep
  doing so.
- ⚠️ **A party write mid-battle desynchronises `wBattleMon` from the party struct** — Gen 1 copies the
  active member out on send-out and back on switch-out — so the sidecar gates its top-up on
  `!in_battle` *and* on the black-out window (`wIsInBattle == $ff`) being closed. Both refusals are
  counted and both have a test.

## The battle refusals

`integration_tests/battle_refusals.rs` is `docs/coverage-plan.md` step 7: the seven cells the battle
matrix audit found nowhere, every one of them a refusal, each as one default-tier test through
`LlmRun`. Read it before touching `BattleState::UsingItem`, `PokemonTextReader` or
`tools::not_on_the_menu`.

- ⚠️ **Two of the seven need a `debug_` write, because Gen 1 has no state in which a ball certainly
  fails or an escape certainly does.** `debug_set_catch_rate(0)` leaves about **one throw in 720**
  catching anyway and `debug_set_battle_speeds(1, 255)` leaves **one first escape in 256** working;
  both residues are on the primitive in `postgame/debug.rs` and in the test that carries one. Written
  once before the first tick rather than held every tick: the cartridge only moves either value on a
  send-out or a Safari BAIT/ROCK.
- ⚠️ **Assert on the *front* of a quoted sentence, never a phrase from the middle.** The reader
  samples the screen once per agent tick and the item driver dismisses the box a tick or two later,
  so a quoted line arrives a character or two short — measured over eight runs as "The trainer
  blocked the BA" and "…BAL", never the closing "BALL!".
- ⚠️ **A refused id is answered *inside* the turn**, so the request that proves the model was told
  anything is the one *after* the refusal. A test that stops at two battle turns sees the acceptance
  and not the rejection.
- ⚠️ **Take the brain's lock inside a `tick_until` predicate, never across it.** The brain writes to
  the same log from the worker thread.

## Turns the game takes back

`LlmPolicy` cancels a turn when the agent asks a different kind of question. Measured on a deployed
run that is one turn in 2430: the agent presses nothing while a turn is in flight, so a battle is
the next turn rather than an interruption of this one. `SlowPolicy` in
`integration_tests::interruption` is the guard and has to key turns exactly as `LlmPolicy` does.
`Worker::give_up` also publishes `turn_cancelled`, so count by `reason`.

## Benchmarking and the goldens

- ⚠️ **`bench_core_throughput` measures two different machines, and `BENCH_AUDIO=off` picks the one
  that matters.** It gates the APU's output side, which is what every agent tier and the deployment
  run; the default is a listener attached, which is the desktop UI and a viewer who pressed the
  speaker. The gap used to be 89x against 108x and [emulator-performance](emulator-performance.md)
  §6.3 closed most of it, but the mixer still only exists in the open configuration, so a number
  quoted without saying which one it is is not a number.
- ⚠️ **`--features bench` builds a binary with two APU tests missing**, and that is deliberate:
  the control machine they need is a `#[cfg]`'d field on `Audio` that measured **2.5%** in the
  layout of `MMU::update` if the bench build carried it — [emulator-performance](emulator-performance.md)
  §6.3. `cargo test --release` has them. Do not "fix" the cfg without re-measuring.
- ⚠️ **blargg's `dmg_sound` tests run the *un-batched* APU**, because nothing sets up a control
  machine for them. `game_boy::tests::deadline_driving_the_channels_is_invisible_to_the_game` is
  the one that runs the same suite batched, and
  `batching_the_channels_under_a_listener_is_inaudible` is the only thing anywhere that compares
  the *samples* rather than the machine — do not let either be deleted as a duplicate.
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
