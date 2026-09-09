# Full exploration through the LLM path

**The goal, in one sentence.** Every action the game offers, taken once, through the stack exactly
as it is deployed — `LlmPolicy`, the worker and the wire, against a mock endpoint in-process — with
a verdict on each, and every defect that turns up fixed, until a sweep of the whole of Kanto comes
back clean and finds nothing new.

**Status.** Rewritten 2026-09-09 as a step list, from a review of the first three days' work;
**steps 0 and 1 taken the same day**. The harness, the cheats, the oracle and the frontier walk are
all **built**; the baseline sweep reaches **153 maps of 248** and came back **red with 106 defects
and 41 silences**, with **82% of every turn it took spent in Route 16's gate**. §2 is that baseline
and every later step measures against it. **The 41 silences are now 0 and a silence fails the tier**
(step 1); the 106 defects and the gate are untouched, and are steps 2 to 4. The steps in §4 are the path from there to a
clean fixpoint, in the order to take them. §7 keeps what the first draft got wrong and what the
sweeps have found, condensed, because most of that was agent bugs a paying run would have met.

⚠️ **Code comments cite section numbers from the first draft** (`§2.2.1`, `§3.3`, `§5.2.6`, …).
Those refer to the 2026-09-06 plan, which is in git history at commit `7343616`; §7.3 says where
each one's argument lives now.

---

## 1. What is built

| | Where | What it is |
|---|---|---|
| The harness | `integration_tests/llm_harness.rs` | `MockEndpoint`, a `Brain` that is handed **strings and nothing else**, seven injectable faults, and `LlmRun`: the real worker, policy, agent and emulator with a run directory and a restart |
| The cheats | `integration_tests/cheats.rs`, `postgame/debug.rs` | A god party (Mewtwo with four attacks, one slave with Cut/Surf/Strength/Flash, one with Fly), badges and key items, applied by the driver **between ticks**. `play_path_contains_no_debug_ram_writes` guards the line |
| The oracle | `integration_tests/coverage.rs` `CoverageLog` | Every action id the agent starts gets `Completed`, `Blocked`, `Defect`, `Silent` or `Unreached`, folded from `AgentEvent`s so it works under any driver. Drops a save state where a defect happened |
| The walk | `coverage.rs` `ExploringBrain`, `coverage_walk_of_the_finished_game` | Takes every unvisited row on the map, then the least-taken exit. Starts from one of eight **finished-game** fixtures (`GB_COVERAGE_START`) so every gate is open because the cartridge opened it. Behind `--features coverage-tests` |
| The cross-check | `coverage.rs` `rom_cross_check` | Warps and objects in the ROM's tables for the maps entered that never once appeared as a row. Printed, never asserted |
| The god run's machinery | `integration_tests/godmode.rs` | `Intent`, `ScriptedBrain`, `godmode_turn_cost`. Parked — see §5 |

## 2. Where we are

**The baseline, 2026-09-09** (step 0). Eight walks in parallel on an otherwise idle machine, 6
game-hours each, 5.6 to 7.5 minutes of wall clock apiece and about 7.5 minutes for the set. Every
number below came out of that run; the 2026-09-08 table it replaces is in git history.

| | | | | | |
|---|---|---|---|---|---|
| **start** | **maps** | **ids** | **turns** | **defects** | **silent** |
| `phase0` | 30 | 315 | 776 | 0 | 4 |
| `cerulean` | 86 | 696 | 3 781 | ⛔ 28 | 15 |
| `vermilion` | 58 | 476 | 4 971 | 0 | 0 |
| `lavender` | 34 | 218 | 6 427 | 0 | 0 |
| `celadon` | 86 | 725 | 4 551 | ⛔ 38 | 15 |
| `saffron` | 49 | 373 | 6 425 | ⛔ 36 | 3 |
| `fuchsia` | 69 | 533 | 4 970 | ⛔ 4 | 4 |
| `cinnabar` | 33 | 341 | 618 | 0 | 0 |
| **union** | ⭐ **153 of 248** | ⭐ **1 445** | 32 519 | ⛔ **106** | **41** |

**Not one of the eight settled.** All eight spent the whole 21 600 s game-time budget with the
frontier still open, so every one of these numbers is a floor rather than a plateau.

- **Defects: 106, on four maps.** `SaffronGym` **99**, `SilphCo1F` **3**, `SeafoamIslandsB4F` 2,
  `SafariZoneCenter` 2. Every one of them reads *"there is no route to …"*. Saffron Gym is step 3
  and the Safari Zone pond is step 2, both reproduced exactly as those steps describe. ⭐ **Silph
  Co 1F is new** and was in no earlier sweep; it has warp pads of the same family as the gym's, so
  step 3's fix is expected to take it, and if it does not it is a finding of its own.
- **Silent: 41, and they were no longer only fishing.** `Fish` 35, `Grass` 3
  (`Route18:39,13`, `Route24:5,18`), `Warp` 3 (`SeafoamIslandsB3F:20,17`, `:21,17`, `:25,14`).
  ✅ **All of it closed by step 1 the same day, and the count is 0 in all eight regions**; a silence
  now fails the tier. Two of the three families were one bug each and the third was not a family at
  all: the fishing walk mounted Surf on the *turn* that faces the water, the `Grass` pair is a
  trainer noticing the player mid-pace through a door `PacingForEncounters` does not own, and the
  three Seafoam warps score `Defect` on every walk that reaches them (step 2's ⚠️), the `Silent` here
  being the variance in §6.2 rather than a third thing. ⚠️ **Step 1 was written believing fishing was
  the whole list**, which is why the assertion was its last item and not its first.
- **Where the turns went: Route 16's gate, and it is worse than the last table said.** The
  `Route16` / `Route16Gate1F` / `Route16FlyHouse` triangle took **26 573 of the sweep's 32 519
  turns, 82%**, and in six of the eight regions it is between 57% and **97%** of everything that
  region did. `lavender` spent 6 265 of 6 427 turns there and reached 34 maps. Step 4.
- **`phase0` did not reach the Hall of Fame this time.** It spent its whole budget in Victory Road
  and came back with 30 maps; `cinnabar` did the same and came back with 33. ⚠️ **That is a third
  outcome §6.2 does not list** (38-and-stopped, or ~61-having-missed-Indigo). Read §6.2's "two
  modes" as "several", and take the terminus off any single run rather than off this file.

⚠️ **Take every baseline from a run, never from this table.** The walk is not deterministic, and its
spread is not noise: see §6.2. The union here is 153 against the previous sweep's 159 while the id
count rose from 1 409 to 1 445, which is what that variance looks like from outside.

**The 95 maps no walk entered** (step 5 makes the walk print this itself; this list is the diff of
`Map::iter()` against the union). 22 of them are the ROM's `UnusedMap*` padding and two are the
link-cable rooms `Colosseum` and `TradeCenter`, which leaves **71 real maps**:

`BluesHouse` `OaksLab` `RedsHouse1F` `RedsHouse2F` `ViridianMart` `Route2TradeHouse`
`DiglettsCave` `DiglettsCaveRoute2` `DiglettsCaveRoute11` `Route11` `Route11Gate1F`
`Route11Gate2F` `Route17` `Route12SuperRodHouse` `Route16Gate2F` `VermilionDock` `VermilionGym`
`VermilionMart` `VermilionPokecenter` `VermilionOldRodHouse` `VermilionPidgeyHouse`
`VermilionTradeHouse` `SSAnne1F` `SSAnne1FRooms` `SSAnne2F` `SSAnne2FRooms` `SSAnne3F`
`SSAnneB1F` `SSAnneB1FRooms` `SSAnneBow` `SSAnneCaptainsRoom` `SSAnneKitchen` `CeladonGym`
`CeladonDiner` `CeladonHotel` `CeladonChiefHouse` `GameCorner` `GameCornerPrizeRoom`
`RocketHideoutB1F` `RocketHideoutB2F` `RocketHideoutB3F` `RocketHideoutB4F`
`RocketHideoutElevator` `PokemonFanClub` `CinnabarGym` `CinnabarLab` `CinnabarLabFossilRoom`
`CinnabarLabMetronomeRoom` `CinnabarLabTradeRoom` `CinnabarMart` `CinnabarMartCopy`
`CinnabarPokecenter` `PokemonMansion1F` `PokemonMansion2F` `PokemonMansion3F` `PokemonMansionB1F`
`CeruleanCave1F` `CeruleanCave2F` `CeruleanCaveB1F` `SafariZoneWestRestHouse` `IndigoPlateau`
`IndigoPlateauLobby` `LoreleisRoom` `BrunosRoom` `AgathasRoom` `LancesRoom` `ChampionsRoom`
`HallOfFame` `CeruleanTrashedHouseCopy` `UndergroundPathRoute6Copy` `UndergroundPathRoute7Copy`

⚠️ **A cluster in that list is a start that is missing, not necessarily a gate doing its job.**
Vermilion's seven maps and the whole S.S. Anne sit next to a `vermilion` start that reached 58
maps without entering any of them, and the Elite Four's six rooms are behind a walk that no longer
gets there. Step 6.3's first tool is another start, and this list is its input.

## 3. Rules that hold for every step

Each one is argued in full in a code comment; this is the index.

- **Play path is button input only.** Cheats live in the driver between ticks and reach the policy
  only through an ordinary `GameState`. `play_path_contains_no_debug_ram_writes` must keep passing
  unchanged. (`postgame/debug.rs` module comment, `cheats.rs`.)
- **A brain sees strings.** No `GameState`, no fixture handle. If a brain cannot find what it needs
  in the rendered turn, a model cannot either, and that is a finding about `llm::prompt`.
  (`llm_harness::Brain`.)
- **No event-flag writes.** Start from a save the cartridge itself finished. A save with flags set
  behind its scripts makes every stall found in it a false positive. (`coverage::Start`.)
- **A terminus is reported, never made unreachable.** Filtering the Hall of Fame out of the menu
  left the walk at the title screen for 20 000 turns reporting zero defects.
  (`ExploringBrain::respond`, the `HallOfFame` arm.)
- **A block is not a defect until it repeats.** Being stopped is how this game says almost
  everything; `REPEAT_IS_A_DEFECT` is 10. (`coverage.rs`.)
- **Do not build while you measure.** `step_coarse` hands the agent however long the last loop
  iteration took, so CPU load is an input to the walk. Eight walks in parallel on an idle box agree
  to ±1.3%; one walk beside a `cargo build` does not agree with itself. (§6.2.)
- **Promise-first exit ordering loses.** Tried twice, 41 maps to 21. The ordering is
  `(times taken, promise)` with a two-try priority for doors into unseen maps. Anything done to the
  frontier has to survive that. (`ExploringBrain::respond`, the `min_by_key`.)
- **A routing or agent change is behind the leg chain, `full_playthrough` and, where the scripted
  route walks there, `hall_of_fame`.** `full_playthrough` is a golden RNG replay: a correct change
  fails it hundreds of steps from the cause, so A/B against HEAD before debugging the stall site.
- **Fix the driver, not the walk.** A row the menu offered that the agent cannot then execute is an
  agent bug a deployed model would meet. The walk is a test file and changes to it are for the
  walk's own faults only.
- **No em dashes in the strings the agent generates.** Anything a step adds to an event's prose is
  in scope.
- **A new invariant goes into a code comment first**, then one line in the area doc:
  [pokemon-agent](pokemon-agent.md) for the agent and the oracle, [test-suite](test-suite.md) for
  the tiers, [llm-turn-loop](llm-turn-loop.md) for the harness.

---

## 4. The steps

In order. Each says what "done" is as something checkable, and roughly what it costs. Steps 1 to 5
are the ones that turn the tier green; step 6 is the loop that runs until the goal is met; 7 to 9
are what the walk cannot reach.

### Step 0 — Take the baseline ✅ done 2026-09-09

Run the eight regional walks in parallel on an idle machine (§6.1) and write the per-region table
and the union into §2, replacing the 2026-09-08 numbers. Every later step measures against this.

**Done:** §2 holds today's numbers and the date. **Cost:** 10 minutes of wall clock, and nothing
else may be building during it.

✅ **Taken.** §2 is the result. Three things it changed elsewhere in this file: `TMPDIR` is now part
of §6.1's recipe and the reason is a paragraph of its own; §6.2's "two modes" for `phase0` is
"several"; and steps 1 and 3 each carry a correction the baseline forced, because both were written
against numbers this run disagrees with.

### Step 1 — Make silence a failure, and stop fishing from a surf ✅ done 2026-09-09

**What was wrong.** A `Fish` row's walk to "the water's edge" surfed onto the water to get there, so
`postgame/fishing.rs` refused with *"cannot fish while surfing"* and dropped to `Idle`. That refusal
and its two siblings (the tick budget, unreachable water) emitted a text box the agent made up and no
terminal event, so the oracle scored the id `Silent` and the walk did not fail. It was the shape the
grass pace and the cut tree were fixed for, one driver over. Reproduce:

```shell
GB_COVERAGE_MINUTES=6 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
# silent     {"Fish": 1}     ViridianCity:8,25:Fish
```

✅ **Taken, and all three items are in.**

1. **The route to a `Fish` row ends on land, and the cause was not the one this step assumed.** The
   step was written believing `actions()` picked a shore across the water, and it does not: the
   Fish block is `route_to_face_within`'s body inlined over the same `bfs_from_player` prices in the
   same order, so it always lands on the square `nearest_castable_water` had already validated a
   land route to. What the walk actually mounted Surf on was **the last button**, which is the *turn*
   toward the water, and `OverworldMovement`'s Surf-mount arm could not tell it from a step into it.
   The arm now suppresses the mount for a `MetaTile::Fish` row outright. ⚰️ A first attempt filtered
   the shore squares by `route_stays_on_land` and was provably a no-op; it was removed rather than
   left in looking load-bearing.
2. **All three refusal exits end in `OverworldActionAborted`** — `CastRefused(CastRefusal)` for the
   three `blocked_by` reasons, `CastNeverFinished` for the tick budget, `NoRoute` for a shore it
   cannot reach. ⚠️ **The words moved into the reason rather than staying in a `TextBox` beside it**,
   which is what `NothingAppeared` did for the grass pace: a box the agent writes says the cartridge
   said it. `PokemonAgent::abort_overworld` and `player_at` are `pub(crate)` for this.
3. **`Silent` fails the walk** (`Verdict::fails_the_walk`, `CoverageLog::failures`, and the
   `WalkOutcome` field is `failures` now). The counts stay apart in `summary()`: a defect is a row
   the agent could not carry out, a silence is one it carried out and never spoke about.

**And the other two families the baseline warned about:**

- **The `Grass` pair was one bug and it was not in the pacing arm.** `Route18:39,13:Grass` and
  `Route24:5,18:Grass` are a trainer noticing the player mid-pace: the walk-up runs as
  `GameMode::Script`, commits past the rollback deadline, and drops the state into
  `AwaitingOverworldAction`. `PacingForEncounters` reports all three of its *own* exits, but the two
  that take the state away from outside — `assert_script_state`'s commit and `assert_text_box_state`
  — knew only about `OverworldMovement`. `AgentState::open_overworld_action` is now the one list of
  states that carry a row and both doors read it.
- **The three Seafoam warps are not silences.** Six `fuchsia` walks scored
  `SeafoamIslandsB3F:21,17:Warp` and `SeafoamIslandsB4F:21,17:Warp` `Defect` every time, on
  `DidNotArrive`, which the walk already fails on. The baseline's `Silent` for them was the variance
  §6.2 describes, not a third family. They are step 2's ⚠️ and stay there.
- **No `PushBoulder*` exemption is needed and the note claiming one is retired.** The row is a
  `BoulderGoal` now and the *goal* completes: 52 of them across six sweeps of 2026-09-09, none
  silent. The shove itself is still invisible; that is a different sentence and it is on
  `AgentState::PushingBoulder`.

**Done, measured 2026-09-09** on the eight regional walks in parallel (§6.1): **`silent {}` in every
one of the eight**, against 41 on the baseline the same day. Defects are unchanged in shape at 103
(SaffronGym 95, SilphCo1F 3, Seafoam 2, SafariZoneCenter 1, and two one-off "no route to <person>"
on a moving NPC in a mart and a Pokémon Centre); union 151 maps and 1 381 ids, inside the variance
§6.2 describes. `cargo test --release` green (1 579), the leg chain green (215),
`full_playthrough` green.

### Step 2 — The Safari Zone pond: rows minted before the map is the map

**What is wrong.** From the `fuchsia` start, the gate script takes the fee and auto-walks the player
into `SafariZoneCenter`; on that tick `actions()` minted `SafariZoneCenter:Nugget` and
`SafariZoneCenter:0,10:Warp`, both across a pond Surf is refused on, and the walk aborted with
*"there is no route"* on the next tick. The screenshot beside the dropped state is black. It is the
`position_settled` family (`MetaTileMap`'s own ⚠️) one map-load earlier. Reproduce in 30 seconds:

```shell
GB_COVERAGE_START=fuchsia GB_COVERAGE_MINUTES=3 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
```

**What to do.** Do not mint rows while a map transition is in flight; pin it with a `mechanics` test
off the dropped state, next to `a_coordinate_that_underflows_a_map_edge_is_not_a_position`.

⚠️ The same region has two rows this is *not*: `SeafoamIslandsB3F:21,17:Warp` and
`SeafoamIslandsB4F:21,17:Warp` give up after 60 s standing on the square they were walking to.
Undiagnosed. Cut the state fresh from a `fuchsia` walk and argue from it.

**Done:** the `fuchsia` walk at the full budget has 0 defects; `cargo test --release` green;
`full_playthrough` green. **Cost:** small to medium.

### Step 3 — Saffron Gym's teleport pads

**What is wrong.** 30 of the gym's 32 warps lead back into the same map. The route to one pad
crosses others, stepping on one relocates the player, and the row is gone by the time the walk
looks for it: 37 to 40 defects, and `celadon` spent 3849 of its 4592 turns in the room. The agent
already has the model for this shape — `spinners`, the Rocket Hideout arrow tiles, where the BFS
treats stepping onto an arrow as landing at its destination — and does not apply it here.

```shell
GB_COVERAGE_START=celadon GB_COVERAGE_MINUTES=360 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
```

**What to do.** Treat a teleport pad as the spinners are treated: a square you cannot walk across,
whose landing is its destination. Then a route to a pad is a route that ends at the pad, and a route
that would cross one is not a route.

⭐ **And Silph Co 1F is the same shape.** The 2026-09-09 baseline put 3 defects on `SilphCo1F`,
which no earlier sweep had ever reached; its warp pads are the same family as the gym's, so the fix
here is expected to take them. If it does not, that is a second finding and not this step's.

**Done:** `celadon`, `cerulean` and `saffron` walks with 0 defects in the gym **or on Silph Co 1F**
and the gym no longer their busiest map; the leg chain, `full_playthrough` **and** `hall_of_fame`
green, because the scripted route walks this gym for the Marsh Badge. **Cost:** medium, and all of
it in the routing and its verification.

### Step 4 — Route 16's gate, in the brain

**What is wrong.** Route 16 has four squares facing the gate's four doors, so the pair of maps
carries eight warp ids and every one works. Once a map's other rows are done the brain takes the
least-taken exit, and on a two-map ping-pong that is always the door back. Four walks spent 60% to
97.6% of their turns there, and the two worst were two of the three lowest map counts. `times` is
per **id** and the thing being oscillated over is a **pair of maps**.

**What to do.** Count exits by the map they lead to rather than by id, so eight doors into one gate
are one option taken eight times. `ExploringBrain::maps` already tallies turns per map and nothing
in the ordering reads it. ⚠️ This is not promise-first ordering, which is the fix that looks obvious
and lost 20 maps twice.

**Done:** measured as §6.2 says — eight regional walks a side on an idle machine — the four affected
regions' `busiest` line is no longer the gate and the union has not fallen. **Cost:** small to
write; the whole cost is the measurement. The brain is a test file, so nothing that ships is behind
it.

### Step 5 — Print the maps never entered

The sweep prints a union of 159 and nothing about the 89. `Map::iter()` minus the union, printed by
the multi-region summary, is what tells a gate doing its job from a walk that never arrived, and it
is the input to step 6.

**Done:** `GB_COVERAGE_START=all` prints the unreached maps sorted, and §2 carries the list from
the next baseline. **Cost:** small.

### Step 6 — The loop, until the fixpoint

Everything above is one pass. The goal is a fixpoint, and this is the loop:

1. Sweep (§6.1). Nothing else building.
2. Every `defect` and every `silent` is fixed in the agent, or argued into `Blocked` with a comment
   on the code and a line in [pokemon-agent](pokemon-agent.md). Every fix is behind the gates in §3.
3. For the maps the sweep did not enter, take the cheapest tool that reaches them, in this order:
   - **Another start.** A start is one square of a finished game and nothing else
     (`coverage::Start`); any postgame fixture that stands in an unreached cluster will do, and
     `every_coverage_start_stands_where_it_says_on_a_finished_game` pins it in the default tier.
   - **Fly.** The god party carries it, `use_field_move` takes a destination, and the brain already
     issues non-menu field moves for the PC. A walk that can fly to a town it has no ids for can
     cross Kanto on its own, which is what a sweep run by CI rather than by hand would want.
     Outdoors only (`Map::is_overworld`); it is a complement to the frontier, not a replacement.
   - **Connection ids that move with the player.** A `Connection` row's coordinate is the nearest
     crossing and re-picks as the player moves, so Viridian carries 6 ids for 3 neighbours. Unlike
     the sprite fix the coordinate is meaningful here (a model may ask for a specific landing), so
     the answer is to carry the `MetaTile` on the log entry rather than to drop the number.
   - **Mt Moon B1F's entry-dependent regions**, and whatever else the cross-check prints as *"on
     the grid, no sibling, never a row"*.
4. Re-sweep. Stop when two sweeps taken the same way agree: **zero defects, zero silent, and a
   union that has not grown.** That is §5.1's original termination condition — a full pass that
   discovers no id not already seen — measured as the only thing that can be measured.

**Done:** two consecutive clean sweeps with the same union, and that union and date written into
§2. Whatever is still unreached is listed there with one line each saying why it is gated.
**Cost:** each turn of the loop is a sweep plus whatever it found. Do not guess the number of
turns.

### Step 7 — The battle refusals

A walk scores overworld ids only; nothing above touches a battle decision. Six cells exist nowhere
in the suite and every one is a refusal, which is where the findings in a battle were always going
to be: a ball that fails, a run that fails, a run from something that cannot flee, a ball thrown at
a trainer, an item with none left, the Safari step counter expiring mid-battle, and the old man's
tutorial (`wBattleType == 1`, which `read_battle_state` reads as `Wild`).

**Done:** one test each through `LlmPolicy` and the worker, default tier where the state is cheap
and `slow-tests` where it needs a walk. **Cost:** medium. Self-contained; can be taken in parallel
with anything above.

### Step 8 — Branch-point snapshots

Content that is exclusive per save: the starter (3), the fossil (2), Hitmonlee or Hitmonchan (2),
the Bike Voucher against buying (2), and each in-game trade. One save state before the branch, N
tests after it, cut under `--features regen-fixtures`. Exploration is destructive and mostly
one-shot, so a branch costs a resume rather than a replay.

⚠️ This is the one place an event-flag write might be argued for, and none is admitted yet. Argue
the specific flag on the specific branch.

**Done:** each branch covered by snapshot × N. **Cost:** medium.

### Step 9 — Decide the soak tier on evidence

Halve `soak::SOAK_GAME_TIME` now (it is one constant, still 40 minutes) rather than cutting states.
After two clean sweeps from step 6, compare what the soak found in the same period that the walk
did not. Its only remaining value is state-space randomness — a jam that needs a particular action
in a particular state — so if that column is empty, drop it.

**Done:** the constant halved now; a row in this file with the comparison and the decision, later.

---

## 5. Parked, and why

- **The god run (C2).** A fresh save to the Hall of Fame through `LlmPolicy`, meant to replace
  `full_playthrough`. The walk no longer needs it: it starts from finished-game fixtures the
  scripted route already produces, and the god party's turn cost was measured at about a
  millisecond, so the only unknown was game time. What it would prove — chaining, resumes,
  compaction and a restart across a whole game — is worth having, but it is not on the path to the
  goal above, and `full_playthrough` stays the pre-push gate. The machinery (`Intent`,
  `ScriptedBrain`, `godmode_turn_cost`) stays default tier and is not to be removed. ⚠️ It is
  Alex's call to unpark it.
- **The sixteen battle cells proved under `DeterministicPolicy` only.** Promoting each to the LLM
  path is a long tail behind step 7.

---

## 6. Running it

### 6.1 The sweep

Test names are `coverage_walk_of_the_finished_game` and `godmode_turn_cost`. ⚠️ A filter that
matches nothing prints `0 passed` and exits 0; check the count.

```shell
# One region, a smoke budget (~10 s). The default start is phase0.
cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture

# One region at a coverage budget (~5 min).
GB_COVERAGE_START=celadon GB_COVERAGE_MINUTES=360 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture

# Every region in one process, serially, with the union printed at the end (8x the wall clock).
GB_COVERAGE_START=all GB_COVERAGE_MINUTES=360 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
```

**For a measurement, run the eight in parallel** — same 7 minutes as one, and they agree with each
other. Build once, then launch the test binary eight times, one directory each:

```shell
bin=$(cargo test --release --features coverage-tests --bin gb --no-run 2>&1 \
        | grep -o 'target/release/deps/gb-[0-9a-f]*')
S=/var/tmp/gb-sweep; rm -rf $S; mkdir -p $S
for r in phase0 cerulean vermilion lavender celadon saffron fuchsia cinnabar; do
  mkdir -p $S/$r $S/tmp/$r
  ( cd $S/$r && TMPDIR=$S/tmp/$r GB_COVERAGE_START=$r GB_COVERAGE_MINUTES=360 \
      GB_COVERAGE_PATIENCE=100000 GB_COVERAGE_WALL_SECS=2400 \
      $bin coverage_walk --nocapture > walk.log 2>&1 ) &
done
wait
grep -h '^frontier\|^settled\|^verdicts\|^silent\|^busiest' $S/*/walk.log
# the union: an id's prefix is the map it was minted on. ⚠️ drop the header row or it counts as a map
cat $S/*/target/test-artifacts/coverage/walk-*.tsv | grep -v '^id\t' \
  | cut -f1 | cut -d: -f1 | sort -u | wc -l
```

⚠️ **`TMPDIR` is not decoration, and leaving it off cost this measurement three attempts.** Each
walk gets a real run directory from `run::tests::Scratch`, which is `std::env::temp_dir()` — and on
a box where `/tmp` is a tmpfs that directory is **RAM**. A walk appends to `transcript.jsonl` and
`conversation.jsonl` on every one of its thousands of turns, so eight of them quietly write
hundreds of megabytes apiece into memory that no `ps` column attributes to them: the walks' own RSS
peaked near 1 GB for the set while the machine was being emptied underneath them, and all eight
were killed twice at 60 to 180 seconds with the kernel reporting no pressure at all
(`/proc/pressure/memory` flat, no OOM in the journal). Point `TMPDIR` at real disk. ⚠️ A killed
walk's `Scratch` never runs its `Drop`, so check for orphaned `/tmp/gb-coverage-walk-*` before
blaming the next run's numbers on the machine.

| Knob | |
|---|---|
| `GB_COVERAGE_START` | `phase0` (default) or one of `cerulean vermilion lavender celadon saffron fuchsia cinnabar`, or `all`. An unknown name panics rather than walking the default |
| `GB_COVERAGE_MINUTES` | game-minutes **per walk**; 90 is a smoke budget, 360 a coverage one |
| `GB_COVERAGE_PATIENCE` | barren turns before the frontier is called settled. Patience, not the budget, is what stopped every early sweep; set it high |
| `GB_COVERAGE_WALL_SECS` | a wall-clock stop, so a wedged sweep fails instead of running all night |

Artifacts go to `target/test-artifacts/coverage/`: `walk-<region>.tsv` with every id and its
verdict, and `defect-<id>_state.bin` plus a screenshot taken **at the moment the verdict turned**,
because the square cannot be stood on again by the end of the walk. ⚠️ `target/` is swept; cut
states fresh rather than looking for old ones.

### 6.2 How to read a number

- **The `phase0` walk has several modes**, not a spread: 38 maps stopped on the Hall of Fame, about
  61 having missed the Indigo Plateau and spent its budget in Victory Road, or — 2026-09-09 — **30**,
  having spent 64% of its turns on Victory Road's three floors and never left. A single run is a
  sample of which happened, and the list is open: `cinnabar` walked the same three floors to 33 maps
  the same day, from a start on the other side of Kanto.
- **Load is an input.** Three runs beside a `cargo build -j24` came out 30/30/38; eight on an idle
  box came out 38 seven times. Never build while measuring, and prefer eight in parallel to one.
- **Which id fails is not reproducible by re-running**; the dropped save state is. Argue from the
  state, as [deployed-run-defects](deployed-run-defects.md) does.
- **The union is the number for the goal**, and it is only meaningful across several starts: eight
  identical `phase0` walks union to 86, eight different starts to 159.

---

## 7. The record

### 7.1 Faults the walk has found and fixed

All in the agent or the oracle, none in the walk's own brain but the last two. Each has a test.

| What the sweep showed | Root cause | Test |
|---|---|---|
| `Grass` and `CutTree` rows scored `Silent` | The agent ended both in a text box it made up, with no terminal event; `resume_after_battle` had never applied to tall grass | `agent::tests::walking_in_grass_says_what_became_of_it`, `a_tree_that_was_cut_down_says_so_in_the_agents_own_voice` |
| Pewter's east exit taken 59 times | The brain scored exits by promise; a blocked door never stops being promising | the ordering comment in `ExploringBrain::respond` |
| A successful walk north reported as *"no route"* | `wYCoord` reads 255 for one tick on a north or west crossing, and the clamp put the player on the far edge | `mechanics::a_coordinate_that_underflows_a_map_edge_is_not_a_position` |
| The scripted heal detour routed for ever without arriving | No hop bound on a detour that always routed | `policy.rs` `MAX_HEAL_HOPS`; proved on the harness, no test of its own |
| One boulder puzzle at one action a minute for 2½ hours | A `BoulderGoal` id carried the boulder and the start square, both of which move per push | `tile::a_boulder_goal_is_the_target_and_not_the_boulder` |
| *"given up after 60 s"* one square from the push tile | `OverworldMovement` re-derived its row with `==` | `MetaTile::is_same_row_as` |
| A cast inside Viridian Gym | Shore tile ids accepted in every tileset with water | `map_metadata::a_shore_tile_id_is_only_a_shore_in_the_overworld` |
| A Strength floor abandoned three pushes from the end | A total push cap on a 27-push floor; it now bounds pushes without progress and has its own `PuzzleRanLong` | `endgame::victory_roads_hardest_switch_is_one_decision_however_many_shoves_it_takes` |
| A wedged floor blamed the pathfinder | `PuzzleUnsolvable` names the cure: leave and come back | `endgame::a_wedged_strength_floor_is_reported_as_a_reset_rather_than_a_missing_route` |
| Every solved puzzle scored `Silent` | Completion fired in `GameMode::Script` and an early `return` skipped the event drain | same |
| A quarter of the frontier was one object many times | A sprite id carried the player's approach square; it is `{map}:{name}` now | `actions::tests::a_sprite_name_is_unique_within_its_map` |
| 197 turns walking to exits after the credits | No agent state for the Hall of Fame's soft reset; `PokemonAgent::ending` latches on `wNumHoFTeams` | `phase0::the_agent_stops_playing_a_world_the_cartridge_has_reset` |
| The oracle called a gate a defect for one try per pass | `REPEAT_IS_A_DEFECT` was 3; it is 10 | `being_stopped_by_the_same_thing_over_and_over_is_the_defect` |
| The 402 death loop of 2026-09-05 | A failed turn's messages kept, the plan re-appended every turn, compaction with nothing to drop | `llm::an_undated_hard_failure_does_not_ratchet_the_history` |
| 35 `Fish` rows scored `Silent`, and a cast refused "because you are surfing" one tile from dry land | A fishing route's last button *faces* the water, and the Surf-mount arm read it as a step onto water | `tile_map::a_fishing_rows_last_button_faces_the_water_rather_than_entering_it` |
| The three ways a cast can fail said nothing the oracle could score | All three printed a `TextBox` the agent made up and closed no action | `agent::a_cast_that_could_not_be_made_says_why_rather_than_going_quiet` |
| `Route18:39,13:Grass` chosen and never reported | A trainer's walk-up commits as `Script` and drops the pace; the script and text-box doors knew only about `OverworldMovement` | `agent::a_pace_is_an_open_overworld_action_like_the_walk_that_started_it` |

⚰️ Two of these were diagnosed wrongly before they were diagnosed rightly, and the lesson is the
same both times: argue from the dropped save state, not from the sentence the agent printed.

### 7.2 Where the first draft was wrong

Kept because the reasoning is the useful part.

1. **The action universe cannot be enumerated from the ROM.** Whether a row exists is a function of
   live state, so the universe is discovered and the termination condition is a fixpoint. The ROM
   tables are a cross-check only.
2. **Gates are to be cheated past, not respected** — with the debug tier, never with event flags.
3. **The god party is three, not two.** Five HMs do not fit in one slave's four slots and Fly is
   the one that travels.
4. **The oracle needed the id on the event.** `OverworldAction::id` is minted once and carried on
   `StartedOverworldAction`.
5. **"38 maps" was a terminus, not a ceiling.** The walk wins the game. Two sections were spent
   reasoning about Mt Moon when the answer was the Elite Four being nearer than Cerulean.
6. **"A couple of per cent of noise" was measuring the machine.** §6.2.
7. **"Fishing is fixed" covered one of four exits.** Step 1. A fix to a driver's happy path is not
   a fix to the driver, and a silence the walk does not fail on is a silence nobody reads.
8. **Step 1 named the wrong cause of its own headline bug, and the step list is not evidence.** It
   said the walk picked a shore across the water; the shore was always right and it was the *turn*
   at the end of the route that mounted Surf. The fix written from the plan's account was a provable
   no-op. Argue from the log and the state, then from the step (§6.2's third rule, one layer up).

### 7.3 Where the old section numbers went

Code comments cite the 2026-09-06 draft (git `7343616`). What each pointed at, and where the
argument lives now:

| Old | Was | Now |
|---|---|---|
| §1.1, §1.2 | the play-path line; no event-flag writes | §3; `postgame/debug.rs`, `coverage::Start` |
| §2, §2.3 | the harness and `LlmRun`'s four seams | `llm_harness.rs` module comment |
| §2.2.1 | the 402 death loop, in full | `worker::TurnOpen`, `Worker::drop_unanswered`, [llm-turn-loop](llm-turn-loop.md) |
| §3, §3.1–§3.3 | the cheat tier and the god party | `cheats.rs`, `postgame/debug.rs` |
| §4, §4.2 | the god run and whether it replaces `full_playthrough` | §5 (parked); `godmode.rs` |
| §5, §5.1, §5.2 | the frontier and the oracle | `coverage.rs` module and `ExploringBrain` comments |
| §5.2.2–§5.2.6, §5.2.8 | the sweeps' findings | §7.1 |
| §5.2.7 W2 | one walk reaches 38 maps | §2, step 6; `coverage::Start` |
| §5.2.9 | the 42 defects and Route 16 | steps 2, 3, 4 |
| §5.3 | the ROM cross-check | `coverage::rom_cross_check` |
| §5.4 | branch points; the state must be dropped at the defect | step 8; `TestFixture::observe_coverage` |
| §5.5 | acceptance and the variance | §6.2 |
| §6, §6.0 | the battle matrix audit | step 7 and §5 |
| §7 | the soak decision | step 9 |
