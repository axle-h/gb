# Full exploration through the LLM path

**The goal, in one sentence.** Every action the game offers, taken once, through the stack exactly
as it is deployed — `LlmPolicy`, the worker and the wire, against a mock endpoint in-process — with
a verdict on each, and every defect that turns up fixed, until a sweep of the whole of Kanto comes
back clean and finds nothing new.

**Status.** The harness, the cheats, the oracle and the frontier walk are built, and **step 1 is
taken but for one map**: the sweep of 2026-09-10 that §2.1 tables is **215 of 220 maps, 2 170 ids, 0
defects and 0 silences**, with every one of its ten walks spending its whole game-time budget and
none of them stopping on the credits. Four of the five maps it missed are behind a row that was
*offered and not chosen*, which is a property of the frontier rather than a fault; the fifth,
`SafariZoneWestRestHouse`, is the whole of what §4's step 1.4 has left. The nine steps that got here
are one line each in §7.4 and their findings are §7.1's rows, every one with a test.

⚠️ **A clean sweep was never the goal, and this file used to say so too quietly.** The walk asserts on
defects and silences; it does **not** assert that every map was entered, that every action offered was
taken, or that the ROM's own tables were exhausted. All three used to be printed and going unread. Two
of them are assertions now — the map denominator is honest (1.2) and every kind of row the game can
offer has to have been offered somewhere or the tier fails (1.5) — and the third, `unreached`, is
printed by kind with a reason per family (1.6).

⭐ **Step 2 is built too.** `godmode_run` plays a fresh save at Pallet Town to the **Hall of Fame**
through `LlmPolicy`, the worker and the wire, in **33-41 s** against `full_playthrough`'s 231 —
about six times faster, on 46 requests, none of them battle turns. ⛔ **It does not replace
`full_playthrough` and the reason is not speed**: it never touches `PolicyStep`, so retiring that
tier would leave `--policy deterministic` — a shipped feature — with no end-to-end test, and it walks
18 maps against the whole of Kanto. It joins the pre-push tier instead. §4's step 2 has the table,
the measurement and the four faults building it turned up.

**This file is self-contained.** `CLAUDE.md` points here and says nothing about the plan that this
file does not: §3 is the rules, §6 is how to run a sweep and how to read its numbers, §7 is the
record. Where this file and another disagree, the run the number came from is the authority.

⚠️ **Code comments cite section numbers from the first draft** (`§2.2.1`, `§3.3`, `§5.2.6`, …).
Those refer to the 2026-09-06 plan, which is in git history at commit `7343616`; §7.3 says where
each one's argument lives now.

---

## 1. What is built

| | Where | What it is |
|---|---|---|
| The harness | `integration_tests/llm_harness.rs` | `MockEndpoint`, a `Brain` that is handed **strings and nothing else**, six injectable faults (`Fault`), and `LlmRun`: the real worker, policy, agent and emulator with a run directory and a restart |
| The cheats | `integration_tests/cheats.rs`, `postgame/debug.rs` | A god party (Mewtwo with four attacks, one slave with Cut/Surf/Strength/Flash, one with Fly), badges and key items, applied by the driver **between ticks**. `play_path_contains_no_debug_ram_writes` guards the line |
| The oracle | `integration_tests/coverage.rs` `CoverageLog` | Every action id the agent starts gets `Completed`, `Blocked`, `Defect`, `Silent` or `Unreached`, folded from `AgentEvent`s so it works under any driver. Drops a save state where a defect happened |
| The walk | `coverage.rs` `ExploringBrain`, `coverage_walk_of_the_finished_game` | Takes every unvisited row on the map, then the least-taken exit. Starts from one of ten fixtures (`GB_COVERAGE_START`), eight of them **finished games** so every gate is open because the cartridge opened it; the other two are argued on `Start::before_the_credits`. Behind `--features coverage-tests` |
| The cross-check | `coverage.rs` `rom_cross_check` | Warps and objects in the ROM's tables for the maps entered that never once appeared as a row. Printed, never asserted |
| The kind check | `coverage.rs` `kind_cross_check` | ⭐ Every kind of row `MetaTile::id_kind` can return, against the kinds a sweep was offered — a `match` on `MetaTile`, so a new variant is a compile error. **Asserted**, over the union of a coverage-budget sweep, with a two-entry allow-list. Step 1.5 |
| The god run | `integration_tests/godmode.rs` | `Intent`, `ScriptedBrain`, and `godmode_run` — Pallet Town to the Hall of Fame through `LlmPolicy` in ~40 s (`--features godmode`). The machinery is default tier and also drives step 8's branch arms |
| The branch points | `integration_tests/branch_points.rs` | Four snapshots cut one decision before a choice that is **exclusive per save**, nine arms after them and three trade tests, driven by `Intent` against the rendered menu. Step 8 |

## 2. Where we are

The 2026-09-09 baseline that used to be tabled here — eight walks, 153 maps, 106 defects, 41
silences, 82% of every turn in Route 16's gate — is §7.4's row 0, and every bullet under it was
closed by a step in that table. ⚠️ **Take every baseline from a run, never from a table in this
file.** The walk is not deterministic and its spread is not noise (§6.2).

### 2.1 Where the sweep stands — 2026-09-10, after step 1

Ten walks in parallel on an idle machine, 6 game-hours each, about 14 minutes for the set (§6.1).
Every number here was re-derived from the walk logs and `walk-*.tsv` files the §6.1 recipe leaves
behind, and the map list is the diff of `Map::iter()` against their union.

| **start** | **maps** | **ids** | **defects** | **silent** | **before step 1** |
|---|---|---|---|---|---|
| `phase0` | 136 | 1 302 | 0 | 0 | 38 ⭐ stopped on the Hall of Fame |
| `cerulean` | 135 | 1 263 | 0 | 0 | 120 |
| `vermilion` | 135 | 1 274 | 0 | 0 | 135 |
| `lavender` | 135 | 1 257 | 0 | 0 | 135 |
| `celadon` | 134 | 1 299 | 0 | 0 | 134 |
| `saffron` | 135 | 1 251 | 0 | 0 | 134 |
| `fuchsia` | 105 | 993 | 0 | 0 | 70 ⭐ stopped, and ⛔ 1 defect |
| `cinnabar` | 131 | 1 151 | 0 | 0 | 41 ⭐ stopped |
| `ssanne` | 106 | 1 025 | 0 | 0 | 119 |
| `ssanneship` | 111 | 1 001 | 0 | 0 | 82 |
| **union** | **215 of 220** | **2 170** | **0** | **0** | 199 of 220, 1 defect |

⚠️ **Read the pair, never the row** (§6.2). This is one sweep. What it is good for is the list below,
and the two facts the right-hand column is there for: **no walk stopped on the credits** — `cinnabar`
and `fuchsia` both reached the Hall of Fame (at 45% and 55% of budget), reported it, and were rewound
to the Indigo Plateau lobby to spend the rest — and **every one of the ten spent its whole game-time
budget with the frontier still open**, where three used to settle at under half of it.

**The five maps this sweep missed, and ⭐ four of them are behind a row that was offered and not
taken.** That is a different fact from the last sweep's twenty-one and it moves them out of step 1.4
and into step 1.6: nothing is withholding these rows, the frontier simply spent its budget elsewhere.

- `CeruleanCaveB1F` — behind **`CeruleanCave1F:3,11:Warp`, offered and `unreached`**. The ladder chain
  is no longer shut: the row that `probe_route_to_cerulean_cave` says is the only way onto the B1F
  half of 1F is on the menu, and no walk chose it.
- `Route17` and `Route16Gate2F` — both behind **`Route16:24,10:Warp`, offered and `unreached`**, the
  south door of the Route 16 gate. ⚠️ **The guard is not the reason and this file said he was.**
  `Route16Gate1FDefaultScript` stops a player only when the Bicycle is **not in the bag**
  (`Route16Gate1FIsBicycleInBagScript` → `IsItemInBag`), and every walk carries it — being *on* the
  bike is not required. Route 18 and both of its gate floors are in the union, reached from the
  Fuchsia side, so Cycling Road is walled at one end only.
- `CeladonGym` — behind **`CeladonCity:13,27:Warp`**, the sibling of the gym door at (12, 27), which
  the sweep of 2026-09-10 `completed` and this one left `unreached`. Pure walk-to-walk variance
  (§6.2).
- ⭐ `SafariZoneWestRestHouse` — behind **`SafariZoneNorth:8,35:Warp` and `:9,35:Warp`, both offered
  and `unreached` in both sweeps**, and this one is worth the paragraph because it is the only
  interesting shape in the list. `SafariZoneWest` is **two shelves that one-way ledges seal off from
  each other**, and `SafariZoneNorth` has four doors into it in two pairs that land on opposite
  sides: the western pair lands on the Gold Teeth plateau and the eastern pair on the shelf the rest
  house is on. All four are rows — `actions()` mints one per unique destination — but the frontier
  counts a way out **per crossing** (`SafariZoneNorth → SafariZoneWest`), so the moment either pair
  is taken the crossing is "done" and the other pair waits behind every other exit on a very large
  map. Twenty walks took the western pair twenty times. ⚠️ **`SafariZoneWest:11,11:Warp` is a row
  from the eastern landing and simply is not on the menu from the western one**, which is the same
  shape as Seafoam's two holes one level up: two doors that look interchangeable and are not.

⚠️ **And the Safari Zone's own clock is what bounds exploration inside it**, which nothing in this
file said before: `SafariZoneWest:20,0:Warp` and `SafariZoneNorth:35,3:Warp` are both scored
`blocked` with the cartridge's own sentence quoted — *"PA: Ding-dong! Time's up! PA: Your SAFARI GAME
is over!"*. 500 steps is the budget the game gives, the walk spends it wandering, and the North rest
house is reached only when the frontier happens to spend the steps in that direction.

**The arithmetic.** `Map::iter()` yields **248** because the enum is the map *byte*: 22 `UnusedMap*`
have no header at all, 2 are link-cable rooms, and 4 are duplicate headers **no warp from any map
targets** (`CeruleanTrashedHouseCopy`, `CinnabarMartCopy`, `UndergroundPathRoute6Copy`,
`UndergroundPathRoute7Copy`, checked against `pokered/data/maps/objects/`). So the denominator is
**220**, and `unreached_report` prints it beside the count rather than leaving it to be worked out.

⭐ **2 170 ids were offered across the sweep and 1 018 of them were never chosen by any walk.** That
is step 1.6's number and the sweep prints it broken down by kind now, per walk and over the union.
The shape is stable and it is the same one the first count showed: **exits and grass**, on maps a
walk left by one door before it had finished the others. It will not go to zero by walking longer —
a frontier that leaves a map by its least-taken exit leaves that map's other exits behind by
construction, and a `Grass` row is deliberately not re-takeable — so what is tracked is the ratio and
the shape.

⚠️ **The ROM cross-check is printed and still worth reading.** Across the sweep: the
`CeladonMansion` `(6, 1)` doors, the `CeruleanCave` ladders and ~40 people per walk who were never a
row. ⚠️ **Not a diagnosis** — every warp finding in this file that was argued from the ROM was wrong;
it needs `probe_button_at_state` on a dropped state first (§7.1's ⚰️).

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
  iteration took, so CPU load is an input to the walk. Ten walks in parallel on an idle box land
  in the modes §6.2 lists; one walk beside a `cargo build` does not agree with itself. (§6.2.)
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

**Both are taken.** Steps **0 to 8** of the 2026-09-09 draft
are §7.4, one line each, kept because code comments still name them by number; their arguments are
§7.1 and the comments those rows point at. Step 9 was never taken and is folded into step 2 below.
Steps 1 and 2 below are both **done** and are kept in full rather than shortened to a line, because
three of step 1's six items and one of step 2's assumptions were misdiagnosed in writing, and the
corrections are the useful part.

Each step below says what "done" is as something a fresh agent can check by running one command.
⚠️ **§3's rules hold for both**, and the two that bite hardest here are *fix the driver, not the
walk* and *argue a defect from the dropped save state, never from the sentence the agent printed* —
to which step 1 added a third: ⚠️ **a state dropped at a defect cannot reproduce a fault in a clock
the agent carries**, because restoring sets it to zero. The walk's own log is what settles those.

---

### Step 1 — Nothing unreached, and nothing unexplained ✅ **done**

**The goal in one sentence: one sweep in which every real map is entered, every action offered is
either taken or explained, and the cross-check that says so is an assertion rather than a
paragraph.** §2.1 is where that stands after the six items below: **215 of 220 maps, 0 defects, 0
silences, and no walk stopping short of its budget** — and every one of the five maps it missed
behind a row that *was* offered and not taken, which is the frontier's shape rather than a fault.

**1.1 ✅ The boulder defect, and it was not a boulder.** `VictoryRoad3F:3,5:PushBoulderOntoSwitch`
gave up after 34 pushes, five times, on the `fuchsia` walk — three shoves from the end of a puzzle it
had already solved twice from the same floor, with the plan getting shorter on every one of them.
`DRIVER_ESCAPE_SILENCE` was measured against `cycles_since_poll`, the clock only a **decision
point** resets, and a `BoulderGoal` is one decision that walks a boulder across a whole floor: past
60 s of game time with no wild battle to poll the policy, every entry into `PushingBoulder` was
escaped on its first tick and three of those is `MAX_SILENT_SHOVES`. ⚠️ The dropped state does not
reproduce it — a restored agent starts the clock at zero — which is the one case §6.2's third rule
does not cover; the walk's own log does, and the tell is three escapes in a row with no 60 s between
them. Fixed with a second clock a landed shove also resets
(`PokemonAgent::cycles_since_driver_answer`), and the test holds a Repel up so VictoryRoad3F's
hardest puzzle runs 45 s of game time with the policy unasked.
**Done:** `endgame::a_boulder_goal_that_keeps_shoving_is_not_a_driver_the_game_has_gone_quiet_on`,
default tier; and the row `completed` on the sweep above.

**1.2 ✅ `unreached_report` is honest.** The four `*Copy` duplicates are their own bucket with the
reason, and the denominator — 220 — is printed beside the count.
**Done:** `coverage::UNREACHABLE_DUPLICATES`, the report's `N of 220 reachable maps`, and
`mechanics::a_duplicate_map_is_not_a_coverage_gap` — which reads the four out of the ROM's own warp
tables rather than trusting the list, so a duplicate that moves upstream fails a test instead of
quietly moving the denominator.

**1.3 ✅ No walk spends half its budget on the credits.** `phase0`, `fuchsia` and `cinnabar` used to
`settle` by winning the game at 42-60% of budget and stop. The driver now checkpoints the first time
the brain is asked a turn on `IndigoPlateauLobby`, reports the Hall of Fame exactly as before — §3's
rule — and then rewinds the emulator to that checkpoint and hands the walk the rest of its budget.
The emulated time carries across, so the rewind buys no extra budget; the Lorelei door is barred
afterwards, because an exit is re-takeable by least-taken count and a door taken once in a lobby
whose other two have been taken never is the *best*-scoring row in the room.
**Done:** `LlmRun::restart_from_last_checkpoint`, `ExploringBrain::carry_on_after_the_credits`; and
the sweep above, in which two walks reached the Hall of Fame, neither stopped there, and every one of
the ten spent its whole budget. `phase0` went 38 maps to 136 and `cinnabar` 41 to 131.

**1.4 ✅ There is no map the agent shuts out.** ⚠️ **All five of the maps this step was written for
have come off the list, and three of them were misdiagnosed here.** The Cerulean Cave ladder chain is
not a routing fault — `CeruleanCave1F:3,11:Warp` is on the menu and no walk chose it. Route 17 is not
the gate guard, who stops a player only when the **Bicycle is not in the bag**
(`Route16Gate1FIsBicycleInBagScript` → `IsItemInBag`) and never asks whether it is being ridden; it is
behind `Route16:24,10:Warp`, offered and `unreached`. And the Safari rest houses are not one door
each: `SafariZoneNorth` has **four** doors into `SafariZoneWest`, in two pairs landing on opposite
sides of one-way ledges, and all four are rows. Every one of the five is §2.1's `unreached`, which is
step 1.6.

⚠️ **A cluster that is "unreachable" can be a door the frontier ranks last rather than a row the
agent withholds, and this step assumed the second for a month.** §7.2's item 16 already said a cluster
can be gated by the *fixture* rather than by the game; this is the fourth thing again — it can be
gated by the walk's own exit ordering, and a count of maps cannot tell the two apart. What can is the
`unreached` column, which is why 1.6 exists.

**Done:** `postgame::safari::the_safari_wests_rest_house_is_a_row_from_the_shelf_it_is_on`, off a
committed fixture cut at the eastern landing (`safari-west-shelf.bin`), because which shelf a walk
stands on is not reproducible by re-running (§6.2). The rest is 1.6's ratio.

**1.5 ✅ Every kind of row, asserted.** `rom_cross_check` covers warps, objects and connections and
nothing else, so `Fish`, `Grass`, `CutTree`, both boulder goals, every `Switch` and `Pc` had no "was
this ever offered anywhere" check at all — and the first run of the new one said that **`Pc`,
`Statue` and `CellSeparator` had never appeared once in a ten-walk sweep**. `Statue` came back the
moment 1.3 let a walk reach the Pokémon Mansion. The other two are the allow-list, and both carry
their argument: `llm::tools` withholds `MetaTile::Pc` on purpose (a PC operation is a
`use_field_move`, and "one way in, not two"), and Bill's cell separator is offered only before Bill
has been turned back into a person, which every `COVERAGE_STARTS` save is long past.
**Done:** `coverage::kind_cross_check` — a `match` on `MetaTile`, so a new variant is a compile error
— asserted over the union of a sweep and never per map per walk, with the `Pc` allow-list checked
against the PC-operation count rather than merely described.

**1.6 ⭐ `unreached`, with a reason per family.** 1 018 of the sweep's 2 170 ids were offered
somewhere and chosen nowhere. The sweep prints the breakdown by kind, per walk and over the union;
the families and why each stays:

| kind | why it stays |
|---|---|
| `Warp`, `Connection`, `ConnectionWater` | ⭐ **The frontier's own shape.** A walk leaves a map by its least-taken exit, which by construction leaves that map's other exits behind. Every one of §2.1's five missing maps but `SafariZoneWestRestHouse` is one of these. |
| `Grass` | Deliberately not re-takeable (`ExploringBrain`'s `re_takeable`): a second pace discovers nothing and costs a 60 s budget, and the grind is the right answer for a model playing the game and the wrong one for a walk whose job is breadth. |
| `Fish` | One row per rod per water's edge, and a map's edge carries many; taking one is enough to prove the mechanism, which `postgame::fishing` also pins. |
| `Sprite`, toggleable objects | A person already talked to and an item ball already in the bag. `rom_cross_check`'s object scan is what reports these per map. |

**Done:** the sweep prints `unreached` by kind and this table is the reason for every family above
50 ids.

**Cost:** taken. 1.4 is what is left, and it is one map.

---

### Step 2 — The god run ✅ **built**, and the tier it does *not* replace

⭐ **A fresh save at Pallet Town played to the Hall of Fame through `LlmPolicy`, the worker and the
wire.** `godmode_run`, `--features godmode`. It is the first test that plays the *deployed* policy
over a real game: 46-47 requests from the title save to the credits, chaining,
`resume_after_battle`, the battle script, the agent's boulder goals and the Elite Four.

**What it costs, measured 2026-09-10 on this machine, three consecutive runs:**

| | game time | wall clock | rate | requests |
|---|---|---|---|---|
| `godmode_run` | 1 457 – 2 104 s | **32.7 / 38.0 / 41.2 s** | 45-51× | 46-47, **0 of them battle turns** |
| `full_playthrough` | 18 322 s | **231.3 s** | 79× | — |

⭐ **It is about six times faster**, which settles §4.2's question with room to spare. The reason is
the one the first draft predicted: `full_playthrough` spends its 18 322 game-seconds levelling a
Squirtle into a Blastoise, and a god party skips all of it. The battle script does the rest — every
battle on the route, the rival in Oak's lab and all twenty-six Pokémon of the Elite Four included,
is decided on the emulator thread and costs no request at all, which the run asserts rather than
hopes.

**It does not play the scripted route, and the cartridge is why it does not have to.** The badges are
cheated, so the only question is which gates read the **badge byte** and which read an **event
flag** — and both gates on the west road read the byte: `Route22GateGuardText` is
`ld a, [wObtainedBadges] / bit BIT_BOULDERBADGE`, and `Route23CheckForBadgeScript` is
`ld hl, wObtainedBadges` for all seven of its guards (the `EVENT_PASSED_*_CHECK` flags it also
touches are only a memo that the guard has already asked). So the run is Pallet → Viridian →
Route 22 → Route 23 → Victory Road → the Elite Four, 18 maps. ⚠️ **Pewter is the counter-example**:
its east exit reads the *event flag* for having beaten Brock, which is why the coverage walk starts
from a finished game instead.

#### ⛔ It is not a replacement for `full_playthrough`, and the reason is not speed

The bar was "not much slower" and it is six times faster. It still should not take that tier's place:

| what `full_playthrough` gates | does the god run? |
|---|---|
| `PolicyStep` — **the scripted route itself** | ⛔ **No.** It uses `Intent`, not `PolicyStep`, so nothing would be left testing `DeterministicPolicy` end to end — and that is a *shipped* feature (`--policy deterministic`, the README's free-to-watch mode). |
| The whole of Kanto | ⛔ 18 maps against ~120. |
| Catching, teaching an HM, buying, selling, the PC | ⛔ The party is installed and the badges are given. |
| ⛔ **"Can a party the run *earned* actually win?"** | ⛔ Nothing else covers this either; `hall_of_fame_playthrough` is the only test that ever has. |
| The deployed `LlmPolicy` over a real game | ⭐ **Only the god run**, and nothing else ever has. |

⭐ **So the two gate different things and both are cheap.** `godmode_run` joins the pre-push tier
rather than replacing anything: 40 seconds beside `full_playthrough`'s 231 buys the first end-to-end
proof that the policy people actually watch can play the game.

**What would make it a replacement** is the full-route intent list — all eight gyms and the whole of
Kanto, mirroring `complete_game_steps` — and that is now a bounded job rather than a gamble, because
the machinery is proven over a whole game. §4.1 of the first draft is still right that
`PolicyStep::complete_game_steps()` has variants with **no menu row behind them** — `UseBagItem`,
`Fish`, `UsePcBox`, `UseItemsInBattle` — and that **where a step does not map, that is the finding**.

#### What building it found

Four faults, none of them in the product and all of them in the harness or in this file's own
assumptions — which is itself the useful result, because it says the deployed surface carried
everything the run needed.

| What went wrong | Root cause |
|---|---|
| The run panicked with *"Invalid Pokemon species"* the instant it took Squirtle out of Oak's ball | `LlmRun::map()` unwraps `game_state()`, and over a whole playthrough that **will** be called on a tick with no readable state. `map_if_readable()` is the seam a long-running predicate uses. |
| It ping-ponged between VictoryRoad2F and 3F for the rest of its budget | `Intent::Row(kind)` and `Intent::Enter(map)` both take whichever row sorts first, and Victory Road's top two floors are joined by **four** ladder pairs landing in four different pockets, with two `PushBoulderOntoSwitch` rows on one floor. ⭐ The rendered menu already distinguishes them — a goal names its target square, a warp names its landing and its side of the map — so `Says` expresses it and **a model has enough to choose correctly**. |
| It gave up on the first turn a row was missing | Rows come and go: a person stands on a doormat, a route is lost to a wandering pet for `MAX_ROUTE_LOST_TICKS`, a sprite table is incomplete for a couple of dozen ticks after a warp. The agent is patient about all three; `ScriptedBrain::PATIENCE` is 20 turns. |
| ⭐ It passed, then failed the next run with the boulder still on the floor | **`Says` confused *asking* with *arriving*.** VictoryRoad3F's hole goal was interrupted by wild encounters **six** times, `resume_after_battle` gives up after five, and the intent had advanced on the turn it was *chosen* — so the list moved on and then waited for a row the cartridge had no reason to mint. `Intent::Repeat` re-issues until the row stops being offered, which for a boulder goal is exactly "finished". |

⚠️ **The one thing that looked like a product fault and was not.** `enter_at(VictoryRoad2F, 27, 7)`
names a landing no VR3F row offered, which is the shape of §3's forbidden private seam — the scripted
route reaching a warp the model is never shown. It is not: `enter_map_action` filters the same
`actions()` list the menu is built from, and a probe from the leg's own fixture found
`VictoryRoad2F:9,16:PushBoulderOntoSwitch` and the (26, 8) ladder both present from the right
pocket. The difference was which pocket, and `PolicyStep::enter` picks the **nearest** matching row
by route length where `Intent::Enter` takes the first in menu order. ⭐ **That is the one place the
two are not one for one**, and it is a fact about the harness rather than about the prompt: the menu
carries no step counts, and it does not need to, because it carries the landings.

**Fold in, because both are one-liners next to the above:**

- **The soak tier's decision** (the old step 9, never taken). Halve `soak::SOAK_GAME_TIME` — one
  constant, 40 minutes today — then compare what the soak finds that the sweep does not. Its only
  remaining value is state-space randomness, so if that column is empty, drop the tier.
- **The battle cells proved under `DeterministicPolicy` only** (the first draft's §6.0 audit, at
  `7343616`). What they prove is that the *agent* can carry the action out, not what a model is
  offered in each. ⭐ The god run now fights its way to the credits through `LlmPolicy` and takes
  none of them, because its battle script decides every turn — so that re-audit is still owed and
  the god run does **not** discharge it.

---

## 5. Parked, and why

Nothing. The god run was the entry here and is now §4's step 2. The heading stays because §7.3 and
code comments cite it by number.

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

# Every region in one process, serially, with the union printed at the end (10x the wall clock).
# ⭐ This is the only form that does the three union-wide things: the maps no walk entered (the diff
# needs `Map::iter()`), the ids offered somewhere and chosen nowhere, and the kind cross-check —
# which is the only one of the three that is an *assertion*, and only on a coverage budget
# (`COVERAGE_BUDGET_MINUTES`; a shorter run prints it and says it was not asserted).
GB_COVERAGE_START=all GB_COVERAGE_MINUTES=360 GB_COVERAGE_PATIENCE=100000 \
  cargo test --release --features coverage-tests --bin gb -- coverage_walk --nocapture
```

**For a measurement, run the ten in parallel** — about 14 minutes for the set on an idle box, and
they agree with each other. ⚠️ **It was 7 before step 1.1**: a Victory Road Strength floor now runs
to the end rather than being abandoned, and the two walks that go through one early emulate at
**26×** against the other eight's 53–58× (§6.2), so they are the last out by a factor of two. Build once, then launch the test binary ten times, one directory each:

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

⚠️ **Two shell traps around it.** `pgrep -f coverage_walk` matches the waiting shell's own command
line and never exits, so wait on `[ ! -d /proc/<pid> ]`; and two launchers that both `rm -rf $S`
give eighteen walks in nine directories, so count exactly ten `coverage_walk` processes before
believing a number.

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

- **A walk has several modes rather than a spread, and it used to be the *terminus* that varied.**
  `phase0` stopped on the Hall of Fame at 38 maps, spent its budget in Victory Road at 30 or 61, and
  — once the bag was fixed — walked 123 and 143; `fuchsia` came back with 163, then 71, then 147,
  the 71 being the run that won the game at 55% of its budget. ⭐ **Step 1.3 removed that mode**: a
  walk that reaches the Hall of Fame is now rewound to the Indigo Plateau lobby and spends the rest
  of its budget, so `phase0` went 38 → 136 and `cinnabar` 41 → 131. What is left of the spread is
  ordinary: where a walk's budget happened to go. ⚠️ **Victory Road is the big one, and it costs
  *rate* rather than only budget** — its Strength floors are legitimately expensive now that a goal
  runs to the end rather than being abandoned. The 2026-09-10 sweep measured it exactly: eight walks
  emulated at **53–58×** realtime and the two that solved those floors at **26×**, finishing on 131
  and 105 maps where their siblings reached 135. `solve_boulder_push` and `route_to_push_tile` are
  per-tick costs on a floor with four boulders, so a walk inside one runs at half speed. ⚠️ **A
  `NOT MOVING` heartbeat on `VictoryRoad3F` is that, and it is healthy**: a boulder goal is **one
  turn**, so the turns/min counter cannot move while one is being solved.
- ⚠️ **A start that cannot win is worth keeping for what it alone reaches**: `ssanne` and
  `ssanneship` are cut before the S.S. Anne sails and are the only two that can enter its ten rooms
  (§7.2's item 16).
- **Load is an input.** Three runs beside a `cargo build -j24` came out 30/30/38; ten on an idle
  box agree with each other. Never build while measuring, and prefer the ten in parallel to one.
- **Which id fails is not reproducible by re-running**; the dropped save state is. Argue from the
  state and from `probe_button_at_state` (§6.1), as
  [deployed-run-defects](deployed-run-defects.md) does. ⚠️ **A `silent` drops no state**, so its
  evidence is the walk's own log: `grep -n` the id and read the agent-state lines after it. Every
  driver prints its state every tick, so the trail names which one was holding the walk when it went
  quiet — `move→X`, then `surf`, then `BattleStarted` with no abort between is the whole diagnosis.
- **The union is the number for the goal**, and it is only meaningful across several starts: eight
  identical `phase0` walks union to 86, ten different starts to 215 of 220 (2026-09-10, after step
  1; 189–215 before it). ⚠️ **And the complement is the
  number to act on** — a count says nothing about whether a gate is doing its job or a walk never
  arrived, which is what the `unreached` line and the ROM cross-check are for.

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
| Between one and nine key items refused, on all eight starts, for a whole month of sweeps | Gen 1's bag is twenty *kinds* and a finished save arrives with fourteen to twenty used; `Cheats` reported the refusal and carried on, so no S.S. Ticket, no Lift Key, no Coin Case and no rod were a *silent* coverage gap. Room is made first now | `cheats::every_coverage_start_can_be_handed_all_of_the_key_items`; `phase0`'s map count going 38→123 |
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
| **The outcome of every bag item used in a battle, never reported** (step 7, not a sweep) | `BattleState::UsingItem` advanced the game's own text with a bare `press_button(A)`, so `ItemUseBall`'s five sentences, `ThrowBallAtTrainerMon`'s two and `ItemUseNotTime`'s refusal were all dismissed unread. A model that threw a ball was told only what the *enemy* then did. The read is separated from the press (`PokemonTextReader::accumulate`, timing-neutral) and the reader is carried out of the state, refusal net included | `battle_refusals::a_poke_ball_that_fails_hands_the_battle_back_rather_than_ending_it`, `a_ball_thrown_at_a_trainer_is_blocked_and_the_ball_is_spent_saying_so` |
| **Every battle refusal claimed the id belonged to another map** (step 7) | `not_on_the_menu`'s map clause tested "the menu's first id contains a colon", and `fight:Peck` / `item:PokeBall` / `switch:1` all do — so an item the bag had run out of was answered with three false statements about maps. Guarded on the name being a real `Map`, which also stops a made-up id being reported as another map's | `tools::a_refused_battle_id_carries_the_rule_and_says_nothing_about_maps`, `battle_refusals::an_item_the_bag_has_run_out_of_leaves_the_menu_and_is_refused_by_name` |
| **A refused battle id was never told which rule kept it off the menu** (step 7) | The turn's own `### On screen` line reads `FIGHT Pokémon ITEM RUN` and the system prompt forbids prior knowledge of Red, so "that id is not one of this turn's actions" is a contradiction with no way out — the shape behind the ViridianGym and Route 22 issue reports. `tools::battle_rule_behind` adds the cartridge's rule where the menu can settle it, and says nothing where it cannot | same test |
| **Route 17 and `Route16Gate2F`**, ditto — and a hole in the deployed tool surface behind them | `use_field_move`'s `use_item` required a `target` tile and the Bicycle has none, so `FieldMove::UseBagItem` and the whole of `UseTarget::Nothing` had a driver, a refusal table and a test that rides a bike, with no way in from any LLM turn. With it went every out-of-battle Potion, vitamin, Repel and Itemfinder. +189 bytes of catalogue | `tools::a_bag_item_with_nothing_to_aim_at_is_a_call_that_can_be_made` |
| **An in-game trade handed over whatever the cursor was left on** (step 8, not a sweep) | Every party menu a *conversation* opens — a trade, the Day Care, the Name Rater — calls `DisplayPartyMenu` without resetting `wCurrentMenuItem`, so the agent's A-mash acted on an arbitrary party member: the give-species in slot 0 traded and the same Pokémon one slot back did not. A trade has **one** legal row and the cartridge says which (`wInGameTradeGiveMonSpecies`), so `PartyMenuAnswer` navigates to it — no policy callback, because a list of one is not a decision. The other two have no right answer and are declined | `branch_points::a_trade_finds_the_give_species_wherever_it_is_in_the_party`, `a_trade_with_nothing_to_give_backs_out_and_says_so`, `postgame::trades::every_in_game_trade_can_be_made_by_talking_to_the_trader` |
| **…and whether that menu was confirmed at all was a matter of timing** (step 8) | Same root, and the worse half: with the driver removed the identical menu in the identical state was *confirmed* at the Cerulean trader and *bounced* at the Day Care, because `MENU_HANDOVER_TICKS` is a window after a box opens and a conversation may reach its party list outside it. Boarding a Pokémon at the Day Care is irreversible and can cost a run its only Cut carrier | `postgame::gifts::talking_to_the_day_care_does_not_board_a_pokemon_nobody_chose` |
| **The dojo's second Poké Ball stays a row** (step 8) | Not a fault, and the *test* was the thing that was wrong: `FightingDojoHitmonchanPokeBallText` `HideObject`s only the ball that was taken, where Mt Moon's Super Nerd hides the fossil you leave. The other object stands there and answers "Better not get greedy...", so the menu is right and what has to hold is that the model is told | `branch_points::the_dojo_branch_can_be_taken_to_hitmonlee`, `…_to_hitmonchan` |
| **The bike shop without a voucher is not a mart** (step 8) | `BikeShopClerkText`'s `.dontHaveVoucher` branch is a hand-rolled `TextBoxBorder` + `HandleMenuInput` list rather than `DisplayPokemartDialogue`, so no `buy_item` turn is ever put to the model — it reads as one text box ending in "Sorry! You can't afford it!". ⭐ ¥1,000,000 is above Gen 1's ¥999,999 money cap, so the A-mash inside it can never buy anything on any save | `branch_points::the_bike_branch_can_be_taken_without_the_voucher` |

| **A boulder goal abandoned three shoves from the end, five times, on one floor** (step 1.1) | Not the boulder and not either of its bounds. `DRIVER_ESCAPE_SILENCE` was measured against `cycles_since_poll` — the clock only a **decision point** resets — and a `BoulderGoal` is one decision that walks a boulder across a whole floor. Past 60 s of game time with no wild battle to poll the policy, every entry into `PushingBoulder` was escaped on its **first tick**, and three of those is `MAX_SILENT_SHOVES`. The `fuchsia` walk solved VictoryRoad3F's switch twice from the same floor in about 30 shoves each and lost the third at 34. `cycles_since_driver_answer` is a second clock that a landed shove also resets | `endgame::a_boulder_goal_that_keeps_shoving_is_not_a_driver_the_game_has_gone_quiet_on` |
| **Every map count this file ever printed was four too pessimistic** (step 1.2) | `unreached_report` set aside the 22 `UnusedMap*` and the two link-cable rooms and not the four `*Copy` duplicates, which **no warp in any map targets**. The denominator is 220 and it is printed beside the count now | `coverage::UNREACHABLE_DUPLICATES`, and the report's own `N of 220` |
| **Half of three walks spent playing the game rather than exploring it** (step 1.3) | `phase0`, `fuchsia` and `cinnabar` reached the Hall of Fame at 42-60% of their budget and stopped, because there is no world left to walk after the cartridge soft-resets. The terminus is still *reported* — §3's rule — and the driver now rewinds the emulator to a checkpoint taken at the Indigo Plateau lobby and spends the rest of the budget walking. The Lorelei door is barred afterwards, because an exit is re-takeable by least-taken count and that door is the best-scoring row in the room | `LlmRun::restart_from_last_checkpoint`, `ExploringBrain::carry_on_after_the_credits`; `phase0` going 38 maps to 136 |
| **Five maps no sweep had ever entered, and none of them was withheld by the agent** (step 1.4) | The step assumed a shut door and every one was a door the *frontier* ranked last. The sharpest is the Safari Zone's west area: it is two shelves one-way ledges seal off from each other, `SafariZoneNorth` has **four** doors into it landing on opposite sides, and all four are rows — but the frontier counts a way out **per crossing**, so the moment either pair is taken the crossing is "done" and the other pair waits behind every exit on a very large map. Twenty walks took the western pair twenty times, which is why `SafariZoneWestRestHouse` had never been in a union | `postgame::safari::the_safari_wests_rest_house_is_a_row_from_the_shelf_it_is_on`, off `safari-west-shelf.bin` |
| **`Pc`, `Statue` and `CellSeparator` had never been offered once in a ten-walk sweep, and nothing could see it** (step 1.5) | `rom_cross_check` covers warps, objects and connections and nothing else, so eight of the kinds `MetaTile::id_kind` can return had no "was this ever offered anywhere" check at all. `kind_cross_check` is a `match` on `MetaTile` — a new variant is a compile error — and it **fails the tier** rather than printing, with an argued allow-list of two | `coverage::kind_cross_check` |

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
   now fits in every start's bag, and they did not move. That is what `Start::before_the_credits` is
   for, and it is the only exception to the finished-game rule admitted so far.
17. **A "starting over" with nothing counting it is a livelock with a friendly sentence.**
   `DRIVER_ESCAPE_SILENCE` was written as the net under every self-driving driver and it is one —
   except where the state it drops to *restores* the thing that jammed. A boulder goal does exactly
   that, and neither of its two bounds could see a shove that never landed, so the net became a 60 s
   loop that ran for two thirds of a region's budget. **Every hatch needs to ask what picks the work
   back up.**
18. **…and it needs to ask what its own clock measures.** The same hatch was read against
   `cycles_since_poll`, on the reading that "nothing is happening" and "the policy has not been asked
   anything" are the same fact. They are for every driver that is a conversation with one menu, and
   they are not for the one driver that is carried out dozens of times inside a *single* decision. A
   boulder goal that ran past 60 s was then escaped on the first tick of every shove, and the
   sentence it printed said the game had gone quiet about a game that had just moved a boulder
   fourteen tiles. ⚠️ **The dropped state does not reproduce it**, because a restored agent starts
   the clock at zero — the one case §6.2's third rule does not cover, and the walk's own log is what
   settles it.
19. **A map nothing ever reached is not evidence that anything is shut.** Step 1.4 was written as
   five gates to open and every one of them turned out to be a door the *frontier* ranked last —
   including `SafariZoneWest`'s rest house, whose door is a row from one of the two shelves the area
   is cut into and not from the other. §7.2's item 16 said a cluster can be gated by the fixture
   rather than by the game; it can also be gated by the walk's own exit ordering, and **a count of
   maps cannot tell any of the three apart**. The `unreached` column can, which is why step 1.6 is
   the one that stayed.

### 7.3 Where the old section numbers went

Code comments cite the 2026-09-06 draft (git `7343616`). What each pointed at, and where the
argument lives now:

| Old | Was | Now |
|---|---|---|
| §1.1, §1.2 | the play-path line; no event-flag writes | §3; `postgame/debug.rs`, `coverage::Start` |
| §2, §2.3 | the harness and `LlmRun`'s four seams | `llm_harness.rs` module comment |
| §2.2.1 | the 402 death loop, in full | `worker::TurnOpen`, `Worker::drop_unanswered`, [llm-turn-loop](llm-turn-loop.md) |
| §3, §3.1–§3.3 | the cheat tier and the god party | `cheats.rs`, `postgame/debug.rs` |
| §4, §4.2 | the god run and whether it replaces `full_playthrough` | step 2; `godmode.rs` |
| §5, §5.1, §5.2 | the frontier and the oracle | `coverage.rs` module and `ExploringBrain` comments |
| §5.2.2–§5.2.6, §5.2.8 | the sweeps' findings | §7.1 |
| §5.2.7 W2 | one walk reaches 38 maps | §2.1, §7.4 row 6; `coverage::Start` |
| §5.2.9 | the 42 defects and Route 16 | §7.4 rows 2–4, §7.1 |
| §5.3 | the ROM cross-check | `coverage::rom_cross_check` |
| §5.4 | branch points; the state must be dropped at the defect | §7.4 row 8, `branch_points.rs`; `TestFixture::observe_coverage` |
| §5.5 | acceptance and the variance | §6.2 |
| §6, §6.0 | the battle matrix audit | §7.4 row 7, `battle_refusals.rs`; the cells still under `DeterministicPolicy` only are step 2's fold-in |
| §7 | the soak decision | step 2's fold-in |

### 7.4 The ten steps that got here

Kept as one line each, because code comments name them by number and because the shape of the work
is the useful part. Steps 0-8 were taken on 2026-09-09 and 2026-09-10 and are numbered from the
2026-09-09 draft; step 1 is §4's, and the two numbering schemes overlap because code comments already
name the old ones. The arguments are in §7.1 and in the comments those rows point at.

| Step | What it did |
|---|---|
| 0 | The baseline: eight walks, **153 maps, 106 defects, 41 silences**, 82% of every turn in Route 16's gate |
| 1 | Made a silence fail the tier, and fixed the fishing walk that mounted Surf on the turn that faces the water |
| 2 | The Safari Zone pond and the two Seafoam warps a script cancels |
| 3 | Saffron Gym's teleport pads: 99 defects, and the intra-map warp that never reported arriving |
| 4 | Route 16's gate: a cut tree grows back on a map reload and a once-only frontier will not cut it twice |
| 5 | Printed the maps never entered, which is what made steps 6 and 8 possible |
| 6 | **The loop, to its fixpoint** — two clean sweeps whose union did not grow. Nine defects and six silences over two turns; the two biggest were a bag with no room for the key items, and one crossing per adjacent map when a bridge and a river seam lead to the same place |
| 7 | The battle refusals: seven cells that existed nowhere, three defects, the largest being that **no bag item used in a battle ever reported its outcome** |
| 8 | The branch points: content that is exclusive per save, and the trade that **handed over whatever the party-menu cursor was left on** |
| 1 | **Nothing unreached, and nothing unexplained** — 199 maps to **215 of 220**, one defect to none, and three walks that used to stop on the credits now spending their whole budget. The two largest were a driver-escape hatch reading the *policy's* clock while a boulder goal ran for minutes, and a terminus that ended the walk rather than being rewound past |
