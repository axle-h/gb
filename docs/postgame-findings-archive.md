# Post-game coverage — raw findings archive

This is the append-only findings log (the old §11) of `docs/postgame-coverage-plan.md`, moved out
verbatim on 2026-08-04 when that document was condensed to a handoff. **Nothing here is a to-do
list** — it is the record of what was actually true while Phase 0 and workstreams A–H were built.

The durable conclusions were lifted into the plan itself (its scope table, per-stream records and
trap list). Come here for the *evidence*: probe dumps, ROM `file:line` citations, and the wrong
assumptions that cost the most. Still append-only — never edit or delete an entry.

---

## The log

**Append here. Never edit or delete an existing entry.** Newest at the bottom.

**Log an entry when you:**

- find an assumption in this document that is **wrong** — the single most useful thing you can record;
- discover a fact that cost you more than ~15 minutes to establish;
- finish a workstream (even if everything went to plan — say so, so nobody re-checks);
- get blocked, including what you tried and what you'd try next;
- change something other workstreams depend on (a shared enum, a fixture, a seam).

**Don't** log routine progress. Ticking a box in §6 is the progress record; §11 is for knowledge.

### Template

```markdown
### [YYYY-MM-DD] <stream> — <one-line headline>
**Status:** verified ✅ / corrected ❗ / blocked 🟡
**What the plan said:** …
**What is actually true:** …
**Evidence:** test name, ROM file:line, or probe output — not "I think".
**Impact on others:** which workstreams or sections this changes, or "none".
```

Prefer **evidence over assertion**. This project has a strong track record of misdiagnosis — the
"HandleMenuInput input-delay wall" was believed for a long time and was simply wrong. A ROM
`file:line` or a probe dump settles an argument; a recollection does not.

### Entries

### [2026-07-29] Planning — baseline established
**Status:** verified ✅
**What the plan said:** n/a — this is the founding entry.
**What is actually true:** `post-hall-of-fame.bin` measures as in §2: all 8 badges, ¥37,774, party 4,
dex **7 owned / 111 seen**, bag **20/20 full**, `wBoxCount=0`, `wCurrentBoxNum=0`, `wPlayerCoins=0`.
96 of 248 maps are unreferenced in `policy.rs`. The trade table in §6-G was read from
`data/events/trades.asm` plus the nine scripts referencing `TRADE_FOR_*`; there are 9 usable trades,
not the ~10 commonly quoted (Butterfree→Beedrill is dead code). The Coin Case comes from
`CeladonDiner`, not the Mart. Pokémon Center PCs are at (13,3) in every centre.
**Evidence:** a temporary `probe_endgame_coverage` fixture test (reverted; re-create it as Phase 0
task 0.1), `data/events/trades.asm`, `engine/menus/players_pc.asm:243`,
`engine/pokemon/bills_pc.asm:341`, `data/events/hidden_objects.asm`, `scripts/CeladonDiner.asm`.
**Impact on others:** everything. The full bag and the empty box are why Phase 0 exists.

<!-- Append new entries below this line. -->

### [2026-07-30] Phase 0 / 0.1 — the probe lands; §2's bag listing is incomplete (5 TMs missing)
**Status:** corrected ❗ (everything else in §2 verified ✅)
**What the plan said:** §2 lists the `post-hall-of-fame.bin` bag as 15 named entries — TownMap, TM34,
HelixFossil, SSTicket, HM01 Cut, LiftKey, SilphScope, PokeFlute, HM03 Surf, HM04 Strength, CardKey,
SecretKey, GreatBall×9, FullRestore×6, Revive×4 — while also saying the bag is 20/20.
**What is actually true:** every other number in §2 reproduces exactly (badges 255, ¥37,774, party 4,
dex **7 owned / 111 seen**, `wBoxCount=0`, `wCurrentBoxNum=0`, `wPlayerCoins=0`). But the bag list is
5 entries short: §2 was written from `GameState::bag`, which walked straight into the §10 trap it
warns about — it silently drops every id `ItemId` can't name. The **five missing slots are all TMs**:

```
bag[20/20]: TownMap, TM34(Bide), HelixFossil, TM11, SSTicket, HM01 Cut, TM24, TM21, LiftKey,
            SilphScope, PokeFlute, TM06, HM03 Surf, HM04 Strength, CardKey, SecretKey,
            GreatBall×9, TM27, FullRestore×6, Revive×4
```

TM06 Toxic, TM11 Bubblebeam, TM21 Mega Drain, TM24 Thunderbolt, TM27 Fissure, TM34 Bide. **This is
good news for 0.9**: a quarter of the bag is TMs nobody has a plan for, so freeing space does not
require depositing anything a workstream needs. Also note `Slowpoke` is in the party and owned — §2's
dex-7 list is Bulbasaur/Ivysaur/Venusaur/Slowpoke/Eevee/Vaporeon/Articuno.

Consequences for the probe itself: it reads the bag from raw `wNumBagItems`/`wBagItems`, **not**
`GameState::bag`, and decodes unnamed machine ids by number (`constants/item_constants.asm`: HM01–05
at `$C4`, TM01–50 at `$C9`) so they print as `TM11` rather than `$d3`. Anything else reading the bag
for occupancy should do the same — the trap is live, not theoretical.
**Evidence:** `pokemon::integration_tests::fixture::probe_coverage` (`#[ignore]`d, run with
`--exact --ignored --nocapture`); output pasted above.
`pokered/constants/item_constants.asm:139-210` for the HM/TM id bases.
**Impact on others:** §2's bag listing — treat the probe output as ground truth, not §2. 0.9 (which
TMs to shed). Anyone counting bag occupancy. The probe's per-fixture body is exposed as
`fixture::print_coverage(name, save_state)` so a workstream can call it on a state it has just driven
to, not only on a committed file; append your `postgame-*.bin` to its `FIXTURES` list when you commit
one.


### [2026-07-30] Phase 0 / 0.35 — `post-hall-of-fame.bin` is a cutscene, not a playable state
**Status:** corrected ❗
**What the plan said:** §4.2 — *"Root every workstream at `post-hall-of-fame.bin`. It has all 8
badges, Surf and Strength in hand, and no remaining main-quest obligations."*
**What is actually true:** it has all of that, but you cannot *drive* from it. The file is saved by
`endgame.rs:125` at `run_until(map == HallOfFame)` — i.e. on **arrival**, before the ceremony. Loading
it and letting it run does nothing: the agent reports `Overworld @ HallOfFame (4,2)` and sits there
for 30 emulated minutes, because pokered's `HallOfFameResetEventsAndSaveScript` ends with
`WaitForTextScrollButtonPress` → **`jp Init`**, a soft reset to the title screen, and the empty policy
never presses anything. The state is parked in that wait loop, not idling in an overworld.

The full sequence, measured by mashing A from the fixture:

```
[    0.0s] Script    HallOfFame     ceremony
[   12.7s] Overworld HallOfFame     Hall-of-Fame animation + credits (mode misreads as Overworld)
[  169.9s] None      $00            jp Init — soft reset, title screen
[  180.0s] Overworld HallOfFame     main menu → CONTINUE → save-info screen (wCurMap from SRAM)
[  182.1s] Overworld PalletTown     playable, at (5,7)
```

Three things worth having:

1. **No title-screen or main-menu driver is needed.** CONTINUE is the first main-menu entry, so a
   blind A-mash carries the whole thing. I had budgeted for writing one; it was unnecessary.
2. **You land in Pallet Town at (5,7), not in your bedroom.** `engine/menus/main_menu.asm:116-125`
   special-cases it: with `wNumHoFTeams != 0` and a saved `wCurMap == HALL_OF_FAME`, CONTINUE sets
   `wDestinationMap = 0`, sets `BIT_FLY_OR_DUNGEON_WARP` and calls `PrepareForSpecialWarp` instead of
   restoring the saved position.
3. **Nothing is lost across the reset** — badges 255, party 4, ¥37,774, dex 7/111 and the bag all come
   back identically (probe output, `postgame-post-credits.bin`). The script does reset the Indigo
   Plateau event range (so the E4 is rechallengeable) and sets `wLastBlackoutMap = PALLET_TOWN`.

**So: root at `postgame-post-credits.bin`, not `post-hall-of-fame.bin`.** It is the state §4.2
*meant*. Cost to produce: ~3 min of game time, 7 s wall clock, via
`postgame::phase0::can_walk_out_of_the_hall_of_fame`. `postgame-phase0.bin` (0.9) will be this plus
freed bag space; until it lands, use this one.

⚠️ One thing to plan around: the party arrives in poor shape — **Venusaur is at 0/228 HP** and most
moves are at 0 PP (Articuno has *no* usable attack). Any workstream that expects to battle must heal
first. A Pokémon Center visit fixes both, and is where the PC is anyway.
**Evidence:** `pokemon::integration_tests::postgame::phase0::can_walk_out_of_the_hall_of_fame`;
`pokered/scripts/HallOfFame.asm:22-56`; `pokered/engine/menus/main_menu.asm:116-125`;
`pokered/engine/movie/credits.asm:1` (`HallOfFamePC` = animate + roll credits);
`probe_coverage` output for both fixtures.
**Impact on others:** **every workstream** — §4.2's "root every workstream at `post-hall-of-fame.bin`"
is wrong as written. §5 gains a task 0.35. Anyone who has already started from
`post-hall-of-fame.bin` and found their agent doing nothing at `HallOfFame (4,2)`: this is why.

### [2026-07-30] Phase 0 / 0.3+0.4 — a PC can only be used from *below*, and the reason is not the table
**Status:** corrected ❗ (0.3's coordinates verified ✅, and expanded)
**What the plan said:** 0.3 — *"Every Pokémon Center's PC is a hidden object at (13,3) facing up — one
constant covers all of them (a few non-centre maps differ, e.g. `RedsHouse2F` at (0,1))."*
**What is actually true:** the coordinates are right, but "facing up" is load-bearing in a way that
cost me most of this task, and the exception list is longer than "a few".

**The trap.** `hidden_object x, y, SPRITE_FACING_UP, …` does **not** restrict the direction you may
interact from. `data/events/hidden_objects.asm:191` says so in as many words —

> *Some hidden objects use SPRITE_FACING_\* values, but these do not actually prevent the player from
> interacting with them in any direction.*

— and `CheckIfCoordsInFrontOfPlayerMatch` confirms it: matching is purely on the tile in front of the
player, whatever way they face. So the third argument is inert **at the table level**. But the
*routines* check it themselves:

```asm
OpenPokemonCenterPC:                        ; and BillsHousePC, identically
	ld a, [wSpritePlayerStateData1FacingDirection]
	cp SPRITE_FACING_UP
	ret nz
```

The failure mode this produces is nasty, because every intermediate signal says success. The agent
walked to the tile *west* of the PC (nearer than the tile below it — `pc_locations`'s old
nearest-adjacent-tile search picked it), faced right, and pressed A. `hCoordsInFrontOfPlayerMatch` =
`$00` (**matched**), `hDidntFindAnyHiddenObject` = `$00` (**found**), the routine was dispatched — and
it `ret nz`'d without drawing anything. Nothing on screen, no error, no abort: the agent stood there
pulsing A for the entire budget. I verified the joypad was toggling cleanly, `wJoyIgnore = 0`,
`wXCoord/wYCoord = (12,3)`, facing `$0c`, and read the hidden-object table straight out of
`pokered.gbc` to rule out a stale submodule, before finding it in the routine.

**The fix** (in `tile_map.rs` §5 of `actions()`): a PC is approached from `(pc.x, pc.y + 1)` facing
**Up**, full stop — no four-way adjacency search. `BillsHouse` worked before only because the other
three sides happen to be walls. `OpenRedsPC` is the one routine with no facing check, but the
stand-below approach works there too, so this is uniform.

**The full PC table** (all 22 objects, 21 maps), read from `data/events/hidden_objects.asm` and
attributed through `HiddenObjectMaps` rather than by label — *the labels in that file are stale*:
`SafariZoneRestHouse2` is `SAFARI_ZONE_WEST_REST_HOUSE` and `CinnabarLab4` is
`CINNABAR_LAB_FOSSIL_ROOM`, so reading the label alone will mis-attribute them.

| Where | PC at |
|---|---|
| all 11 Pokémon Centers, `CeladonHotel`, `SafariZone{West,East,North}RestHouse` | **(13,3)** |
| `BillsHouse` | (1,4) |
| `RedsHouse2F` | (0,1) |
| `CeladonMansion2F` | (0,5) |
| `IndigoPlateauLobby` | (15,7) |
| `CinnabarLabFossilRoom` | (0,4) **and** (2,4) — the only map with two |
| `SilphCo11F` | (10,12) |

`SafariZoneCenterRestHouse` is the one rest house *without* a PC. Unit tests
`tile_map::pc_location_tests::*` pin all of this.

**Free result for workstream A.** Once the PC opens the agent's generic text-advance keeps mashing A,
walks into the first entry, and shows the next menu down. The post-Champion parent menu is five
entries — `BILL's PC · <PLAYER>'s PC · PROF.OAK's PC · <PKMN>LEAGUE · LOG OFF` — so **PLAYER's PC is
index 1** (0.5/0.6) and **BILL's PC is index 0** (A2). A2's warning is correct and now has numbers
behind it: `DisplayPCMainMenu` adds PROF.OAK's only with `EVENT_GOT_POKEDEX` and <PKMN>LEAGUE only
with `wNumHoFTeams != 0`, so on this save it is 5 entries and on an earlier one it is 3 or 4. The
Bill's-PC submenu came out exactly as §6-A3 transcribed it: `WITHDRAW Pokémon · DEPOSIT Pokémon ·
RELEASE Pokémon · CHANGE BOX · SEE YA!`.
**Evidence:** `pokemon::integration_tests::postgame::phase0::can_open_the_pokemon_center_pc`
(prints the menu); `pokered/engine/events/hidden_objects/pokecenter_pc.asm:1-4`;
`pokered/engine/events/hidden_objects/bills_house_pc.asm:3-5`;
`pokered/engine/overworld/hidden_objects.asm:88-128`; `pokered/data/events/hidden_objects.asm:177-191`;
`pokered/engine/pokemon/bills_pc.asm:1-86` (`DisplayPCMainMenu`). Regression-checked against
`early_game::can_reach_vermilion` (Bill's PC leg, still green) and the default tier.
**Impact on others:** workstream **A** (the menu indices above are measured, not guessed) and anything
else that routes to a hidden object. **General lesson worth carrying:** a hidden object matching and
being dispatched does *not* mean it did anything — several `SPRITE_FACING_*` routines re-check the
facing and silently `ret`. If an interaction looks like it fires and nothing happens, read the
routine, not just the table.

### [2026-07-30] Phase 0 — **COMPLETE**. `postgame-phase0.bin` is live; A–H are unblocked
**Status:** verified ✅
**What the plan said:** 0.5–0.9 as written in §5.
**What is actually true:** all of it worked essentially as described. The two things that did *not*
match the plan are logged above (the root fixture, and the PC facing check); everything from 0.5
onward went to plan. Specifics worth having:

**Item storage (0.5/0.6).** The menu chain in 0.5 is exactly right, and `TossingItem` was the right
model. One state machine covers both directions —
[`postgame::item_storage`](../src/pokemon/postgame/item_storage.rs) — reached by
`PolicyStep::deposit_item(item, qty, map)` / `withdraw_item(...)`. Notes for anyone extending it:

- **The driver owns the walk, not just the menus.** It cannot be `[UsePc, DepositItem]`: the generic
  overworld executor's A press opens the PC, after which the agent's generic text-advance mashes A
  and walks straight into `BILL's PC` (that is how 0.4 got its free look at the box submenu). So the
  step routes to the *map* only, and `pick_field_move` hands over as soon as we are on it; the driver
  then walks the last tiles itself with `route_to_face_dir(pc, Up)` and owns the A press.
- **The step pops the moment the driver takes over**, like `MovePokemonToFront`. Safe, because
  `pick_field_move` is not polled again until the driver returns to `Idle`. But it means **a test must
  wait on the effect, not on the queue** — `step_until_exhausted()` returns long before the item moves.
- **Completion is measured against a baseline quantity** captured before any menu is touched, so a
  partial move (`qty` < stack) is detected as precisely as a whole one.
- `PLAYER's PC` is index **1** and `BILL's PC` index **0**, *unconditionally* — the entries that vary
  with progress (`PROF.OAK's PC`, `<PKMN>LEAGUE`) all come after them. A2's warning is about `LOG OFF`,
  which does move. The two menus are also indistinguishable by geometry (both `wTopMenuItemY = 2`,
  `wTopMenuItemX = 1`), so the driver tells them apart by text.
- New read-only API: `bag_item_quantity`, `pc_box_item_position`, `pc_box_item_quantity`
  (`wNumBoxItems`/`wBoxItems`, same `(id, quantity)` layout as the bag). Use these, not `GameState::bag`.
- **`ItemId` gained the five unnamed TMs the save carries** (TM06/11/21/24/27, joining TM14/28/34), so
  they can be addressed at all. Regression-checked: default tier and the `early_game` legs including
  `can_reach_vermilion` (Bill's PC) are green.

**Debug tier (0.7).** `PokemonApi::debug_set_money` / `debug_set_coins` / `debug_give_item` /
`debug_give_item_id` (raw id, for the TMs `ItemId` still does not name) / `debug_set_dex_owned` /
`debug_set_party`, in [`postgame::debug`](../src/pokemon/postgame/debug.rs). The guard
`play_path_contains_no_debug_ram_writes` reads `policy.rs`, `agent.rs` and **every file under
`postgame/` scanned from disk** — not a hard-coded list, so a workstream added later is covered
without opting in — and fails if `debug_` appears outside comments. Verified in both directions:
green when clean, and it names file:line when a `debug_set_money` call is planted in `fishing.rs`.

**Seams (0.8).** `PolicyStep` + `AgentState` + `FieldMove` variants are landed for A (4 steps + box
op), B (`Fly`), C (`Fish` + a `Rod` enum), G (`TradePokemon`), H (`SearchHiddenItem`), each with a
delegating arm to a `todo!()` in the owning module. **Taking a workstream is now a one-line edit to
`policy.rs`**: move your variant out of the grouped reserved arm into its own arm. ⚠️ **Treat the
signatures as drafts** — they are transcribed from §6, which was written before anyone tried the
mechanic. Reshaping them is expected and is not a breach of §4.1; the seam is the point, not the shape.
D, E and F got no reserved variants because §6 names no new step for them (D reuses `CatchPokemon`'s
static-encounter branch, E extends `pick_battle_action`, F needs a mart *sell* path).

**The entry fixture (0.9) — `postgame-phase0.bin`:**

```
== postgame-phase0
   map:     ViridianPokecenter @ (3, 3)
   badges:  255 · money: ¥37,774 · coins: 0 · dex: OWNED 7 / SEEN 111
   storage: wBoxCount=0 wCurrentBoxNum=0
   party[4]: Articuno lv73 259/259 · Venusaur lv70 228/228 · Vaporeon lv71 315/315 · Slowpoke lv30 98/98
   bag[14/20]: TownMap, HelixFossil, SSTicket, HM01 Cut, LiftKey, SilphScope, PokeFlute,
               HM03 Surf, HM04 Strength, CardKey, SecretKey, GreatBall×9, FullRestore×6, Revive×4
   PC storage: TM06, TM11, TM21, TM24, TM27, TM34
```

Two things beyond what 0.9 asked for, both deliberate:

1. **The party is healed** (full HP *and* PP). The credits leave Venusaur at 0 HP and Articuno with
   every offensive move at 0 PP, so the first battle any workstream started would have behaved
   strangely for reasons nothing in the plan would explain. The nurse is in the same room as the PC.
2. **It is parked in the Viridian Pokémon Center**, not Pallet Town — i.e. standing at a PC, next to a
   nurse, one map from Route 1. Withdraw any banked TM with
   `PolicyStep::withdraw_item(tm, 1, Map::ViridianPokecenter)`.

Six free bag slots, and nothing any workstream needs was given up.

**➡️ Start here: `postgame-phase0.bin`.** Not `post-hall-of-fame.bin` (a cutscene — see the 0.35
entry) and not `postgame-post-credits.bin` (full bag, hurt party; kept only as 0.9's input).
**Evidence:** `pokemon::integration_tests::postgame::phase0::*` — 5 tests, all green under
`--features slow-tests`, ~7 s wall clock for the lot; `fixture::probe_coverage` output above;
`pokemon::postgame::debug::*` and `pokemon::tile_map::pc_location_tests::*` in the default tier
(813 passing, up from 809). No fixture drift: `git status src/pokemon/data/` shows only the two new
files.
**Impact on others:** **everything**. Phase 0 is done and A–H are unblocked. Claim a row in §9.

### [2026-07-30] A / A1 — the box you can read is in **WRAM**, and the other eleven are unreachable
**Status:** corrected ❗
**What the plan said:** A1 — *"Expose `GameState.boxed_pokemon` (read `wBoxCount` + the SRAM box data
via `encoding.rs`)."*
**What is actually true:** the box the PC menus operate on is in **WRAM**, at `wBoxDataStart` `$da80`
— count, a `$ff`-terminated species list, twenty 33-byte `box_struct`s, then parallel 11-byte OT and
nickname arrays. SRAM holds the *other* eleven: `sBox1`…`sBox12` in banks 2–3, and `CHANGE BOX` is
precisely the operation that copies WRAM↔SRAM (`engine/menus/save.asm:377-387`).

That distinction is load-bearing, because **the eleven inactive boxes cannot be read at all today**:
`DmgPointerRead for MMU` does `panic!("SRAM banking not implemented")` on every `DmgBank::SRAM` arm
(`symbols.rs:115`). So `GameState.boxed_pokemon` is the open box only, and a workstream that banks a
mon and then changes box will see its `boxed_pokemon` go empty. `GameState.current_box` says which box
that is. Implementing SRAM-banked reads would make all twelve visible; nothing in A needed it.

Two things worth copying if you read a game array:

- **A `box_struct` is not a `party_struct`.** It is the first 33 bytes of one: the level is at offset
  **3** (`BoxLevel`, which a party mon also carries as a duplicate of its offset-33 `Level`), and there
  are no computed stats. Reading a boxed mon at the party's 44-byte stride puts slot 1 eleven bytes
  into slot 0's neighbour. Hence a separate `BoxedPokemon` rather than reusing `Pokemon`.
- **Never drop a slot you can't decode.** `read_current_box` *ends* the list at an undecodable species
  byte instead of skipping it, so the entry at index `i` is always box slot `i`. The menus are
  navigated by index, and silently skipping is exactly how `GameState::bag` mis-numbers the bag rows
  (§10). Pinned by `postgame::pc_box::reads_a_boxed_pokemon_out_of_wram`, which plants two members and
  checks the second — the one that catches a wrong stride.
**Evidence:** `pokered/macros/ram.asm:9-39` (`box_struct` / `party_struct`); `pokered/pokered.sym`
(`wBoxCount` `$da80`, `wBoxMon1` `$da96`, `wBoxMon2` `$dab7` → 33-byte stride, `wBoxMonOT` `$dd2a`,
`wBoxMonNicks` `$de06`); `pokered/engine/menus/save.asm:377-387`; `src/pokemon/symbols.rs:115`.
**Impact on others:** anyone reading box contents — `boxed_pokemon` is one box, not 240 slots. Anyone
reading any other banked game array will hit the same SRAM wall.

### [2026-07-30] A — **COMPLETE**. Box storage works; the four reserved `PolicyStep`s are now one
**Status:** verified ✅ (with one shape correction ❗)
**What the plan said:** A2–A7, with `PolicyStep::DepositPokemon { slot }`,
`WithdrawPokemon { box_slot }`, `ChangeBox { n }`, `ReleasePokemon { box_slot }`.
**What is actually true:** every menu index and warning in §6-A was correct, and the whole workstream
came in at **~19 s of wall clock across 7 tests** — the entry fixture parks the player in the Viridian
Pokémon Center, so there is no travel to pay for. Details worth having:

**The seam changed shape (this is the bit that affects you).** The four reserved variants are **gone**,
replaced by one `PolicyStep::UsePcBox { op: PcBoxOp, map: Map }` with four `const fn` constructors in
`postgame/pc_box.rs`. Two reasons: none of the four could say *which* PC to use, and collapsing them
let the routing arm become a one-word edit — `PolicyStep::UseItemPc { map, .. } | PolicyStep::UsePcBox
{ map, .. } =>` — reusing the item PC's existing "route to `map`, then hand over" body verbatim. Build
steps with `PolicyStep::deposit_pokemon(slot, map)` / `withdraw_pokemon(box_slot, map)` /
`change_box(n, map)` / `release_pokemon(box_slot, map)`. `AgentState::UsingPcBox` now carries a
`PcBoxState` (op + PC coordinate + count baselines + press/tick bookkeeping) rather than a bare op.
**This confirms §4.1's premise**: shared-file cost for the whole workstream was one enum variant, one
`|` added to an existing arm, one 8-line `pick_field_move` block, one `AgentState` type parameter and
two one-line arms. Everything else is in two owned files.

**One driver covers all four operations**, because they are the same chain with different indices
(`postgame/pc_box.rs`). Menu order is measured, not guessed: `WITHDRAW` 0, `DEPOSIT` 1, `RELEASE` 2,
`CHANGE BOX` 3, `SEE YA!` 4. Three ordering traps in the screen-matching, all real:

1. `DisplayDepositWithdrawMenu` draws the `DEPOSIT|WITHDRAW / STATS / CANCEL` box straight over the mon
   list **without touching `wTextBoxID`**, so that screen still reports `ListMenuBox`. It has to be
   matched (on `STATS` + `CANCEL`) *before* the list-menu check, or the driver drives the list forever.
2. The Bill's PC menu itself contains the string **`CHANGE BOX`**, so the change-box list is matched on
   its prompt (`Choose a`) and never on the word BOX.
3. `SEE YA!` is checked before `LOG OFF` because the Bill's menu is drawn **over** the parent menu one
   row at a time, so mid-redraw frames carry both menus' text — e.g. the captured screen
   `WITHDRAW Pokémon DEPOSIT Pokémon PROF.OAK's PC PokémonLEAGUE LOG OFF`. A driver that matched
   `LOG OFF` first would read a half-drawn box submenu as the parent menu and re-select entry 0.

**A2's warning is right, but not for the reason given.** The parent menu really does vary in length —
`PROF.OAK's PC` only with `EVENT_GOT_POKEDEX`, `<PKMN>LEAGUE` only with `wNumHoFTeams != 0` — but every
conditional entry comes **after** the first two, so `BILL's PC` at index 0 and the player's item PC at
index 1 are both safe, and it is `LOG OFF` whose index moves. What is genuinely unsafe is the **label**:
without `EVENT_MET_BILL`, `DisplayPCMainMenu` writes `SOMEONE's PC` instead. So match the screen on
`LOG OFF` and select index 0. Measured on this save: `BILL's PC · CLAUDE's PC · PROF.OAK's PC ·
PokémonLEAGUE · LOG OFF`.

**`CHANGE BOX` saves the game — and the first one ever wipes SRAM.** `ChangeBox`
(`engine/menus/save.asm:358`) prints a YES/NO, and if `BIT_HAS_CHANGED_BOXES` (bit 7 of
`wCurrentBoxNum`) is clear it calls `EmptyAllSRAMBoxes` before anything else. That runs *before* the
open box is copied out to SRAM, so a mon deposited seconds earlier survives — verified by switching to
box 2 and back and finding it. `postgame-pc-box.bin` has that bit set, so no later change re-triggers
it. Mask `wCurrentBoxNum` with `$7f` (`BOX_NUM_MASK`) or the box number reads as 128.

**Guards are pre-checked, not read off the screen.** pokered answers "deposit your last mon", "box
full", "party full", "box empty" with a message and a bounce straight back to the Bill's PC menu — from
which a driver that re-picks the same entry loops forever. `PcBoxOp::blocked_by` refuses those four up
front from `wPartyCount`/`wBoxCount` and aborts with a reason on the event stream. There is also a
1200-tick budget so a wedge reports itself instead of pulsing A for the whole test budget.

**⚠️ A test that checks intermediate states must not call `step_until_exhausted`.** `UsePcBox` pops the
moment the driver takes over (like `UseItemPc` and `MovePokemonToFront`), so the queue empties when the
*last* step is **issued** — by which point every earlier operation has already completed. My
four-step A7 chain blew straight past `current_box == 1` that way and died on the cycle cap with all
four operations having visibly succeeded in the log. Chain `run_until` instead; it checks every tick
while the policy advances. This is a sharper version of the note Phase 0 left for `UseItemPc`, and it
will bite any workstream whose step list is longer than two.

**Output fixture `postgame-pc-box.bin`** — `postgame-phase0.bin` plus proven, initialised box storage.
The party is deliberately **restored** (Slowpoke is the only holder of Strength *and* Dig, so banking it
would quietly cost two field moves): party 4, box 1 empty, `wCurrentBoxNum=128`, bag 14/20, standing at
the Viridian PC. Party space is now one `deposit_pokemon` step away at any Pokémon Center, so arrange it
yourself rather than depending on this fixture's layout.
**Evidence:** `pokemon::integration_tests::postgame::pc_box::*` — 7 tests, ~19 s wall clock for the lot
(6 under `slow-tests`, 1 in the default tier). Full slow tier re-run green: **56 passed, 8 pre-existing
ignores**, and the default tier 814 passed (up from 813). `git status src/pokemon/data/` shows only the
one new file — no drift. ROM: `engine/pokemon/bills_pc.asm:1-176, 207-339, 382-431`,
`engine/menus/save.asm:358-402, 437-500`, `data/text/text_2.asm:1548-1618`, `macros/ram.asm:9-39`.
**Impact on others:** **C, D, E, F and G are unblocked** for holding more than six Pokémon. §6-A's step
names are stale — use the constructors above. §8's dependency graph is satisfied for A.

### [2026-07-30] B / B5 — the town map has no cursor in RAM and no flag on it; it is identified by its **broken font**
**Status:** corrected ❗ (B5's warning was right, and understated)
**What the plan said:** B5 — *"⚠️ The town map is a **bespoke screen**, not a `HandleMenuInput` list —
budget real time for this one."*
**What is actually true:** it is worse than "not a menu". `LoadTownMap_Fly` keeps the cursor in the `hl`
register and never writes it to RAM, and it sets no state a driver can key on. Three separate traps, in
the order I walked into them:

1. **`wTownMapSpriteBlinkingEnabled` is not the flag it looks like.** It shares its byte with
   `wPartyMenuAnimMonEnabled` (`ram/wram.asm:1444-1447`), which the **party menu** sets. A driver keyed
   on it starts "driving the town map" while still in the party list, presses Up at the party cursor
   forever, and reports a wedge 1200 ticks later. (wram.asm even says the town cursor blinks regardless
   of the value.)
2. **The screen cannot be read.** `on_screen_text` returns `None` for it, because `LoadTownMap_Fly`
   copies its up-arrow glyph to `vChars1 tile $6d` — **inside** the font block at `vFont` ($8800, $80
   tiles) — so `pokemon_font_loaded()` is false. That is deterministic and happens *before* the input
   loop opens, which turns the bug into the detector: **no font + not the overworld + `wTownMapCoords`
   holding a real town coordinate ⇒ the fly screen, and nothing else.** One caveat that cost a run: the
   clobbered font *survives the landing* until some menu redraws it, so a fixture captured just after a
   flight looks like a town map. The driver therefore also requires having seen the font loaded during
   this flight's menu chain (`FlyState::saw_font`).
3. **Choosing FLY changes nothing a geometry test can see, and the load takes ~30 ticks.** `wTopMenuItemY`
   still says field-move menu while the map loads, so a driver that keeps driving it keeps pressing A —
   and the first A that survives into the fly screen's input loop confirms whatever the cursor starts on:
   **Pallet Town, every time.** The fix is a hard input barrier: from the A that selects FLY until the
   town map is recognised, the driver presses **nothing** (`FlyState::chose_fly_at`, with a
   150-tick fallback because that first A is routinely swallowed by a still-drawing menu).

**How the cursor is steered.** `wTownMapCoords` (low nibble x, high nibble y) is rewritten on every
redraw by `DrawPlayerOrBirdSprite`, so it *is* the cursor position — compared against
`ExternalMapEntries + 3 * map_id` read straight from ROM bank `$1c`. All eleven towns have distinct
coordinates; `town_map_coordinates_match_the_rom_table` pins the table in the default tier. Only **Up**
is ever pressed: `.pressedUp` walks forward through `wFlyLocationsList` skipping unvisited towns and
wraps at the end, so one button reaches everything and an overshoot costs a lap, not a flight. Presses
are dropped on purpose — each cursor move ends in `ld c, 15 / call DelayFrames` — so the driver re-reads
the coordinate every tick instead of counting presses, and A is idempotent while the cursor is on target.

**Pre-flight guards, because none of these are visible afterwards:** the target must be one of the 11
towns, **visited** (`wTownVisitedFlag`; an unvisited town is absent from the cursor's list, so the driver
would circle forever), on an **outside** map (`CheckIfInOutsideMap`: tileset `OVERWORLD` or `PLATEAU`),
and some party member must know Fly.
**Evidence:** `pokemon::integration_tests::postgame::fly_bike::can_fly_between_towns` (Route 16 → Pewter,
cursor walks `b2` Pallet → `82` Viridian → `32` Pewter → A); `pokemon::postgame::fly_bike::tests::*`;
`pokered/engine/items/town_map.asm:141-252, 347-371, 559-587`; `pokered/ram/wram.asm:953-965, 1444-1447`;
`pokered/ram/vram.asm:6-14`; `pokered/data/maps/town_map_entries.asm`.
**Impact on others:** **everyone** — `PolicyStep::Fly { to }` now works from any outdoor map, so no
workstream needs to walk across Kanto again. If you write a driver for another bespoke screen, the lesson
is that this codebase has three menu signals (`wTextBoxID`, `wTopMenuItem*`, on-screen text) and a screen
like this one poisons all three; find something the ROM changes *as a side effect* and prove it.

### [2026-07-30] B — two **shared-file** fixes landed: field-move menu detection, and `can_surf` on Cycling Road
**Status:** corrected ❗ — read this if a Cut / Surf / Strength / Dig driver misbehaves
**What the plan said:** §4.1 — a workstream should touch shared files on four delegating lines. These two
changes are not that, and are logged because they change behaviour every workstream depends on.

**1. `agent.rs`: the field-move menu was being detected two different wrong ways.** `CuttingTree`,
`Surfing` and `UsingFieldMove` all tested `tbid == FieldMoveMonMenu || top_y == 10`, first, before the
START- and party-menu tests. Both halves of that are wrong:

- `wTextBoxID` is only written when a text box is **drawn**, so it lingers on `FieldMoveMonMenu` for the
  whole of a flight. The first `CutTree` after a Fly therefore "recognised" the field-move menu while the
  START menu was open, pressed A on entry 0, and sat in the **Pokédex** until the budget ran out.
- `top_y == 10` is only true for a mon with **one** field move: the box's row counts down from 12 by two
  per field move (`engine/menus/start_sub_menus.asm:36-47`), so Slowpoke (Strength + Dig) is at 8. And
  `top_x` cannot help — it is `wFieldMovesLeftmostXCoord`, pulled left to fit the longest move name, so
  it is 12 for `CUT` and 10 for `STRENGTH` (`engine/menus/text_box.asm:382-492`).

Now: the unambiguous menus are matched first (START at (11,2), party at (0,1)/(0,3)), and the field-move
box needs **both** signals — `tbid == FieldMoveMonMenu && top_y ∈ {4,6,8,10}`. Geometry says "a box of
that shape is configured", the id says "and it is what is on screen, not the message box over it".
Getting this wrong in either direction is silent: with only geometry, `can_catch_articuno` re-selected
STRENGTH forever on the "used STRENGTH." text box; with only the id, Cut opened the Pokédex.

**2. `mod.rs`: `can_surf` must be false while the player is forced onto the bike.** `IsSurfingAllowed`
refuses Surf on Cycling Road outright — the ROM comment says so in as many words
(`engine/overworld/field_move_messages.asm:21-45`, answered with "Cycling is fun! Forget SURFing!") —
and it matters because **Routes 16–18 run along the sea**. With Surf believed available the BFS treats
water as pass-through, so the shortest path down Cycling Road is *straight down the water*: the agent
rode to the shore, tried to mount Surf, was refused, and pulsed A at the party menu for the rest of the
budget. `can_surf` now also requires `wStatusFlags6` bit 5 (`BIT_ALWAYS_ON_BIKE`) clear. (The Seafoam
"current too fast" half of the same routine is already modelled separately as `no_surf_mount`.)
**Evidence:** both found by `postgame::fly_bike::can_ride_cycling_road_to_fuchsia`; the field-move fix is
pinned by the full `slow-tests` tier — **62 passed, 0 failed, 8 pre-existing ignores** — and
`cinnabar::can_catch_articuno` is the test that fails if either half of the new predicate is dropped
(verified against a clean `git archive HEAD` build, where it passes, so the regression was mine).
Default tier 816 passed.
**Impact on others:** anything that uses a field move, i.e. all of A–H. If you add a field-move driver,
copy the predicate — do not re-derive it. And if you route across a coastal map on the bike, `can_surf`
now tells the BFS the truth.

### [2026-07-30] B — **COMPLETE**. Fly, the Bicycle and Cycling Road, plus four route facts worth having
**Status:** verified ✅
**What the plan said:** B1–B7 as written in §6-B.
**What is actually true:** all seven, and the whole workstream is **~50 s of wall clock across 6 tests**
(5 slow-tier legs plus 2 default-tier unit tests) because each leg starts from the previous one's fixture.
The chain is `postgame-phase0` → `-bike-voucher` → `-bicycle` → `-hm02` → `-fly` → `-fly-bike`; it is a
chain rather than five siblings because the travel is the cost, and after B5 the travel is free.

Beyond the two shared fixes above, four things about the routes that no amount of reading the plan would
have told me:

- **§6-B's ordering hides a dependency: B3 needs B2.** `Route16Gate1F` is not one corridor but two —
  west/east door pairs at Route 16 y=10/11 (the Cycling Road road) and y=4/5 (where the Fly house is) —
  joined only by the middle column past a guard who blocks it unless the **Bicycle** is in the bag
  (`scripts/Route16Gate1F.asm:16-46`, stop coords (4,7)…(4,10)).
- **Nothing ever *uses* the Bicycle.** Owning it satisfies the gate guards, and Route 16 (17,10)/(17,11)
  are in `ForcedBikeOrSurfMaps` (`data/maps/force_bike_surf.asm:7-8`), so stepping out of the gate mounts
  it for you. No bike-riding driver is needed, and none was written.
- **Route 18's connection strip is water on both flanks** — it reads `~~~~~CCCCCCCC~~~~~~`, i.e.
  `ConnectionWater` at x=1–5 and x=14–19 — so a plain `enter(Route18)` picks a water tile and strands the
  agent on the last dry tile of Cycling Road. Land explicitly: `enter_at(Route18, 13, 0)`. Any coastal
  connection is worth checking for this.
- **Viridian ↔ Vermilion is Diglett's Cave**, not the Pewter/Mt Moon loop, and it needs Venusaur rotated
  to slot 0 first: the `CuttingTree` executor only ever asks party **slot 0**, and Route 2's east side has
  cuttable trees on both sides of `Route2Gate`. Also a reminder the hard way: Saffron → Celadon must cross
  at `enter_at(Route7, 19, 10)`; the plain connection lands in a ledge-sealed pocket at (20,2), exactly as
  `eevee_vaporeon_surf_steps` already warns.

**One thing the plan feared and I could not reproduce:** the `#[ignore]`d `endgame::can_solve_victory_road_1f`
blames `TeachMove` for wedging on "an HM deep in the bag" (HM04 at index 11 of 16). HM02 sits at **index
15 of 16** here and `can_teach_fly` taught it in 0.6 s, first try, no retries. So bag depth is not the
cause of that failure and whoever picks it up should look elsewhere — my guess is the party slot, since
both ignored tests also address a mon by slot on a fixture whose party has shifted.

**Output fixture `postgame-fly-bike.bin`** — Fuchsia City (1,16), Fly on **Articuno (slot 1)**, Bicycle in
the bag (16/20), badges 255, dex 7 owned / **112 seen** (Snorlax), ¥41,209 from the Cycling Road bikers.
⚠️ **Heal before you fight anything**: the ride leaves Venusaur at 205/232 with **Solarbeam at 0 PP**. The
party order is Venusaur / Articuno / Vaporeon / Slowpoke — Venusaur leads because Cut needs slot 0, and
that is worth preserving.
**Evidence:** `pokemon::integration_tests::postgame::fly_bike::*` (5 slow-tier tests, all green) and
`pokemon::postgame::fly_bike::tests::*` (2, default tier); `fixture::probe_coverage` output for all five
new fixtures; full slow tier 62 passed / 0 failed; default tier 816 passed. `git status src/pokemon/data/`
shows only the five new files — no drift.
**Impact on others:** **C, D, E, F, G, H** — travel is now one step. `PolicyStep::Fly { to }` reaches any
of the 11 towns from any outdoor map and refuses (with a reason on the event stream) rather than wedging
when it cannot. The §8 dependency graph's note that "B makes every other stream cheaper" is satisfied.

### [2026-07-30] B — follow-up: the field-move menu chain is now **one shared function**, not four copies
**Status:** corrected ❗ — this supersedes the "copy the predicate" advice in my entry above
**What my earlier entry said:** *"If you add a field-move driver, copy the predicate — do not re-derive
it."* That was the wrong conclusion from the right observation: the original bug existed *because* the
test was written twice and drifted, and I had just made it four copies.
**What is actually true now:** there is one predicate and one chain driver, and a new field-move driver
should call them rather than copy anything.

- **[`MenuState::is_field_move_menu`]** (`src/pokemon/menu.rs`) — the two-part test (`wTextBoxID` **and**
  `wTopMenuItemY ∈ {4,6,8,10}`), with the whole ROM rationale attached to it once. It sits beside the
  existing `is_main_battle_menu` / `is_mart_item_list` / `is_switch_stats_cancel_menu` predicates, which
  is where this family of tbid-plus-geometry questions already lived.
- **`agent::field_move_menu_button(api, slot, move_index)`** — everything after the overworld branch of
  the chain: START menu → POKéMON, party list → `slot`, field-move box → `move_index`, A for the text in
  between. `CuttingTree` (0, 0), `Surfing` (slot, 0), `UsingFieldMove` (slot, move_index) and the Fly
  driver all call it; each keeps only its own overworld behaviour (face a tree, face the water, press
  START). That removed ~45 lines, a fifth copy of the `nav` closure, and three restated comments — two of
  which had already drifted into saying the opposite of what the code did, inside a single change.

Also from the same clean-up pass, worth knowing: `policy::field_move_index_of(mon, want)` now exists
beside `field_move_index` for callers that hold one Pokémon rather than a whole `GameState`, which is how
the Fly driver gets its slot and row from **one** party read instead of two party reads plus two full
`GameState` builds per tick. And `MetaTileMap::can_surf`'s doc comment now states its third condition;
it is policy-visible as `state.map.can_surf`, so a stale doc there misleads every future workstream.

**Two things deliberately *not* done**, so nobody re-opens them expecting a quick win:

1. **Route knowledge stays duplicated between `policy.rs`'s step lists and mine.** The Cerulean
   trashed-house bridge, the Route 6↔5 Underground Path chain and the Saffron↔Celadon (19,10) crossing
   are each written out in two or three places now, and a shared `cerulean_terrace_bridge()` /
   `saffron_to_celadon()` would be better. But those coordinates live inside `complete_game_steps`'
   legs, which §4.2 **freezes**, and touching them means re-cutting the whole leg-fixture chain. Someone
   doing a deliberate routing-helper pass should do it; a workstream should not.
2. **`CuttingTree` still hardcodes party slot 0**, which is why every step list here inserts
   `MovePokemonToFront { slot: 1 }` before a `CutTree`. Teaching it to find the Cut holder itself is the
   right fix and would delete that dance everywhere, but it changes behaviour on every committed fixture,
   so it is out of proportion to a workstream and belongs with its own test run.

**Evidence:** full `slow-tests` tier after the refactor — **62 passed, 0 failed**, 8 pre-existing ignores
(and `cinnabar::can_catch_articuno` is the canary: it fails if either half of the predicate is dropped);
default tier 816 passed; `git status src/pokemon/data/` still shows only the five new fixtures.
**Impact on others:** anyone writing a driver that opens the party menu — call the two functions above.

### [2026-07-30] D — §6-D's "cheapest workstream" is wrong: the **catch formula**, not the navigation, is the work
**Status:** corrected ❗
**What the plan said:** §6-D — *"Cheapest workstream by far — the machinery already exists.
`CatchPokemon`'s static-encounter branch routes to a map sprite named after the species and presses A;
this is exactly how Articuno was caught. Expect mostly navigation work, not new mechanics."*
**What is actually true:** the routing half is exactly as described and cost nothing. But **Articuno was
caught with the Master Ball**, the only one in the game, and it is spent. Everything after it goes
through the real Gen 1 formula (`engine/items/item_effects.asm`, `ItemUseBall`), and Moltres, Zapdos and
Mewtwo all have `db 3 ; catch rate` — the joint lowest in the game. The formula is:

```
Rand1 ∈ [0,255] Poké / [0,200] Great / [0,150] Ultra   (rejection-sampled, so those are the real ranges)
Status subtracts 12 (burn/paralysis/poison) or 25 (freeze/sleep); if that underflows → caught outright
else if Rand1 - Status > catchRate → the ball fails, whatever the target's HP
else a second roll against X = min(255, (MaxHP*255/BallFactor) / max(HP/4,1)), BallFactor 8 Great / 12 other
```

On catch rate 3 that makes **status the only lever that matters**. Measured per-throw odds against a
full-HP legendary:

| | no status | paralysed |
|---|---|---|
| Poké Ball (¥200) | 0.52 % | 5.1 % |
| Great Ball (¥600) | 1.0 % | 6.7 % |
| Ultra Ball (¥1,200) | 0.89 % | 8.6 % |

Two consequences worth carrying:

- **Weakening the target is nearly pointless here and is dangerous.** The HP term only decides the
  sliver of outcomes where `Rand1 - Status` is still ≤ 3, so it is worth a fraction of a percent — and
  the generic catch policy's "weaken below 50 % first" can KO a target that exists once per cartridge.
  `postgame::legendaries::pre_catch_action` therefore short-circuits that branch entirely.
- **Poké Balls are the best value per yen** (0.026 %/¥ paralysed, against the Ultra Ball's 0.007 %/¥),
  but the number of throws is capped by the target's PP, so the mix is "fill the turn budget with Poké
  Balls, spend the rest making the early turns count". `Bag::best_pokeball` sorts by item id, so Ultra
  Balls are thrown first automatically.

**The only paralysis the party can get is TM45 Thunder Wave**, a free pickup at Route 24 (10,5) — and
**Slowpoke is the sole compatible party member**; Venusaur, Articuno and Vaporeon all lack THUNDER_WAVE
in their `tmhm` list. There is exactly one TM45 in the game and TMs are consumed, so think before
spending it (I spent it on Slowpoke; see the next entry for why that turns out to be the wrong mon).
**Evidence:** `pokered/engine/items/item_effects.asm` (`ItemUseBall`, the `.loop`/`.checkForAilments`/
`.skip1`–`.skip3` blocks); `data/pokemon/base_stats/{moltres,zapdos,mewtwo}.asm:7`;
`data/pokemon/base_stats/*.asm` tm/hm lists; `data/maps/objects/Route24.asm:26`;
`pokemon::integration_tests::postgame::legendaries::can_arm_for_the_legendaries`.
**Impact on others:** **E (Safari)** — the same formula governs Safari Balls, and Chansey/Kangaskhan/
Tauros/Dratini are catch rate 30–60, so Bait/Rock and the HP term matter far more there than they do
here. Anyone adding a catch: use `pre_catch_action`'s shape rather than the generic weaken-then-throw.

### [2026-07-30] D — trapping moves and the one-shot legendary: **never `Run`**, and the paralyser must **outspeed**
**Status:** blocked 🟡 — this is D's blocker, written up so the next agent does not rediscover it
**What the plan said:** nothing; §6-D assumed the fights were free.
**What is actually true:** two ROM rules interact to make a *slow* paralyser useless against Moltres.

**1. A Gen 1 partial-trapping move negates every move the trapped side picks.** `MainInBattleLoop`
displays the battle menu, *then* checks `wEnemyBattleStatus1` bit 5 `USING_TRAPPING_MOVE` and, if it is
set, overwrites the player's choice with `CANNOT_MOVE` (`engine/battle/core.asm:305-322`). So while
Moltres is mid-**Fire Spin**: items work, switching works, running works — **moves do not**. Measured
across four runs, a lv30 Slowpoke selected Thunder Wave on 6, 10 and 6 consecutive turns and its PP
never moved off 20. Moltres at lv50 knows only Peck and Fire Spin (`base_stats/moltres.asm:13`; its
learnset starts at level 51), so it re-traps roughly every other selection and a slower Pokémon
essentially never gets a free turn. This is now readable as `BattleState::enemy_trapping`.

*Corollary that cost me a whole run:* my first fix was to heal the paralyser through the trap. That is
backwards. A trapped turn is one where an item is the **only** thing that resolves, so healing on it is
free — but the generic ordering healed at 60 % HP and therefore spent the *un*trapped turns on potions
too. Paralysis has to be picked ahead of healing, not after it.

**2. Fleeing destroys the legendary.** The birds and Mewtwo are `trainer`-flagged objects
(`scripts/VictoryRoad2F.asm:100`, `trainer EVENT_BEAT_MOLTRES, …`). `EndTrainerBattle`
(`home/trainers.asm:185-213`) sets the trainer flag and, because the opponent is a Pokémon rather than a
trainer class, calls `HideObject` on it — on **every** exit path except `wIsInBattle == $ff`, i.e. a
blackout. So running away is exactly as final as killing it: verified by fleeing once and then finding
`Moltres hidden=true` on VR2F across map reloads for the rest of the run. **Blacking out is the only
recoverable way to lose one of these fights.** (`TryRunningFromBattle` itself has no trap check, so RUN
*is* selectable while wrapped — that is the trap, in both senses.)

**Where that leaves D1b.** The paralyser must land Thunder Wave on a turn the trap is not already
active, which for a slower Pokémon means winning a coin flip on the target's move — and losing it means
losing the encounter, because the fallback (throwing ~59 unparalysed balls at ~1 % each) is ~35 %. A
**faster** paralyser has no such problem: it moves before Fire Spin can start, every time, on turn 1.

Nothing in the party can be that. The obtainable species that learn TM45 *and* outspeed a lv50 legendary
(~110) are, in practice, one: **Electrode** — Power Plant, lv43, base speed 140, catch rate 60. Hence
the re-cut §6-D: **D2/D2a before D1b**, and TM45 goes on the Electrode. (Second-best: Kadabra, base
speed 105, but Abra needs levelling from lv8. Voltorb at base 100 is *not* fast enough at lv40.)

Two smaller things found on the way, both already fixed and both shared-file:

- **`MetaTileMap::is_step_on_warp`** (`map_header.rs`, `tile_map.rs`, used in `agent.rs`). The overworld
  executor treated *any* warp tile on the map border as the kind that only fires when you press the
  outward direction. Victory Road 3F's ladder at (2,0) is on the border **and** is a step-on warp, so
  the agent pressed Up into the top wall for the whole budget. The tileset's warp-tile list
  (`data/tilesets/warp_tile_ids.asm`, read by `CheckWarpsNoCollision`) is the discriminator; note the
  `db`-without-terminator fallthroughs in that file — Gate inherits RedsHouse's ids, Facility inherits
  Cemetery's and Underground's.
- **`CatchPokemon` now gives up in 50 polls instead of 400 when the target sprite is `hidden`.** A
  hidden static-encounter sprite means the battle already happened and was not won; no amount of waiting
  on that map brings it back.
**Evidence:** `pokered/engine/battle/core.asm:280-330`; `pokered/home/trainers.asm:185-213`;
`pokered/data/pokemon/base_stats/moltres.asm`; `pokered/data/tilesets/warp_tile_ids.asm`;
`pokemon::integration_tests::postgame::legendaries::{can_catch_moltres (#[ignore]d, the blocker),
probe_route_to_moltres, probe_stall_artifact}`. Regression-checked: full `slow-tests` tier **63 passed,
0 failed**, default tier **816 passed**, `git status src/pokemon/data/` shows only the one new fixture.
**Impact on others:** **E (Safari)** — Safari encounters flee rather than trap, but the "an item resolves
on a turn a move does not" rule is the same. **Anyone routing through a dungeon**: the step-on-warp fix
changes how the executor leaves a warp tile it is already standing on. **Anyone writing a battle policy
for a one-shot encounter**: RUN is not an escape hatch.

### [2026-07-30] D / D1b — Victory Road 2F is **three** sealed regions, and Moltres is in the third
**Status:** corrected ❗
**What the plan said:** D1 — *"`VictoryRoad2F`. The map is already traversed by `complete_game_steps` —
the agent walks straight past it. Do this first; it should be near-trivial."*
**What is actually true:** the agent walks past two of 2F's three regions and Moltres is in neither.
Measured with `probe_route_to_moltres`, which dumps the reachable set at each stage:

| Entered from | Reaches | Moltres? |
|---|---|---|
| Route 23's (14,31) door → VR2F (29,7) | that door, the (27,7) stairs to 3F | **no** — sealed pocket |
| VR1F's ladder → VR2F (0,8), after its (1,16) boulder switch | 5 trainers, 3 items, the (23,7) stairs | **no** |
| VR3F's (2,0) warp → VR2F (1,1) | Super Nerd 2, the Guard Spec, **Moltres** (11,5) | yes |

And Victory Road can only be entered from the **bottom**. Route 23 has its two doors four tiles apart —
`warp_event 4, 31, VICTORY_ROAD_1F` and `warp_event 14, 31, VICTORY_ROAD_2F` — but they are on different
terraces: coming down from the Indigo Plateau the only reachable warp is the 2F one (47 steps), which
leads to the sealed pocket. So the route is the long one, and it is now written and green as
`PolicyStep::moltres_steps()`: Fly Viridian → heal → Route 22 → the gate (badge check) → Route 23
(the agent Surfs up its pools by itself) → VR1F → its (17,13) boulder switch → VR2F → its (1,16) switch
→ VR3F → back down at (1,1). ~25 s of wall clock, both `SolveBoulders` calls required.

Two fixture rules this leg re-taught, both cheap to get wrong:

- **A fixture must be saved outdoors** if the next leg starts with `Fly`. `FlyState::blocked_by` refuses
  a flight from inside a shop, pops the step with a reason, and the leg then tries to *walk* from the
  Cerulean Mart doormat to Viridian. `arm_for_legendaries_steps` ends with an explicit
  `enter(CeruleanCity)` for exactly this.
- **Do not stop a `run_leg` on "the target sprite is gone".** It reads as absent for a few ticks after
  any battle on that map, so the predicate fires on the first flee rather than on a real loss.
**Evidence:** `pokemon::integration_tests::postgame::legendaries::probe_route_to_moltres` (`#[ignore]`d;
the reachable dumps above are its output); `pokered/data/maps/objects/{Route23,VictoryRoad2F,
VictoryRoad3F}.asm`.
**Impact on others:** none outside D, except the two fixture rules — which are general.

### [2026-07-30] D — **Moltres and Zapdos are caught**, with a debug-seeded Master Ball; Cerulean Cave is unreachable
**Status:** verified ✅ (routes) / blocked 🟡 (the fights, and D5)
**What the plan said:** §3's RAM-write rule allows `PokemonApi::debug_*` for "fixture construction, test
seeding, and diagnostics".
**What is actually true:** taking §3 at its word is what unstuck this workstream. The two problems D
faces are independent — *can the agent get there* and *can it win the fight* — and the second one
(catch rate 3 with no fast paralyser, previous entry) was hiding the first. Seeding a **Master Ball**
from the test file separates them:

- `seed_master_ball()` lives in `integration_tests/postgame/legendaries.rs` and calls
  `debug_give_item(MasterBall, 1)`. The guard `play_path_contains_no_debug_ram_writes` scans
  `policy.rs`, `agent.rs` and `postgame/*.rs` — **not** the test tree — so this is inside the line §3
  draws, and nothing a `Policy` can reach knows about it.
- Nothing else had to change: `Bag::best_pokeball` ranks by item id and `MASTER_BALL` is `$01`, so the
  step lists still say `ball: None`. `pre_catch_action` gained one short-circuit — if the ball to throw
  is a Master Ball, throw it and skip the paralyse-and-nurse routine, which can only lose turns.

**Result: Moltres (dex 8/114) and Zapdos (dex 9/117) are in the party.** Fixtures `postgame-moltres.bin`
and `postgame-zapdos.bin`; tests `can_catch_moltres` (~22 s) and `can_catch_zapdos` (~14 s), both in the
slow tier. ⚠️ **Both fixtures are debug-seeded and everything built on them inherits that.** They prove
the routes, the static-encounter engagement and the catch plumbing — not a legitimate playthrough.

**Two route facts that cost a run each, and are not D-specific:**

- **Cerulean is split by one-way ledges and Fly lands on the wrong side.** From the Pokémon Center
  terrace, Route 9 is unreachable — the leg sat at (20,19) for its whole budget. The bridge is the
  **trashed house**: in the front door, out the back at (27,9), exactly as `cerulean_to_lavender_steps`
  already did it. Anything Flying into Cerulean and heading east needs those two steps.
- **Mt Moon's mouth is on Route 4, not Route 3** (`Route4.asm:11`, `warp_event 18, 5, MT_MOON_1F`).
  Route 3 only reaches Route 4's western end.

**D5 is blocked, and on *navigation*, which is a different blocker from the fight.** Cerulean Cave's
door is Cerulean ROM (4,11). That tile is on a **west strip of the city that no reachable terrace
touches**: ledges above it, a solid wall at x=8 to its east, and its lake is a closed system whose only
other shore is Route 4's equally-sealed pond. Its one land entrance is Cerulean's **west map edge**, on
the row that lines up with Route 4 **y=4** — and that row, though continuous from x=61 to the (90,4)
connection tile, is entered only across the one-way ledge at (60,4) or down the y=3 ledges at x=80-89.
From Mt Moon's exit at (24,5) the BFS reaches (57,4) and stops. Measured, after walking the whole
Pewter → Route 3 → Route 4 → Mt Moon approach:

```
connection_action(Cerulean, (0,12)) -> None                  <- the west strip; the cave is on it
connection_action(Cerulean, (0,13)) -> None
connection_action(Cerulean, (0,18)) -> Some(((90,10), 74))   <- the main terrace, 6 rows too far south
```

So `enter_at(CeruleanCity, 0, 12)` silently falls through to `route_toward`, which takes the (0,18)
crossing and lands on the wrong terrace — which is exactly what the failing runs did. `mewtwo_steps()`
and `can_catch_mewtwo` are written and `#[ignore]`d behind this. **What I would try next:** the ledge
decoding. Every other way in is provably walled, so either (60,4) is a ledge the player really can jump
eastward and `MetaTile::Jump`'s direction is wrong there, or the y=3 x=80-89 ledges are the intended
descent and the BFS is not modelling a jump onto them. `probe_route_to_cerulean_cave` is left in place
and prints the reachable set, the tile grid and the three `connection_action` results in one run.
**Evidence:** `pokemon::integration_tests::postgame::legendaries::{can_catch_moltres, can_catch_zapdos,
can_catch_mewtwo (#[ignore]d), probe_route_to_cerulean_cave}`; `pokered/data/maps/objects/Route4.asm`,
`CeruleanCity.asm:24`. Full `slow-tests` tier **65 passed, 0 failed**, default tier **816 passed**;
`git status src/pokemon/data/` shows only the three new fixtures.
**Impact on others:** the two route facts above are general. The Master-Ball technique is reusable by
**E** (Safari) and **G** (gifts/trades) for the same reason it worked here — it separates "can the agent
get there" from "can the agent win", and only the first is usually the interesting question.

### [2026-07-30] D / D5+D6 — **Mewtwo is caught.** Cerulean Cave is three walls in a row, and none of them is the fight
**Status:** corrected ❗ (this supersedes the "Cerulean Cave is unreachable" conclusion in my previous
entry, which was wrong — I had missed the river seam)
**What my previous entry said:** *"D5 is blocked … Cerulean Cave's door is on a west strip of the city
that no reachable terrace touches … What I would try next: the ledge decoding."*
**What is actually true:** the ledge decoding is fine and the strip is not meant to be walked to at all.
**You Surf in, from Route 24.** Alex pointed at the map; the rest fell out of it. Three walls, in order:

1. **Cerulean City is cut in two.** The Fly landing, the gym and the marts are all east of a lake and a
   solid wall at x=8; the cave door at ROM (4,11) is west of it, ringed by ledges. Cerulean's own west
   edge meets only Route 4's *raised* Mt Moon path, whose east end the BFS cannot climb back onto — so
   every land approach really is walled, which is what made the wrong conclusion look convincing. The
   way across is **Route 24's left river seam**: north to Route 24, then Surf south down the *left* of
   its two seams, which lands in Cerulean on the cave's side of everything.
2. **That seam cannot be asked for.** `MetaTileMap::actions()` emits exactly **one crossing per adjacent
   map** — the nearest — so wherever a footbridge sits beside a river seam to the same place, the seam
   is invisible to `enter()`, and Route 24's footbridge is two steps from where the agent arrives.
   `connection_action` does not help either: it matches `MetaTile::Connection`, and a water edge is
   `ConnectionWater`, which carries **no landing position** at all. Added
   **`MetaTileMap::water_connection_action(to_map)`** as its companion, wired in two places:
   - `policy.rs`, `EnterMap`: after `connection_action(to_map, pos)` finds no land crossing at the
     requested landing, try the water edge. So `enter_at(CeruleanCity, 14, 0)` — Cerulean's own ROM x
     for the seam, where its land bridge is at x=20-21 — asks for the seam by name.
   - `agent.rs`, the overworld executor: it re-derives the route every tick and had the same
     `Connection`-only fallback, so the walk aborted on its first tick with `NoRoute` and the policy
     re-issued it for ever. **Both** sites are needed; fixing only the policy looks like it works and
     then live-locks.
3. **Inside, 1F's ladder to B1F at (0,6) is behind an elevation boundary.** The strip in front of it is
   raw tile **32**, the room below is tile **5**, and `(32, 5)` is one of the Cavern tileset's
   `TilePairCollisions` — a pair the player may not step between, correctly modelled and correctly
   impassable. It is reached over the floor above: up at 1F **(3,11)**, across 2F, down at 2F **(1,3)**,
   which lands on 1F (1,3) inside the walled-off western section. Not any other pair of ladders — 2F is
   itself cut into pieces, and the one 1F (23,7) leads to reaches nothing but 1F (27,1) and itself.

**A fourth thing, which is not navigation and cost two full-budget runs:** with **six in the party** a
caught Pokémon is sent to the box, and the nickname screen on *that* path **wedges the agent** — it sat
printing `name:Mewtwo` and burned every remaining cycle, identically at 150 and at 240 emulated minutes,
having already set the Pokédex flag. Banking Moltres at the Cerulean PC first (workstream A's
`deposit_pokemon`) puts the catch back on the ordinary path and the whole leg drops to **18 s**. Two
lessons: **leave a party slot free before a catch you care about**, and a leg that dies at exactly the
same log position under two different budgets is wedged, not slow. The boxed-catch nickname screen is a
real, unfixed bug — worth its own look for **E** (Safari), which will fill the party fast.

**Result: all three legendaries are caught — dex 10 owned / 121 seen.** Party Venusaur / Articuno /
Vaporeon / Slowpoke / Zapdos / Mewtwo, box 1 holding Moltres, fixture `postgame-legendaries.bin`.
⚠️ Debug-seeded, like the other two.
**Evidence:** `pokemon::integration_tests::postgame::legendaries::{can_catch_mewtwo,
probe_route_to_cerulean_cave}`; `pokered/data/maps/objects/{CeruleanCity,CeruleanCave1F,CeruleanCave2F,
CeruleanCaveB1F}.asm`; the `(32, 5)` pair from the Cavern `TilePairCollisions` table, printed by the
probe alongside the raw tile ids. Full `slow-tests` tier **66 passed, 0 failed**, default tier
**816 passed**; `git status src/pokemon/data/` shows only the four new fixtures.
**Impact on others:** `water_connection_action` is general — **any** map whose only route onward is a
water edge next to a land bridge was previously unreachable, which is worth remembering for **C**
(fishing) and **E** (the Safari Zone's water). The party-slot rule and the "same log position under two
budgets ⇒ wedged" heuristic are general too.

### [2026-07-30] C — **COMPLETE**. The one hard thing was a **deadlock with the game**, not a menu
**Status:** verified ✅ (with one seam reshape ❗)
**What the plan said:** C1–C5 as written in §6-C, with `PolicyStep::Fish { rod, at }`.
**What is actually true:** all five, in **~31 s of wall clock across five legs**. The rod pickups are
three copies of B1's chairman (one `Interact`, `YesNoChoice` opens on YES, the generic A-mash answers
it) and cost nothing. Everything interesting is in the driver, and one thing in it is worth the price
of this whole entry.

**⚠️ The fishing animation must be *mashed through*, not waited out.** This cost most of the task.
`FishingAnim` sets `wMovementFlags` bit 6 (`BIT_LEDGE_OR_FISHING`) for its whole duration, which is the
obvious "a cast is in progress" flag and is exactly right — the animation reloads the overworld under
itself, so without it `game_mode` reads `Overworld` mid-cast and a driver declares the cast resolved
with the rod still in the water. The trap is what you do while it is set. The animation **ends** by
printing `NoNibbleText` / `ItsABiteText`, both of which end in `prompt` — i.e. they block until a
button is pressed — and only *then* does it clear the bit
(`engine/overworld/player_animations.asm:447-449`). So a driver that keeps its hands off while the flag
is set is deadlocked with the game: the flag it is waiting on cannot clear until it presses something.
`DelayFrames` does not read the joypad and there is no menu between the rod and the bite, so the fix is
simply to keep mashing A across the whole animation.

The failure mode is worth describing because it argues for the wrong diagnosis at every step: the cast
*works*, the bite is real (`wRodResponse` = 1, `wCurOpponent` filled in), and the moment anything else
presses a button the battle starts — so the first symptom was "the driver times out and then a battle
immediately begins", which reads as a slow cast. Raising the tick budget just made it time out later.
Two things settled it: the screen (`hDisableJoypadPolling`, `wJoyIgnore` and `wStatusFlags5` were all
clean, and a screenshot showed the ▼ prompt arrow), and the **program counter** — parked in
`DelayFrame.halt` with the return address in `EmotionBubble` and `c` counting down, which is what
proved the animation itself was *fine* and the wait after it was not. A one-off `dbg_registers()`
accessor on `Core` (reverted) plus reading `[sp]` against `pokered.sym` is a fast way to answer "what is
the ROM actually doing" and I would reach for it sooner next time.

**The seam changed shape** (`PolicyStep::Fish { rod, at }` → `Fish { rod, map, goal }`), for a reason
other workstreams will meet: **a cast's outcome is invisible to the policy.** The bite is a wild battle,
`assert_battle_state` takes the driver's state away before it can observe anything, and by the time
`pick_field_move` is polled again the battle is over and nothing in `GameState` records that it
happened. `pokedex_seen` is no help either — this save has **seen 112 of 151 species**, so every fish
in every rod's table is already on it and a "fish until you see something new" goal never terminates.
So the driver does exactly **one cast** and returns to `Idle`, and the policy owns the repetition
against the two things it *can* see:

- `FishGoal::Casts(n)` — a cast counter the policy keeps (`DeterministicPolicy::fish_casts`).
- `FishGoal::Catch { species, max_casts }` — the Pokédex **owned** set, with a bound.

Everything not the target is fled from (`fishing::pick_battle_action`, one delegating call from
`pick_battle_action`, placed *before* the low-PP/heal detours so a session is not abandoned mid-cast).
No weakening pass before a throw: every fishable species is catch rate 155–255, so the HP term is worth
a few per cent and a stray critical hit from a lv73 lead costs the encounter. That is
`legendaries::pre_catch_action`'s reasoning arriving from the opposite end of the range.

**Rooted on `postgame-fly-bike.bin`, not `postgame-phase0.bin`** (§9's row said the latter). The three
rods are in Vermilion, Fuchsia and Route 12; with Fly each is one step and the whole workstream is 31 s,
without it each leg is a cross-Kanto walk. §4.2's "siblings off the root, not a chain" is still the right
default — this is a deliberate exception, and B's row explicitly offers it.

**Four smaller facts, all of which cost a run:**

- **A fixture whose next leg starts with `Fly` must be saved outdoors.** D recorded this for Moltres; it
  bit again here immediately. `rod_pickup` therefore ends with an `enter(town)`. The failure is silent:
  `FlyState::blocked_by` pops the step with a reason and the *rest of the queue* is then discarded for
  want of a route, so the test dies with `queue_len=0` and no obvious cause.
- **Route 12's road is blocked by its own gate building**, and the Super Rod house is at
  `warp_event 11, 77` (`data/maps/objects/Route12.asm:19`) — 56 tiles *south* of it. Lavender's
  connection lands on the north tip, so the leg has to go in the gate's north warp and out its south
  one, disambiguated by landing `(10,21)` or `EnterMap` takes the north warp straight back out and
  loops. `poke_flute_steps` already did this; I did not, and sat at `(10,1)` for the whole budget.
  (Below the gate the BFS Surfs down the route's water on its own, which is why the walk is quick.)
- **`run_until(|s| s.pokemon.len() == n)` fires mid-deposit.** `wPartyCount` drops to its post-deposit
  value partway through the box menus while the on-screen list still shows the old party, so a wait for
  "party is 4" returned in the middle of the *first* of two deposits and the next assertion then read a
  half-banked party. Wait on `boxed_pokemon.len()` instead. Same family as A's "don't
  `step_until_exhausted`" warning, one level down: the queue is not the only thing that lies about
  progress.
- **The catch waits on the party, not the dex bit.** `ItemUseBall` sets `wPokedexOwned` inside the catch
  routine, several text boxes and a nickname screen before the mon is actually appended, so
  `run_until(dex owned)` returns with the party still its old length.

**Two guards worth copying.** `FishState::blocked_by` refuses up front when the rod is not in the bag,
when surfing, or when the map's tileset is not in `WaterTilesets` — because the last two answer "Not the
time to use that!", which looks exactly like a resolved cast, so the policy would re-issue for ever. And
`fishing::pick` rejects a water tile whose walk crosses water: with `can_surf` set the BFS treats water
as pass-through, and the overworld walker cannot mount Surf.

**Output fixture `postgame-fishing.bin`** — Pallet Town (4,14), all three rods (bag 19/20), **dex 10
owned / 113 seen**, party Venusaur / Articuno / Vaporeon / Slowpoke / **Tentacool**, box 1 holding
**Goldeen** and **Magikarp**. Pallet Town is the cheapest complete fishing spot in the game: a Fly
destination *and* a Super Rod group (`.Group1`, `data/wild/super_rod.asm:4`), so all three rods have
something to catch one step from anywhere.
**Evidence:** `pokemon::integration_tests::postgame::fishing::*` — 5 slow-tier tests, ~31 s wall clock
for the lot; `pokemon::postgame::fishing::tests::*` (2, default tier). Full `slow-tests` tier
**889 passed, 0 failed**, 13 pre-existing ignores; default tier **837 passed**.
`git status src/pokemon/data/` shows only the four new files — no drift. ROM:
`engine/items/item_effects.asm:1826-1885, 2817-2882` (the three rods, `FishingInit`,
`IsNextTileShoreOrWater`, `ReadSuperRodData`), `engine/overworld/player_animations.asm:378-450`,
`data/wild/{good_rod,super_rod}.asm`, `data/tilesets/water_tilesets.asm`,
`data/text/text_1.asm:21-33`.
**Impact on others:** **E (Safari)** — the "an outcome the policy cannot see" problem is the same shape
there (a Safari encounter that flees leaves no trace either), and the cast-counter pattern is the
answer. **Anyone writing a driver around a ROM animation**: check whether the flag you are waiting on
is cleared *after* a `prompt`; if it is, waiting is a deadlock, not patience. `PolicyStep::fish(rod,
map, goal)` is available to anyone who wants a water encounter — the whole table is Magikarp, Goldeen,
Poliwag, Tentacool, Krabby, Horsea, Staryu, Shellder, Psyduck, Slowpoke, Dratini (Safari) and their
evolutions.

### [2026-07-30] F — **COMPLETE**. Selling is not buying-in-reverse, and a *gift* mon **is** named
**Status:** verified ✅ (with two corrections ❗)
**What the plan said:** F1–F4 as written in §6-F.
**What is actually true:** all four, in **~9 s of wall clock across four legs**, and the two halves the
plan treated as afterthoughts (F3's sell path, F4's menu) were the entire cost. F1 and F2 are free.

**F1/F2 were exactly as described.** The Coin Case giver is the Diner's **gym guide** — one `Interact`,
`GiveItem COIN_CASE` behind an event flag, no menu, the same shape as B1's chairman and C1's gurus. The
counter clerk is the same shape again: `YesNoChoice` opens on YES, so the generic A-mash buys, and
`BuyGameCoins { target }` just re-issues one `Interact` per purchase and stops on `wPlayerCoins`. What
*did* need care is that **every way the counter can refuse looks identical from outside** — no Coin
Case, a full case and ¥999 all print one text box and return — so all three are pre-checks in
`buy_coins_action`. A driver that simply kept talking would have looped to the budget in all three.

**❗ F3: selling is a different chain from buying, not a mirror of it.** Four differences, each of
which breaks a driver written from the buy path (`engine/events/pokemart.asm:36-115`):

1. **The list is the bag, not the shop's stock**, so the row comes from `bag_item_position` and not
   `mart_item_list`.
2. **`mart_in_quantity_selector()` is useless here.** It keys on `wMaxItemQuantity == 99`, which only
   the *buy* path writes; selling's maximum is the size of the stack. The quantity box is drawn over
   the bag list without touching `wTextBoxID`, so both screens report `ListMenuBox` — the same trap
   A hit with `DisplayDepositWithdrawMenu`. The discriminator is **`wListMenuID`**: `.sellMenuLoop`
   sets `ITEMLISTMENU` (0) before each list and `PRICEDITEMLISTMENU` (2) before the quantity box.
3. **A completed sale returns to the bag list**, not to BUY/SELL/QUIT (`jp .sellMenuLoop`), so "done"
   is only visible in the bag — and B is the only way back out, twice.
4. **Key items and HMs are refused with a bounce back to the main menu**, from which re-picking SELL
   loops for ever. `SellState::blocked_by` refuses them up front, which needed `ItemId::is_key_item()`
   / `is_hm()` — new, and **pinned bit-for-bit against the ROM's `KeyItemFlags` array** by
   `key_item_predicate_matches_the_rom_bit_array`, because the set is not guessable from the names
   (the fossils and the three fishing rods are key items; the Nugget and the Poké Doll are not).

**❗ The one that cost the most: `route_to_face_dir` cannot reach a mart clerk.** A clerk stands behind
a `Counter` and pokered reaches *through* the counter tile, so the standing position is two tiles away,
not adjacent — and only `MetaTileMap::actions()` models that (`tile_map.rs` §2's `counter_extra`).
`route_to_face_dir` knows nothing about it and returned `None`, i.e. **"can't reach the clerk at
(0,5)"** for a clerk standing in an open room. `pick_sale` therefore resolves the standing tile out of
`actions()` and hands the driver a *(tile to face, direction to face it from)* pair. Worth knowing
generally: **`actions()` and `route_to_face_dir` do not model the same map.** A driver that owns its own
walk to a *sprite* — rather than to a hidden object, which is what the PC/vending-machine drivers do —
has to take the position from `actions()` or it will fail on every counter in the game.

**❗ F4: a Game Corner prize *is* offered a nickname, and this is the opposite of D's boxed-catch bug.**
`_GivePokemon` → `AddPartyMon` names the mon whenever `wMonDataLocation` is 0, and nothing on the prize
path sets it otherwise (`engine/pokemon/add_mon.asm:43-52`) — so the run goes through the same naming
screen a catch does, and the agent's generic handler answers it (`[policy] pick name=Celina` in the
log). With a **full party** the prize goes to `SendNewMonToBox` instead, which skips naming entirely.
So for a *gift*, a full party is the **safe** path and an empty slot is the interesting one — exactly
inverted from the catch path D found wedging. Anyone doing G's gifts (Lapras, the fossils, the Dojo
mon) inherits this: they are all `GivePokemon`.

The prize menu itself is a **third bespoke screen**, after B's town map and A's deposit box:
`CeladonPrizeMenu` draws its box with `TextBoxBorder` and **never writes `wTextBoxID`**, so the id on
that screen is whatever the last real text box left — which is `TwoOptionMenu`, the very thing the next
step keys on. The driver therefore matches the prize list on its screen text (`NO THANKS`, placed by
the menu code for all three vendors) **before** the yes/no test, not after. Which of the three vendors
you get is decided by `hTextID` — i.e. by the bg-event tile you are standing in front of — so the three
counter tiles (2,2)/(4,2)/(6,2) *are* the three menus, and `Prize` encodes tile, row and price
together. The whole nine-prize table is pinned against ROM by `prize_table_matches_the_rom`; note the
`db …, "@"` terminator is **`$50` in pokered's charmap**, not ASCII `@`, which is how that test first
failed. Completion is read from **coins**, not the party: `HandlePrizeChoice` subtracts the price
*last* and returns without charging if both party and box are full, so a coin drop proves delivery for
all nine prizes including the three TMs, where no count moves.

**⚠️ What is *not* covered, and what would unblock it.** F4 proves the mon branch and vendor window 0
only. The TM branch is a genuinely different code path (`GiveItem` rather than `GivePokemon`) and the
cheapest TM prize is **3300 coins ≈ ¥66,000**; this save has ¥43,209 and there is no more junk worth
selling (the three TMs left in PC storage are worth ¥4,000 together). Porygon at 9999 coins is
**¥200,000**. So the prize economy's expensive half needs a *money* source, not more mechanism: the
Elite Four rematch is the in-scope one, and it is also several minutes of emulated time per lap. I left
it rather than grind, since §1's target is mechanism coverage.

**Shared-file cost, for §4.1's record:** two `PolicyStep` variants plus one word added to an existing
routing arm, two `FieldMove` variants, two `AgentState` variants, four delegating arms, two additions
to `agent.rs`'s exclusion lists (one of them load-bearing: `assert_pokemart_state` would otherwise hand
a *sale's* Buy/Sell/Quit menu to the buy-only `PokemartState` and answer it with BUY), plus one
`GameState.coins` field and `DeterministicPolicy::route_toward` widened to `pub(crate)` so a postgame
module can route. Everything else is in the two owned files. The seam held.
**Evidence:** `pokemon::integration_tests::postgame::game_corner::*` — 4 slow-tier tests, ~9 s wall
clock for the lot; `pokemon::postgame::game_corner::tests::*` (2, default tier, both ROM-pinned). Full
`slow-tests` tier **896 passed, 0 failed**, 13 pre-existing ignores; default tier **839 passed**
(up from 837). (895/839 before the TM-branch follow-up entry below added a fifth leg.)
`git status src/pokemon/data/` shows only the four new fixtures — no drift. ROM:
`engine/events/pokemart.asm:36-115`, `engine/events/prize_menu.asm` (all of it),
`engine/events/give_pokemon.asm:1-52`, `engine/pokemon/add_mon.asm:1-55`,
`engine/items/item_effects.asm:2616-2644` (`IsKeyItem_`), `home/list_menu.asm:197-300`,
`scripts/{CeladonDiner,GameCorner,GameCornerPrizeRoom}.asm`, `data/events/prizes.asm`,
`data/items/{key_items,tm_prices}.asm`.
**Impact on others:** **G** most of all — `SellToMart` exists now, every gift mon in G1–G4 goes through
the `GivePokemon` naming path described above, and the counter-mediated-walk finding applies to any
driver that walks to a *sprite* rather than a tile. **E** gets `ItemId::is_key_item()` (the Safari Ball
is a key item). **H**: dex owned is now **8**, so Flash at 10 is two species away.

### [2026-07-30] F — follow-up: the TM prize branch **is** covered; "unfunded" was the wrong call
**Status:** corrected ❗ — this supersedes the "⚠️ What is *not* covered" paragraph of my entry above
**What my previous entry said:** *"the cheapest TM prize is 3300 coins ≈ ¥66,000; this save has
¥43,209 … the prize economy's expensive half needs a money source, not more mechanism … I left it
rather than grind."*
**What is actually true:** that reasoning stopped one step short. Alex pointed it out: §3 already
allows `PokemonApi::debug_*` for *test seeding*, and D had already proved the pattern with the Master
Ball — so the money was never the obstacle, only the assumption that a test had to earn it.

The distinction that matters, and that I should have drawn the first time: **seeding an input is not
short-circuiting the mechanism.** `seed_money(fixture, 100_000)` in the test file replaces the answer
to *"can the agent earn ¥66,000?"* — an economy question whose only in-scope answer is grinding the
Elite Four, minutes of emulated time a lap, and which says nothing about menus. Everything downstream
still runs for real: **66 separate conversations** with the counter clerk, the walk to the third vendor
tile, the item-name menu, the purchase. The right test to ask of a debug seed is not "did it write
RAM?" but "does the write stand in for the thing under test, or for a prerequisite?" Here it is a
prerequisite; the Master Ball was the same shape.

`can_redeem_a_prize_tm` is therefore green (~15 s), and it does earn its place — the TM vendor is a
real fork, not a cosmetic one. `HandlePrizeChoice` branches on `wWhichPrizeWindow == 2` into
`GetItemName` + `GiveItem` instead of `GetMonName` + `GivePokemon`, so it is the case where **no party
count moves**, which is exactly what the driver's coin-based completion check exists for and had never
actually been exercised. Result: TM23 Dragon Rage in the bag, party unchanged at 5, coins 20 → 3320 →
20, ¥100,000 → ¥34,000. `ItemId` gained `Tm23DragonRage` (`$df`) so the bag can be addressed at all —
the same reason Phase 0 added the other five TMs.

**One deliberate asymmetry:** this test writes **no fixture**. F's committed chain still ends at
`postgame-game-corner.bin`, which is honestly earned, so nothing downstream inherits seeded money —
unlike D, where the seeded catches *are* the chain and every later state carries that caveat. If a
test only needs the seed to reach a branch, it can prove the branch and throw the state away.
**Evidence:** `pokemon::integration_tests::postgame::game_corner::can_redeem_a_prize_tm`;
`engine/events/prize_menu.asm` (`HandlePrizeChoice`, the `cp 2 ; is prize a TM?` fork).
**Impact on others:** anyone who catches themselves writing "I left it rather than grind" — check
whether the thing you would grind for is the mechanism or a prerequisite for it. **E (Safari)** will
meet this directly: Safari Balls and the 500-step budget are prerequisites, and the entry fee is ¥500
a go.

### [2026-07-31] G-gifts — a **full party does not skip the naming screen**; F's entry is wrong on that point
**Status:** corrected ❗
**What the plan said:** F's §11 entry — *"With a **full party** the prize goes to `SendNewMonToBox`
instead, which skips the naming entirely. So for a *gift*, a full party is the **safe** path and an
empty slot is the interesting one."* §6-F repeats it, and my own module header did too until this
entry.
**What is actually true:** both branches name. `_GivePokemon` picks between `AddPartyMon` and
`SendNewMonToBox`, and `SendNewMonToBox` ends with its **own** `predef AskName`
(`engine/items/item_effects.asm:2731-2733`) — the box path is not a shortcut past the naming screen,
it is a different route to the same one. Measured rather than read: `a_full_party_sends_the_silph_
lapras_to_the_box` runs the Silph Lapras gift at party 6, and the mon arrives in box 1 **nicknamed**,
which is why that test asserts the nickname and not just the species. A default name would have meant
the screen never ran.

**The part that matters more, and that I could not resolve:** D reports that a **boxed catch**'s
naming screen *wedges the agent* — "it sat printing `name:Mewtwo` and burned every remaining cycle,
identically at 150 and at 240 emulated minutes" — and warns E to expect it. A boxed **gift** goes
through the same `SendNewMonToBox`, and here it was answered cleanly (`[policy] pick name=Celina`,
then `Lapras "Celina"` in the box) with the party still at 6. So whatever D hit is **narrower than
"the boxed naming screen"**; the two paths differ in that the catch happens inside a battle, i.e. in a
different `AgentState`. I have not reproduced D's wedge and am not claiming it is fixed — only that
"full party ⇒ safe" is not the reason a gift works, and that E should not assume the box path is
inherently wedged.
**Evidence:** `pokemon::integration_tests::postgame::gifts::a_full_party_sends_the_silph_lapras_to_
the_box`; `pokered/engine/events/give_pokemon.asm:1-52`;
`pokered/engine/items/item_effects.asm:2648-2733` (`SendNewMonToBox`, the `AskName` at its end).
**Impact on others:** **E (Safari)** most of all — it was told the box path skips naming and is safe.
**G5/G6 (trades)** and anyone doing G8c: the gift/box distinction is not a naming distinction.

### [2026-07-31] G-gifts — two shared-file bugs the Silph floors turned up, one fixed
**Status:** corrected ❗ (one fixed, one worked around)
**What the plan said:** §6-G7 — *"Skipped Silph floors. 2F/4F/6F/8F/10F — item pickups only."*
Nothing about either of these.

**1. `UseElevator` rides you back where you came from — and reports success.** Issued while the player
is still standing on (or beside) the lift tile they just arrived by, the ride silently returns them to
the floor they came from. It then *pops as complete*, because `pick_field_move`'s completion test is
"we are no longer in an elevator room" (`policy.rs`) and a wrong floor satisfies that as happily as the
right one. Measured cleanly: of seven rides in one leg, the three issued from the lift tile failed and
the four issued after a walk succeeded, and the failures never printed `elevator→floor n (sel=true)` at
all. Two floors of G7 carry no items, so the leg has nothing to walk to — which is why
`SILPH_FLOORS` gives 2F and 8F a worker to `Interact` with — collecting an item walks away from the
door as a side effect, so a floor with nothing to collect needs something else to. **That column is
load-bearing, not decoration.** I did not fix it: the failure is in the shared `UseElevator` completion test, which
should compare against the floor that was *asked for* rather than merely "not in a lift", and that is a
change every elevator leg in `complete_game_steps` would have to be re-run against.

**2. `deposit_item` hangs on a full stack, which is the common case.** `DisplayChooseQuantityMenu`
wraps at `wMaxItemQuantity`, which the list menu sets to the **live** stack size
(`home/list_menu.asm`), so a target above what is held is *unrepresentable*: the driver pressed Up for
ever watching the counter cycle 1…8…1 past the 9 it wanted. Two separate things made that happen at
once and both are worth knowing:

- **A partial deposit frees no bag slot.** `deposit_item(GreatBall, 1)` on a stack of nine moves one
  ball and leaves the row exactly where it was. If you are depositing to make *room*, the quantity has
  to be the whole stack — my first attempt banked six entries, freed three slots, and wedged on "No
  more room for items!" at the last Silph ball, forty tiles from the lift.
- **The stack shrank underneath the step.** `ItemPcState::new` captured `start_qty = 9`, and by the
  time the quantity box opened the bag held **8** — one Great Ball had already gone into storage
  during the *previous* item's menu navigation (PC storage ends the run holding `GreatBallx9`, so it
  arrived in two pieces). A stray A press on an item list is a silent deposit; the list is always
  under the cursor.

Fixed in `postgame/item_storage.rs`, in two places, because the two causes need different clamps:
`ItemPcState::new` clamps `qty` to `start_qty` (so a caller can pass `u8::MAX` for "all of it" without
knowing the count), and the selector branch clamps to the **live** source quantity each tick (so a
stack that shrinks mid-operation still completes — the completion test is measured against
`start_qty`, so moving the smaller amount still satisfies it). `silph_floors_steps` now takes
`&[(ItemId, u8)]` and its test passes `u8::MAX` throughout.

**One thing that went exactly as hoped:** the PC accepts **key items**. `IsKeyItem` in
`engine/menus/players_pc.asm:164` only suppresses the quantity prompt — it does not refuse — so the
S.S. Ticket, Lift Key and Silph Scope are three free bag slots for anyone who needs them.
**Evidence:** `pokemon::integration_tests::postgame::gifts::can_clear_the_skipped_silph_floors` and
its `probe_silph_item_floors` diagnostic; `pokered/home/list_menu.asm` (`DisplayChooseQuantityMenu`,
`.incrementQuantity`); `pokered/engine/menus/players_pc.asm:95-140`. Full `slow-tests` tier after the
`item_storage` change: **903 passed, 0 failed**, 16 pre-existing ignores; default tier **839 passed**.
`probe_coverage` now prints **PC item storage** alongside the bag, which is how the two-piece Great
Ball was spotted — it showed half the picture before.
**Impact on others:** **everyone who deposits anything.** The clamp is a behaviour change to Phase 0
infrastructure. Anyone driving an elevator: check *which floor* you arrived on, not just that you left.

### [2026-07-31] G-gifts — three routes that are not one room, and how each announced itself
**Status:** corrected ❗
**What the plan said:** §6-G3 — *"Silph is already traversed; the gift was simply never taken."*
§6-G2 — *"`Museum1F/2F` (Pewter), behind a Cut tree."* §6-G8 — *"`Daycare` (Route 5)."*
**What is actually true:** each of those three is a **terrace problem**, and each fails differently —
which is the useful part, because only one of them looks like a failure.

- **Silph 7F is three pockets and the lift opens onto the wrong one.** 7F has its own elevator door at
  (18,0) and `SilphCoElevatorFloors` lists all eleven floors, so riding to menu index 6 *is* one step
  — and from (18,0) the reachable set is workers 2/3/4, the Calcium, the TM and three warps. The
  Lapras worker is **not in it**. He is in the walled rival pocket, reached the way
  `silph_giovanni_steps` reaches it: lift to 3F, then 3F's (11,11) pad. **How it announced itself:**
  the agent walked *to another floor* and talked to a different sprite with the same display name.
  `MapSprite` matches on name, and the router will happily cross a map boundary to satisfy it, so
  "unreachable sprite" presents as "wrong sprite, silently".
- **The Pewter Museum's back room needs the Cut tree, and `enter_at` does not say so.** Pewter has two
  warps to `Museum1F` and the Old Amber scientist is behind the *back* one, so the step is
  `enter_at(Museum1F, 16, 7)`. With the Cut tree still standing, that landing is unreachable and the
  step **falls through to the front door** at (10,7) — no warning, no abort, just the wrong side of a
  wall and a leg that dies later for no visible reason. Measured by deleting the `CutTree` step and
  re-running. Same silent-fallthrough shape D recorded for `connection_action`, and the general lesson
  is the same: `enter_at`'s position is a *preference*, not an assertion.
- **Route 5 is three parallel corridors joined only at the bottom, and the Day Care is in the middle
  one.** The rungs between them are one-way ledges. From the Cerulean crossing the agent lands at
  (18,1) in the right-hand corridor, and `actions()` there lists the Route 5 Gate, the Underground
  Path and Cerulean — **and no Day Care**. Its whole top row is connection tiles, so the corridor is
  chosen by *which crossing you ask for*: `enter_at(Route5, 10, 0)` lands in the middle one, and from
  there it is a straight walk south, hopping ledges, to the door. **How it announced itself:** the
  agent stood still. `EnterMap` with no matching action does nothing at all, which is the cheapest
  failure of the three to diagnose and the only one that looks like a bug from the log.

**The diagnostic that answered all three is the same six lines** — drive to the arrival tile, step 50
ticks for the sprites to settle, print `state.map.actions()`, and (for Route 5) an ASCII dump of the
meta-tile grid. `probe_silph_7f_pockets`, `probe_silph_item_floors` and `probe_route5_terraces` are
left `#[ignore]`d in the test file. D's `probe_route_to_moltres` was the template; it is worth
reaching for **before** writing the route, not after the first stall, because two of these three
never stall.
**Evidence:** the three probes above; `pokered/data/maps/objects/{SilphCo7F,Museum1F,PewterCity,
Route5}.asm`; `pokered/scripts/Museum1F.asm:190-205`.
**Impact on others:** **G5/G6 (trades)** — `Route11Gate2F`, `Route18Gate2F` and `Route2TradeHouse` are
all gate interiors with the same shape. **H4 (hidden items)**. And anyone tempted to trust a sprite
name: it does not carry a map.

### [2026-07-31] G-gifts — the Day Care needed the plan's first **party-menu** driver, and trades will want it
**Status:** verified ✅
**What the plan said:** §6-G8 lists the Day Care among the "colour rooms", alongside text-only houses.
**What is actually true:** it is the one mechanic in G that could not be done with `Interact`, and the
reason is a single missing instruction in pokered. `DaycareGentlemanText` calls `DisplayPartyMenu`
**without resetting `wCurrentMenuItem`** (`scripts/Daycare.asm:26-32`), so the list opens on whatever
the last party menu left behind. An A-mash therefore hands over an arbitrary party member — and this
party's other five all carry Cut, Fly or Strength, which `KnowsHMMove` refuses with "I can't accept a
POKéMON that knows an HM move", for ever. The log is unmistakable once you know to read it: the party
list re-renders three times with the lead visibly *first* and the refusal every time.

`PolicyStep::UseDaycare { slot }` + `AgentState::UsingDaycare` + [`postgame::gifts::tick`]. The driver
is deliberately small and the shape is worth copying:

- It matches the party list on **box origin** — `top_x == 0 && (top_y == 1 || top_y == 3)` — which is
  the same signal `agent::field_move_menu_button` already uses. Everything else in the conversation
  (two YES/NOs and the money box) opens on entry 0, so `_ => A` covers it and there are exactly two
  cases.
- **Completion is "the party count changed", which serves both halves.** Deposit drops it by one,
  collection raises it by one, `wDayCareInUse` picks the branch, and one step does both — so the caller
  writes `UseDaycare` twice and never says which operation it wants.
- It owns the walk, resolving the standing tile from `actions()` rather than `route_to_face_dir`,
  copying F's `pick_sale` verbatim for the reason F recorded: the two do not model the same map.
- Three smaller things measured on the way. **Nothing needs to restore the lead afterwards** — handing
  over slot 0 promotes the mon behind it, so the Cut holder is leading again for free and a collected
  mon is appended at the end. The **bill is ¥100** for a same-visit round trip (¥100 × levels grown +
  1; a mon gains one exp point per step walked, so nothing at level 30 grows in a corridor) — and the
  ¥100 is the assertion that matters, because the party count returning to six proves the mon came
  back but only the money proves it came back *through the counter*. And **one visit each way**: a
  second conversation takes the other branch and would collect the mon straight back.
**Evidence:** `pokemon::integration_tests::postgame::gifts::can_leave_a_pokemon_at_the_day_care`
(~3 s); `pokered/scripts/Daycare.asm:9-140`. Shared-file cost, for §4.1's record: one `PolicyStep`
variant, one `FieldMove` variant, one `AgentState` variant, one `None` routing arm, one 6-line
`pick_field_move` block, two delegating arms and one addition to `agent.rs`'s text-state exclusion
list. Everything else is in the two owned files. The seam held again.
**Impact on others:** **G5/G6 (trades)** — `DoInGameTradeDialogue` opens the same script-driven party
menu, so the trade driver is this one with a different completion test, and it is the reason that row
is worth taking next. **G8c's Name Rater** likewise (`NameRatersHouse.asm:57` is another
`DisplayPartyMenu`), with one wrinkle: `DeterministicPolicy` answers every naming screen with the same
name, so the mon renamed has to be one not already called it — this party is five `Celina`s and one
`Leslee`.

### [2026-07-31] G-gifts — the boring half went to plan; here is what is left
**Status:** verified ✅
**What the plan said:** G1–G4 and G7 as written in §6-G.
**What is actually true:** every route surprise is logged above; the *mechanics* were all as cheap as
§6-G assumed, and five of the seven legs were one `Interact` and a text mash. Worth recording so
nobody re-checks:

- **G1's fossil revival is a two-visit mechanic**, which §6-G's "Omanyte is one interaction away" does
  not say. The scientist takes the fossil and asks you to go for a walk, and the walk is not a step
  counter — `CinnabarIsland_Script` resets `EVENT_LAB_STILL_REVIVING_FOSSIL` on every load of the
  island (`scripts/CinnabarIsland.asm:6`), so "a walk" is precisely out of the lab and back, four
  warps. The bespoke fossil-choice menu (`TextBoxBorder` + `HandleMenuInput`, no `wTextBoxID`) needed
  no driver: with one fossil in the bag the cursor already sits on it.
- **G2's Old Amber giver is `MUSEUM1F_SCIENTIST2`**, not the `SPRITE_OLD_AMBER` object standing beside
  him, which is scenery with a text pointer and no item id.
- **G4 spends Hitmonchan.** Taking either dojo ball `HideObject`s the other, so that species is gone
  from this cartridge. The Karate Master is engaged by `Interact` rather than his (4,3) coordinate
  trigger — his text script does the whole `EngageMapTrainer` dance itself.
- **G3 and G4 were deliberately run at different party sizes** so the two `_GivePokemon` branches are
  both covered: Lapras at party 6 (boxed), Hitmonlee with a slot banked first (party). See the naming
  entry above for why that is *not* the naming distinction F described.

**Output `postgame-daycare.bin`** — Route 5, **dex 11 owned / 113 seen**, party Venusaur / Articuno /
Vaporeon / Slowpoke / Aerodactyl / **Hitmonlee**, box 1 holding **Lapras** + Omanyte, ¥44,284, bag
17/20 (TM29, TM31, TM03, TM26 and the ten Silph items), PC storage holding twelve entries. All seven
fixtures are on `probe_coverage`'s list.

**What is left, and it is not blocked by anything:** **G8c** — the Name Rater plus four text-only
rooms (`ViridianSchoolHouse`, `CeladonHotel`, `CeladonChiefHouse`, and `CeladonDiner`, which F already
visits for the Coin Case). I stopped rather than start a fifth mechanic; the Name Rater's recipe is in
the entry above and is perhaps an hour.
➡️ **Two consequences for other rows.** **H1 is unblocked**: dex owned is **11**, past Flash's gate of
10. And **G5/G6 (trades)** is the row this workstream most directly helps — the party-menu driver is
the piece it was missing, and four of its nine give-species (Abra, Slowbro, Poliwhirl, Nidorino) are
now either in hand or one evolution away between F's prize Abra and this party's Slowpoke.
**Evidence:** `pokemon::integration_tests::postgame::gifts::*` — 7 slow-tier tests, **~53 s of wall
clock for the lot**, plus 3 `#[ignore]`d probes. Full `slow-tests` tier **903 passed, 0 failed**
(from 896), default tier **839 passed**. `git status src/pokemon/data/` shows only the seven new
files — no drift.

### [2026-07-31] G-gifts — **COMPLETE**. G8c lands, and the party-menu driver is now an enum with a hole for trades
**Status:** verified ✅ (supersedes the "what is left" paragraph of my previous entry)
**What my previous entry said:** *"**G8c** … I stopped rather than start a fifth mechanic; the Name
Rater's recipe is in the entry above and is perhaps an hour."*
**What is actually true:** it was about an hour, almost all of it in one wrong assumption, and the
result generalised the driver rather than copying it.

**The Name Rater is the Day Care with a different ending**, so `AgentState::UsingDaycare` became
`AgentState::UsingPartyScript` and the state carries a `PartyScript` (which NPC, and where) plus a
`Baseline` (what "done" means): `PartyCount` for the Day Care, the chosen slot's raw `wPartyMonNicks`
bytes for the Name Rater. Nothing else differs — same stale-cursor party menu, same walk resolved out
of `actions()`, same two-case button table. **G5/G6's in-game trades belong here as a third variant:**
`DoInGameTradeDialogue` opens the same menu, so that row needs a `PartyScript::Trade` and a completion
test, not a driver.

**The assumption that cost the hour, and it is a trap for any test that renames or names anything:**
I wrote "`DeterministicPolicy` answers every naming screen with the same name, so rename a mon not
already called it" — correct, but I then read the *policy* rather than the *fixture* and picked
Hitmonlee. `PokemonNamePicker` draws without replacement, so within one run no name repeats; but a
`DeterministicPolicy` is constructed **per leg**, always from the same seed, so **every leg's first
draw is the same name**. Five of this party's six are called it, having each been named by the first
draw of the leg that obtained them. The rename ran perfectly and was invisible: `"Celina" → "Celina"`,
and the test timed out waiting for a change that had already happened. **Articuno is the only
uniquely-named party member** — "Leslee", from the main quest, where it was not the first draw — so it
is the only slot whose rename can be observed at all.

That accident bought a better assertion than I had planned. The entry fixture's `wCurrentMenuItem` is
**0**, left there by G8b's deposit, so renaming **slot 1** and asserting *slot 0 kept its name* is a
direct test of the cursor being driven: a driver that did not move it would rename Venusaur and leave
Articuno alone. Both assertions are in the test.

**One more thing worth knowing about naming screens:** `assert_naming_screen` runs *before* the
state-machine exclusion list (`agent.rs`), so it takes the agent away from any driver the moment the
screen opens and returns it to `Idle`, not to the driver. The Name Rater therefore never sees its own
completion. That is harmless — the step pops on issue and the effect still lands — but it means **a
driver whose script ends in a naming screen must not rely on running to completion**, and its test
must wait on the effect rather than on the driver's event.

**Output fixture `postgame-name-rater.bin`** — Celadon City, dex **11 owned / 113 seen**, party
Venusaur / **Articuno "Celina"** / Vaporeon / Slowpoke / Aerodactyl / Hitmonlee, box 1 holding Lapras
+ Omanyte, ¥44,284, bag 19/20, PC storage 15 entries. Three more never-visited maps closed
(`ViridianSchoolHouse`, `CeladonHotel`, `CeladonChiefHouse`).
**Evidence:** `pokemon::integration_tests::postgame::gifts::*` — **8 slow-tier tests, ~57 s of wall
clock**, plus 3 `#[ignore]`d probes. Full `slow-tests` tier **904 passed, 0 failed**, 16 pre-existing
ignores; default tier **839 passed**. `git status src/pokemon/data/` shows only the eight new files —
no drift. ROM: `pokered/scripts/NameRatersHouse.asm:1-90`.
**Impact on others:** **G5/G6 (trades)** — the driver it needs exists, as an enum expecting a third
variant. **Anyone writing a test that names a Pokémon**: the picker's first draw is a constant across
legs, so "the name changed" is only observable on a mon that draw has not already hit.

### [2026-07-31] G-trades — **four trades, dex 11 → 19**; the driver was already written, and the NPC is not the one you'd pick
**Status:** verified ✅ (with one seam deletion ❗)
**What the plan said:** §6-G5 — *"`PolicyStep::TradePokemon { give_slot, at }` driving the offer/accept
flow"*, with a reserved `PolicyStep` variant and an `AgentState::Trading` from task 0.8.
**What is actually true:** there is no offer/accept flow to drive. `DoInGameTradeDialogue` is a YES/NO
that opens on YES, then `InGameTrade_DoTrade` calls **`DisplayPartyMenu`** — the same stale-cursor
party menu the Day Care and the Name Rater open. So a trade is a third
`postgame::gifts::PartyScript` variant and **both reserved seams are deleted**, the way A deleted its
four. Build trades with `PolicyStep::trade_steps(give, catch_on, bank, bank_at)`.

One improvement fell out of doing it third: **a trade finds its own slot**. `PartyScript::Trade`
carries the give-species, so `pick` locates it in the party rather than trusting a caller-supplied
index. That is not tidiness — the alternative is `MovePokemonToFront`, and promoting the mon to be
traded *demotes the Cut holder*, which breaks the very next leg that meets a tree. Two of these four
legs meet a tree. Completion is likewise index-independent (`Baseline::SpeciesGone`), because
`InGameTrade_DoTrade` does `RemovePokemon` then `AddPartyMon`: the received mon is **appended**, so
every slot after the traded one shifts.

**❗ The trader is not the NPC you would guess, and a wrong guess does not fail — it chats.** Three of
the four legs died on this, each costing a full run, because talking to the wrong sprite prints a
perfectly ordinary text box and the driver then waits out its whole budget:

| Room | The trader | The obvious wrong choice |
|---|---|---|
| `Route2TradeHouse` | **Gameboy Kid** | the Scientist standing in front of him |
| `CeruleanTradeHouse` | **Gambler** | the Granny |
| `CinnabarLabTradeRoom` | **Gramps** (Raichu) and **Beauty** (Venonat) | the Super Nerd |

`data/events/trades.asm` cannot help: the ROM table has give/get/nickname and **no location at all**.
Only the nine scripts that set `wWhichTrade` say who trades, so read those. `TRADES` records all nine
rows and `trade_table_matches_the_rom` pins the give/get pairs bit-for-bit — worth having because the
ROM addresses trades by *index* and this table by *map*, and nothing else would catch a drift.

**Three route facts, all of the same family as G-gifts' terraces:**

- **Route 2 is two halves** split by `Route2Gate` at y=35/39, and the trade house is the *north* one
  at (15,19). Flying to Viridian lands at y=72 and `enter(Route2TradeHouse)` then stands still. From
  Pewter it is still walled off: the reachable set is the forest gate, Pewter and **one cut tree** at
  (5,10) — every Route 2 ledge sits under a wall, so that tree is the only link to the eastern column.
- **`goto` cannot see a gate building.** It pops the instant the map matches, so
  `goto(Route15) + CatchPokemon` paced on the grassless Fuchsia-side strip for ninety emulated minutes
  with **no battle ever starting** — the quietest failure in this workstream. Route 15's grass is all
  east of `Route15Gate1F`; the fix is `enter(gate)` then `enter_at(Route15, 14, 8)`. Any hunting
  ground behind a gate needs the same, and `Route15` is one entry in a `to_hunting_ground` match that
  exists to hold the next one.
- **A trade room is indoors**, so every leg ends with an explicit walk back out. C and D both recorded
  that rule and it caught this workstream anyway: the first `postgame-mr-mime.bin` was saved inside
  `Route2TradeHouse`, and the next leg's `Fly` was refused with the whole queue then discarded.

**One thing worth knowing before catching anything:** the bag had **no Poké Balls**. G7 banked the
whole Great Ball stack to free bag slots, and `CatchPokemon` gives up immediately without one —
`"[policy] want to catch a Abra, but no Pokéballs left!"`, then the leg carries on to the trade and
fails there instead. `trade_steps` withdraws them at the same PC it banks the party at, passing
`u8::MAX` and letting `ItemPcState::new`'s clamp resolve the count. Nine balls covered four catches.

**Output fixture `postgame-tangela.bin`** — Cinnabar Island, **dex 19 owned / 113 seen**, party
Venusaur / Articuno / Vaporeon / **Tangela**, eight mons in box 1, ¥44,564, six Great Balls.

**What is left of the nine, and why I stopped at four.** The five untouched trades are not blocked,
they are *expensive*, and the cost is the give-species rather than the trade:

| Trade | What it needs |
|---|---|
| Nidorino → Nidorina | a second Nidoran♂ levelled to 16, or the Safari Zone (**E**) |
| Poliwhirl → Jynx | Poliwag by fishing (**C**'s `fish` step), then level 25 |
| Slowbro → Lickitung | Slowpoke to level **37** — and trading it away costs the party its only **Dig**, since TM28 is spent |
| Raichu → Electrode | a Pikachu *and* a Thunder Stone |
| Ponyta → Seel | the Pokémon Mansion's wild table, i.e. a maze **D** already has routes for |

Each is a levelling or navigation errand attached to a mechanic this workstream has already proved
four times, so §1's target — *mechanism* coverage — is met. Whoever wants the dex entries should take
**E (Safari)** first: it supplies Nidorino directly and is a whole unclaimed mechanic.
**Evidence:** `pokemon::integration_tests::postgame::trades::*` — 4 slow-tier legs, **~55 s of wall
clock**, plus 2 `#[ignore]`d probes and 2 default-tier ROM-pinned unit tests. Full `slow-tests` tier
**910 passed, 0 failed**, 18 pre-existing ignores; default tier **841 passed**.
`git status src/pokemon/data/` shows only the new files — no drift. ROM:
`pokered/engine/events/in_game_trades.asm`, `pokered/data/events/trades.asm`,
`pokered/scripts/{Route2TradeHouse,VermilionTradeHouse,UndergroundPathRoute5,CinnabarLabTradeRoom,CeruleanTradeHouse,Route11Gate2F,Route18Gate2F}.asm`,
`pokered/data/wild/maps/Route15.asm`.
**Impact on others:** **E (Safari)** is now the highest-value unclaimed row — it is a whole mechanic
*and* it unlocks the Nidorino trade. **H**: dex owned is **19**, so Flash (10) is long past and the
Itemfinder's gate of 30 is eleven species away — reachable, where before it was not.

### [2026-07-31] H — H1/H2 done, **H3–H5 are a catching problem, not a mechanism one**
**Status:** verified ✅ (H1, H2) / blocked 🟡 (H3–H5)
**What the plan said:** §6-H — three items gated on dex owned, *"check the gate with the probe before
travelling — don't guess"*, and H2's observable as *"a dark cave renders lit"*.
**What is actually true:** the advice about checking the gate is right and the gate is the *only*
thing that was ever in the way. With 19 owned the aide hands HM05 over on sight, and his own text
prints the count back — *"Great! You have caught 19 kinds of POKéMON!"* — which makes the probe's
number and the ROM's agree out loud.

**H2's observable turned out to be exactly readable**, which I had not expected from a phrase like
"renders lit". `wMapPalOffset` **is** the darkness: `home/overworld.asm:497-501` sets it to 6 on
entering `ROCK_TUNNEL_1F` specifically, and the Flash branch of the field-move menu
(`engine/menus/start_sub_menus.asm:183-191`) does nothing but clear it to 0 and print a text box. So
the test asserts the map is dark **on arrival** and lit after — the first half matters, or "lit"
proves nothing. Exposed as `GameState::map_is_dark`, and driven by a new `PolicyStep::UseFlash
{ slot }` shaped exactly like `UseStrength`: re-issued until the effect shows in RAM, popping
immediately on an already-lit map so it is safe to leave in a step list.

**⚠️ The trap in H2 is *which mon can hold Flash*.** Of everything this save has ever owned — Venusaur,
Articuno, Vaporeon, Lapras, Aerodactyl, Hitmonlee, Omanyte, Tangela, Farfetch'd, Nidoran♀ — exactly
**two** learn it: **Slowpoke** and **Mr. Mime**. Both were in the box by this point, so the leg
withdraws Slowpoke first (it is already the Strength/Dig holder, so the HMs stay together). A leg that
assumed the lead could take it would have failed inside `TeachMove` with no useful message. Two
smaller preconditions, both silent if missed: the bag was **20/20**, and `OaksAideScript` refuses a
full bag with a text box that reads exactly like success; and Route 2's gate is on the route's **north
half**, past the cut tree G-trades documents.

**Why H3–H5 are blocked, and what would unblock them.** Nothing about them is unimplemented — H3 and
H5 are the same aide script at different gates, and H4's hidden items are bg-event objects with an
existing reserved seam. They are blocked on **dex owned**:

| Sub-step | Gate | Held | Short by |
|---|---|---|---|
| H3 Itemfinder | 30 | 19 | **11 species** |
| H4 Hidden items | (needs H3) | — | — |
| H5 Exp.All | 50 | 19 | **31 species** |

That is a *catching* errand, and §1 rules out an exhaustive dex sweep — so the honest answer is that
H3–H5 wait for whoever raises the count as a side effect of their own workstream. **E (Safari Zone)**
is by far the cheapest source: its table alone holds Nidoran♂/♀, Nidorino, Nidorina, Parasect,
Venomoth, Exeggcute, Rhyhorn, Chansey, Scyther, Tauros, Dratini and Kangaskhan — more than the eleven
H3 needs, in one location, and it is a whole unclaimed mechanism besides. G-trades' remaining five
rows would add another handful. I have left H's row `🟡 blocked` with that written down rather than
starting a sweep the plan explicitly excludes.
**Evidence:** `pokemon::integration_tests::postgame::aides::can_get_hm05_and_light_rock_tunnel`
(~6 s); `pokered/scripts/Route2Gate.asm:9-27`; `pokered/home/overworld.asm:495-501`;
`pokered/engine/menus/start_sub_menus.asm:183-193`; `pokered/data/pokemon/base_stats/*.asm` tm/hm
lists for the Flash-compatibility sweep. Full `slow-tests` tier **911 passed, 0 failed**, 18
pre-existing ignores; default tier **841 passed**. No fixture drift.
**Impact on others:** **E** — taking it now pays for H3 and H4 as a side effect, which is the
strongest argument for that row being next. `GameState::map_is_dark` and `PolicyStep::UseFlash` are
available to anyone; the only other dark map worth them is Rock Tunnel B1F.

### [2026-07-31] E — the agent **could not press BALL**, and the Safari's water is a wall
**Status:** corrected ❗ — two shared-file fixes, both of which cost a run before they were found
**What the plan said:** §6-E2 — *"`BattleAction::SafariBall/Bait/Rock` already exist and are already
offered — write a real catch policy."* True as far as it goes, and it hides both of these.

**1. `SafariBall` was offered, selectable, and unpressable.** `battle_options` has emitted the four
Safari actions since before this plan, and `menu.rs` maps each to its 2×2 cursor position, so
everything looked wired. But the battle executor confirms a chosen menu entry in two different ways:
a *sub-menu* target (MoveList/ItemList/PokemonList) is handed to `WaitingForMenu`, which presses A on
its own list, while a *terminal* target has to be pressed right there. The only terminal option in the
match was `Run` (`agent.rs`, the `BattleState::Navigating` arm) — because RUN is the only Safari
option any leg had ever chosen.

The failure is silent and self-concealing: the Safari menu **opens on BALL**, so `menu_state ==
menu_target` on the very first tick, the executor hands off, `WaitingForMenu` sees the main menu again
and bounces straight back to the policy, which picks BALL again. No error, no wrong button, no
movement — an encounter that lasts until the enemy flees. All three throwables are terminal (BALL,
BAIT and ROCK all resolve the turn on the spot), so the fix is to match all four.

**2. Surf is refused in the Safari Zone, and the BFS did not know.** The four areas' tileset is
`FOREST`, which `TilePairCollisionsWater` gives two rules for — `db FOREST, $14, $2E` and
`db FOREST, $48, $2E` (`data/tilesets/pair_collision_tile_ids.asm:20-23`) — so mounting Surf from the
bank answers **"No SURFing here!"** and nothing happens. With `can_surf` left true the BFS treats the
centre's pond as pass-through, and this is the part worth remembering: **a route across the water can
*tie* with the route around it.** The nearest grass from the entrance is (13,22), five steps away
either through (14,22) — water — or around it. The BFS picked the water, the mount was refused, the
policy re-issued the same walk, and the leg timed out 90 emulated minutes later having never left the
entrance. `GameState::can_use_surf` now excludes the four areas, exactly as B excluded the Cycling
Road, and for the same reason.

That second one is also an argument for the traps memo's six-line probe. `probe_safari_centre_from_the
_entrance` prints the action list *and* an ASCII meta-tile grid, and it answered "is the grass on our
side of the water" in 3 seconds against a 3½-minute timeout — it is left `#[ignore]`d in the tree.
**Evidence:** `pokemon::integration_tests::postgame::safari::{probe_safari_centre_from_the_entrance,
can_catch_a_safari_exclusive}`; the failure artifact's screenshot
(`target/test-artifacts/test_timeout_screenshot.png`) reading *"No SURFing on Celina here!"*;
`pokered/engine/items/item_effects.asm:669-680` (`ItemUseSurfboard` → `SurfingAttemptFailed`);
`pokered/data/tilesets/pair_collision_tile_ids.asm:20-23`.
**Impact on others:** **anyone driving a Safari battle** — the executor fix is what makes BALL/BAIT/
ROCK usable at all. **Anyone routing on a map with water and Surf in the party**: a tie between a wet
route and a dry one is resolved by BFS insertion order, not by preference, so a map where Surf is
refused needs `can_surf` false rather than a hope that the dry route is shorter. Fishing's
`route_stays_on_land` is the other half of this lesson, arrived at from the other direction.

### [2026-07-31] E — **BAIT and ROCK are never worth throwing**, and it is not close
**Status:** corrected ❗
**What the plan said:** §6-E2 — *"Rock raises catch rate *and* flee rate; Bait does the inverse"* —
presented as the trade-off a real catch policy would have to weigh. And §11's D entry expected them to
matter here: *"Bait/Rock and the HP term matter far more there than they do here."*
**What is actually true:** both descriptions are accurate and the conclusion drawn from them is wrong.
The two throwables lose to a plain ball against every species in the zone, including the ones with the
most to gain from them, because of two asymmetries the one-line description hides:

- **Both effects decay, and can decay to nothing immediately.** `PrintSafariZoneBattleText`
  (`engine/battle/safari_zone.asm:1-14`) decrements the live counter once per turn **before** the flee
  check in `core.asm:186-199` reads it. The counter is rolled uniform on 1..=5, so a throw that rolls
  a 1 buys *zero* protected turns, and the expectation is about two.
- **Bait's penalty is permanent; its benefit is not.** Only the *escape* counter's expiry reloads
  `wMonHCatchRate` into `wEnemyMonActualCatchRate`. A bait halves the catch rate for the rest of the
  encounter and then stops protecting.

Worked exactly — every branch of the ROM's turn, not a simulation — in
`postgame::safari::tests::bait_and_rock_are_never_worth_throwing`, against a lv23 **Chansey** (catch
rate 30, Speed stat ~35), the zone's hardest catch and a middling runner, i.e. the species with the
most to gain either way:

| opening | per encounter |
|---|---|
| balls only | **21.3 %** |
| bait, then balls | 13.1 % |
| rock, then balls | 11.9 % |

Rock loses even where it looks strongest (a slow Exeggcute whose doubled rate saturates the ball's
first roll): 54.6 % against 35.2 %. So `pick_battle_action` throws balls and nothing else, and the two
`BattleAction`s stay in `battle_options` for an LLM policy to reach for.

Two numbers worth carrying, both from `ItemUseBall` with the Safari branch's constants:

- A Safari Ball shares the **Ultra Ball's** `[0,150]` rejection range, so its first roll is
  `(catch_rate + 1)/151`, not `/256`.
- At **full HP** the second roll is a constant: `X = ((MaxHP·255)/12)/(MaxHP/4) = 85`, i.e. 86/256 ≈
  **33.6 %**, whatever the species. There is no weakening pass to write here even in principle — the
  player has no moves — and the flat 33.6 % is why the *catch rate* is the only thing that varies.
**Evidence:** `pokered/engine/battle/safari_zone.asm`; `pokered/engine/battle/core.asm:181-207`;
`pokered/engine/items/item_effects.asm:104-300` (`ItemUseBall`), `:1433-1480` (`ItemUseBait`,
`ItemUseRock`, `BaitRockCommon`); `postgame::safari::tests::{bait_and_rock_are_never_worth_throwing,
rock_loses_even_where_it_looks_strongest, a_full_hp_throw_collapses_to_the_ball_range}` (default tier).
**Impact on others:** the D entry's aside about Bait/Rock mattering in the Safari is superseded. Its
*other* conclusion still holds and generalises: on a one-shot encounter, weakening is a trap.

### [2026-07-31] E — a trip is **502 steps**, and the ejection has a gap that costs ¥500
**Status:** corrected ❗ / verified ✅
**What the plan said:** §6-E1 — *"the **500-step** counter and the ejection back to the gate"*.
**What is actually true:** the gate writes `HIGH(502)`/`LOW(502)` into `wSafariSteps`
(`scripts/SafariZoneGate.asm:189-192`) — the counter is **502**, and the signs in the game are the
ones rounding. Not that the two matter apart, but the first value a test can *observe* is **500**,
because paying ends in a scripted three-tile auto-walk north and those tiles are charged like any
others. A test asserting 502 on arrival fails against a perfectly correct read.

Three more facts about the trip model, all of which the driver depends on:

- **`wSafariSteps` is big-endian** (`ld a, HIGH(502)` into the low address), unlike most 16-bit WRAM
  in this game. Read it with `read_pointer_u16_be`.
- **Running out of *balls* ends the trip too**, on the same code path as running out of steps
  (`SafariZoneCheck` → `SafariZoneGameOver`), so a hunt that throws freely gets ejected early rather
  than standing around with an empty pocket.
- ⚠️ **Ejection is not instantaneous, and the gap re-pays.** `SafariZoneGameOver` warps the player to
  the gate and sets `EVENT_SAFARI_GAME_OVER`, but `EVENT_IN_SAFARI_ZONE` stays set until the gate
  script's `CheckAndResetEvent` runs a few ticks later. In that window the player is standing on the
  gate mat with the trip over while the state still reads "inside" — and a hunt that routes back
  toward the zone there walks straight into the join prompt and pays another ¥500, blowing its trip
  budget silently. `safari::pick` therefore treats an ejected trip as *outside* and issues nothing
  until the script lands. This is why the reader is keyed on the **event** rather than on `wCurMap`:
  the map says "gate" a few ticks before the game agrees the trip is over.

The happy consequence of all this is that **a trip is re-entrant and cheap**: being ejected is the
ordinary end of a trip, not a failure, so one `SafariHunt` step spans as many ¥500 entries as its
`max_trips` allows and the policy never has to model the ejection as an error path.
**Evidence:** `pokered/scripts/SafariZoneGate.asm:150-230`;
`pokered/engine/events/hidden_objects/safari_game.asm`;
`pokemon::integration_tests::postgame::safari::runs_the_step_budget_down_and_is_ejected` (~25 s, one
whole trip spent deliberately), whose log shows the PA announcement, the warp, "Did you get a good
haul?", and the hunt stopping *without* a second payment.
**Impact on others:** **H3/H4** — this is the mechanism that makes a multi-trip catching leg possible
at all. Anyone reading `GameState::safari`: `Some` does not mean "still hunting", check `game_over`.

### [2026-07-31] E — the **boxed-catch wedge is real, reproduced, and fixed**; it was START at an A prompt
**Status:** corrected ❗ — a shared-file fix that unblocks every workstream, not just this one
**What the plan said:** D's §11 entry: *"with **six in the party** a caught Pokémon is sent to the box,
and the nickname screen on *that* path **wedges the agent** … The boxed-catch nickname screen is a
real, unfixed bug — worth its own look for **E** (Safari), which will fill the party fast."* G-gifts
then could not reproduce it for a boxed *gift* and narrowed it to "whatever D hit is narrower than the
boxed naming screen".
**What is actually true:** D's report is exactly right, E reproduced it on the first sweep — 20 emulated
hours burned on `name:Parasect`, from a party of six — and the cause is one line in the naming driver.

The failure artifact is the whole diagnosis. The screenshot is not the naming grid at all: it reads
**"Leslee was transferred to"** with the ▼ prompt arrow. The name had been accepted; what the agent was
stuck on was the text box *after* it. `AgentState::NamingPokemon { decided: true }` pulses **START**
until the game leaves the naming/battle modes — correct for the grid, which START submits, and a
deadlock afterwards, because `WaitForTextScrollButtonPress` accepts only **A or B**. The flag the
driver waits on cannot clear until it presses something the game is listening for. Exactly C's fishing
animation, one screen along.

Why a gift never hit it: `_GivePokemon`'s box branch ends at `AskName`, while `SendNewMonToBox` — the
**catch** path — prints its transfer text *after* naming (`item_effects.asm:2648-2733`). Same screen,
different tail.

**The fix is to press A once the grid is gone**, and the discriminator is not `game_mode`:
`write_naming_screen_buffer` fills `wStringBuffer`, and the strict NamingScreen detection tests for an
*empty* buffer, so from `decided: true` onward the mode reads `TextBox` both while the grid is up and
long after. `wNamingScreenSubmitName` has no such ambiguity — 0 while the grid waits, 1 the moment it
is taken. START until then, A after.

Two consequences worth taking:

- **The "leave a party slot free before a catch you care about" rule is retired.** It was a workaround
  for this bug. C banked its party down to four before fishing and D banked Moltres at the Cerulean PC
  for the same reason; neither needs to now. E's sweep catches twelve species into a full party.
- `can_catch_a_safari_exclusive` is the regression test: it runs at party 5, so the first catch fills
  the party and the second takes the box path, and it asserts the box grew by one.
**Evidence:** the timeout artifact's screenshot; `pokemon::integration_tests::postgame::safari::
can_catch_a_safari_exclusive` (10 s, fails without the fix); `pokered/engine/items/item_effects.asm:
2648-2733`; `pokered/engine/events/give_pokemon.asm`; `pokered/home/text.asm`
(`WaitForTextScrollButtonPress`).
**Impact on others:** **everyone who catches or is given a Pokémon.** The box path is now drivable, so
a full party is no longer a reason to detour to a PC.

### [2026-07-31] E — **COMPLETE**. The Safari Zone is a **chain**, not a hub, and that is most of the work
**Status:** verified ✅ (with three corrections ❗)
**What the plan said:** §6-E's four sub-steps, and §9's *"Entry fixture `postgame-phase0.bin`"*.
**What is actually true:** all four, plus an **E5** the plan did not have (the four-area sweep), in
**~7 minutes of wall clock across three legs** — 10 s for the mechanism, 25 s for the ejection, 6.5 min
for the sweep. Rooted on **`postgame-flash.bin`**, H's output, for the reason C and F give: the dex
count is the thing E is for, and starting from the chain head means the 19 already owned are not
re-caught. ⚠️ It is saved **inside Rock Tunnel**, so every leg opens with a `Dig` off Slowpoke.

**Output `postgame-safari.bin` — Fuchsia City, dex 31 owned / 116 seen, party 6, box 18 of 20,
¥35,564.** Twelve new species in 21 paid trips: Rhyhorn, Exeggcute, Nidorino, Nidorina, Parasect,
**Scyther** (centre), Doduo, **Kangaskhan** (east), Paras, Venomoth, **Chansey** (north), **Tauros**
(west). The four in bold exist nowhere else on a single Red cartridge.

**The three corrections, in order of how much they cost:**

1. **The zone is a chain — Gate ↔ Centre ↔ East ↔ North ↔ West — and only once water is a wall.** With
   `can_surf` true the centre's action list offers all three areas and the map reads as a hub; on foot
   the pond cuts it in two and **only the east warp is reachable** from the entrance side. Worse, the
   *far* side of the pond is a genuinely separate region: the west's own shortcut warp back to the
   centre lands at (0,10) over there, and from that landing the gate **cannot be reached at all**. So
   the way out is the way in, reversed — which is what the pre-existing `safari_zone_strength_steps`
   does by hand, and why `PolicyStep::SafariExit` exists rather than an `enter(gate)`.
2. **Which of the north's four west-warps you take decides whether the hunt works.**
   `safari_zone_surf_steps` pins the western pair (landing (21,0)) because that is the Gold Teeth and
   Secret House plateau. A *hunt* wants the eastern pair (26,0) for the same reason that leg avoided
   it: one-way ledges seal the shelves off from each other and **all the west's grass is on the eastern
   one**. `probe_safari_areas` prints `grass: None` from (21,0) against `grass: Some(((6,20), 44))`
   from (26,0). A hunt on the plateau does not fail — it stands still, and a wait for grass has to be
   *bounded* for exactly that reason: the trip's step counter only moves when the player walks, so the
   budget that bounds every other case never runs down.
3. **`route_toward` cannot recover from an ejection.** The world graph is keyed by *(map, entry
   position)* and its nodes come from walking; being ejected is a **warp** onto the gate's third mat, a
   node no walk ever created, so from there the graph offers no path anywhere. A sweep lost four
   species to `no route from SafariZoneGate to SafariZoneEast` — two warps apart, both walked minutes
   earlier. The chain does not need a graph, and a route that reads as absent for a few ticks after a
   warp needs the same patience `CatchPokemon` gives it.

**What the sweep actually costs, since the plan will want it again:** the rare targets are all in
**4.3 %** encounter slots and are 18–22 % per encounter, so each costs 3–10 trips. Centre 10 (Scyther),
east 1, north 7 (Chansey), west 3 (Tauros) — and hunting each species in the area where its slot is
fattest is the difference between that and an hour: Chansey is 4.3 % in the north and 1.2 % in the
centre, Tauros 4.3 % west and 1.2 % north, Kangaskhan 4.3 % east and 1.2 % west. [`safari::grounds`]
records the assignment.

**Two things the next agent should know about the output fixture.** The box is **18 of 20** and the
party is full, so the next catching leg needs `change_box` or a `release`. And the ROM's own ceiling is
now close: 31 of the 125 obtainable species, with the Safari's table spent.
**Evidence:** `pokemon::integration_tests::postgame::safari::*` — 3 slow-tier legs and 2 `#[ignore]`d
probes; `pokemon::postgame::safari::tests::*` (5, default tier). Full `slow-tests` tier **919 passed,
0 failed**, 20 pre-existing ignores; default tier **846 passed**. `git status src/pokemon/data/` shows
only the one new fixture — no drift.
ROM: `pokered/data/wild/maps/SafariZone*.asm`, `pokered/data/wild/probabilities.asm`,
`pokered/data/maps/objects/SafariZone{North,West}.asm`, `pokered/scripts/SafariZoneGate.asm`.
**Impact on others:** **H3 and H4 are unblocked** — 31 owned against the Itemfinder's gate of 30, and
H4's hidden items need only H3. H5 wants 50 and is still 19 away, which is now a *catching* errand with
no cheap source left. **G5/G6**: the remaining Nidorino→Nidorina trade's give-species is in the box.
And the boxed-catch fix in the entry above is the one every workstream should notice.

### [2026-08-03] H / H3+H4 — a hidden item needs **no Itemfinder** and no driver; the seams are deleted
**Status:** corrected ❗ (H3 verified ✅ exactly as written)

**What the plan said:** §6-H orders the three aides H3 → H4 → H5 and §9 records *"H4 needs H3's
Itemfinder"*. §6-H4 asks for `PolicyStep::SearchHiddenItem { at }`, and task 0.8 duly reserved a
`FieldMove::SearchHiddenItem` and an `AgentState::SearchingHiddenItem` to drive it.

**What is actually true:**

1. **The Itemfinder does nothing for H4.** `engine/events/hidden_items.asm` `HiddenItems` never looks
   at the bag: it matches the tile in front of the player against `HiddenItemCoords`, tests the flag,
   and gives the item. The Itemfinder is a *detector* — it reports whether one is within range and,
   as its own description says, "can't pinpoint it". `can_collect_a_hidden_item` would pass without
   H3. H3 still belongs first, but for a different reason: it is what leaves a **free bag slot**, and
   `FoundHiddenItemText` on a full bag prints "found ESCAPE ROPE!", fails `GiveItem`, and leaves the
   flag unset — so the pick-up silently does not happen.
2. **There was nothing to drive.** A hidden item is `hidden_object`, dispatched by
   `CheckForHiddenObject` when A is pressed and `CheckIfCoordsInFrontOfPlayerMatch` succeeds — the
   same path as a Vermilion Gym trash can and a Mansion statue switch. So it is
   `FieldMove::CheckTrashCan { target, facing: None }` and nothing else. **Both reserved seams are
   deleted** (`FieldMove::SearchHiddenItem`, `AgentState::SearchingHiddenItem`), exactly as
   `postgame::trades` deleted `TradePokemon`. `CheckTrashCan` now has three riders and its doc comment
   says so; it is misnamed, not miswritten.
3. **`PolicyStep::SearchHiddenItem` takes `{ map, item }`, not `{ at }`** — and the reason is the trap
   below.

**⚠️ The coordinate trap, which is silent.** `HiddenItemCoords` stores **raw** map coordinates.
Outdoor maps are widened by a strip for every neighbour they connect to, so on Route 11 (`WEST | EAST`)
raw (48,5) is tile **(49,5)** to the agent — and on a `NORTH`-connected map the y would shift too.
Hand-copying a raw coordinate into a step does not error: the agent walks to a plausible tile, presses
A, and nothing at all happens. The offset is therefore applied in the one place the other raw tables
(Strength switches, floor holes) are already corrected, `MetaTileMap::new`, into a new
**`MetaTileMap::hidden_items: Vec<(Point8, ItemId)>`** — so a policy *discovers* hidden items on the
current map rather than being told where they are.

**All 54 are ROM-derived, not transcribed.** `postgame::aides::hidden_items(map)` crosses the two
tables that each hold half the answer: the per-map lists in `data/events/hidden_objects.asm` hold the
**item id** but mix hidden items in with bookshelves, PCs, gym statues and the Safari scripts (the
discriminator is the object's routine pointer being `HiddenItems`), while
`data/events/hidden_item_coords.asm` holds only map/x/y — but its **row order is the flag numbering**
in `wObtainedHiddenItemsFlags`. Three unit tests pin it both ways.

**Two smaller corrections.** §6-H's table says the Itemfinder is at `Route11Gate2F`, which is right,
but the gate is a building sitting *inside Route 11* — its west doors are Route 11 (49,8)/(49,9) and
its east doors (58,8)/(58,9), both on the same map — so the whole errand stays on the Vermilion side
and never touches Route 12. And all four of `Route11Gate1F`'s doors are `LAST_MAP` warps, two of which
come out on the **east** side of that building, walled off from both Vermilion and the hidden item:
name the landing with `enter_at(Route11, 50, 8)` rather than letting `enter` choose.

**Evidence:** `pokered/engine/events/hidden_items.asm`, `engine/overworld/hidden_objects.asm:89`
(`CheckIfCoordsInFrontOfPlayerMatch`), `data/events/hidden_objects.asm:620` (Route 11's Escape Rope),
`data/maps/objects/Route11{,Gate1F}.asm`, `scripts/Route11Gate2F.asm`. Tests
`postgame::aides::{can_get_the_itemfinder, can_collect_a_hidden_item}` and the three
`postgame::aides::tests::*` pinning tests; probe `probe_route11_hidden_item` prints the offset
directly. Fixtures `postgame-itemfinder.bin`, `postgame-hidden-item.bin`; the whole `slow-tests` tier
is green (94 passed) with the `MetaTileMap` field added.

**Impact on others:** **Anyone routing off a raw pokered coordinate on an outdoor map** — the
connection strip is a silent one-tile lie, and `MetaTileMap::new` is where it gets corrected.
`MetaTileMap::hidden_items` is free to any policy now, including 4 Nuggets, 4 Rare Candies, 5 PP Ups
and 4 Ultra Balls nobody has picked up. **H5** is unaffected and still wants 50 owned (31 today).

### [2026-08-03] H / H5 — two silent freezes on the way to a dex sweep, and where route contents really come from
**Status:** corrected ❗

**What the plan said:** §6-H5 is one line — *"Exp.All at 50 owned"* — and §3 ruled an exhaustive dex
sweep out of scope. Both still stand; what neither says is that getting from 31 to 50 needs a **place
to stand**, and picking those places off a walkthrough is how you end up hunting a 1.2 % slot.

**What is actually true:**

**1. The encounter tables are in the ROM, so read them.** `crate::pokemon::wild` decodes
`WildDataPointers` (bank 3): per map, a grass block and a water block of ten `(level, species)` slots
each, weighted by the **cumulative** `WildMonEncounterSlotChances`. Two things fall straight out that
are not obvious and that change which grounds are worth visiting:

- **Slot count is not rarity.** Route 11's Drowzee holds four of the ten slots and is its *rarest*
  species (25 %); its Ekans holds three and is its commonest (40 %). Only the weights say so.
- **A zero rate omits its ten slots entirely** (`macros/asserts.asm` asserts it), so the water block is
  not at a fixed offset. Reading it as one would silently parse twenty bytes of the next map's table.

With that, the shortlist writes itself: Diglett's Cave is **94.5 % Diglett**, Rock Tunnel is 55/25/15
Zubat/Geodude/Machop at catch rates 255/255/180, and the Pokémon Mansion is four species at 190. It
also settles the ball question — the party is level 71 and the targets are level 3–24, so the policy
never weakens anything and every throw is at full HP, where a Poké Ball's second roll is 86/256 against
a Great Ball's 128/256. **1.5× the catch for 3× the price**: buy Poké Balls.

**2. ⚠️ A trainer's Pokémon can be an unowned species — and a ball thrown at one is an infinite loop.**
The game answers with "the TRAINER blocked the BALL!" and consumes no turn, so a catch policy keyed on
species alone re-picks the throw forever. `CatchPokemon` was safe only because it names one species;
the moment the test became "anything the dex is missing", the first trainer whose line of sight crossed
the grass killed the run. `pick_battle_action`'s throw block now guards on `BattleType::Wild`.

**3. ⚠️ `adjacent_grass` did not check `pair_blocked`, and the failure is completely silent.** The
pacing driver alternates between two tiles to farm per-step encounters. If the second tile is across a
**tile-pair collision** — a ledge, an elevation boundary — the move is illegal, so the player presses
into it forever: no step, no encounter roll, no error, and no agent state change to log. It looks
exactly like "this route just has a low encounter rate". Route 11's western grass runs along a ledge
row, which is where it surfaced; the artifact is a screenshot of the player standing in grass doing
nothing. Its sibling `adjacent_pacing_pair` had the check from the start. Fixed, plus a fallback to a
plain-tile pair when no *steppable* grass neighbour exists — the ROM rolls on the tile being stepped
**onto**, so a grass↔plain pair still fires on every other step, which is infinitely better than never.

**Evidence:** `pokered/data/wild/{grass_water.asm,probabilities.asm}`, `macros/asserts.asm:140`,
`engine/battle/wild_encounters.asm:56`. Tests `pokemon::wild::tests::*` (four, including a water-only
map and an indoor map). Diagnostics `probe_dex_sweep_candidates` (every unowned species, its best map
and share) and `probe_sweep_pacing` (position per tick — the only way to tell "not encountering" from
"not moving"). The pacing freeze cost two full 300-emulated-minute runs before the probe named it.

**Impact on others:** **anything that paces for encounters** — `CatchPokemon`, `GrindUntilLevel`, E's
Safari hunt — was one ledge away from the same silent freeze. And `crate::pokemon::wild` is free to
anyone: it is the honest answer to "where do I catch X", and it is what a text interface for an LLM
agent would want to expose.

### [2026-08-03] H / H5 — **COMPLETE**, and the two traps that cost the most were both invisible
**Status:** verified ✅ (with three corrections ❗ to grounds §6-H5 named)

**Result.** Dex **31 → 52 owned**, Exp.All collected, `postgame-aides.bin` committed. Four legs,
~110 s of wall clock together once the routes were right — the cost was never the emulation, it was
finding out which grounds actually work.

**⚠️ The Silph Scope must be in the *bag* for Pokémon Tower, and nothing says otherwise.**
`IsGhostBattle` (`engine/battle/core.asm:3308`) turns every wild on `POKEMON_TOWER_1F..7F` into an
uncatchable **GHOST** unless `IsItemInBag SILPH_SCOPE` succeeds — and **Phase 0 banked the Scope** to
free bag space, so every chain rooted on `postgame-phase0.bin` arrives without it. The failure is
perfectly quiet: encounters happen, balls are selected, thrown and *consumed* exactly as normal, and
simply never catch. It burned **71 balls and 400 emulated minutes**, and what finally named it was
counting nickname prompts — 12,137 ball-throw ticks and **zero** `name:` lines. Withdraw the Scope
before the tower; H5c does.

**⚠️ An indoor sweep collects item balls by accident.** With no grass, `SweepDex` wanders by walking to
the farthest reachable *sprite*, and arriving at a sprite presses A — so a floor with item balls on it
picks one up. Pokémon Mansion 1F did, which exactly cancelled the one bag slot H5d had freed, and Oak's
aide then refused the Exp.All for want of room. Free **two** slots before an aide if a sweep runs first.

**Three grounds §6-H5's shortlist wanted that do not work, all of them stalls rather than errors:**

- **Route 8** has grass in the ROM and **none the agent can reach**: from either end its action list is
  the two `Route8Gate` doors, the Underground Path and nine trainers, with no `Grass` at all. Its
  Mankey and Growlithe come from Route 7 instead. Same shape as the Route 15 trap already in §11.
- **Pokémon Tower 6F/7F** cannot be climbed on a save that has already *finished* the tower —
  `enter(PokemonTower6F)` finds no route from 5F. The mainline climb works only because it passes
  through while the Channelers are still being fought and collects the Rare Candy that unblocks 6F's
  chokepoint. 3F is 89.5 % Gastly, which is all H5 needed.
- **Rock Tunnel's Machop is 14.8 %, not 15 %.** A `min_share` of 15 silently drops it. Round down.

**What made the working grounds findable** is `crate::pokemon::wild` plus two diagnostics worth
keeping: `probe_dex_sweep_candidates` (every unowned species, the map where its share is fattest, and
every map ranked by how many it offers) and `probe_sweep_grounds` (does this ground offer *reachable*
grass — run it **before** adding a ground, not after the leg times out). `probe_timeout_artifact` and
`probe_stall_artifact` reload whatever `target/test-artifacts/` was left holding and ask the map the
same questions the driver did; between them they identified four of the five traps in one run each.

**Impact on others:** the four shared-file fixes in the entry above and this one — the trainer-ball
loop, `adjacent_grass`, the pacer's stall guard, and `MetaTile::Water` matching impassable tiles — are
all on paths every workstream uses. The whole `slow-tests` tier is green with them in.

### [2026-08-03] H — correction to the water fix above: **passability is the wrong test**, the tileset is
**Status:** corrected ❗ (correcting this session's own first attempt)

The entry above describes six phantom `MetaTile::Water` tiles in Viridian Forest. The first fix tried
was *"a tile is only water if it is also in the tileset's collision list"*, on the reasoning that real
water must be walkable or surfing could not cross it. **That is false, and it fails loudly**: Gen 1
routes movement on water through `CollisionCheckOnWater`, a separate path from the land collision list,
so tile `$14` is *not* in that list. Requiring it drops every genuine water tile in the game — measured,
**12 surf and fishing legs failed** (all four Cinnabar legs, all three fishing legs, all three
legendaries, Earth Badge, and the offline Seafoam check).

The correct fix is narrower and is about the **tileset**, not the tile. The over-match is only ever the
two *shore* ids `$32`/`$48`, which mean something else in some of the tilesets `WaterTilesets` lists —
`ShipPort` was already excluded for exactly this reason (there `$32` is dock planks). **`Forest` needs
the same exclusion**: its only map is Viridian Forest, which has no water at all and six impassable
bush blocks carrying those ids. Tile `$14` stays unconditional everywhere.

**Evidence:** `pokered/data/tilesets/water_tilesets.asm`, `engine/items/item_effects.asm`
(`IsNextTileShoreOrWater`). Viridian Forest reports **0** water tiles either way, so the narrow fix
solves the original problem completely; the whole `slow-tests` tier is green (98 passed) and the
default tier is green (853 passed).

**Impact on others:** if another tileset in `WaterTilesets` turns out to have no water, add it to the
same exclusion — do **not** reach for passability.

### [2026-08-03] D / D2a — the Electrode **Selfdestructs**, and there are only two of them
**Status:** blocked 🟡 (with one real bug fixed on the way, and D2a's route fully verified)

**What the plan said:** §6-D re-cut D2a as *"catch an Electrode — it is catch rate 60, i.e. an ordinary
catch"*, and made it the unblock for honest legendary fights.

**What is actually true: it is not an ordinary catch, because losing the fight destroys the target.**
The Power Plant's eight disguised Poké Balls are `trainer`-flagged objects exactly like the birds
(`data/maps/objects/PowerPlant.asm`), so `EndTrainerBattle` hides the object on every exit but a
blackout — the same rule the 2026-07-30 entry established for Moltres. And a lv43 Electrode's last four
learnt moves are **Sonicboom (17), Selfdestruct (22), Light Screen (29), Swift (40)**
(`data/pokemon/evos_moves.asm:1638`), so about a quarter of its turns end the encounter by killing
itself. Measured: **both** Electrodes Selfdestructed, on turns 3 and 2, and the floor was empty.

**The arithmetic, which is why this is not a tuning problem.** Electrode's base speed 140 means it acts
*before* the ball every turn. With `p` the per-ball catch chance and `q = 0.25` the Selfdestruct rate,
`P(catch) = 0.75p / (1 − 0.75(1 − p))`. At full HP the best ball is the **Great Ball** — first roll
`61/201 = 30 %`, second `128/256 = 50 %`, so `p = 15.2 %` (the Ultra Ball's better first roll `61/151`
loses to its worse second roll `86/256`; the Poké Ball is 8 %). That gives **31 % per Electrode, 52 %
across both.** This run lost both.

**What I would try next, in order.**

1. **Weaken to ≤25 % HP, then throw Ultra Balls.** Below a quarter HP the second roll saturates at
   `255/256` for *every* ball, so only the first roll matters and the Ultra Ball's `61/151` becomes the
   best in the game. That lifts `p` to ~40 % and the encounter to ~41 %, ~65 % across both. The blocker
   is that `pick_battle_action` skips weakening when the attacker out-levels the target by 12+ (a lv71
   Venusaur against a lv43 Electrode), and relaxing that guard touches the move-selection path every
   leg depends on. It is also only a coin flip, not a fix.
2. **A sleep move.** Sleep and freeze are worth −25 on the first roll *and* stop the target acting at
   all, which removes the Selfdestruct clock entirely — this is the only change that makes the catch
   near-certain rather than likelier. Nothing in the party has one and no mart sells one; the cheapest
   source is a **Venusaur that kept Sleep Powder**, or a caught sleeper (Butterfree/Gastly/Jynx).
   Worth checking against H5's chain, which now owns Gastly.
3. **Blacking out is the only recoverable loss** — so an attempt made with a party that *cannot win but
   can faint* keeps the Electrode alive for a second try. Perverse, but it is the ROM's own escape
   hatch and it is what makes retries possible at all.

**One real bug fixed on the way, and it is not D-specific.** `CatchPokemon`'s static-encounter branch
matched a map sprite's name against the species name **exactly**. A map with more than one of a species
numbers them — `Electrode 1`/`Electrode 2`, `Voltorb 1..6` — so the match found none of them and the
step fell through to pacing for a wild encounter on a floor that has none, i.e. a silent stall.
`sprite_is_species` now strips the number; pinned by
`policy::policy_helper_tests::numbered_static_encounters_match_their_species`.

**Everything except the fight is verified.** `PolicyStep::electrode_steps` reaches the Power Plant
(Cerulean's trashed-house bridge, Route 9's cut tree, the BFS Surfs to the door), engages both
Electrodes, and picks up TM45 afterwards — the leg only fails at `TeachMove`, for want of a live
Electrode. It is kept as `#[ignore]`d `postgame::legendaries::can_catch_an_electrode` rather than
deleted, because a future fix needs exactly this leg to test against. ⚠️ **Note the ordering it
establishes:** TM45 is one per cartridge and a Gen 1 TM is consumed on use, so the Electrode must be
caught **before** the TM is collected — D1a's Slowpoke version cannot simply be extended.

**Impact on others:** the `sprite_is_species` fix is on `CatchPokemon`'s shared path. D's three
legendaries remain caught-but-Master-Ball-seeded; the routes are honest and are what the tests prove.

### [2026-08-04] D / D2a — **sleep beats speed**, and TM45 was never the scarce thing
**Status:** corrected ❗ — this supersedes the *approach* in the entry above, not its measurements

Alex pushed back on the Electrode premise: *"isn't there another way to paralyse? and once we have
TM45 we can teach it to a Raichu, Zapdos, Electrode, Magneton — it doesn't have to be an Electrode."*
Checked against the ROM, and the premise was weaker than §6-D assumed in **two** ways.

**1. TM45 is not one-per-cartridge in any way that matters — Pikachu learns Thunder Wave at level 9.**
`data/pokemon/evos_moves.asm` gives it as a *level-up* move, so any caught Pikachu already has it or is
nine levels away. D1a's whole shape — collect the single TM, agonise over which party member is
compatible, never spend it twice — was built on a constraint that only ever applied to the *TM*. H5b
caught a Pikachu in Viridian Forest, so this chain already owns one, and Pikachu → **Raichu** (a ¥2,100
Thunder Stone at Celadon Mart 4F) is base speed 100 and keeps the move.

**2. Paralysis is the wrong status. Sleep is worth −25 on the catch roll against paralysis's −12, and
— the part that actually matters — a sleeping Pokémon does not act at all.** That is the whole
Selfdestruct problem and half the Fire Spin problem, gone. Stun Spore, which Alex suggested, is
paralysis and buys nothing over Thunder Wave; Victreebel is a Blue exclusive (§7); and Venusaur *does*
learn Sleep Powder — at **level 55** — but ours declined it at level-up and Gen 1 has no relearner.

**What this save actually owns**, cross-referenced from `base_stats/` and `evos_moves.asm`:

| species | base spd | TM45 | status move |
|---|---|---|---|
| **Parasect** | 30 | — | **Spore @30** (100 % accuracy sleep) — *already known, box 1* |
| Venomoth | 90 | — | Stun Spore @30, Sleep Powder @43 |
| Gastly | 80 | — | Hypnosis @27 (Haunter is base 95) |
| **Pikachu** | 90 | yes | **Thunder Wave @9** (Raichu base 100) |
| Chansey | 50 | yes | Sing @24 |
| Oddish | 30 | — | Stun Spore @17, Sleep Powder @19 |
| Electrode | 140 | yes | — |

**The revised D2a.** Keep the Electrode — it is still the fastest TM45 learner and needs no grind —
but stop trying to out-throw its Selfdestruct clock and **put it to sleep first**. Parasect's speed 30
means it acts second on turn 1, so the 25 % Selfdestruct risk survives for exactly one turn; after that
the Electrode cannot act, and the balls land against a −25 first roll. That moves the encounter from a
measured **31 %** to roughly **75 %**, and from 52 % to ~94 % across the two on the floor.

⚠️ **This re-roots D on the H chain.** The Parasect is in box 1 of `postgame-safari.bin` and everything
downstream of it; D's own root, `postgame-fly-bike.bin`, predates every catch. `postgame-aides.bin` is
the natural entry now that H is finished, and it also carries the Pikachu and the Gastly.

**What it does not fix.** Speed still decides *Moltres*, because Fire Spin cancels the trapped side's
move from the turn it lands — a sleeper that goes second can be locked out before it ever acts, exactly
as the slow Thunder Wave was. Approximate lv50 speeds (DV 8, no stat exp): Moltres **103**, Zapdos
**113**, Mewtwo at lv70 **198**; against Electrode-at-43 **132**, Raichu-at-50 **113**, Haunter 108,
Venomoth 103, Parasect-at-30 **27**. So Raichu and Haunter are viable against Moltres *after a grind*,
Electrode is viable immediately, and **nothing obtainable outspeeds Mewtwo** — that fight still has to
be built around surviving a hit rather than pre-empting one.

**Impact on others:** none outside D, but two facts are worth having anywhere: **Pikachu is a free
Thunder Wave**, and **sleep is the strongest catching status in the game** — `postgame::safari`'s
`ball_catch_chance` models the roll if anyone wants the arithmetic.

### [2026-08-04] D — **the honest-legendary arc is closed by decision**, and the two findings worth keeping
**Status:** closed 🛑 — a scope decision from Alex, not a blocker

**What the plan said:** D2a would catch a fast paralyser so the three legendary catches could stop being
Master-Ball-seeded, and the three entries above it (2026-07-30 *trapping moves*, 2026-08-03 *the
Electrode Selfdestructs*, 2026-08-04 *sleep beats speed*) each pushed that further.

**What is actually true: it was the wrong thing to be testing.** Alex's call, and it is now recorded in
[§3](#3-scope)'s "Out" table: *"we are not testing whether the agent has covered the game mechanics and
instead that our strategy in a deterministic policy is sound, which in deployment the LLM will take
care of."* Beating a catch-rate-3 encounter that must not be KO'd, fled or lost is battle tactics.
The **route** is the mechanism, the route is covered, and the Master Ball seed is the right answer to
the rest. ⚠️ **The three entries above are history, not a to-do list.** Do not resume them.

**What was reverted.** `PolicyStep::electrode_steps`, `can_catch_an_electrode`, `probe_power_plant`, and
a session's worth of unshipped work on top: a sleep-ranking rewrite of `pre_catch_action`
(`status_move_rank` reading `PokemonMoveMetadata`, re-applying sleep at `counter == 1`, keeping a
sleeper in rather than benching it, preferring a *healthy* statuser over a better-armed one), a
Voltorb-based `electrode_steps`, and the `postgame-electrode.bin` fixture. `pre_catch_action` itself
stays — it is what keeps the generic "weaken below 50 % first" branch away from a once-per-cartridge
encounter — as do D1a and the three catch legs, which are green and unchanged.

**One finding survives the revert, and one "finding" was wrong.**

1. **`CatchPokemon` matches on species alone, so a wild encounter can gatecrash a static one.** The
   Power Plant's disguised Poké-Ball Voltorbs are **lv40**; its *wild* Voltorbs are lv21/23 and fill two
   of ten encounter slots. One interrupted the walk to the object
   (`OverworldActionAborted { … reason: Battle }`), was caught exactly as asked, and left the leg
   holding a lv21 it had no use for. Anything that wants a *specific* static encounter needs to check
   the level, and note the asymmetry: fleeing a **wild** one is free, fleeing the **object** deletes it.
2. ❗ **"`UseRareCandy` only works on party slot 0" was a misdiagnosis — there is no such bug.** It was
   filed off a mis-read log: the leg showed a block of `teach:RareCandy` ticks and a run that ended
   ~518 k lines later having achieved nothing, and the two were assumed to be the same event. They were
   not. The candy chain ran for **447 ticks** — normal, and the log says
   *"Taught the HM to party slot 5"* followed by *"UseRareCandy: consumed — done"*. The 518 k lines were
   the idle loop *after* every step had finished, spinning out the budget because the test was waiting
   on a dex flag that could never arrive: the Voltorb it had candied was the **wild lv21** gatecrasher
   from finding 1, so lv21 + 1 = lv22 and it never reached its lv30 evolution. Everything worked; the
   wrong Pokémon was in the slot.

   Four probes were run before this was clear — slot 5, the candy mid-bag, the candy as the *last* bag
   row, and the step issued on residual text — and all four drove the chain correctly.
   `mechanics::rare_candy_works_on_a_late_party_slot` now pins it (~0.3 s, default tier) so the claim
   cannot be re-filed. ⚠️ The lesson is the cheap one: **a spinning log is not a wedge until you have
   counted the ticks**, and this file's own rule (§11: evidence over assertion) is what should have
   been applied to the first report.

**Impact on others:** none in code beyond one added test — the working tree is otherwise back to what
shipped. The default tier (855) and all four `postgame::legendaries` legs are green.

---

### [2026-08-04] J — battle animations off buys ~20–25 %, and the plan's option byte was wrong
**Status:** verified ✅ / corrected ❗
**What the plan said:** §8-J: set `wOptions` to **`0b1000_0001`** (text delay fast + bit 7 "animations
off"), apply it in the fixture loader, and leave battle style alone.
**What is actually true:** three corrections and one measurement.

1. ❗ **`0b1000_0001` is battle style SHIFT.** Bit 6 (`BIT_BATTLE_SHIFT`) *set* is SET, and the harness
   has written SET since long before this workstream — `TestFixture::new` already called
   `write_game_options(&GameOptions::default())`, and `GameOptions::default` is
   `{ animations on, style Set, text Fast }`. So the byte the plan asked for would have silently
   flipped the whole suite to SHIFT and reintroduced the "will you switch?" prompt every driver has
   been tuned against — which is the exact change §8-J's own last bullet forbids. The value shipped is
   **`0b1100_0001`**: the only bit J actually changes is bit 7.
2. **Only text speed was already being set, not the animations.** The pre-existing loader call was
   doing half of J's job and no one had noticed the other half.
3. ⚠️ **`wOptions` really does drift, twice, across the credits** — see J3 below.

**Evidence (J4, measured before/after, same machine, `--release`):**

| Workload | animations ON | animations OFF | saving |
|---|---|---|---|
| default tier (`cargo test --release`, 855 tests) | 19.88 s | 14.90 s | **25 %** |
| `postgame::aides::can_sweep_the_viridian_grounds` (wild-battle-heavy leg) | 38.51 s | 30.42 s | **21 %** |

Both numbers are the whole-process wall clock reported by `cargo test`. The saving is real and it
scales with how battle-heavy the workload is, which is what the two rows are there to show.

⚠️ **Turning animations off changes the RNG stream.** The same sweep leg reached the same dex count
(41 owned) but with **66 balls left** before and **52** after: fewer frames per battle means different
`hRandomAdd`/`hRandomSub` sampling, so encounters and catch rolls diverge. Nothing depends on the old
stream — fixture writes are gated behind `regen-fixtures` — but a leg regenerated after J will not be
byte-identical to one regenerated before it, and a leg whose assertions are tuned to an exact ball
count would now be wrong. None are.

**J3 — the drift is real and the per-tick re-apply is load-bearing.** `wOptions` ($D355) sits inside
`wMainDataStart` ($D2F7) `..wMainDataEnd`, the block `engine/menus/save.asm:220` copies to `sMainData`
on a save and `:64` copies back on CONTINUE. The Hall-of-Fame walk-out is a save → soft reset →
CONTINUE, and `postgame::phase0::options_survive_the_hall_of_fame_reset` measures **two** drifts across
it. Writing the SRAM copy instead is not the fix: `sMainDataCheckSum` (`save.asm:240`) is computed over
the whole block, so a poked byte fails the checksum and the game answers *"the file data is
destroyed."* `TestFixture::step` therefore re-applies every tick (a byte compare) and counts the
drifts, and the test asserts the count is **> 0** — without that assertion the test would keep passing
on a harness where the re-apply had become dead code.

**Impact on others:** every tier is ~20 % faster, including `full-playthrough`. `TestFixture` gained
one public field (`options_drifts`). `PokemonApi::debug_set_options` is the J1 entry point and
`postgame::debug::FAST_FIXTURE_OPTIONS` the value; the old `write_game_options` call in the loader is
gone. Default tier green at 855.

---

### [2026-08-04] K — a sixth trade, and the four "unproven" ones are cheaper than the plan says
**Status:** verified ✅ / corrected ❗
**What the plan said:** §8-K: prove **one** of the five unproven trades; Ponyta → Seel needs no
seeding because `postgame-aides.bin`'s box 3 slot 2 already holds a lv32 Ponyta; the give-species is
why the other four were skipped; ⚠️ "the Cinnabar Lab is the Gramps and the Beauty, not the Super
Nerd."

**What is actually true:**

1. ✅ **K1 ships.** `postgame::trades::can_trade_a_boxed_ponyta_for_a_seel` — Fuchsia PC, bank Rhyhorn
   (slot 5), withdraw the boxed Ponyta, Fly to Cinnabar, trade. **Dex 52 → 53**, ~5 s wall clock,
   output `postgame-seel.bin`. New constructor `PolicyStep::trade_boxed_steps`, which is
   `trade_steps` with the catch replaced by a deposit + withdraw; no new driver, no debug seeding.
2. ❗ **The Ponyta trader is in `CinnabarLabFossilRoom`, not `CinnabarLabTradeRoom`.** The plan's
   Gramps/Beauty warning is about the *other* room. `scripts/CinnabarLabFossilRoom.asm:102` sets
   `TRADE_FOR_SAILOR` from `CINNABARLABFOSSILROOM_SCIENTIST2` at (7,6) — the room where fossils are
   revived. `TRADES` already had this right; the prose did not. `to_trade_npc`/`out_of` gained the
   arm.
3. ❗ **A `PartyScript::Trade` does not fit in `TICK_BUDGET` (1200).** K1 first ran to
   *"party-script: nothing changed in 1200 ticks"* — and then **completed anyway**, because the
   generic A-mash inherited a conversation whose mon had already been chosen. Worked by fall-through,
   read as a failure in the log, and would have been a real failure if the fall-through had ever
   picked differently. `Baseline::SpeciesGone` cannot be met until the *end* of
   `InGameTrade_DoTrade` — after both cries, the transfer animation and four text boxes — and a
   six-mon party costs extra ticks getting the cursor there first. Fixed with a per-script
   `TRADE_TICK_BUDGET = 2400`; the Day Care and Name Rater keep 1200. All five trade legs green
   (21.97 s).
4. ❗ **K2: eight of the nine give-species are plain wild encounters**, not the evolution grinds the
   plan's framing implies. `postgame::trades::tests::every_trade_give_species_is_obtainable` (default
   tier, ROM-derived) prints the lot:

   | Give | Source |
   |---|---|
   | Nidorino | wild — `SafariZoneEast` |
   | Abra | wild — Route 24 |
   | Ponyta | wild — `PokemonMansion1F` |
   | Spearow | wild — Route 3 |
   | Slowbro | wild — `SeafoamIslandsB2F` |
   | **Poliwhirl** | **the only evolution** — Poliwag on the Super Rod (`data/wild/super_rod.asm:45,50`) at lv25 |
   | Raichu | wild — `CeruleanCaveB1F` |
   | Venonat | wild — Route 12 |
   | Nidoran♂ | wild — Route 22 |

   ⚠️ **Three of those were guessed wrong before the test was run.** Nidorino, Slowbro and Raichu were
   each written down as "obviously an evolution" (lv16, lv37, Thunder Stone) and the ROM has all three
   wild; the test failed three times in a row correcting them. §8-K's table can be trusted on *what*
   each trade wants, and should not be read as saying any of them needs a grind.

**Evidence:** `can_trade_a_boxed_ponyta_for_a_seel`, `every_trade_give_species_is_obtainable`,
`scripts/CinnabarLabFossilRoom.asm:102`, `data/maps/objects/CinnabarLabFossilRoom.asm`,
`data/wild/super_rod.asm:45`.
**Impact on others:** `gifts.rs` gained `TRADE_TICK_BUDGET` (behavioural, but only widens a wedge
detector). `trades.rs` gained `trade_boxed_steps` and two match arms. New fixture `postgame-seel.bin`.

---

### [2026-08-04] I — the item-use table is one driver and three ways to be silently wrong
**Status:** verified ✅ / corrected ❗
**What the plan said:** §8-I: seven sub-steps, all of I1/I2/I7 riding the existing START → ITEM → bag
→ USE chain, "the work is mostly the extra menu each item opens"; root at `postgame-aides.bin` and
"withdraw rather than buy".

**What is actually true.** The plan's shape was right and its *hard* part was somewhere else. One
`PolicyStep::UseBagItem { item, target }` and one driver (`postgame::items`) cover I1, I2, I5, I6 and
I7; `PolicyStep::UseItemsInBattle` covers I3 and I4. All seven ship green. What cost the time was
three menus that each **look** correct from outside and each fail without an error.

1. ❗ **`text_box_id` lingers, so the PP-restore move list reads as the bag.** `MoveSelectionMenu`
   never calls `DisplayTextBoxID` (`engine/battle/core.asm:2460+`), so `wTextBoxID` still says
   `ListMenuBox` from the bag underneath it. A driver that tests `text_box_id` first walks the move
   list toward a *bag row number* and never presses A.
2. ❗ **…and its geometry lingers too, so keying on that instead is worse.** `SelectMenuItem`
   **decrements** `wCurrentMenuItem` when the move is chosen (`core.asm:2623-2625`) and leaves
   `wTopMenuItemX/Y` at (5,7). A geometry-keyed branch sees the cursor drop 1 → 0 and spends the rest
   of the leg pressing **Down** at a "PP was restored." prompt that only takes A.
3. ❗ **And the prompt that *does* identify it is lower-case.** `_RaisePPWhichTechniqueText` is
   *"Raise PP of which technique?"* (`data/text/text_6.asm:130`); the upper-case `WhichTechniqueString`
   `SelectMenuItem` places is only for `wMoveMenuType == 1`, the **Mimic** menu. Matching `"TECHNIQUE"`
   never fires, the driver falls through to its trailing `A`, and A on the cursor's *starting* row
   picks **move slot 0** — which is indistinguishable from correct for as long as the caller only ever
   asks for slot 0. The Ether test passed on it. The PP Up test, asking for slot 1, put the PP Up on
   Solarbeam.

   ⚠️ All three share a failure signature: **the item is eventually used anyway**, by the generic
   A-mash after the driver's tick budget aborts — so the log claims a wedge that did not happen and
   the effect lands on the wrong target. §10's "detect menus by text, never geometry" needs a sibling:
   *and match the text the ROM actually prints.* `ITEMS_TRACE=1` now dumps every menu the chain walks.

4. ❗ **`PokemonMove::pp` is the raw PP byte.** `encoding.rs:52` reads it unmasked and the ROM packs
   the **PP Up count into bits 6–7**, so a PP-Upped move reads 64 higher than its PP and any naive
   `pp >= max` refuses every use on it. `items::move_pp` / `pp_ups` / `max_pp` are the accessors; bits
   6–7 moving is also the **only** observable a PP Up has.
5. ❗ **A PP Up raises the current PP as well as the maximum** — `.PPNotMaxedOut` calls
   `RestoreBonusPP` immediately (`item_effects.asm:2008`), bonus = `base / 5`. Razor Leaf went 25 → 30
   on *both*. The first draft of the assertion said the opposite.
6. ❗ **`USING_X_ACCURACY` is bit 0 of `wPlayerBattleStatus2`, not bit 6** (bit 6 is `USING_RAGE`) —
   `constants/battle_constants.asm:92`. The run reported `$07`, all three bits set, and failed anyway.
7. ✅ **Nothing needed debug seeding except the Ether**, and §8-I's "withdraw rather than buy" was
   beaten by "find it and buy it": the Repel is on Vermilion's shelf, all seven stat items and the
   Poké Doll are on Celadon Mart 5F and 4F (`data/items/marts.asm:29,32`), and the PP Up is **lying in
   Celadon's street** as an uncollected hidden item. No Kanto mart sells an Ether and every one on the
   floor is behind a trek, so that one is seeded and says so.
8. ✅ **The Itemfinder is provable, but only just.** `HiddenItemNear` skips anything already flagged
   and wants the item within x ± 5, y − 5..+ 4 (`engine/items/itemfinder.asm:11-41`). The Fly stop is
   outside every town's window; stepping in and out of `VermilionTradeHouse` (raw (15,13)) is inside
   it. And the item it points at — Vermilion's Max Ether at raw (14,11) — is **walled into a fence
   block with no adjacent standable tile**, so its flag can never be set and the test is stable rather
   than single-use. Cerulean's hidden Rare Candy is the same; Viridian's Potion and Celadon's PP Up
   are the reachable ones. Both texts are asserted: "Yes! ITEMFINDER indicates…" in Vermilion, "Nope!
   ITEMFINDER isn't responding" in Fuchsia, which has no hidden items at all.
9. ✅ **The Bicycle is a toggle**, so `Effect::TogglesBicycle` completes on "the mount state changed",
   not "we are on the bike" — with the latter a dismount step is satisfied before it starts and pops
   without pressing anything. Measured: Celadon → Route 7 is **25.3 s walked, 15.5 s cycled**, so
   §8-I6's "may pay for itself in emulated minutes" is true, ~39 %.
10. ✅ **The "no effect" guard works and is tested end to end.** `items::blocked` refuses a use the
    ROM would decline — a potion at full HP, a Revive on a living mon, an Ether on a full-PP move, the
    Bicycle where `IsBikeRidingAllowed` says no (decoded from `BikeRidingTilesets`, not transcribed).
    `can_revive_and_heal_a_party_member` hands it a Full Restore on a full-HP Vaporeon and asserts the
    queue drains *and* the item stays in the bag.

**Evidence:** `postgame::items` (6 legs, all green), `postgame::items::tests` (3 ROM-pinned unit
tests), and the fixtures `postgame-medicine.bin` → `postgame-finder.bin` → `postgame-ether.bin` →
`postgame-items.bin`.
**Impact on others:** `GameState` gained `repel_steps` and `on_bicycle`. `agent.rs`'s in-battle item
whitelist gained the seven stat items and the Poké Doll (without it the generic navigator backs out of
the bag on CANCEL and nothing is ever spent). `gifts.rs` is untouched. Two new `PolicyStep` variants,
one `FieldMove`, one `AgentState`.

---

### [2026-08-04] L — 220 maps audited statically, 96 toured, and eight kinds of door that do not open
**Status:** verified ✅ / corrected ❗
**What the plan said:** §8-L: ~220 visitable maps; L1 a static audit, L2 an emulated tour per Fly hub,
L3 the awkward set, L4 the report. "Expect this to be the most expensive workstream here in emulated
time."

**What is actually true.** It is the *cheapest* — the whole of L1 runs in 0.01 s in the default tier
and the four tours take 37 s of wall clock together. And it found eight distinct reasons a door does
not open, six of them by failing.

**L1 — the static audit** (`postgame::maps::tests`, default tier). 248 `Map` variants: **25
headerless**, 2 link-cable, 4 duplicates, **220 visitable**. Every one of the 220 has a readable
header, block data the size its header claims, a tile grid that builds, and a sprite table wherever
the ROM has object events. **801 warps** checked; every destination resolves. Two corrections:

1. ❗ **The plan's arithmetic double-counts.** Three of the four duplicate slots
   (`CeruleanTrashedHouseCopy`, `CinnabarMartCopy`, `UndergroundPathRoute6Copy`) have **no `*_h`
   label at all** — they are already in the headerless 25. Only `UndergroundPathRoute7Copy` is a
   headered map that has to be struck by name. Adding the four lists gives 251 for 248 maps, which is
   how this was found.
2. ❗ **`SilphCoElevator` has two warps to `UnusedMapEd`** and they are not broken — the floor menu
   rewrites the destination at runtime. Exactly the "missing metadata" L1 was written to catch, and
   the answer is to name it (`RUNTIME_REDIRECTED_WARPS`), not to widen the check.

**L2 — the tours** (4 tests, one per group of hubs). **96 rooms and roads entered** across the eleven
Fly stops, each checked for a non-empty action list *and an exit*. Design notes that cost time:

- ⚠️ **Tour by path, not by set.** The first version returned a flat room list and walked back to the
  hub with a plain transition between rooms. From Red's bedroom, "go to Pallet Town" is not a
  transition that exists, and the world-graph fallback only knows what the agent happens to have
  observed — two of Pallet Town's four rooms were skipped. A tour that knows how it got in knows how
  to get out.
- ⚠️ **Check the room's health on the *best* tick, not the first.** The meta-tile grid is briefly
  unsettled on arrival (§10), so a check on the tick the warp lands sees an empty action list in an
  ordinary Pokémon Center. That failed Viridian before it had walked a step.
- ⚠️ **Cut before every room, not once at the top.** Celadon's gym door is behind cuttable trees, and
  a cut tree **regrows** on every map reload while `PokemonAgent::cut_tiles` is **cleared on every map
  change** — so by the third building the trees are back and the agent has forgotten it ever cut them.
- ⚠️ **Roads last, and never followed.** With roads first, Cerulean reported nine unenterable
  buildings that are all perfectly enterable: the tour stepped onto a route and never got back.
- ⚠️ **`MAX_ENTER_WAIT` is *attempts*, and both 600 and 200 were wrong at opposite ends of the map.**
  Indoors a sealed door burns polls fast and two consecutive give-ups outlasted the harness's
  ten-minute stall window; outdoors one poll is a walk of several minutes, so 200 attempts is hours
  and Lavender ran out of cycle budget. 60.

**L3/L4 — the doors that do not open.** Eight distinct reasons, all ROM- or run-cited:

| Why | Maps |
|---|---|
| the S.S. Anne has sailed | the ship's 10 maps **and `VermilionDock` itself** — `VermilionCityDefaultScript` (`scripts/VermilionCity.asm:41-58`) intercepts the player facing **down** at `SSAnneTicketCheckCoords` and pushes them back once `EVENT_SS_ANNE_LEFT` is set |
| a one-way script-gated climb | `PokemonTower6F/7F` (H5c already recorded it) |
| needs another Champion run | `HallOfFame` |
| the hub map is cut into sealed regions | `CeruleanCave1F/2F/B1F`, **Route 4**, **Route 7** — the last two rediscovered independently what the archive already says about Cerulean's river and Saffron's ledge-sealed Route 7 pocket |
| only the elevator goes there | `CeladonMart4F/5F/Roof` — the ROM's warps make them one door from the lift; they are three flights of stairs from the street |
| behind a script-opened gate | `PokemonMansionB1F` |
| behind a receptionist who wants paying | `Museum2F` — *sometimes* enterable depending on whether the A-mash lands on YES, which is worse than never |
| the tour must not go in | the E4 rooms (the door seals behind you), the Safari areas (¥500 and the step counter), the three elevators (**you can get in and not get out** — Saffron's tour sat in `SilphCoElevator` and reported the Pokémon Center as unenterable), **Route 8** (nine trainers on sight; the tour arrives with whatever PP the last leg left, and Lavender's run spent its entire cycle budget in a fight it could not finish) |

**Evidence:** `postgame::maps::tests` (6 default-tier tests), `postgame::maps` (4 tour legs +
`probe_tour_report` / `probe_tour_plan`).
**Impact on others:** one new `PolicyStep::EnterMapIfReachable` (a give-up instead of a stall — no
existing step's behaviour changes), listed in `current_step_is_long_running` because it carries its
own bound. No fixtures; the tours are read-only and root at `postgame-aides.bin`.

---

### [2026-08-04] J (fallout) — the in-battle party menu reads as a naming screen, and always has
**Status:** corrected ❗ (root-caused and fixed)
**What the plan said:** nothing — this is a **pre-existing agent bug** that workstream J's timing
change exposed, and it is the one thing in this whole session that would have shipped as a
regression if the full slow tier had not been re-run.

**What happened.** With battle animations off, two previously-green A–H legs failed:
`aides::can_get_the_exp_all` (ran to its cycle cap) and `legendaries::can_catch_mewtwo` (stalled).
Both logs were almost empty; the only clue in either was a single line — `name:Venusaur`,
`name:Articuno` — naming a mon that was *already in the party*. Flipping `battle_animations_on` back
to `true` made both pass, which localises it to J but does **not** make it J's bug.

**What is actually true.** `read_game_mode`'s in-battle branch identified the nickname screen by
`wNamingScreenType == 2 && wNamingScreenSubmitName == 0`, with a comment claiming those two "are
specific enough in the battle context". They are not:

- `wNamingScreenType` is **aliased with `wPartyMenuTypeOrMessageID`**, and
- **`BATTLE_PARTY_MENU` is `$02`** — byte-identical to `NAME_MON_SCREEN`
  (`constants/menu_constants.asm:70` and `:92`).

So *every* in-battle party menu reads as a naming screen: a voluntary switch, a potion's "use on
which POKéMON?", and — the one that killed Mewtwo — the **"Use next POKéMON?"** prompt after a faint.
The agent then writes a nickname for whatever is already out, and `AgentState::NamingPokemon`'s exit
test ("the mode has left the naming/battle family") can never become true, because the battle is still
running. It pulses START at a battle waiting for a move, for the rest of the run.

It was latent only because it needs the agent to *sample* inside that window. J's frame-timing change
moved the sample.

**The fix, in two parts.**

1. **The discriminator.** `DisplayNamingScreen` writes `wTopMenuItemY = 3`, `wTopMenuItemX = 1`
   (`engine/menus/naming_screen.asm:101-104`); `PartyMenuInit` writes (0,1). Unlike the aliased type
   byte, those are not shared, so the in-battle branch now also requires the naming grid's geometry.
2. **The bound**, because the first fix removes this instance and not the class:
   `AgentState::NamingPokemon` gained a `ticks` counter and gives up after 1500 ticks (30 s of game
   time) with a named event. §10 already says *every wait needs a bound*; this one did not have one,
   and that is why the failure was silent rather than loud.

**Evidence:** `postgame::legendaries` (4/4) and `postgame::aides` (7/7) green with animations **off**
after the fix; both failed with it before.
**Impact on others:** `read_game_mode` is shared by everything, and this makes it *stricter* in a
branch that was over-firing — no behaviour that was correct changes. Worth knowing for anyone who
touches battle timing: **a timing change is a fuzz test of every RAM-inference in the agent**, and
this repo has more inferences than it has flags.

---

### [2026-08-04] J (fallout 2) — four mainline legs are pinned to the old RNG stream, on purpose
**Status:** verified ✅
**What happened.** After the naming-screen fix, the **whole** slow tier (not just `postgame`) still had
four failures, all of them mainline legs untouched by this session: `celadon::can_reach_lavender`,
`saffron::can_enter_saffron`, `cinnabar::can_get_volcano_badge`, `endgame::can_beat_elite_four`. All
four pass with `battle_animations_on: true` and fail with it off, so the cause is J and only J.

**Why they are not bugs.** `can_reach_lavender` stalls on **Route 10 at (12,20)** trying to enter
Lavender: fewer frames per battle means a wild encounter interrupts the walk at a different tile, and
the agent ends up in Route 10's *southern* pocket, from which Lavender is not reachable. That is the
leg's **route** being tuned against a particular RNG path, not the agent misbehaving —
`EnterMapIfReachable` would have reported it and walked on; `EnterMap` correctly hard-stalls.
`can_beat_elite_four` is the same class one level up: the win is a tuned sequence of switches and
Blizzards across five long fights, and shifting the stream re-rolls every accuracy and crit check in
all of them.

**Why they are not re-cut.** §4.2 **freezes** `complete_game_steps` and `full_playthrough`, and §3 puts
battle tactics out of scope on the stated grounds that in deployment they are the **LLM's** decisions,
not the deterministic policy's. Re-tuning four mainline routes to a new RNG stream is exactly the work
those two decisions rule out.

**What shipped instead.** `TestFixture::with_original_battle_timing()` — an explicit, documented pin to
the pre-J options, used by those four legs and nothing else. The default stays fast, so the default
tier and all **66** postgame legs keep the 20–25 %.

⚠️ **The general lesson, and it is the expensive one:** a leg's inputs are its fixture *and* the RNG
stream, and only the first of those is committed. Anything that changes frame timing silently re-cuts
the second. Re-run the **whole** slow tier — `-- pokemon::integration_tests`, not just your own module
— before believing a timing change is free.

---

### [2026-08-05] mainline legs — the four `#[ignore]`d tests, and why three of the four diagnoses were wrong
**Status:** corrected ❗
**What the plan said:** `src/pokemon/integration_tests/mod.rs` carried a "Known failures" table naming
four blocked tests: `fuchsia::can_get_poke_flute` (Rocket Hideout elevator), `saffron::can_get_vaporeon`
and `saffron::can_beat_silph_giovanni` (stale fixtures, fixable by regeneration), and
`endgame::can_solve_victory_road_1f` (`TeachMove` wedging on an HM deep in the bag).

**What is actually true:** only the elevator one was even in the right *area*, and it was still wrong.

- **`can_get_poke_flute` — the elevator works; B1F is two disconnected halves.** A full-width wall at
  row 16 splits `RocketHideoutB1F`, and B2F has a staircase into each: (21,22) → B1F (21,24) in the
  south, (27,8) → B1F (23,2) in the north. Only the north half holds the Game Corner stairs out; the
  south half's only other exit is the elevator, behind the still-shut Rocket-5 door. `enter()` picks
  the *nearest* warp, which off the elevator is the southern one — 10 steps against 33. So the run
  reached B1F fine and stalled on `EnterMap { GameCorner }` with no Game Corner warp on the map. ⚠️
  **A map is one node in the world graph, so the graph cannot see an intra-map partition** — only an
  explicit `to_position` can. Fixed by naming the landing; the leg then passed in 32 s and ended on
  `MrFujisHouse (3,2)`, byte-identical ground to the committed `post-poke-flute.bin`.
- **Regenerating a fixture could never have fixed the saffron pair.** The lv4 Route-1 Pidgey that
  displaced the gift Eevee enters at `post-cascade.bin` — a committed **root** no test produces — so
  every downstream snapshot inherits it and re-deriving `at-saffron.bin` re-derives the Pidgey too.
  The defect was `eevee_vaporeon_surf_steps` hard-coding `target_slot: 1` when the Eevee appends at 2.
  Fixed by `PartyRef { Slot, Species }` (below).
- **"Deep-bag item-menu scrolling" was already disproven in this file** (the 2026-07-30 B entry: HM02
  at bag index 15 of 16, taught first try in 0.6 s). That entry's *guess* — "my guess is the party
  slot" — was right. `victory_road_1f_steps` took the slave's index as a parameter and its two callers
  disagreed: `complete_game_steps` passed 4, the leg test passed 2. On a party where 2 is not the
  Machop the teach aims at a mon that cannot learn Strength, and because the completion check reads
  that same slot it can never finish. With `PartyRef::Species(Machop)` the teach lands in ~20 ticks.

**Two bugs found underneath, both of the same "silent failure" family:**

1. ⚠️ **`BuyFromMart` was buying nothing whenever the wallet could not cover the whole order.** Gen 1
   answers an unaffordable quantity with "You don't have enough money!" and hands over *nothing* — it
   does not sell you as many as you can afford. From outside the menu that is indistinguishable from a
   dropped YES-confirm, so the policy retried, hit `MAX_MART_ATTEMPTS`, printed "gave up" and the leg
   walked on empty-handed. `silph_co_card_key_steps` had been ordering **15 Hyper Potions (¥18,000) on
   ¥7,838** and buying **zero** every run since it was written — which is what left
   `can_beat_silph_giovanni` blacking out on Silph 11F. Now trimmed to what the wallet covers in
   `agent.rs` before ordering, from the ROM's own `ItemPrices` table; the leg buys 3 and wins.
   This is the *same shape* as the already-documented bag-full failure, and worth checking for
   wherever a step asks the game for a quantity.
2. **`item_price` past the end of `ItemPrices`.** The table is 97 entries (MASTER_BALL id 1 →
   FLOOR_B4F id 97); HM/TM ids start at `$C4` and are priced in a separate `TMPrices` table. Unbounded,
   an HM decoded three bytes of the *next* ROM table as a BCD price. Caught by
   `mechanics::item_prices_match_the_rom_table`, which is why that test asserts on `Hm01Cut`.

**What shipped:** `PartyRef { Slot(u8), Species(PokemonSpecies) }`, resolved against the live
`GameState` **every tick** (a step may name a mon the party does not hold yet — the Celadon Eevee is
still a Poké Ball on the floor when `eevee_vaporeon_surf_steps` is composed). `TeachMove`,
`EvolveWithStone` and `UseStrength` take one; the `machop_slot` parameter is gone from both Victory
Road builders. `Slot` is kept where the *position* is the point — `CuttingTree` only ever asks slot 0.

⚠️ **A second chain divergence, still open.** The saffron pair failed because the leg chain had
`can_get_silph_card_key` seeded from `at-saffron.bin` while `complete_game_steps` fetches Vaporeon
first — re-pointing it at `vaporeon-ready.bin` fixed that. The endgame chain has the *same* divergence
and it is not fixed: `complete_game_steps` runs `seafoam_articuno_steps` between the Volcano and Earth
badges, but `post-volcano-lone.bin` predates that leg and is explicitly the two-mon "lone" party, so
`can_solve_victory_road_1f` is asked to clear Victory Road's nine trainers with a lv56 Venusaur, a lv30
Vaporeon and a lv24 Machop, and blacks out around the last cooltrainer — twice out of two, on either
RNG stream (`with_original_battle_timing` makes no difference, so this is not J fallout). The test
seeds the Articuno instead (`seed_seafoam_articuno`, the `seed_master_ball` device). **The honest fix
is to re-cut `post-volcano-lone.bin` from a Seafoam-era `full_playthrough`** — note it, like
`at-saffron-post-silph.bin` and `at-mansion-blizzard.bin`, is an orphan root that no test produces.

**General lesson:** when a leg list and `complete_game_steps` disagree about *ordering*, the leg
fixtures quietly encode the older order, and the failure surfaces somewhere else entirely — as an
unwinnable fight, or as a mon in the wrong slot. Check the leg chain against `complete_game_steps`'
`extend` order before believing any leg-test diagnosis.

**Evidence:** `fuchsia::probe_hideout_b1f_halves` (dumps both B1F halves and both landings);
`dump_fixture_states` before/after; `mechanics::item_prices_match_the_rom_table`; full `slow-tests`
tier **115 passed / 1 failed** (only `can_solve_victory_road_1f`, pre-seeding) / 25 ignored, against
25 → 21 ignores after. `git status src/pokemon/data/` shows exactly the three intended saffron
fixtures.
**Impact on others:** anyone writing a step that buys, teaches, evolves or pushes with a named mon —
use `PartyRef::Species`, and expect `BuyFromMart` to buy fewer than you asked for rather than none.

---

### [2026-08-05] `full_playthrough` — it does not reach the end, and had not before this session either
**Status:** blocked 🟡
**What the plan said:** `playthrough::full_playthrough`'s doc comment (and §9, and CLAUDE.md) describe
it as playing a fresh save to all 8 badges and on to Victory Road 2F.

**What is actually true:** it stalls partway, on both RNG streams, and **this predates the 2026-08-05
`PartyRef` pass** — provably, not by inference: `complete_game_steps` contains `poke_flute_steps`,
whose leg test carried `#[ignore = "…also fails at HEAD"]`, so the run could not have got past the
Rocket Hideout exit. Fixing that leg moved this run strictly forward rather than breaking it.

Measured after the fix, 800-minute budget, `--features full-playthrough`:

- **Default (fast) stream:** stalls in **Rock Tunnel**, on `EnterMap { RockTunnel1F, (37,17) }` with
  `queue_len=306`. Wild encounters abort the walk repeatedly (`OverworldActionAborted … reason: Battle`)
  while the party wears down to `HP critical, no heal/switch — fleeing to RockTunnelPokecenter`, and the
  queue sits unchanged past the 10-game-minute stall threshold.
- **Pinned with `with_original_battle_timing()`:** Rock Tunnel is **cleared** — the run goes on through
  Lavender, Celadon, the Rocket Hideout (out via the fixed B1F north staircase) and reaches **Mr Fuji's
  House**, where it stalls on `Interact(MRFUJISHOUSE_MR_FUJI)` at (2,7) with `queue_len=233`. Note the
  leg test `fuchsia::can_get_poke_flute` runs *unpinned* and finishes this same stretch cleanly, ending
  at (3,2) with the Flute — so the pinned stream arrives in a different state at the same door.

So both failures are the same shape as the four legs already pinned: **a route tuned against one RNG
stream**, and neither stream is one this run was tuned against. `celadon::can_reach_lavender` covers
the Rock Tunnel stretch, is pinned, and passes.

**Why it was not fixed here:** re-tuning mainline routes to an RNG stream is exactly the work §4.2
(`complete_game_steps` and `full_playthrough` are **frozen**) and §3 (battle tactics are the LLM's in
deployment) rule out, and it is not what the four `#[ignore]`d leg tests needed. The leg tier is green —
**116 passed, 0 failed** — and covers the same ground.

**What I would try next:** pin `full_playthrough` the way the four legs are pinned and chase the Mr Fuji
`Interact` first, since the pinned stream demonstrably gets 200+ steps further. The stall artifact
(`target/test-artifacts/test_stall_screenshot.png`) is the thing to read before theorising — it
identified the Rocket Hideout half-map in one look.
**Evidence:** two 800-minute runs on 2026-08-05 (unpinned, 273 s wall → Rock Tunnel; pinned, 370 s wall
→ Mr Fuji's House); `fuchsia::can_get_poke_flute` green unpinned; full `slow-tests` tier 116/0.
**Impact on others:** ⚠️ do not treat `full_playthrough` as a green baseline — it is not one, and has
not been for some time. Use the leg tier.

---

### [2026-08-05] `full_playthrough` — fixed, and the four bugs between 40 % and 100 % were all "silent skip"
**Status:** verified ✅ — supersedes the "blocked 🟡" entry above
**What that entry said:** the run stalled in Rock Tunnel on the default stream and at Mr Fuji's House
when pinned, and both looked like routes tuned against one RNG stream (workstream-J fallout).

**What is actually true:** the RNG stream was a red herring — pinning changed *where* it died, not
*that* it died. Four distinct bugs sat between 40 % and the end, and **every one of them was something
failing without saying so**, then surfacing tens or hundreds of steps later:

1. **`complete_game_steps` never bought a single healing item.** Not one, in 515 steps. The
   `HP critical — using healing item` branch works; it simply had nothing to reach for, so Rock Tunnel
   and Pokémon Tower — both Pokémon-Center-less, both crossed by a **lone** starter — ended the run by
   attrition. Fixed with Super Potions at **Vermilion** (the first mart on the route that stocks them —
   Cerulean sells only the +20 Potion) and a top-up at **Lavender** before the tower. Rock Tunnel then
   cleared on the *default* stream, no pin needed.
2. **`Interact` pops when it issues its walk, so the Poké Flute was never collected.** Mr Fuji's tower
   script warps the player into his house *standing on the door tile*; the queued
   `Interact(MRFUJISHOUSE_MR_FUJI)` popped while the warp was still resolving, the run walked away
   without the Flute and wedged on the next step. ⚠️ **The leg test cannot see this**:
   `fuchsia::can_get_poke_flute` uses `run_leg`, which keeps stepping after the queue empties until the
   Flute appears — the agent finishes the conversation unprompted and the leg goes green. The mainline
   gives it no such grace. `run_leg` now prints a ⚠️ when its post-exhaustion wait is long; treat that
   as a failure in waiting. Fixed by repeating the interact, the `silph_giovanni_steps` idiom.
3. **A full bag silently ate the Master Ball.** The Silph President's thank-you speech ends
   "You have no room for this." and the `Interact` completes looking exactly like a success — a *gift*
   has no "gave up" to detect, unlike a purchase. The Master Ball never arrived, and 100 steps later
   `CatchPokemon { ball: Some(MasterBall) }` fell back to the best ball in the bag, threw a **Great
   Ball at a lv50 Articuno** and lost the party. Fixed by tossing TM34 Bide before the President.
   ⚠️ This was *triggered* by (1): one extra item type is all it took to reach 20/20.
4. **`BuyFromMart` bought nothing it could not fully afford** — see the entry above. Visible in the
   mainline as `mart:buy(HyperPotion×9)` where the step asked for 20; before the fix that was zero.

**The through-line worth carrying:** every one of these is a step that *completed* while its effect did
not happen. None threw, none logged an error, and each cost between 100 and 300 steps of distance
between cause and symptom. When a long run wedges, the question is not "what is wrong here" but
"which earlier step lied about succeeding" — `RESUME_QUEUE_LEN` + `probe_resume_playthrough` exists to
answer that in seconds rather than 10-minute re-runs.

**Also shipped, so this cannot rot the same way again:** failures now report how far they got
(`completed 488/515 policy steps (94%)`) instead of a bare stall; `run_leg` warns when it is papering
over an unfinished step list; CLAUDE.md now requires a `full_playthrough` run after every major work
item and before any push; and the "played to the Hall of Fame" claim is corrected everywhere it
appeared (its real end point is **Victory Road 2F** — the Elite Four is proved separately).

---

### [2026-08-05] agent — **a black-out looks exactly like a completed warp**, and that is why runs die far from the cause
**Status:** verified ✅
**What is actually true:** `AgentState::OverworldMovement` calls a warp done when the **map changes**
(`agent.rs`, the `game_state.map.map != expected_map` arm). A black-out changes the map — it teleports
the player to the last Pokémon Center — so a party wiping mid-walk is reported as
`OverworldActionCompleted { destination: Warp { to_map: VictoryRoad2F, .. } }` *while standing in
Viridian City*. Nothing in the event stream says the run lost; the very next line is the policy
re-issuing the same step from four maps away.

That is the single most misleading log line in this project, and it cost real time twice in one
session: once reading `endgame::can_solve_victory_road_1f` (where the same "completed" line is
immediately followed by `map=ViridianCity`) and once in `full_playthrough`. ⚠️ **When a run's last
event is a completed warp and the next map is a town with a Pokémon Center, it did not warp — it
died.** Cross-check `grep -c "blacked out"`, and do not trust its absence either: the text driver can
miss the message entirely if the fade starts before it reads the box, as it did in `f4.log`.

**The mitigation that shipped**, rather than changing the completion rule (a warp genuinely *is* done
when the map changes, and every other caller relies on that): the mainline no longer leaves a
single-hop `enter` as the step a black-out has to be recovered from. `victory_road_1f_steps` now ends
`goto(VictoryRoad1F)` → `enter(VictoryRoad2F)`, and `poke_flute_steps` has the same `goto` before the
Mr Fuji rescue. `goto` re-routes across maps; `enter` cannot, so it re-issues forever. **Anywhere a
leg's next step is unreachable from the last Pokémon Center, a black-out is terminal — put a `goto`
in front of it.** The climb back is cheap: beaten trainers stay beaten and solved boulders stay solved.
**Evidence:** `f4.log` lines around the VictoryRoad1F → ViridianCity transition (`OverworldActionCompleted`
for the VR2F warp, then `queue_len=1` in Viridian for the rest of the budget); the same shape in the
pre-fix `vr1f2.log`.

---

### [2026-08-05] `full_playthrough` — **green**, and the last two bugs were both "which mon is slot 1?"
**Status:** verified ✅ — supersedes both "blocked 🟡" entries above
**What is actually true:** a fresh save now plays to **all 8 badges and Victory Road 2F** in one run —
`1 passed; 0 failed`, 591 s wall, 516 policy steps, ending Venusaur lv58 / Articuno lv51 / Slowpoke lv30
/ Vaporeon lv26 / Machop lv24 with ¥13,873. `git status src/pokemon/data/` shows only the three
intended saffron fixtures.

Beyond the four "silent skip" bugs in the entry above, two more sat between 94 % and the end, and both
were the **same slot-vs-species mistake** the `PartyRef` pass was created for:

5. **`MovePokemonToFront { slot: 1 }` did not lead Venusaur.** The comment says "Venusaur leads the
   rival (Alakazam nemesis)" and it was true when written, with a two-mon party. By Victory Road the
   run also carries the Seafoam Slowpoke and Articuno, whose arrival order is not fixed — so the step
   could put a **lv30 HM-slave** at the front of every fight. That is why a party which comfortably
   beats VR1F's nine trainers kept blacking out on the walk to the ladder, and it is why the leg looked
   like a party-strength problem when it was a targeting problem. `MovePokemonToFront` now takes a
   `PartyRef` and the mainline names Venusaur by species. Fixing it removed the black-out entirely.
6. **The final `enter(VictoryRoad2F)` could not be recovered from.** Now `goto`, which re-routes every
   tick — see the "a black-out looks exactly like a completed warp" entry.

⚠️ **One fix tried and reverted, so nobody repeats it:** re-observing the world graph on *every* settle
rather than only on arrival. The motivation is real — `SolveBoulders` opens the VR1F (1,1) ladder and
the graph never learns the edge, so `Goto { VictoryRoad2F }` cannot route back after a black-out. But
`WorldGraph::observe` **overwrites** the node (deliberately — that is how the maze solver recognises
dead-ends), and what is reachable depends on where the player stands. On a map split by one-way ledges
— **Cerulean**, the documented one — a re-observation from a terrace replaces the landing's rich edge
set with that terrace's poor one, and the run stalled at 15 % on `enter(Route5)`. Keying the
re-observation to the arrival entry did not help: the second observation still clobbers the first. If
someone wants this, it has to *union* new edges into the node, and it needs its own test run against
the ledge maps. The comment at the `observe` call site records this.

**Evidence:** `full_playthrough` green (attempt 9); the eight preceding attempts are the record of the
six bugs, each caught by the new `completed N/516 policy steps (P%)` progress note — 40 % → 54 % →
94 % → 99 % → 100 %. That number is what made this tractable; a bare "policy stalled" looks identical
at every one of those points.

**Determinism confirmed (2026-08-05).** Five runs — one sequential under `cargo test`, then **four
concurrently** from the same test binary in separate working directories — all passed, and their
`--nocapture` logs are **byte-identical**: 30,015 lines, one MD5 across all five, `diff` clean pairwise.
So the run is reproducible and CPU contention does not perturb it, which is what makes a
`full_playthrough` failure meaningful rather than a coin flip. To repeat it, run the test binary
directly rather than four `cargo test`s (they serialise on the build lock):

```bash
BIN=$(cargo test --release --features full-playthrough --bin gb --no-run --message-format=json \
      | jq -r 'select(.profile.test and .executable) | .executable')
for i in 1 2 3 4; do (mkdir -p /tmp/pt$i && cd /tmp/pt$i && \
  $BIN pokemon::integration_tests::playthrough::full_playthrough --exact --nocapture > log$i.txt) & done; wait
```
⚠️ `--exact` needs the **full module path**; `-- full_playthrough --exact` matches zero tests and
reports a cheerful `0 passed`. Separate working directories keep each run's `target/test-artifacts/`
from clobbering the others on failure.
