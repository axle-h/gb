# Full exploration through the LLM path

**The goal, in one sentence.** Every action the game offers, taken once, through the stack exactly
as it is deployed — `LlmPolicy`, the worker and the wire, against a mock endpoint in-process — with
a verdict on each, and every defect that turns up fixed, until a sweep of the whole of Kanto comes
back clean and finds nothing new.

**Status.** Rewritten 2026-09-09 as a step list, from a review of the first three days' work;
**steps 0 to 5 taken the same day**. The harness, the cheats, the oracle and the frontier walk are
all **built**; the baseline sweep reaches **153 maps of 248** and came back **red with 106 defects
and 41 silences**, with **82% of every turn it took spent in Route 16's gate**. §2 is that baseline
and every later step measures against it. **The 41 silences are now 0 and a silence fails the tier**
(step 1); `fuchsia`'s four defects are 0 at the full budget (step 2), and so are **Saffron Gym's 99
and Silph Co 1F's 3** (step 3), which leaves the 106 at **one** — a wandering shopper, and step 6's.
Route 16's gate is closed too (step 4) — not by the scoring change the step predicted but by a cut
tree that regrows — and the sweep prints what it did *not* reach (step 5). **§2.1 is where the sweep
stands now**: 186 maps of 248 and 1 813 ids, with 38 real maps unreached and six defects left, most
of both traceable to a full bag. The steps in §4 are the path from there to a
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

### 2.1 Where the sweep stands after steps 1 to 5 — 2026-09-09

Taken with `GB_COVERAGE_START=all` on an idle machine, 6 game-hours per region, 40 minutes of wall
clock. This is the state every remaining step measures against; §2's table above stays as the
**baseline**, which is what the closures in it are relative to.

| | | | |
|---|---|---|---|
| **start** | **maps** | **ids** | **only it reached** |
| `phase0` | 38 ⭐ Hall of Fame | 367 | 1 |
| `cerulean` | 119 | 996 | **0** |
| `vermilion` | 133 | 1 158 | **0** |
| `lavender` | 133 | 1 155 | **0** |
| `celadon` | 132 | 1 268 | **0** |
| `saffron` | 129 | 1 128 | **0** |
| `fuchsia` | ⭐ **163** | 1 416 | ⭐ **30** |
| `cinnabar` | 41 ⭐ Hall of Fame | 371 | 4 |
| **union** | ⭐ **186 of 248** | ⭐ **1 813** | |

⚠️ **Five of the eight starts now contribute nothing no other start reaches.** That column used to
be the argument for having eight; after step 4 a single walk from `fuchsia` reaches 163 of the 186
on its own, and `cerulean`, `vermilion`, `lavender`, `celadon` and `saffron` between them add none.
Five minutes of wall clock each, for nothing. ⭐ **Step 6.3's first tool is "another start", and this
says the ones to replace** — a start is worth having only where it stands in a cluster nothing else
reaches, which is what `phase0` (1) and `cinnabar` (4) still do and the middle five no longer do.

**The 62 maps no walk entered**, printed by the sweep itself (step 5). 22 are the ROM's `UnusedMap*`
padding and two are the link-cable rooms `Colosseum` and `TradeCenter`, which leaves **38 real
maps** — and they are almost entirely four clusters and a handful of doors:

- ⭐ **The S.S. Anne, all of it** (10) and `VermilionDock`: `SSAnne1F` `SSAnne1FRooms` `SSAnne2F`
  `SSAnne2FRooms` `SSAnne3F` `SSAnneB1F` `SSAnneB1FRooms` `SSAnneBow` `SSAnneCaptainsRoom`
  `SSAnneKitchen`. **The bag refuses the S.S. Ticket** — see the `cheats` line in §6.1.
- **Cinnabar Island's buildings** (8): `CinnabarGym` `CinnabarLab` `CinnabarLabFossilRoom`
  `CinnabarLabMetronomeRoom` `CinnabarLabTradeRoom` `CinnabarMart` `CinnabarMartCopy`
  `CinnabarPokecenter`. The `cinnabar` start stands on the island and wins the game instead.
- **Pokémon Mansion** (4): `PokemonMansion1F` `2F` `3F` `B1F`.
- **Cerulean Cave** (3): `CeruleanCave1F` `2F` `B1F`.
- **Pallet Town's interiors** (4): `BluesHouse` `OaksLab` `RedsHouse1F` `RedsHouse2F`.
- **Three `*Copy` duplicates** the ROM never warps to: `CeruleanTrashedHouseCopy`
  `UndergroundPathRoute6Copy` `UndergroundPathRoute7Copy`.
- **Six doors on their own**: `CeladonGym` `ViridianMart` `Route17` `Route16Gate2F`
  `SafariZoneWestRestHouse`. ⚠️ `Route17` is **Cycling Road**, and the Bicycle is one of the few key
  items that *does* fit every start's bag — so that one is not the bag, and is worth a look.

⚠️ **A cluster in that list is a gate, a missing start or a full bag — never "the walk is bad".**
Rocket Hideout and the Game Corner prize room came off this list between the baseline and now
without anyone touching them; the S.S. Anne will come off it the moment the ticket fits.

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

That is most of §2's list of maps no walk has ever entered: no S.S. Ticket is the whole of the S.S.
Anne's nine rooms, no Lift Key is Rocket Hideout's four floors, no Coin Case is the prize room, no
rods are every `Fish` row on the map. The Bicycle fits in all eight, so Cycling Road was never the
blocker. **Making room in the bag is step 6's cheapest tool by a distance** and is now the first
thing to try there.

### Step 5 — Print the maps never entered ✅ done 2026-09-09

The sweep printed a union and nothing about its complement, and the complement is the only thing
left to act on: from a count, a gate doing its job and a walk that never arrived look identical.
`Map::iter()` minus the union was being assembled by hand with a shell diff, which is exactly the
sort of arithmetic that gets done once and then quoted for a week after it stopped being true —
§2's list below was.

✅ **Taken.** `unreached_report` prints it from the multi-region summary, sorted and wrapped six to a
line so a *cluster* is visible — the S.S. Anne's nine rooms, Rocket Hideout's four floors — which a
column of sixty names hides. ⚠️ It counts the ROM's `UnusedMap*` padding and the two link-cable
rooms and then sets them aside: 24 of the 62 that look missing are neither.

**Done:** `GB_COVERAGE_START=all` prints `unreached` under `union`, and §2 carries the list from the
sweep of 2026-09-09. **Cost:** small, as billed.

### Step 6 — The loop, until the fixpoint

Everything above is one pass. The goal is a fixpoint, and this is the loop.

⭐ **Start with the bag.** Step 4's `cheats` line says every one of the eight starts arrives with all
twenty of Gen 1's bag kinds used and refuses between one and nine key items — the rods, the S.S.
Ticket, the Lift Key, the Coin Case, the Silph Scope, the Itemfinder. That is most of the unreached
list above and it is one `Cheats` change, not a walk: shed what a finished save is carrying that the
walk does not need (or put it in the PC) before handing over `COVERAGE_KEY_ITEMS`. Nothing else in
this section buys as many maps per line.

⚠️ **The `all` sweep of 2026-09-09 (§2.1) came back with 6 defects and 2 silences, and they are the
queue.** Every one is a row the earlier sweeps never reached — the union went 159 maps to 186 and
1 410 ids to 1 813 — so this is the loop working rather than a regression:

- **`Route11:13,6:Grass`, three regions** — *"it stopped making progress (standing at (14, 6))"*.
  New, reproducible across starts, and the only one of these that is not a warp.
- **`SeafoamIslandsB4F:20,17:Warp` and `:21,17:Warp`** — `DidNotArrive` at (21, 15), with
  `SeafoamIslandsB3F:20,17` and `:25,14` `Silent` beside them. ⚠️ **This may be the cartridge rather
  than the agent**: `SeafoamIslandsB4FDefaultScript` force-walks the player off those staircases,
  `res BIT_FORCED_WARP` cancelling the warp, until `SEAFOAM3` is set (`policy.rs` carries the whole
  argument). Argue it from the dropped state; if it is the script, it is a `Blocked`, not a fix.
- **`SilphCoElevator:1,3:Warp`** — `DidNotArrive` standing on the square. An elevator door.
- **`CeruleanMart:CooltrainerFemale`**, intermittent (in the step-3 sweep, not this one) — a shopper
  who wanders, so a `NoRoute` to a **sprite still on the map** is a sprite that moved rather than a
  row the agent could not carry out.

The loop:

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
# ⭐ This is the only form that prints `unreached` — the maps no walk entered, which is step 6's
# input. The parallel recipe below is faster and cannot: the diff needs `Map::iter()`.
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

Every walk's own summary carries a **`cheats`** line saying which key items would not fit the bag.
⚠️ **Read it before believing a coverage gap**: Gen 1 holds twenty *kinds* and every finished-game
start arrives with all twenty used, so a sweep can look healthy while the S.S. Anne, Rocket Hideout
and every `Fish` row are unreachable for want of a ticket, a lift key and three rods.

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
   missing key items that gate whole clusters of §2's unreached maps.

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
