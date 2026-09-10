# Full exploration through the LLM path

**The goal, in one sentence.** Every action the game offers, taken once, through the stack exactly
as it is deployed — `LlmPolicy`, the worker and the wire, against a mock endpoint in-process — with
a verdict on each, and every defect that turns up fixed, until a sweep of the whole of Kanto comes
back clean and finds nothing new.

**Status.** Rewritten 2026-09-09 as a step list; **steps 0 to 5 taken that day, and step 6 — the
loop — reached its fixpoint on 2026-09-10 after two turns.** The harness, the cheats, the oracle and
the frontier walk are all **built**. §2 is the 2026-09-09 baseline every closure is measured
against — 153 maps, 106 defects, 41 silences, 82% of every turn in Route 16's gate — and all of that
is closed. ⭐ **§2.1 is where the sweep stands: two consecutive sweeps of ten starts with zero
defects and zero silences in all twenty walks, a union of 215 maps of 248 and 2 114 ids that did not
grow between them, and five real maps left**, each with a line saying why.

The two turns cost nine defects and six silences, and the two biggest closures were neither of them
a change to the walk. **Gen 1's bag holds twenty kinds, every start arrived with all twenty used, and
the key items the walk needs were being refused in silence** (turn 1) — making room moved `phase0`
from 38 maps to 143 and took the Rocket Hideout, the Game Corner's prize room and every `Fish` row
off the unreached list. And **`actions()` emitted the nearest crossing per adjacent map of either
kind, so a footbridge always beat the water beside it** (turn 2) — one row per *kind* opened Cerulean
Cave, which no sweep in this plan's history had entered.

§4's steps 7 to 9 are what is left, and none of them is the walk; §7 keeps what the first draft got
wrong and what the sweeps have found.

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
| The walk | `coverage.rs` `ExploringBrain`, `coverage_walk_of_the_finished_game` | Takes every unvisited row on the map, then the least-taken exit. Starts from one of ten fixtures (`GB_COVERAGE_START`), eight of them **finished games** so every gate is open because the cartridge opened it; the other two are argued on `Start::before_the_credits`. Behind `--features coverage-tests` |
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
  step 3's fix is expected to take it, and if it does not it is a finding of its own — **which is
  what happened**; see step 3. ✅ **All 106 are closed.** Step 2 took `fuchsia`'s four (the two
  `SafariZoneCenter` rows and the two `SeafoamIslandsB*F` warps) and step 3 took Saffron Gym's 99 and
  Silph Co 1F's 3. What a sweep now finds instead is one intermittent *"no route to <person>"* on an
  NPC who walks about — the shape step 1 recorded twice — which is step 6's.
- **Silent: 41, and they were no longer only fishing.** `Fish` 35, `Grass` 3
  (`Route18:39,13`, `Route24:5,18`), `Warp` 3 (`SeafoamIslandsB3F:20,17`, `:21,17`, `:25,14`).
  ✅ **All of it closed by step 1 the same day, and the count is 0 in all eight regions**; a silence
  now fails the tier. Two of the three families were one bug each and the third was not a family at
  all: the fishing walk mounted Surf on the *turn* that faces the water, the `Grass` pair is a
  trainer noticing the player mid-pace through a door `PacingForEncounters` does not own, and the
  three Seafoam warps score `Defect` on every walk that reaches them (step 2's ⚠️), the `Silent` here
  being the variance in §6.2 rather than a third thing. ⚠️ **Step 1 was written believing fishing was
  the whole list**, which is why the assertion was its last item and not its first.
- **Where the turns went: Route 16's gate, and it is worse than the last table said.** ✅ **Closed by
  step 4**, which found the cause was not the one this bullet or the step assumed. The
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

### 2.1 Where the sweep stands — 2026-09-10, and step 6 is done

Ten walks in parallel on an idle machine, 6 game-hours each, about 7 minutes of wall clock for the
set (§6.1). ⭐ **Two consecutive sweeps taken the same way came back with zero defects and zero
silences in all ten regions, and the second added nothing the first had not already reached** — which
is step 6's termination condition, measured as §5.1 said it would have to be.

| | **sweep A** | | | **sweep B** | | |
|---|---|---|---|---|---|---|
| **start** | **maps** | **ids** | **only it** | **maps** | **ids** | **only it** |
| `phase0` | 137 | 1 304 | 0 | 143 | 1 341 | 1 |
| `cerulean` | 132 | 1 218 | 0 | 118 | 999 | 0 |
| `vermilion` | 135 | 1 255 | 0 | 138 | 1 295 | 0 |
| `lavender` | 135 | 1 259 | 0 | 135 | 1 265 | 0 |
| `celadon` | 128 | 1 154 | 0 | 134 | 1 293 | 0 |
| `saffron` | 134 | 1 251 | 0 | 134 | 1 258 | 0 |
| `fuchsia` | 152 | 1 346 | ⭐ **29** | 70 | 654 | ⭐ **29** |
| `cinnabar` | 41 ⭐ Hall of Fame | 408 | 12 | 41 ⭐ Hall of Fame | 405 | 0 |
| `ssanne` | 99 | 954 | **25** | 63 | 646 | 0 |
| `ssanneship` | 69 | 737 | 0 | 69 | 744 | 2 |
| **union** | ⭐ **215 of 248** | ⭐ **2 114** | | 189 | 1 895 | |
| **failures** | **0 defects, 0 silent** | | | **0 defects, 0 silent** | | |

⭐ **215 maps and 2 114 ids is the high-water mark of this plan by a wide margin** — the baseline of
2026-09-09 was 153 maps with 106 defects and 41 silences, and turn 1 finished on 197. **Sweep B is
a strict subset of sweep A**: every one of its 189 maps was already in A, so the union across the
pair is 215 and did not grow. That is what "a full pass that discovers no id not already seen" comes
to when the thing being measured is a random walk.

⚠️ **Read the pair, never the row** (§6.2), and this pair is a textbook of it. `fuchsia` came back
with 152 maps and then 70; `ssanne` with 99 and then 63; `cinnabar` is 41 both times because it wins
the game at 40% of its budget and stops. Not one of those is a regression — they are different walks.

**The five real maps neither sweep entered**, which is the whole of what is left:

- ⭐ **Cerulean Cave B1F, and the ROM cross-check names the exact ladder.** 1F and 2F are *in* now —
  six of the ten walks reached them, against never once in this plan's history — because a water
  crossing is a row of its own beside the bridge to the same map (§6 turn 2). What is still shut is
  the way down: `CeruleanCave1F (0, 6) → CeruleanCaveB1F`, `1F (3, 11) → 2F` and `2F (1, 3) → 1F`
  all print *"on the grid, no sibling, and never a row"*. `probe_route_to_cerulean_cave` already has
  the cause written down — the strip in front of the B1F ladder is raw tile 32 and the room below is
  tile 5, and `(32, 5)` is in the Cavern tileset's `TilePairCollisions`, so the floor is entered by
  going up at 1F (3, 11) and back down at 2F (1, 3). **All three rungs of that chain are unmintable,
  so the route exists and cannot be asked for.** It is the sharpest remaining item and it is a
  routing fault rather than a gate.
- ⭐ **Route 17 (Cycling Road) and `Route16Gate2F`. The stack can ride a bike now; the *walk* still
  cannot ask for one.** Turn 2 made `use_field_move`'s `target` optional, so `UseTarget::Nothing` is
  reachable from every LLM turn — which was the real hole, and it took the Potions, the Repels and
  the Itemfinder with it. But the coverage walk chooses `actions()` rows, and there is no row that
  means "ride the Bicycle": the gate refuses a walker, so the two maps stay out. ⚠️ **The fix was
  still the right one** — it is about the deployed tool surface, not about two maps — but the maps
  need something else, and it is a `MetaTile` rather than a tool.
- **`CeladonGym` and `SafariZoneWestRestHouse`, one door each.** `SafariZoneWestRestHouse` has its
  own cross-check line (`(11, 11)`, never a row); `CeladonGym` does not and has been in a union
  before, so it is the variance rather than a gate.

⚠️ **`ssanneship` did not do what it was added for, and that is worth writing down.** It stands on
`SSAnne1F` so that the ship is not a coin flip — and on both sweeps it walked straight off, taking
`SSAnne1F` and `VermilionDock` and never going back, because Kanto's frontier is enormous next to
eleven rooms. What actually covered the ship was `ssanne` on sweep A (all eleven, 25 maps nobody else
reached). So the tenth start buys **two** guaranteed maps rather than eleven, and the other nine are
still down to which way a walk turns. Keeping it is cheap; the real answer is something about the
frontier, and §3's "promise-first exit ordering loses" says what not to try.

⚠️ **And the old warning still holds: `cerulean`, `vermilion`, `lavender`, `celadon` and `saffron`
contribute nothing no other start reaches, on both sweeps.** What earns a place is a start that
stands somewhere the others cannot get to — `fuchsia` (the south-east, 29 maps both times),
`cinnabar` (the Indigo Plateau) and `ssanne` (the ship, and the south-west by not having won).

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

### Step 2 — The Safari Zone pond, and the two warps on the water ✅ done 2026-09-09

**What was wrong, and it was two unrelated things on one map cluster.**

**(a) Rows minted before the map is the map.** From the `fuchsia` start, the gate script takes the
fee and auto-walks the player into `SafariZoneCenter`; on that tick `actions()` minted
`SafariZoneCenter:Nugget` and the two warps on the far side of a pond Surf is refused on, and the
walk aborted with *"there is no route"* on the next tick. The screenshot beside the dropped state is
black.

✅ **Taken, and the diagnosis in the paragraph above was half right.** It is the `position_settled`
family, but the flag itself was *true*: `(4, 0)` — the gate's square — is in bounds for the Centre,
so the clamp told no lie the bounds check could catch. What is stale is everything else.
`WarpFound2` writes the destination into `wCurMap` and only then falls into `EnterMap` →
`LoadMapData` → `LoadMapHeader`, so `read_current_map` spends that window taking its *metadata* from
the ROM under the new `wCurMap` and its coordinates, sprite slots and dimensions from the WRAM of the
old one. Measured: **26 agent ticks** on an ordinary warp and **94** at the Safari gate, against the
1 000 ms a settled agent waits before it asks anything, which is why only a walk ever found it.

1. `map_metadata::map_header_is_loaded` compares the ten bytes `LoadMapHeader` copies out of
   `MapHeaderPointers[wCurMap]` against `wCurMapHeader`. ⚠️ **Minus `wCurMapTextPtr`**, which
   `SetMapTextPointer` swaps and Viridian Mart's and Oak's Lab's scripts repoint and keep — comparing
   all ten made every mart visit look like a transition, withheld the clerk, and failed three
   default-tier tests. `CurrentMap::header_loaded` carries it into `MetaTileMap::position_settled`.
2. **`actions()` answers with no rows at all while the position is a fiction.** Every row is a route
   from where the player is standing. On the 255-underflow tick that was already the outcome (the BFS
   from the far edge reaches nothing); on this one it was not.
3. ⚰️ **Holding the agent's turn as well was tried and reverted.** Not polling the policy while the
   header is stale is tidier — `blackout_in_flight` does exactly that one state over — but a
   black-out is rare and this is every door in the game. Deferring the poll by the window re-rolled
   `postgame::items`' wild encounter from a Pidgey to a Rattata, whose Tail Whip held the Defense
   stage that leg asserts on at neutral. An empty menu is a case `llm::prompt` already answers
   ("nothing — the agent can reach no action from here. `wait` and look again").

**(b) The two warps the step said it was *not*.** `SeafoamIslandsB3F:21,17:Warp` and
`SeafoamIslandsB4F:21,17:Warp` are water at the bottom edge of a current channel, and they gave up
after 60 s standing on the square they were walking to.

✅ **Taken.** `home/overworld.asm`'s `.noDirectionChange` tests `wWalkBikeSurfState` for `$02` before
it looks at anything else. On foot, a collision while standing on a warp entry runs `ExtraWarpCheck`
and then `CheckWarpsCollision` and the warp fires — that is how every map-edge ladder in the game is
taken. Surfing, the branch goes to `CollisionCheckOnWater` and the next instruction is
`jp c, OverworldLoop`; `CheckWarpsCollision` is not on that path. So a water entry only fires from
`CheckWarpsNoCollision`, on a completed **step**, in the direction `IsPlayerFacingEdgeOfMap` accepts.
Measured on the dropped state: 120 ticks of Down move nothing, Up-then-Down warps.

⚠️ **The condition lives in two places and fixing one changed nothing.** `MetaTileMap::actions`
builds `[opposite(dir), dir]` for a surfing player standing on the entry, and
`AgentState::OverworldMovement` tests for a border warp **before** it consults the route and pressed
the outward direction itself. Both carry it now.
⚠️ The sibling `20,17` entry passed every sweep because the walk happened to arrive from above and
the arrival fired it, which is the same fact from the other side.

**Done, measured 2026-09-09:** the `fuchsia` walk at the full budget, **0 defects and 0 silences**
across 558 ids on 63 maps. `cargo test --release` green (1 583), the leg chain green (219),
`full_playthrough` green, `hall_of_fame` green. Tests:
`mechanics::a_map_the_cartridge_has_not_finished_loading_offers_no_rows`,
`mechanics::the_header_in_wram_says_which_map_has_actually_been_loaded`,
`mechanics::a_warp_reached_by_surfing_is_entered_rather_than_leant_on`,
`cinnabar::a_seafoam_warp_on_the_water_is_stepped_onto_rather_than_leant_on` — the last off the
walk's own dropped state, committed as `data/seafoam-b3f-on-the-water-warp.bin`, and it fails if
either half of (b) is removed.

### Step 3 — Saffron Gym's teleport pads ✅ done 2026-09-09

**What was wrong.** 30 of the gym's 32 warps lead back into the same map. The route to one pad
crosses others, stepping on one relocates the player, and the row is gone by the time the walk looks
for it: 37 defects on `celadon` at the 60-minute budget, every one of them *"there is no route to the
warp to SaffronGym"*, and **239 of the walk's 454 turns** in that one room.

✅ **Taken, and the step's own account of it was wrong about where the fix goes.** The agent already
treats a pad the way it treats a spinner — `bfs_from_player` records the edge from the square beside
the pad to the pad's **landing**, so routes cross the maze for free. Three things were missing.

1. ⭐ **The `settled` guard skipped the pad the player was standing on.** The neighbour loop dropped
   a settled neighbour before it looked at what the neighbour *was*, and the search's own root is
   settled at price 0 — so standing on a pad threw away the one edge out of the room. Every room in
   that gym is entered by exactly one pad, so that is a ninth of the map: standing on `(1, 5)`, whose
   landing is the centre room, the walk read back that there was no route to **Sabrina**, the Gym
   Guide, or the door out. The intra-map arm is tested before the guard now.
2. ⭐ **No intra-map warp row in the game had ever completed.** Every completion the agent had for a
   `Warp` was the map changing, and a teleport pad does not change the map — so all thirty scored
   `Defect` after sixty seconds of silence. `player_position == to_position` is exact rather than
   approximate: a pad's landing is reached by that pad and by nothing else. The one hole, being
   offered a pad whose landing is already underfoot, is closed where the row is minted.
3. **Silph Co 1F was not the same family and the step's ⭐ was right to hedge.** `SilphCo1F:16,10` is
   `warp_event 16, 10, SILPH_CO_3F, 7 ; inaccessible` — pokered's own comment. Plain floor, no warp
   tile, nothing to press, and `warp_trigger` says `Impossible` correctly. The row survived only
   because `actions()` kept a dud whenever no sibling on the map opened onto the same place, and it
   cost sixty seconds every time any run reached that floor.

   ⭐ **That guard was quietly load-bearing, and finding out what it was protecting is the useful
   part.** A scan of all 248 maps found **7** `Impossible` warps, 4 of them the only way to their
   target — and three of those four are **Pokémon Mansion 3F's floor holes**, the only way onto 1F's
   right side and so to the Secret Key. They are `FACILITY $11`, which lives in
   `data/tilesets/warp_pad_hole_tile_ids.asm`: the cartridge's *other* step-on table, read by
   `IsPlayerStandingOnWarpPadOrHole` rather than by `IsPlayerStandingOnDoorTileOrWarpTile`. Naming
   that table moved the holes to `StepOn`, left the guard with nothing to guard, and let the dud
   rows go. ⚠️ `WarpTrigger::Unknown` is still never dropped — unsure is not the same as no.

⚰️ **Pricing a pad by the square you step onto it from was written and taken out.** It reads better
than what the BFS does, but no map needs it: all three that carry intra-map warps (`SaffronGym` 30,
`SilphCo3F` 2, `SilphCo8F` 2) pair their pads one-to-one, so every pad is already in `dist` as some
pad's landing. A second pricing path nothing can reach is a second pricing path nothing can test.

**Done, measured 2026-09-09** on the three regions at the full budget, in parallel on an idle
machine: **0 defects in the gym and 0 on Silph Co 1F in all three**, and Saffron Gym is **38 turns**
in each — one per row, for a room with 30 pads, 9 people and a door — against 239 of 454 before.
`celadon` 86 maps / 723 ids / 0 defects, `saffron` 49 / 373 / 0, `cerulean` 86 / 685 / 1 (below).
The busiest map everywhere is now Route 16's gate, which is step 4. `cargo test --release` green
(1 587), the leg chain green (223), `full_playthrough` green, `hall_of_fame` green. Tests:
`saffron::every_teleport_pad_in_the_gym_is_a_row_including_the_one_underfoot`,
`saffron::a_teleport_pad_reports_arriving_even_though_the_map_never_changed`,
`mechanics::an_impossible_warp_is_one_the_cartridge_really_will_not_open` (the whole-ROM list that
replaced the guard), and `mechanics::every_committed_fixture_has_a_complete_sprite_table`.

⚠️ **`cerulean`'s one remaining defect is not this step's and is on the record already.**
`CeruleanMart:CooltrainerFemale: there is no route to Cooltrainer Female` — a shopper who wanders,
so the row is minted with a route and the route is gone a tick later. Step 1 saw the same shape twice
("two one-off *no route to <person>* on a moving NPC in a mart and a Pokémon Centre"); it appeared in
one of the three regions this sweep and none of the last. It belongs to step 6's loop: a `NoRoute` to
a **sprite that is still on the map** is a sprite that moved, not a row the agent could not carry out.

### Step 4 — Route 16's gate, in the brain ✅ done 2026-09-09

**What was wrong.** Route 16 has eight squares facing its gate's doors, so the pair of maps carries
eight warp ids and every one works. Four walks spent 60% to 97.6% of their turns there, and the two
worst were two of the three lowest map counts. The step's diagnosis was that `times` is per **id**
while the thing being oscillated over is a **pair of maps**.

⚠️ **That diagnosis was wrong, and counting by map alone proves it.** Counting exits by destination
was written and measured: the eight doors did collapse into two options, the counters alternated
exactly as intended — and the walk ping-ponged just as hard, because it has *nowhere else to go*.
`lavender` finished with `Route16:7,5:Warp` and `Route16FlyHouse:2,7:Warp` chosen **1 593 times
each**, on a menu of three rows.

⭐ **What actually sealed it was a cut tree that regrows.** Route 16's east half is split in two by
a tree, and the way out to Celadon City is on the far side of it. The walk cut it, crossed, and
marked `Route16:34,10:CutTree` done — and cut trees grow back when a map reloads, which
`PokemonAgent`'s `cut_tiles.clear()` says in as many words. Every return found the tree standing and
the row "visited", so the only rows left were the gate and the Fly House.
`Route16:40,10:Connection` — the way out, one tree away — ended the run **offered and never once
chosen**. It is the Pokémon Mansion statue again, and the fallback written for *that* only fires when
the menu has no exit at all; Route 16 has three, so it never fired.

**Two changes, and both are in the brain's ordering** (`ExploringBrain::respond`):

1. **Once nothing on the menu is unvisited, the least-taken row wins whatever kind it is** — not the
   least-taken *exit*. ⚠️ A row that is not a way out is ranked below every exit that ties with it
   (`promise` 3 against 0–2), which is what keeps this from being the promise-first ordering that
   lost 20 maps twice: a re-takeable row is chosen only when it has been taken **strictly fewer**
   times than every way out, which on a map passed through once is never. ⚠️ `Grass` and `Empty` are
   excluded — those are a request for an *encounter*, and a second pace discovers nothing while
   costing a 60 s budget; it is the same line `resume_after_battle` draws two arms down.
2. **Exits are still counted per crossing** — `"{here}->{there}"` rather than per id. It is not what
   fixed the gate, but it is right on its own terms (eight doors into one building are one decision)
   and it costs nothing. ⚰️ Keyed by the destination **alone** it lost 22 maps: a global tally makes
   a *hub* repellent, so a walk inside Celadon Mart found every door back out to the city already
   "taken" a dozen times and cycled the building instead — 4 400 to 5 000 turns in five regions. An
   option is a decision available *here*, so the key has to say where here is.

**Done, measured 2026-09-09** — eight regional walks a side, in parallel on an idle machine:

| | union maps | union ids | Route 16's gate |
|---|---|---|---|
| before | 159 | 1 410 | **23 099 of 26 870 turns (86%)** |
| after | ⭐ **186** | ⭐ **1 752** | **not in any region's `busiest` line** |

Per region the map counts went 30→38, 86→100, 58→**133**, 34→**133**, 86→120, 49→119, 71→71,
41→**132**, and the turn counts collapsed with them (`lavender` 6 427→1 522, `saffron`
6 219→1 020). Every `busiest` line is now an ordinary city or route at 30 to 100 turns. `cargo test
--release` green (1 587); nothing that ships is behind this — the whole change is in a test file.

⭐ **And the reach it bought found the next thing, which is a cheat rather than a walk.** Hunting the
Bicycle down as a suspect turned up `Cheats::bag_was_full`, a counter whose own comment calls it "a
coverage gap worth printing" and which **nothing had ever printed**. The walk's summary now carries a
`cheats` line, and it says that **every one of the eight starts arrives with a full bag** — twenty
kinds is Gen 1's limit — and refuses between one and nine key items:

    ⚠️ the bag was full and refused 8: OldRod, GoodRod, SuperRod, SilphScope, LiftKey, SSTicket,
    CoinCase, GoldTeeth

That is most of the list of maps no walk had ever entered (now §2.1's): no S.S. Ticket is the whole of the S.S.
Anne's nine rooms, no Lift Key is Rocket Hideout's four floors, no Coin Case is the prize room, no
rods are every `Fish` row on the map. The Bicycle fits in all eight, so Cycling Road was never the
blocker. **Making room in the bag is step 6's cheapest tool by a distance** and is now the first
thing to try there.

### Step 5 — Print the maps never entered ✅ done 2026-09-09

The sweep printed a union and nothing about its complement, and the complement is the only thing
left to act on: from a count, a gate doing its job and a walk that never arrived look identical.
`Map::iter()` minus the union was being assembled by hand with a shell diff, which is exactly the
sort of arithmetic that gets done once and then quoted for a week after it stopped being true —
the list this file carried was.

✅ **Taken.** `unreached_report` prints it from the multi-region summary, sorted and wrapped six to a
line so a *cluster* is visible — the S.S. Anne's nine rooms, Rocket Hideout's four floors — which a
column of sixty names hides. ⚠️ It counts the ROM's `UnusedMap*` padding and the two link-cable
rooms and then sets them aside: 24 of the 62 that look missing are neither.

**Done:** `GB_COVERAGE_START=all` prints `unreached` under `union`, and §2.1 carries the list from the
latest sweep. **Cost:** small, as billed.

### Step 6 — The loop, until the fixpoint ✅ done 2026-09-10

Everything above is one pass. The goal is a fixpoint, and this is the loop. ⭐ **It reached one on
turn 2**: two consecutive sweeps of ten regions, zero defects and zero silences in all twenty walks,
and a union that did not grow — 215 maps of 248 and 2 114 ids, with five real maps left and a line
each in §2.1 saying why. Turns 1 and 2 are below, and the loop stays written down because the
condition is a measurement rather than a promise: a change to routing or to the agent puts the tier
back where turn 1 started, and this is how it is checked.

#### Turn 1 — 2026-09-10 ✅ taken

⭐ **The bag, and it was worth more than steps 1 to 5 put together.** `Cheats::apply` now calls
`debug_keep_only_items(&COVERAGE_KEY_ITEMS)` before it hands anything over: the five HMs teach
nothing when the party is written straight into the struct, and a walk never uses a TM, a Revive, a
fossil or a stat item — so the junk goes, all fourteen key items fit in every start, and six slots
are left free, which matters because **a walk with no free slot cannot pick an item up off the
floor**. `cheats::every_coverage_start_can_be_handed_all_of_the_key_items` pins it in the default
tier and the walk's `cheats` line prints what went.

What one line bought, per start, 2026-09-09 against 2026-09-10 on the same recipe:

| | phase0 | cerulean | vermilion | lavender | celadon | saffron | fuchsia | cinnabar |
|---|---|---|---|---|---|---|---|---|
| before | 38 | 119 | 133 | 133 | 132 | 129 | **163** | 41 |
| after | **143** | 124 | 132 | 130 | 120 | 122 | 147 | 41 |

⚠️ **Read the union rather than the row.** Per-start counts move by ±15 between sweeps of the same
tree (§6.2), and `fuchsia`'s 163 was one long walk that happened not to win the game. What the turn
actually did is in the **union: 185 maps → 201, and 38 real maps unreached → 23**. ⚠️ **The bag and
the ninth start take different halves of that credit**: the bag is the Rocket Hideout's four floors,
the Game Corner's prize room and every `Fish` row in the game; `ssanne` is Pokémon Mansion's four,
Cinnabar's buildings, Pallet Town's interiors and Viridian Mart (see §2.1 — it earned those by *not*
having won the game, which is not what it was added for).

**Six defects and five silences closed with it**, each in the agent and each with a test — see §7.1
for the row and the cause. In order of what they cost: a boulder goal that reissued one refused
shove **230 times** and ate two thirds of `cinnabar`'s budget; the Silph Co elevator's door, which
cannot be leaned on because the player *warped* onto it; Seafoam B4F's two staircases, which the
cartridge's own script cancels and which are now withheld rather than tried; Vermilion Gym's
double doors, which the static ROM blocks say are open and the cartridge draws shut; a `NoRoute`
said about a corridor three wandering pets were standing in; a walk eaten by a wild Pokémon while
the party menu was open to mount Surf; and the four the same mount ate by *finishing* the walk and
not saying so.

⚠️ **Three of the six are the same shape, and it is worth naming**: a rule the cartridge enforces
live — a flag on `wMovementFlags`, a script that undoes a warp, a block it redraws — that **no tile
in the static map can express**. `MetaTileMap` is a transcription of the ROM, so every one of them
looked like a pathfinder fault and none was, and each cost 60 s of game time per attempt. When a
sweep reports `DidNotArrive` while standing exactly where it wanted to be, that family is the first
thing to check.

**A ninth start**, `ssanne`, added for a cluster no finished game can reach and earning its keep for
a different reason — §2.1 has both halves.

**Where it left the tier:** `cargo test --release` green (1 591), the leg chain green (229),
`full_playthrough` green, `hall_of_fame` green. ⚠️ **`hall_of_fame` is not optional for this turn**,
whatever §3's "where the scripted route walks there" might suggest at a glance: the elevator rule
lands on `PolicyStep::UseElevator`, the silent-shove bound lands on both Victory Road boulder
puzzles, and the mount-arrival report lands on the Seafoam legs — three of the four fixes are on the
scripted route's own path, and the Vermilion Gym block map is a fourth. **Two sweeps were taken**:
the first came back with one defect and four silences, the second — after the last two fixes — with
**zero silences in all nine regions and one defect, which was then closed**. §2.1 is the second one,
and turn 2 is the sweep that has to agree with it.

#### Turn 2 — 2026-09-10 ✅ taken

⭐ **The sweep came back with zero silences again and three defects, and all three were the same
sentence the cartridge had been saying all along: somebody is standing there.** Turn 1 named that
family — "a rule the cartridge enforces live that no tile in the static map can express" — and
guessed it would show up as flags and scripts. It showed up as *people*. `MetaTileMap` is a
transcription of the ROM with the sprite table painted on top, and every one of the three was a place
where the paint had been applied wrongly, thrown away, or read once and cached:

- **`CeruleanMart:3,7:Warp`, a defect in three regions at once.** A warp beat a sprite in
  `meta_tiles`, so a shopper standing on a doormat left the door showing underneath her; the walk
  routed through a person and held Down for 60 s. It then left through `4,7`, the other half of the
  same two-tile mat, on its very next turn.
- **`CeladonChiefHouse`, twice.** Two corridors one tile wide with a Rocket in one and the Chief in
  the other. `MAX_ROUTE_LOST_TICKS` waits 5 s for a lost row, which outlasts one wanderer and not
  two; the walk said there was no route out of a room it walked out of one turn later.
- **`Route11:13,6:Grass`**, the `PacingForEncounters` stall §2.1 predicted would close on the next
  sweep that dropped a state for it. A Youngster stepped onto the far half of a pacing pair that had
  been chosen while it was empty, and bumping is not a step.

§7.1 has the row and the test for each. ⭐ **The fix for the second one is a question rather than a
bigger number**, and that is the part worth keeping: past 5 s the agent asks
`row_blocked_by_people` — put everybody but the row's own subject back on the floor `underfoot` says
is beneath them, and does the row come back? — and only a `yes` buys 30 s. A warp pokered labels
`; inaccessible` still says so at five seconds, because it is on the other branch. ⚠️ **Boulders are
sprites and are excluded**: a rock will still be there in thirty seconds, and waiting for one would
spend half a minute of a walk's budget.

⚠️ **And one of the three could not be reproduced from its own dropped state**, which is §6.2's
"which id fails is not reproducible by re-running" reaching a layer deeper than it ever had to
before. Restoring a save re-rolls how the NPCs wander, so `CeladonChiefHouse`'s five-second jam
clears in well under one on a replay; what the state *does* carry, exactly as the walk met it, is a
room where the row is missing and the reason is two people. So the test pins the predicate in both
directions rather than the bound, and says so.

**Then the three unreached clusters step 6's loop had queued, all closed:**

- ⭐ **Cerulean Cave, and it was one line in `actions()`.** The menu emitted the nearest crossing per
  adjacent map of *either* kind, so wherever a land bridge and a surfable edge lead to the same
  neighbour the bridge always won — and Route 24's footbridge is two steps from the river seam that
  is the **only** way into the half of Cerulean the cave door is on. One row per kind now.
  `MetaTileMap::water_connection_action` had said this in its own doc comment since the day it was
  written; what it lacked was a caller in the menu. Three floors, and the ROM cross-check had printed
  the cause on every sweep in this plan's history.
- ⭐ **The Bicycle, which was never really about two maps.** `use_field_move`'s `use_item` required a
  `target` tile and a bike has none, so `FieldMove::UseBagItem` and the whole of `UseTarget::Nothing`
  had a driver, a refusal table, an `IsBikeRidingAllowed` decode and a test that rides one — and no
  way in from any LLM turn at all. Route 17 and `Route16Gate2F` are what the sweep could see; what
  went with them was every out-of-battle use of a Potion, a vitamin, a Repel and the Itemfinder.
  `target` is optional now and a `slot` rides along for the party items, at +189 bytes of catalogue.
  ⚠️ `UseTarget::Move` is still unreachable on purpose — it would want a `move_index` on the schema
  and nothing in the game is gated behind an Ether.
- **A tenth start, `ssanneship`, standing on `SSAnne1F`**, cut by `vermilion::regen_on_the_ss_anne_fixture`
  from the same `at-vermilion.bin` its sibling uses. ⚠️ **Both are kept and the pair is not a
  duplicate**: `ssanne` reaches the ship only if a frontier walk out of Vermilion happens to turn
  south, which it did not on its first attempt and did on its second — and it separately earns its
  place as the only start that has *not* beaten the Elite Four, which is what puts Pokémon Mansion,
  Cinnabar's buildings, Pallet Town's interiors and Viridian Mart in the union. One start removes the
  coin flip; the other is about the south-west and has nothing to do with the ship.

**And then a fourth sweep found one silence, which is what a re-sweep is for.**
`Route12:0,63:Connection` — a walk that surfed south out of Route 12, crossed into Route 11, and had
the mount its follower tried on the far side refused: `Surfing` **with a `resume`** is one of the
three states that carry an open overworld action, and the refusal arm dropped straight to `Idle`
with the row still open. The arrival rule is a shared helper now and a refusal on the same map is
`Textbox`, which is exactly what happened. ⚠️ **It is the one fix this turn with no test of its
own** — the refusal needs the cartridge's own terrain check reached from inside the party menu, and
the tile-reader disagreement that produces one is by definition a tile nothing can look up. §7.1
says so in the row.

**Where it left the tier:** `cargo test --release` green (1 596), the leg chain green (233),
`full_playthrough` green, `hall_of_fame` green — all four re-run after the last fix. ⚠️
**`hall_of_fame` is not optional here either**: the sprite-over-warp flip is a routing change on
every map in the game, and the pacing re-pick lands on the ~840 wild battles the scripted grind is
made of.

**Where it left the sweep:** the two clean sweeps in §2.1, which are the ones the loop's stopping
rule is about. **Four sweeps were taken in all** — one to find the three defects, one to confirm them
closed (which found the silence), and then the pair.

#### The loop

1. Sweep (§6.1). Nothing else building.
2. Every `defect` and every `silent` is fixed in the agent, or argued into `Blocked` with a comment
   on the code and a line in [pokemon-agent](pokemon-agent.md). Every fix is behind the gates in §3.
   ⭐ **Argue it from the dropped state and from `probe_button_at_state`, never from the sentence the
   agent printed** — every warp finding so far was misdiagnosed the other way round (§7.2's 8 and 13).
   A silence drops no state, so its evidence is the walk's own log: `grep` the id and read the events
   either side of it, which is how all four of §2.1's were traced to one arm in half an hour.
3. For the maps the sweep did not enter, take the cheapest tool that reaches them, in this order:
   - **Another start.** ⚠️ **And "finished game" is the *usual* rule rather than the whole one now.**
     A finished save has every gate open, which is why eight of the nine are one — but it has also
     sailed the S.S. Anne, and it wins the game rather than exploring. `Start::before_the_credits`
     is the admitted exception and carries the argument; `ssanne` is worth 17 maps for the second
     reason and nothing yet for the first, because a frontier walk out of Vermilion goes north. **A
     start inside `SSAnne1F` is the next thing to cut** (`vermilion::can_clear_ss_anne` walks the
     ship, so the route exists and `--features regen-fixtures` is all it needs).
   - **The Bicycle** — ✅ **the tool half was done on turn 2**: `use_item`'s `target` is optional,
     `UseTarget::Nothing` is reachable, and a `slot` carries the party items. ⚠️ **It did not move
     the two maps and was never going to**: the walk chooses `actions()` rows, and there is no row
     that means "ride the Bicycle". Route 17 needs a `MetaTile`, not a tool.
   - **Fly.** The god party carries it, `use_field_move` takes a destination, and the brain already
     issues non-menu field moves for the PC. Outdoors only (`Map::is_overworld`); a complement to
     the frontier, not a replacement.
   - **Connection ids that move with the player.** A `Connection` row's coordinate is the nearest
     crossing and re-picks as the player moves, so Viridian carries 6 ids for 3 neighbours. The
     answer is to carry the `MetaTile` on the log entry rather than to drop the number.
   - **Whatever the ROM cross-check prints as *"on the grid, no sibling, and never a row"***, which
     is the one part of a sweep's output that names a missing row rather than a missing map. It is
     printed and never asserted on purpose (§5.3), so it has to be read. ✅ Turn 2 closed the entry
     it named all through turn 1 — `CeruleanCity (5, 12) → CeruleanCave1F`, which was Cerulean Cave
     1F and 2F, both reached now. ⭐ **What it names today is the floor below**: `CeruleanCave1F
     (0, 6) → B1F`, `1F (3, 11) → 2F` and `2F (1, 3) → 1F` are all "never a row", so the whole
     ladder chain is unmintable — see §2.1. Mt Moon B1F's entry-dependent regions are the other
     standing entry.
4. Re-sweep. Stop when two sweeps taken the same way agree: **zero defects, zero silent, and a
   union that has not grown.** That is §5.1's original termination condition — a full pass that
   discovers no id not already seen — measured as the only thing that can be measured.

**Done:** ✅ **met on 2026-09-10, on turn 2.** Two consecutive clean sweeps whose union did not
grow — 215 maps of 248 and 2 114 ids — written into §2.1 with a line each for the five real maps
still out. **Cost, for the record rather than as a forecast:** two turns, four sweeps, nine defects
and six silences.

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

⭐ **And it has a first customer that is not a branch at all.** The S.S. Anne is eleven maps behind a
one-way door — `EVENT_SS_ANNE_LEFT`, set before the third badge — so it needs a save cut *before* the
ship sails, which is the same machinery this step is about and none of the flag-writing it rules out.
`Start::before_the_credits` admits such a save into the walk; what is still missing is a fixture that
stands on the dock or on the ship itself. See step 6's loop.

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
# ⭐ This is the only form that prints `unreached` — the maps no walk entered, which is step 6's
# input. The parallel recipe below is faster and cannot: the diff needs `Map::iter()`.
GB_COVERAGE_START=all GB_COVERAGE_MINUTES=360 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
```

**For a measurement, run the ten in parallel** — same 7 minutes as one, and they agree with each
other. Build once, then launch the test binary ten times, one directory each:

```shell
bin=$(cargo test --release --features coverage-tests --bin gb --no-run 2>&1 \
        | grep -o 'target/release/deps/gb-[0-9a-f]*')
S=/var/tmp/gb-sweep; rm -rf $S; mkdir -p $S
for r in phase0 cerulean vermilion lavender celadon saffron fuchsia cinnabar ssanne ssanneship; do
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
peaked near 1 GB for the set while the machine was being emptied underneath them, and every one of
them was killed twice at 60 to 180 seconds with the kernel reporting no pressure at all
(`/proc/pressure/memory` flat, no OOM in the journal). Point `TMPDIR` at real disk. ⚠️ A killed
walk's `Scratch` never runs its `Drop`, so check for orphaned `/tmp/gb-coverage-walk-*` before
blaming the next run's numbers on the machine.

| Knob | |
|---|---|
| `GB_COVERAGE_START` | `phase0` (default) or one of `cerulean vermilion lavender celadon saffron fuchsia cinnabar ssanne ssanneship`, or `all`. An unknown name panics rather than walking the default |
| `GB_COVERAGE_MINUTES` | game-minutes **per walk**; 90 is a smoke budget, 360 a coverage one |
| `GB_COVERAGE_PATIENCE` | barren turns before the frontier is called settled. Patience, not the budget, is what stopped every early sweep; set it high |
| `GB_COVERAGE_WALL_SECS` | a wall-clock stop, so a wedged sweep fails instead of running all night |

⚠️ **Two more traps in the shell around it, both of which cost twenty minutes on 2026-09-10.**
`until ! pgrep -f "full_playthrough"; do sleep 20; done` **never exits**, because the waiting shell's
own command line contains the string it is grepping for — wait on `[ ! -d /proc/<pid> ]` or on a
sentinel written to the output file instead. And two launchers that both `rm -rf $S` will happily
give you eighteen walks in nine directories, which is a measurement of nothing: count
`ps -eo args | grep "deps/gb-<hash> coverage_walk"` and see exactly ten before believing a number.

Every walk's own summary carries a **`cheats`** line saying what was shed from the bag to make room
and which key items still would not fit. ⚠️ **Read it before believing a coverage gap**: Gen 1 holds
twenty *kinds* and every start arrives with fourteen to twenty used, and until 2026-09-10 a sweep
could look healthy while Rocket Hideout, the prize room and every `Fish` row were unreachable for
want of a lift key, a coin case and three rods. Room is made first now
(`debug_keep_only_items`), so the line should read `every key item fit the bag` — and if it does not,
whatever they gate is unreachable and nothing else will say so.

Artifacts go to `target/test-artifacts/coverage/`: `walk-<region>.tsv` with every id and its
verdict, and `defect-<id>_state.bin` plus a screenshot taken **at the moment the verdict turned**,
because the square cannot be stood on again by the end of the walk. ⚠️ `target/` is swept; cut
states fresh rather than looking for old ones.

### 6.2 How to read a number

- **A walk has several modes rather than a spread, and it is the *terminus* that varies.** `phase0`
  has stopped on the Hall of Fame at 38 maps, spent its budget in Victory Road at 30 or 61, and —
  once the bag was fixed — walked 123 and 143. `fuchsia` came back with 163, then 71, then 147; the
  71 is the run that happened to win the game at 55% of its budget. A single row is a sample of
  which happened. ⚠️ **A start that reaches the Indigo Plateau stops exploring**, and that cuts both
  ways: it is why `cinnabar` is 41 maps and why `ssanne`, which cannot win, is worth 17 nobody else
  reaches (§2.1).
- **Load is an input.** Three runs beside a `cargo build -j24` came out 30/30/38; nine on an idle
  box agree with each other. Never build while measuring, and prefer the nine in parallel to one.
- **Which id fails is not reproducible by re-running**; the dropped save state is. Argue from the
  state and from `probe_button_at_state` (§6.1), as
  [deployed-run-defects](deployed-run-defects.md) does. ⚠️ **A `silent` drops no state**, so its
  evidence is the walk's own log: `grep -n` the id and read the agent-state lines after it. Every
  driver prints its state every tick, so the trail names which one was holding the walk when it went
  quiet — `move→X`, then `surf`, then `BattleStarted` with no abort between is the whole diagnosis.
- **The union is the number for the goal**, and it is only meaningful across several starts: eight
  identical `phase0` walks union to 86, nine different starts to 201. ⚠️ **And the complement is the
  number to act on** — a count says nothing about whether a gate is doing its job or a walk never
  arrived, which is what step 5's `unreached` line and the ROM cross-check are for.

---

## 7. The record

### 7.1 Faults the walk has found and fixed

All in the agent, the oracle or the cheats — only two are in the walk's own brain, and the newest is
in a driver rather than in routing. Each has a test.

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
| 86% of a whole sweep's turns in one three-map pocket | Route 16's way east is behind a cut tree, cut trees regrow on a map reload, and a once-only frontier will not cut it twice — the exits it *could* take were a ping-pong | `ExploringBrain`'s `re_takeable`, and the union going 159 maps to 186 |
| A quarter of the frontier was one object many times | A sprite id carried the player's approach square; it is `{map}:{name}` now | `actions::tests::a_sprite_name_is_unique_within_its_map` |
| 197 turns walking to exits after the credits | No agent state for the Hall of Fame's soft reset; `PokemonAgent::ending` latches on `wNumHoFTeams` | `phase0::the_agent_stops_playing_a_world_the_cartridge_has_reset` |
| The oracle called a gate a defect for one try per pass | `REPEAT_IS_A_DEFECT` was 3; it is 10 | `being_stopped_by_the_same_thing_over_and_over_is_the_defect` |
| The 402 death loop of 2026-09-05 | A failed turn's messages kept, the plan re-appended every turn, compaction with nothing to drop | `llm::an_undated_hard_failure_does_not_ratchet_the_history` |
| 35 `Fish` rows scored `Silent`, and a cast refused "because you are surfing" one tile from dry land | A fishing route's last button *faces* the water, and the Surf-mount arm read it as a step onto water | `tile_map::a_fishing_rows_last_button_faces_the_water_rather_than_entering_it` |
| The three ways a cast can fail said nothing the oracle could score | All three printed a `TextBox` the agent made up and closed no action | `agent::a_cast_that_could_not_be_made_says_why_rather_than_going_quiet` |
| `Route18:39,13:Grass` chosen and never reported | A trainer's walk-up commits as `Script` and drops the pace; the script and text-box doors knew only about `OverworldMovement` | `agent::a_pace_is_an_open_overworld_action_like_the_walk_that_started_it` |
| A menu of walks across a pond, minted from the square in the gate one map back | `wCurMap` names the new map for 26 ticks before `LoadMapHeader` loads it, and `position_settled`'s bounds check cannot see a stale coordinate that happens to be in range | `mechanics::a_map_the_cartridge_has_not_finished_loading_offers_no_rows`, `the_header_in_wram_says_which_map_has_actually_been_loaded` |
| 37 "there is no route to the warp to SaffronGym", and 239 of 454 turns in one room | The `settled` guard skipped a teleport pad before asking what it was, and the search's own root is settled — so standing on a pad threw away the only edge out of the room | `saffron::every_teleport_pad_in_the_gym_is_a_row_including_the_one_underfoot` |
| No intra-map warp row in the game had ever completed | Every `Warp` completion was the map changing, and a teleport pad does not change the map | `saffron::a_teleport_pad_reports_arriving_even_though_the_map_never_changed` |
| 60 s at a Silph Co 1F warp pokered labels `; inaccessible` | `actions()` kept a dud when no sibling led to the same place — and what that guard was really protecting was Pokémon Mansion 3F's holes, whose tile is in the cartridge's *other* step-on table | `mechanics::an_impossible_warp_is_one_the_cartridge_really_will_not_open` |
| A Saffron Gym menu minted off five of nine sprites | An intra-map teleport reloads the map without changing `wCurMap`, so the header check cannot see it; `.loadSpriteData` writes `wNumSprites`, zeroes the slots, then fills them | `mechanics::every_committed_fixture_has_a_complete_sprite_table` |
| 60 s holding Down on a warp the player was standing on, twice a sweep | A warp on water: `CheckWarpsCollision` is on the *walking* side of `home/overworld.asm`'s surf branch, so the entry has to be stepped onto rather than pressed against — in two places, and one of them ignored the route | `cinnabar::a_seafoam_warp_on_the_water_is_stepped_onto_rather_than_leant_on`, `mechanics::a_warp_reached_by_surfing_is_entered_rather_than_leant_on` |
| Between one and nine key items refused, on all eight starts, for a whole month of sweeps | Gen 1's bag is twenty *kinds* and a finished save arrives with fourteen to twenty used; `Cheats` reported the refusal and carried on, so no S.S. Ticket, no Lift Key, no Coin Case and no rod were a *silent* coverage gap. Room is made first now | `cheats::every_coverage_start_can_be_handed_all_of_the_key_items`; the union's per-region map counts going 38→123, 86→130, 41→33…130 |
| 60 s holding Down on `SilphCoElevator:1,3`, and it is not a water tile | `.noDirectionChange` reaches `CheckWarpsCollision` only past `bit BIT_STANDING_ON_WARP`, which `CheckWarpsNoCollision` sets on a completed **step** onto an entry — so it is clear for a player the cartridge *warped* onto one, which is every elevator in the game. Two places again | `saffron::an_elevator_door_you_warped_onto_is_stepped_onto_rather_than_leant_on` |
| `SeafoamIslandsB4F:20,17` and `:21,17` `DidNotArrive`, every sweep that reaches them | Not the agent: `SeafoamIslandsB4FDefaultScript` force-walks the player north off both and clears `BIT_FORCED_WARP` until both SEAFOAM3 boulders are down holes, which no finished game has. A row the game will refuse is now withheld, as a cut tree is | `mechanics::a_seafoam_staircase_the_script_cancels_is_not_a_row` |
| *"There is no route to the warp to CeladonMansion2F"* from a square that had one two ticks later | A route is what people stand on, and that room has three wandering pets and their owner in a two-wide corridor. `NoRoute` now has to hold still for `MAX_ROUTE_LOST_TICKS` (5 s of game time) before it is said | `celadon::a_route_a_wandering_pet_is_standing_on_is_waited_out_rather_than_disputed` |
| `Route21:Fisher1` chosen and never reported | A wild Pokémon appeared while the party menu was open to **mount Surf**, and `assert_battle_state`'s `_` arm says a battle started and nothing about the walk it interrupted. `Surfing { resume: Some(..) }` is the third state that carries a walk | `agent::a_pace_is_an_open_overworld_action_like_the_walk_that_started_it` (extended) |
| `VermilionGym:LtSurge` `DidNotArrive`, and only from a start that had not won the game | `VermilionGymSetDoorTile` draws block `$24` over the doorway until the trash-can puzzle sets `EVENT_2ND_LOCK_OPENED`, and the static ROM blocks carry the *open* layout — so the BFS routed through a closed door and offered a row behind it. Every postgame fixture has the doors open, which is why three days of sweeps never saw it | `vermilion::lt_surge_is_not_a_row_while_his_doors_are_shut` |
| Four ids chosen and never reported, in one sweep, all on water | A Surf mount ends in **one** simulated step onto the water, and for a `ConnectionWater` row or a warp on the water that step is the whole of the action — so the map changed while the state was `Surfing`, the resume's map no longer matched, and the row was dropped to `Idle`. The arm's own comment already said the crossing "is the walk arriving rather than being interrupted" | `cinnabar::a_walk_the_surf_mount_itself_finishes_says_that_it_arrived` |
| Two thirds of one region's whole budget on a single boulder goal, 230 identical shoves | `MAX_PUSHES` and `MAX_PUSHES_WITHOUT_PROGRESS` are both counted off a boulder that *moved*, so a shove the cartridge refuses in silence is invisible to both; `DRIVER_ESCAPE_SILENCE` drops to `Idle` and `Idle` picks the goal back up unchanged | `PokemonAgent::boulder_goal_silences` and `MAX_SILENT_SHOVES` |
| `CeruleanMart:3,7:Warp` `DidNotArrive` in **three regions of one sweep**, from the square beside it | A person standing on a warp left the `Warp` visible *underneath* them in `meta_tiles` — a refactor's "matching the original ordering", never an argument — so an occupied doormat read as open floor, the BFS routed through the shopper and the walk held Down against her for 60 s. A mart's exit is two tiles wide, and `4,7` was taken on the next turn without trouble | `mechanics::a_door_with_somebody_standing_in_it_is_not_a_row_until_they_move` |
| Two `NoRoute`s in `CeladonChiefHouse`, the second disproved by the walk itself one turn later | `MAX_ROUTE_LOST_TICKS` is 5 s and was sized against **one** wanderer taking a step; that room is two corridors one tile wide with a Rocket in one and the Chief in the other, so a row on the far side needs both of them to move at once. Past the short bound the agent now *asks* — `row_blocked_by_people` lifts everybody but the row's own subject onto the floor `underfoot` says is under them — and only a `yes` buys the 30 s `MAX_ROUTE_BLOCKED_TICKS`. A `; inaccessible` warp is still answered at 5 s | `celadon::a_room_whose_corridors_are_both_blocked_is_waited_out_rather_than_called_routeless` |
| `Route11:13,6:Grass` *"it stopped making progress"*, intermittent since 2026-09-09 and closed at last | A pacing pair is chosen once from `adjacent_grass` and then held for the whole pace, so a Youngster stepping onto one half of it leaves the agent bumping into a person — and bumping is not a step, so the ROM never rolls and the row aborts as `Unknown`. It re-picks now, and reports only when there is no other pair at all | `mechanics::a_pacing_pair_somebody_steps_onto_is_re_picked_rather_than_bumped_into` |
| **Cerulean Cave's three floors**, `unreached` on every sweep this plan has taken | `actions()` emitted the nearest crossing per adjacent map of *either* kind, so wherever a land bridge and a surfable edge lead to the same neighbour the bridge always won — and Route 24's footbridge is two steps from the river seam that is the only way into the half of Cerulean the cave door is on. One row per *kind* now. The ROM cross-check had printed the cause every sweep: `CeruleanCity (5, 12) → CeruleanCave1F: on the grid, no sibling, and never a row` | `mechanics::a_water_crossing_is_a_row_of_its_own_beside_the_bridge_to_the_same_map` |
| `Route12:0,63:Connection` chosen and never reported, on the re-sweep | `Surfing` **with a `resume`** is one of the three states that carry an open overworld action, and the refused-mount arm dropped straight to `Idle` with the row still open: the walk surfed south out of Route 12, crossed into Route 11, had the mount its follower tried on the far side refused, and chose its next row on Route 11 with the crossing never reported. The arrival rule is now a helper (`surf_crossed_into`) that both doors out of the mount use, and a refusal on the *same* map is `Textbox` — which is exact, since the cartridge stopped the player to say "No SURFing on <mon> here!" | ⚠️ **No test of its own**, as `MAX_HEAL_HOPS` has none: the refusal needs the cartridge's own terrain check reached from inside the party menu, and the disagreement that produces one is by definition a tile the reader gets wrong. The arrival half is `cinnabar::a_walk_the_surf_mount_itself_finishes_says_that_it_arrived`, which now covers the shared helper; the rest is the next sweep |
| **Route 17 and `Route16Gate2F`**, ditto — and a hole in the deployed tool surface behind them | `use_field_move`'s `use_item` required a `target` tile and the Bicycle has none, so `FieldMove::UseBagItem` and the whole of `UseTarget::Nothing` had a driver, a refusal table and a test that rides a bike, with no way in from any LLM turn. With it went every out-of-battle Potion, vitamin, Repel and Itemfinder. +189 bytes of catalogue | `tools::a_bag_item_with_nothing_to_aim_at_is_a_call_that_can_be_made` |

⚰️ **Four of these were diagnosed wrongly before they were diagnosed rightly, and the lesson is the
same every time: argue from the dropped save state, not from the sentence the agent printed.** Two
of the four are §7.2's items 8 and 13; the other two are 2026-09-10's, and both were settled in
minutes by `probe_button_at_state` after an hour of reading the ROM had settled neither. The Silph
Co elevator's answer is one line of its output — `wMovementFlags` reads `$00` while the player leans
on a warp entry — and no amount of `home/overworld.asm` was going to say so, because the flag is
live state and the source is a program.

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
9. **Step 2 did it again, in the other direction: its ⚠️ said the Seafoam pair was "not this", and
   it was not — but it was the same step's work.** A step's ⚠️ can be right about the diagnosis and
   wrong about the scope; the "Done" line is what binds, and step 2's was "the walk has 0 defects".
10. **A rule the agent and the map layer both encode has to be fixed in both, and the second one is
   invisible from a unit test.** `MetaTileMap::actions` learned that a surfing player cannot lean on
   a border warp and the walk did not change at all, because `OverworldMovement` tests for that shape
   before it ever reads the route. The route test passed the whole time. Only an agent-level test off
   the dropped state caught it.
11. **A guard kept "just in case" is a guard nobody has priced, and the price is the finding.**
   `actions()` kept a warp the cartridge will not open whenever no sibling led to the same place,
   on the honest argument that a false negative would strand a run. Asking *which* rows it was
   actually keeping — 7 in the whole game, 4 of them load-bearing-looking — turned up a real false
   negative it had been covering for since it was written: Pokémon Mansion's floor holes read from a
   tileset table this code had never heard of. The guard went; the scan became the test.
12. **A step's ⭐ hedge is worth reading before its Done line.** Step 3 predicted Silph Co 1F would
   fall out of the gym fix and said, if it does not, that is a second finding. It did not, and it
   was.
13. **Step 4's diagnosis was wrong and implementing it was still how that was found out.** "Count
   exits by the map they lead to" is a good rule and it changed the ping-pong not at all — the
   counters alternated perfectly while the walk went round fifteen hundred more times. What sealed
   Route 16 was a **cut tree that regrows**, one row the frontier had marked done. Building the
   step's own fix and watching it not work is what made the menu worth printing, and the menu had
   three rows on it where the plan assumed eight.
14. **A counter nobody prints is a fact nobody has.** `Cheats::bag_was_full` had been counting
   refused key items since it was written, with a comment calling it "a coverage gap worth
   printing". Printing it took two lines and immediately said that every one of the eight starts is
   missing key items that gate whole clusters of the maps no walk had entered.
15. **…and a fact nobody has is worth more than any change to the walk.** Making room in the bag is
   **one call** in the driver, and it moved `phase0` from 38 maps to 123, `saffron` from 129 to 131
   with the Rocket Hideout in it, and every start onto the same fourteen key items. Four of the six
   defects that remained were then somewhere no sweep had ever been. Steps 1 to 5 were each a day
   of agent work for a handful of ids; step 6's first line was a line.
16. **A cluster in the unreached list can be gated by the *fixture* rather than by the game, and
   the plan's own ⚠️ said the opposite.** "A cluster is a gate, a missing start or a full bag — never 'the
   walk is bad'" left out the fourth thing: the save. `EVENT_SS_ANNE_LEFT` is set before the third
   badge, so the S.S. Anne's ten rooms and `VermilionDock` are unreachable from **every** finished
   game — the plan predicted they would "come off the list the moment the ticket fits", the ticket
   now fits in all nine bags, and they did not move. That is what `Start::before_the_credits` is
   for, and it is the only exception to the finished-game rule admitted so far.
17. **A "starting over" with nothing counting it is a livelock with a friendly sentence.**
   `DRIVER_ESCAPE_SILENCE` was written as the net under every self-driving driver and it is one —
   except where the state it drops to *restores* the thing that jammed. A boulder goal does exactly
   that, and neither of its two bounds could see a shove that never landed, so the net became a 60 s
   loop that ran for two thirds of a region's budget. **Every hatch needs to ask what picks the work
   back up.**

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
