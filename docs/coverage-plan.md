# Full exploration through the LLM path

**The goal, in one sentence.** Every action the game offers, taken once, through the stack exactly
as it is deployed — `LlmPolicy`, the worker and the wire, against a mock endpoint in-process — with
a verdict on each, and every defect that turns up fixed, until a sweep of the whole of Kanto comes
back clean and finds nothing new.

**Status.** The harness, the cheats, the oracle and the frontier walk are built, and the sweep has
been to a fixpoint: two consecutive clean sweeps on 2026-09-10 whose union did not grow. The nine
steps that got there are one line each in §7.4 and their findings are §7.1's rows, every one with a
test. ⚠️ **That is not the goal, and this file said so too quietly.** A clean sweep means *nothing
the walk asserts on went wrong*, and the walk asserts on two things — defects and silences. It does
not assert that every map was entered, that every action offered was taken, or that the ROM's own
tables were exhausted; all three are printed and were going unread. §2.1 is what happened when they
were read, and it is the state to measure against.

**What is left is §4's two steps.** Step 1 closes the gap between "clean" and "complete": one
defect outstanding, 21 reachable maps missed by the last sweep, a cross-check that covers three
action kinds of nine, and 2 124 ids offered and never chosen — ⭐ the largest single cause being that
**three of the ten walks win the game and stop** at 42-60% of their budget. Step 2 is the god run —
Pallet Town to the Hall of Fame through `LlmPolicy` — unparked, to replace `full_playthrough` if it
is not much slower.

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
| The god run's machinery | `integration_tests/godmode.rs` | `Intent`, `ScriptedBrain`, `godmode_turn_cost`. The machinery is default tier and drives step 8's branch arms; the **run** is §4's step 2 |
| The branch points | `integration_tests/branch_points.rs` | Four snapshots cut one decision before a choice that is **exclusive per save**, nine arms after them and three trade tests, driven by `Intent` against the rendered menu. Step 8 |

## 2. Where we are

The 2026-09-09 baseline that used to be tabled here — eight walks, 153 maps, 106 defects, 41
silences, 82% of every turn in Route 16's gate — is §7.4's row 0, and every bullet under it was
closed by a step in that table. ⚠️ **Take every baseline from a run, never from a table in this
file.** The walk is not deterministic and its spread is not noise (§6.2).

### 2.1 Where the sweep stands — 2026-09-10, after step 8

Ten walks in parallel on an idle machine, 6 game-hours each, about 7 minutes for the set (§6.1).
Every number here was re-derived from the walk logs and `walk-*.tsv` files the §6.1 recipe leaves
behind, and the map list is the diff of `Map::iter()` against their union.

| **start** | **maps** | **ids** | **defects** | **silent** |
|---|---|---|---|---|
| `phase0` | 38 ⭐ Hall of Fame at 45% of budget | 365 | 0 | 0 |
| `cerulean` | 120 | 1 039 | 0 | 0 |
| `vermilion` | 135 | 1 267 | 0 | 0 |
| `lavender` | 135 | 1 249 | 0 | 0 |
| `celadon` | 134 | 1 287 | 0 | 0 |
| `saffron` | 134 | 1 265 | 0 | 0 |
| `fuchsia` | 70 ⭐ Hall of Fame at 60% | 663 | ⛔ **1** | 0 |
| `cinnabar` | 41 ⭐ Hall of Fame at 42% | 412 | 0 | 0 |
| `ssanne` | 119 | 1 036 | 0 | 0 |
| `ssanneship` | 82 | 751 | 0 | 0 |
| **union** | **199 of 220 reachable** | **1 973** | **1** | **0** |

⚠️ **Read the pair, never the row** (§6.2). This is one sweep, not a pair: 199 sits inside the
189–215 band the 2026-09-10 fixpoint pair measured, so it is not a regression. What it *is* good for
is the list below, which is stable across all ten walks.

**The arithmetic, which was wrong in this file until now.** `Map::iter()` yields **248** because the
enum is the map *byte*: 22 `UnusedMap*` have no header at all and 2 are link-cable rooms, so the game
has **224 real maps**. ⚠️ **And four of those are unreachable and were counted as real**:
`CeruleanTrashedHouseCopy`, `CinnabarMartCopy`, `UndergroundPathRoute6Copy` and
`UndergroundPathRoute7Copy` are duplicate headers that **no warp from any map targets** (checked
against `pokered/data/maps/objects/`), so the denominator is **220** and `unreached_report` has been
over-reporting by four every sweep it has ever printed.

**The 21 reachable maps this sweep missed, in two groups.** (This paragraph once said twelve, four
and nine, and named sixteen; a count that does not add up to the list under it is how a plan is
wrong.)

- ⭐ **Sixteen are ordinary maps that three walks simply stopped short of** — the Pallet cluster
  (`RedsHouse1F`, `RedsHouse2F`, `OaksLab`, `BluesHouse`, `ViridianMart`), the Cinnabar cluster
  (`CinnabarGym`, `CinnabarLab` and its three rooms, `CinnabarMart`, `CinnabarPokecenter`) and the
  four Pokémon Mansion floors. Every one has been in a union before. **The cause is in the table
  above: `phase0`, `fuchsia` and `cinnabar` all `settled` by reaching the Hall of Fame**, at 45%,
  60% and 42% of their game-time budget, and then stopped with the rest unwalked. Half of three
  walks is being spent playing the game to the credits rather than exploring it.
- **Five were listed as structural, and four are:** `CeruleanCaveB1F` (the ladder chain below),
  `Route17` (the Route 16 gate's guard stops a walker without the Bicycle at column 4 of the south
  corridor, and no row means *ride the Bicycle*), and `SafariZoneWestRestHouse` and
  `SafariZoneNorthRestHouse` (one door each). The fifth, `Route16Gate2F`, **is not**: the stairs at
  (6, 12) are reached from the gate's Celadon-side south door without crossing the guard's column —
  the gate's collision map says so — and `Route16:24,10:Warp`, that door, was **offered and never
  chosen in five walks**. It is one of the 2 124 below, and it sat here as bike-gated until the ROM
  and the TSVs were read.

⛔ **One defect, and it is a boulder.** `VictoryRoad3F:3,5:PushBoulderOntoSwitch`, hit **5 times** on
the `fuchsia` walk: *"the boulder was pushed 34 times without reaching its target, so it was given
up"*. The state it happened in is committed as **`src/pokemon/data/vr3f-boulder-given-up.bin`**. This
is the third boulder finding in §7.1 and the second about a *bound* — `MAX_PUSHES_WITHOUT_PROGRESS`
counts off a boulder that moved, and `MAX_SILENT_SHOVES` off one the cartridge refused, so a solver
that is *making* progress toward the wrong target is bounded by neither.

⚠️ **The ROM cross-check is printed and never read, and it is saying things.** Across the ten walks:

- ⭐ **`(6, 1) → CeladonMansion3F` under `CeladonMansion2F`, and `(6, 1) → CeladonMansion2F` under
  `CeladonMansion3F`, both "on the grid, no sibling, and never a row", in five walks of ten.** One
  line each per walk — which this file first counted as ten walks of ten. ⚠️ **And it is not
  shut:** in two other walks `CeladonMansion2F:6,1:Warp`, `CeladonMansion3F:6,1:Warp` and
  `CeladonMansionRoof:6,1:Warp` all `completed`, while the other door between the same two floors
  (`4,1`) was the row taken in the remaining seven. So the sentence that turned out to be Cerulean
  Cave is describing here a door the frontier reaches only sometimes, and the first question is why
  the cross-check calls a door with a same-destination neighbour "no sibling". ⚠️ **Not a
  diagnosis** — every warp finding in this file that was argued from the ROM was wrong; it needs
  `probe_button_at_state` on a dropped state first (§7.1's ⚰️).
- **41 people never a row** on the `vermilion` walk alone, and **59 of 499 warps**, of which 25 are a
  sibling door and 4 are not on the grid. Nobody has audited the remainder.
- ⭐ **2 124 ids were *offered and never chosen* across the sweep** (`unreached`), against 6 984
  completed and 225 blocked. By kind: **Warp 960, Grass 702, Connection 269, ConnectionWater 76,
  Fish 53, Sprite 16**, and the rest toggleable objects (trash cans, vending machines). That is the
  real shape of "every action taken once", and no number in this file tracked it until now. The shape
  also says what it is: exits and grass on maps the walk left through another exit before it had
  finished them — not people who pace, since a sprite id has carried no coordinate since §7.1's fix.

**The ladder chain, still shut.** `CeruleanCave1F (0, 6) → B1F`, `1F (3, 11) → 2F` and
`2F (1, 3) → 1F` all print *"on the grid, no sibling, and never a row"*.
`probe_route_to_cerulean_cave` has the cause written down: the strip in front of the B1F ladder is
raw tile 32 and the room below is tile 5, and `(32, 5)` is in the Cavern tileset's
`TilePairCollisions`, so the floor is entered by going *up* at 1F (3, 11) and back down at 2F (1, 3).
All three rungs are unmintable, so the route exists and cannot be asked for.

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

**Two, and they are what is left of this plan.** Steps **0 to 8** of the 2026-09-09 draft are §7.4,
one line each, kept because code comments still name them by number; their arguments are §7.1 and
the comments those rows point at. Step 9 was never taken and is folded into step 2 below.

Each step below says what "done" is as something a fresh agent can check by running one command.
⚠️ **§3's rules hold for both**, and the two that bite hardest here are *fix the driver, not the
walk* and *argue a defect from the dropped save state, never from the sentence the agent printed*.

---

### Step 1 — Nothing unreached, and nothing unexplained

**The goal in one sentence: one sweep in which every real map is entered, every action offered is
either taken or explained, and the cross-check that says so is an assertion rather than a
paragraph.** Today the sweep is clean on the two things it asserts (defects, silences) and quiet on
everything it merely prints, which is where all six items below were hiding. §2.1 is the evidence for
each; none of this is speculative.

**1.1 ⛔ Fix the boulder defect.** `VictoryRoad3F:3,5:PushBoulderOntoSwitch` gave up after 34 pushes,
five times, on the `fuchsia` walk. The state is committed: `src/pokemon/data/vr3f-boulder-given-up.bin`.
Neither existing bound catches it — `MAX_PUSHES_WITHOUT_PROGRESS` counts off a boulder that *moved*
and `MAX_SILENT_SHOVES` off one the cartridge refused, and this solver is moving a boulder that is
not getting closer. ⚠️ Start from the state, not from the sentence. **Done:** a default-tier test
off that fixture, and the row `completed` on a sweep.

**1.2 Make `unreached_report` honest.** It sets aside `UnusedMap*` and the two link-cable rooms but
not the four `*Copy` duplicates, which no warp targets, so every map count this file has ever printed
was four too pessimistic. **Done:** the report's denominator is 220, and the four are counted in
their own bucket with the reason.

**1.3 ⭐ Stop three walks spending half their budget on the credits.** `phase0`, `fuchsia` and
`cinnabar` all `settled` by winning the game at 42-60% of budget and then stopped, which is where
sixteen of §2.1's twenty-one missing maps went. ⚠️ **§3's rule is that a terminus is *reported*,
never made unreachable** — that rule came from filtering the Hall of Fame out of the menu and leaving
the walk at the title screen for 20 000 turns, and it is not this. Report it and then carry on from a
checkpoint taken before the ceremony, so the remaining budget is spent walking. The pieces exist:
`LlmRun::checkpoint` writes `state.gbst` and `LlmRun::restart` tears down and resumes from it, so
the driver checkpoints when the brain first reports `IndigoPlateauLobby` and restarts on
`reached_the_end`. Nothing that matters is disturbed by the restore — the brain is a string parser
whose frontier lives in its own memory, and the conversation the restart resumes is read by nobody
(a ranking of the Elite Four door *last* was considered instead, and rejected: it keeps the Elite
Four rooms and the Hall of Fame out of the union unless the walk happens to go barren). ⚠️ **Two
things it must do as well.** The door into Lorelei's room is an exit, and exits are re-takeable by
least-taken count, so after the restore it must be marked taken for good or the walk plays the
gauntlet a second time. And if `total_cycles` comes back with the state, the restore hands back the
game time the gauntlet spent; either carry the spent time across the restart or accept a few per
cent of extra budget on those three walks and print that it happened. **Done:** no walk reports
`settled` on the Hall of Fame with more than 20% of its budget unspent, and the Elite Four rooms are
still in the union.

**1.4 Close the maps that are genuinely shut.** Four of §2.1's five, and three tools: the
**Cerulean Cave ladder chain** (a routing fault, cause already written down in
`probe_route_to_cerulean_cave` — all three rungs are unmintable because of the Cavern tileset's
`TilePairCollisions`); **Route 17** (there is no row meaning *ride the Bicycle*, so it is a
`MetaTile` for the gate's guard rather than a tool — `use_item`'s target was made optional by §7.1's
bag-item row and that correctly did not move it, because the walk's brain takes rows); and the
**two Safari rest houses**. `Route16Gate2F` is not on this list (§2.1), and the `CeladonMansion`
`(6, 1)` doors are a row in some walks and not others, which makes them 1.6's question until a
dropped state says otherwise. **Done:** a default-tier test per map that its row is minted off a
committed fixture, as the boulder has — because which map a walk reaches is not reproducible by
re-running (§6.2) — and then a sweep whose union is 220 of 220 as the confirmation rather than the
proof.

**1.5 ⭐ Widen the cross-check to every action kind, then assert it.** `rom_cross_check` compares the
ROM's tables against what was offered for **warps, objects and connections** and for nothing else —
so `Fish`, `Grass`, `CutTree`, `PushBoulder*`, `Switch`, `Pc`, `Water` and `ConnectionWater` have no
"was this ever offered anywhere" check at all. Fishing was **35 of the 41** silences at the 2026-09-09
baseline, so this is not a hypothetical gap. Every kind is enumerable: `MetaTile::id_kind` is the
list. **Done:** the report covers every kind `id_kind` can return, and it **fails the tier** rather
than printing — asserted over the union of a sweep, or per kind as "offered at least once
anywhere", never per map per walk, because which maps one walk enters is a coin flip (§6.2) — with
an allow-list of things argued to be expected (a toggleable item ball already in
the bag, a boulder that is offered as a goal instead).

**1.6 Drive `unreached` to zero or to a reason.** 2 124 ids were offered and never chosen against
6 984 completed, and §2.1 has the breakdown by kind: exits and grass, on maps the walk left before it
had finished them. It is the literal reading of this plan's goal sentence. ⚠️ It will not go to zero
by walking longer — a frontier that leaves a map by its least-taken exit leaves that map's other
exits behind, and `Grass` rows are deliberately not re-takeable — so the deliverable is the *ratio*
plus a named reason for each family that stays. The breakdown is one `awk` over the `walk-*.tsv`
files (§6.1) rather than a feature. **Done:** the sweep prints `unreached` broken down by kind, and
every family above 50 ids has a line in this file saying why.

**Cost:** the largest step left. 1.1 is a fixture and a test, 1.2 is arithmetic, 1.3 is a checkpoint
and a restart in the driver plus one exclusion in the brain; 1.5 is the one that changes what the
tier means, and 1.6 is reading rather than building.

---

### Step 2 — The god run, and the tier it replaces

⭐ **Unparked deliberately** (it was §5's first entry, "Alex's call"): a fresh save at Pallet Town
played to the **Hall of Fame** through `LlmPolicy`, the worker and the wire, as the pre-push gate
instead of `full_playthrough`. The machinery is built and is no longer speculative — `Intent`,
`ScriptedBrain` and `Cheats` are default tier and drive step 8's twelve branch arms — and
`godmode_turn_cost` (`--features godmode`) is the measurement that was supposed to decide it.

**Why it is worth taking now.** `full_playthrough` proves the *scripted route* still works, which is
a `DeterministicPolicy` fact; the thing actually deployed is `LlmPolicy`, and no test plays it end to
end. A god run would cover chaining, `resume_after_battle`, compaction, a restart mid-game and the
whole turn loop across a real game, which is the list the first draft's §4 said was worth having.

⚠️ **The bar is "not much slower", not "much faster", and the difference is the coverage.** The
first draft said a god run had to beat `full_playthrough` outright; that was written when the run
would have bought only a second way to play the same route. It buys more than that — the deployed
policy end to end, chaining, `resume_after_battle`, compaction and a restart mid-game, none of which
any test covers today — so a gate that costs about the same and proves considerably more is a
straight win. `full_playthrough` measured **234 s** and `hall_of_fame` **904 s** on this machine on
2026-09-10; the README and `docs/test-suite.md` still say about 7 and 26 minutes, from an older
build or an older box. ⚠️ The bar is `full_playthrough` measured on the same machine on the same
day, never either figure.

⭐ **But it should be *faster*, and if it is not, that is the finding rather than the answer.** A god
party wins every battle on the first move, and the grind is most of the scripted route's length —
`full_playthrough` spends its time levelling a Squirtle to Blastoise, and a god run skips all of it.
So a run that comes out slower is telling you something, and there are only two places it can be:

- **Turn count.** An `Intent` is one hop per turn by construction (the first draft's §4.1, the
  correspondence with `PolicyStep::enter`), so a route written as forty hops costs forty round trips. Chaining is the
  lever the deployed policy already has — `choose_action`'s `then` takes three more ids — and a god
  run that does not use it is not playing the way a model would either.
- **Per-turn latency.** A round trip to a localhost mock is not the cost; building the prompt over a
  growing history is, and it grows until compaction bounds it. `godmode_turn_cost` already prints
  mean and worst latency beside the turn count, which is exactly the pair that tells these two apart.

**Take the number first**, and if it is bad, take it apart with `godmode_turn_cost` before writing
any more of the route.

**And check what stops being covered before retiring anything.** `full_playthrough` is a golden RNG
replay of a route that *catches Pokémon, teaches HMs, buys, sells, grinds and solves both boulder
floors with a real party*. A god run does none of that: the party is installed, so catching, move
management and the whole "can this party win" question go with it. Most of it is covered elsewhere —
the leg chain, `postgame::*`, `branch_points`, `battle_refusals` — and the deliverable here is **the
table that says which test covers each**, written before the gate changes and not after.

**Fold in, because both are one-liners next to the above:**

- **The soak tier's decision** (the old step 9, never taken). Halve `soak::SOAK_GAME_TIME` — one
  constant, 40 minutes today — then compare what the soak finds that the sweep does not. Its only
  remaining value is state-space randomness, so if that column is empty, drop the tier.
- **The battle cells proved under `DeterministicPolicy` only** (the first draft's §6.0 audit, at
  `7343616`). What they prove is that
  the *agent* can carry the action out, not what a model is offered in each. None is a refusal, which
  is why step 7 left them; a god run that fights its way to the credits through `LlmPolicy` takes
  most of them for free, so re-audit the list *after* step 2 rather than before it.

**Done:** a number for the god run beside `full_playthrough`'s, measured the same day, **and, if it
is the slower of the two, the reason** from the pair above; a table mapping everything `full_playthrough` uniquely
covers to the test that covers it; and then either the gate changes or this file records why it did
not.

**Cost:** medium, and mostly the intent list. §4.1 of the first draft is still right that
`PolicyStep::complete_game_steps()` has variants with **no menu row behind them** — `UseBagItem`,
`Fish`, `UsePcBox`, `UseItemsInBattle` — and that **where a step does not map, that is the finding**.

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
# ⭐ This is the only form that prints `unreached` — the maps no walk entered, which is step 1.4's
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

- **A walk has several modes rather than a spread, and it is the *terminus* that varies.** `phase0`
  has stopped on the Hall of Fame at 38 maps, spent its budget in Victory Road at 30 or 61, and —
  once the bag was fixed — walked 123 and 143. `fuchsia` came back with 163, then 71, then 147; the
  71 is the run that happened to win the game at 55% of its budget. A single row is a sample of
  which happened. ⚠️ **A start that reaches the Indigo Plateau stops exploring**, and that cuts both
  ways: it is why `cinnabar` is 41 maps and why `ssanne`, which cannot win, is worth 17 nobody else
  reaches (§2.1).
- **Load is an input.** Three runs beside a `cargo build -j24` came out 30/30/38; ten on an idle
  box agree with each other. Never build while measuring, and prefer the ten in parallel to one.
- **Which id fails is not reproducible by re-running**; the dropped save state is. Argue from the
  state and from `probe_button_at_state` (§6.1), as
  [deployed-run-defects](deployed-run-defects.md) does. ⚠️ **A `silent` drops no state**, so its
  evidence is the walk's own log: `grep -n` the id and read the agent-state lines after it. Every
  driver prints its state every tick, so the trail names which one was holding the walk when it went
  quiet — `move→X`, then `surf`, then `BattleStarted` with no abort between is the whole diagnosis.
- **The union is the number for the goal**, and it is only meaningful across several starts: eight
  identical `phase0` walks union to 86, ten different starts to 189–215. ⚠️ **And the complement is the
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

### 7.4 The nine steps that got here

Kept as one line each, because code comments name them by number and because the shape of the work
is the useful part. All were taken on 2026-09-09 and 2026-09-10; the arguments are in §7.1 and in the
comments those rows point at.

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
