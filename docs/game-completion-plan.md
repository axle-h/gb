# Plan: Driving Pokémon Red to completion with the deterministic policy

The `DeterministicPolicy` exists to **prove an agent can finish the game** end-to-end. It is scripted
(a fixed `Vec<PolicyStep>`), but it must play **legitimately** — the same way the live LLM policy
will have to. The live policy is separate and reasons per-map with runtime sprite resolution.

## For a future agent — START HERE

**Read this section first — it captures facts that cost a very long session to establish. Do not
re-derive them.**

### Reference material (provided by the user)
- **`docs/walkthrough.txt`** — tile-level detail for **Pallet Town → Vermilion City** (chapters 8–10:
  routes, gym trainers, Mt Moon path, item locations). Use for the near-term Vermilion push.
- **`docs/walkthrough2.txt`** — full-game strategic FAQ: the **whole main-quest order** and every
  badge/gym through the Elite Four (Boulder → Cascade → **Thunder** (Lt. Surge, Vermilion) → Rainbow
  (Erika, Celadon) → Marsh (Sabrina, Saffron) → Soul (Koga, Fuchsia) → Volcano (Blaine, Cinnabar) →
  Earth (Giovanni, Viridian) → Victory Road → Elite Four). Use to plan every stage past Vermilion:
  which HMs are needed where, gym leader teams, and key items (SS Ticket, Bike, HMs).
- **`docs/map.png`** — full Kanto atlas (zoomed-out; good for orientation, not tile-level collision).
- **Memory** `cerulean-route5-terraces`, `deterministic-policy-navigation`, `mt-moon-navigation-debug`.

### Hard-won facts (do NOT re-investigate)
1. **The collision/ledge model is 100% ROM-faithful.** Verified 3 independent ways (Python re-decode
   of `pokered/maps/*.blk` + `overworld.bst` + `Overworld_Coll`; PNG render; and a **real-engine
   save-state flood-fill** that reaches the exact same tiles the running game does). If the agent
   "can't reach" somewhere, it is almost never a collision bug — suspect **one-way ledges splitting a
   map into terraces**, a **sprite/event gate**, or a **missing multi-map/warp hop**.
2. **Cerulean → Route 5 is a terrace puzzle, solved.** The Pokécenter's terrace can't walk to Route 5
   directly (south hedge tile `0x50` is solid; not a ledge). The path is the **trashed-house bridge**:
   after Bill, `GUARD2` (raw (27,12)) clears, then `enter(CeruleanTrashedHouse)` at (27,11) → its back
   door → land Cerulean **(27,9)**, which is in the Route-5 terrace. `bfs(27,9)` reaches Route 5;
   `bfs(27,11)` does not.
3. **Cerulean rival** is a coord trigger at (20,6)/(21,6); the rival sprite is HIDE-until-trigger.
   Beating it sets `EVENT_BEAT_CERULEAN_RIVAL` (part of the guard-clear chain). `can_reach_bill`
   already triggers + wins it.
4. **Bill's SS Ticket is a multi-step interaction** (`pokered/scripts/BillsHouse.asm`): talk to Bill's
   Pokémon (SUPER_NERD at (4,4)) → it walks into the cell-separator → **use the PC** → Bill exits →
   talk to Bill → SS Ticket. A single `Interact` will NOT do it; needs a scripted sub-sequence.
5. **Menu-driving from the agent's tick model WORKS** (this was doubted for a long time and blamed on
   a "HandleMenuInput input-delay wall" — that was a misdiagnosis). Two rules make it reliable:
   (a) **mash** — press a button one agent tick, `release_all_buttons` the next, so each nav/confirm is
   a fresh rising edge every 2 ticks (holding N ticks = ONE edge); (b) **navigate the cursor to the
   target index using `menu_geometry()`/`menu_state()`, THEN press A** — never press A blind. See
   `AgentState::TeachingMove`/`CuttingTree`. Bag rows: use `api.bag_item_position()` (raw `wBagItems`),
   NOT `GameState::bag` (which drops ids outside `ItemId` and shifts indices). The forget-move menu sits
   at origin **(5,8) in battle but (15,8) in the overworld teach**, so detect it by the on-screen prompt
   text (`is_forget_move_prompt`) + live cursor, not by geometry x==5.

### Reusable techniques (worth the tokens once, cheap to reuse)
- **Real-engine save-state flood-fill / BFS**: `GameBoy::save_state()`/`load_state()` + real joypad
  physics to ground-truth map connectivity and settle "can the game actually do X?" questions. Beats
  static analysis when sprites/events/ledges are involved. (Was used as a temp `#[ignore]` test.)
- **Python map decode/render**: `blk[bx+by*W]` → `bst[block*16 + (tx%4)+(ty%4)*4]`; collision =
  `Overworld_Coll` list; bottom-left sub-tile `tile(mx*2, my*2+1)` is what pokered's collision checks.
- **`ExplorerPolicy`** (in `integration_tests.rs`) drives the real agent to discover warp/connection
  graphs — but it only takes the *nearest* connection, so it can't discover terrace re-entries that
  need walking *within* a map. Good for warp mazes (Mt Moon), not terrace splits.

### DONE (all folded into `complete_game_steps`, each with a fast focused test)
Boulder → Cascade → Bill/SS-Ticket → trashed-house bridge → Vermilion → **S.S. Anne (HM01 Cut)** →
**teach Cut** → cut the gym tree → **trash-can puzzle** → **Thunder Badge (Lt. Surge)** → leave
Vermilion (Route 11) → back to Cerulean → **Rock Tunnel** → Lavender → Underground Path → Celadon →
**Rainbow Badge (Erika)**. Helpers: `cerulean_to_vermilion_steps`, `ss_anne_steps`,
`thunder_badge_steps`, `back_to_cerulean_steps`, `cerulean_to_lavender_steps` (+ `rock_tunnel_traversal`),
`lavender_to_celadon_steps`, `celadon_rainbow_steps`. Tests: `can_reach_vermilion`, `can_clear_ss_anne`,
`can_get_thunder_badge`, `can_return_to_cerulean`, `can_reach_route10`, `can_reach_lavender`,
`can_reach_celadon`, `can_get_rainbow_badge` (+ the earlier focused Vermilion tests).
**Everything is button-input only — no RAM-write shortcuts remain in the play path.**

### Rainbow Badge (Erika, Celadon) — DONE ✅ (2026-07-05). Hard-won facts (see memory `rainbow-badge-route`):
1. **Route 9 (east) is a SEPARATE Cerulean terrace** (like Route 5) — main Pokécenter terrace only
   reaches Route 4/24. Cross via the **trashed-house bridge**: `enter(CeruleanTrashedHouse)` →
   `enter_at(CeruleanCity,27,9)` → `enter(Route9)`.
2. **Route 9 Cut tree at (5,8)** boxes the west-entry pocket — need `CutTree{Route9}` to cross east.
3. **Rock Tunnel warp maze** (No Flash) — solved chain in `rock_tunnel_traversal`: 1F(15,3) →
   B1F(33,25) → 1F(5,3) → B1F(23,11) → 1F(37,17) → Route10(8,53) south exit. Found by real-engine
   probing (`probe_rock_tunnel`); ExplorerPolicy alone got stuck (Route10 warp-landing "unobserved" trap).
4. **Cross Rock Tunnel in ONE push** — a mid-tunnel flee-to-heal or blackout can't resume the scripted
   deep enter_at chain. Fix: **heal at `RockTunnelPokecenter`** (tunnel mouth) right before diving.
   A ~lv34 Venusaur at full HP/PP clears it (emerges ~lv37).
5. **Celadon Gym has REAL internal cut trees** (GYM tileset tile `$50` is cuttable — pokered
   `cut.asm`). Erika (top) is unreachable until `CutTree{CeladonGym}` clears the garden chokepoints;
   junior trainers engage by LOS. Grass moves are resisted but a high-level Venusaur's Normal move
   (Cut/Tackle) + level lead wins outright — no grind/extra catch needed after all.

### NEXT STEP — Stage 4: Silph Scope (Celadon Game Corner → Rocket Hideout → Giovanni)
Mainline after the Rainbow Badge (walkthrough2): **Celadon Game Corner → Rocket Hideout** (get the
Silph Scope / Lift Key), Lavender **Pokémon Tower** (needs Silph Scope; rescues Mr. Fuji → **Poké
Flute**), then **Saffron** opens (Silph Co, Rocket-gated) → **Marsh Badge (Sabrina)**, and **Fuchsia
→ Soul Badge (Koga)**. First hard HM gate here is **Surf (HM03)** and **Strength (HM04)**. Start from
`post-rainbow-badge.bin` (now saved in **Celadon City**, gym exited — see below).

**IN PROGRESS (2026-07-05) — Rocket Hideout, three blockers left:**
- **BLOCKER 0 — Celadon gym exit is fragile (REVERTED an attempted fix).** To continue past Erika the
  agent must walk back out of the gym, but (a) cut trees **regrow after any battle** (the battle reloads
  the map — verified: mashing DOWN into a "cut" tile doesn't move after the Erika fight), and (b) a
  **blackout during Erika** respawns the player in Celadon City *behind the uncut city trees*, where
  `DefeatGymLeader` can't re-route to the gym ("no path there"). An attempt to clear `cut_tiles` on
  battle-end (`saw_battle`) + re-cut/exit in `celadon_rainbow_steps` **passed in isolation but broke the
  full run**: the extra re-cutting triggered more junior-trainer battles, wore the party down, and Erika
  (Stun Spore → paralysis) blacked it out → the unrecoverable respawn above. **Reverted** to keep
  `can_start_game` green (it stops at Erika, Rainbow won). A real fix needs: clear cut memory on
  battle-end **without** the re-cut thrash, and make `DefeatGymLeader` blackout-recovery cut the city
  trees to get back into the gym. (`last_map`-change `cut_tiles.clear()` is kept — it fixed Vermilion.)
- **DONE — Game Corner entrance** (`rocket_hideout_entrance_steps`, `can_reach_rocket_hideout` green).
  The guarding Rocket **is** shown @(9,5) blocking the poster @(9,4) (an earlier hidden@(14,5) reading
  was a corrupted state). Fix: **`Interact` now pops when its target sprite is hidden/gone** (a defeated
  trainer that vanishes, e.g. this Rocket) instead of waiting forever. Then `FlipSwitch(9,4)` opens the
  staircase (completion via `GameState::found_rocket_hideout`, EVENT_FOUND_ROCKET_HIDEOUT = bit 0x1b9 →
  `wEventFlags[55]` bit 1; the warp is always in the static map so it must read the event). Event-index
  parser fix: `const_next $XX` **sets** the index, `const_skip` w/o arg = 1 (verified 0x161).
  `can_reach_rocket_hideout` runs from `at-celadon.bin` (pre-gym) to decouple from BLOCKER 0.
- **DONE — spinner-tile navigation** (B2F/B3F, `probe_rocket_hideout_spinners` crosses B1F→B4F). Added
  `MetaTileMap::spinners` (arrow → slide-destination, decoded from `RocketHideout{2,3}ArrowTilePlayerMovement`
  RLE, read *backwards*; PAD_DOWN=+y UP=−y LEFT=−x RIGHT=+x) + a BFS edge (stepping onto an arrow lands
  at its destination; resolve the start if the player is mid-slide; reconstruct stops at the BFS root).
  The executor re-routes each tick and its inputs are ignored during the forced slide, so no special
  executor handling was needed. Tables are hardcoded in `tile_map.rs::spinner_table`.
- **DONE — Lift Key** (`silph_scope_steps`, `can_get_lift_key` green). B4F is **split**: the stairs land
  in a left room (Rocket 3 + items); Giovanni + the Silph Scope are in a right room reachable **only via
  the elevator**. Beat Rocket 3 → the Lift Key ball appears @(10,2) (it + the Scope are hidden until
  their guards fall) → `CollectItem`. Snapshot `rocket-hideout-lift-key.bin`.
- **DONE — CollectItem waits for hidden-until-revealed item balls** (`collect_item_seen` latch): pop
  only once the item has been *seen* then vanishes (collected), not on the initial hidden state — the
  Lift Key (and Silph Scope) balls stay hidden until their guard's after-battle text `ShowObject`s them.
  NB Rocket 3 (unlike the vanishing Game Corner Rocket) **stays** after defeat and his **second talk**
  (after-battle text) is what reveals the Lift Key — so `Interact` him a few times, not once.
- **DONE — elevator floor-menu mechanic** (`PolicyStep::UseElevator` + `AgentState::UsingElevator`):
  faces the panel bg_event + A → advances the "Which floor?" message → navigates the `SPECIALLISTMENU`
  cursor (`wCurrentMenuItem`) to the target floor (Rocket Hideout: **B4F = index 2**) + A → steps onto
  the (runtime-redirected) exit warp and finishes on the map change. Three subtle bugs fixed:
  1. **Menu detection** — the floor menu's `wTextBoxID` reads `MessageBox` (from the "Which floor?"
     `PrintText`), NOT `ListMenuBox`; detect it instead via `wListMenuID == SPECIALLISTMENU (0x04)`
     (new `PokemonApiTrait::list_menu_id`).
  2. **Pre-menu text** — the "Which floor?" box precedes the list and must be advanced (toggle A while
     `!selected` and non-overworld); the old handler just waited and hung.
  3. **Ride-out** — after selecting, `wTextBoxID`/menu vars linger, so the menu branch is guarded by
     `if !selected` to avoid re-driving a phantom menu instead of walking to the exit warp.
  (`pimp_pokemon` wipes the bag → drops the Lift Key → the elevator takes the "appears to need a key"
  path where the menu never opens; navigation probes for this leg must NOT pimp.)
- **ROOT CAUSE of the B1F "collision" blocker — event-gated `ReplaceTileBlock` door blocks (NOT a
  sub-tile bug).** The static ROM map has (25,16) as plain floor, but the *live* `wTileMap` there is a
  wall (`0x18`): `RocketHideoutB1FDoorCallbackScript` swaps map block (y=8,x=12) between a wall block
  (`$54`) and a floor block (`$0e`) at runtime, gated on `EVENT_BEAT_ROCKET_HIDEOUT_1_TRAINER_4` (beat
  Rocket 5). B4F has the same pattern (block (y=5,x=12), `$2d`, gated on beating B4F trainers 0 & 1).
  The agent read ROM (always "open"), so BFS routed through a shut door. **Fixed** by modelling these
  doors: `map_metadata::{DoorSpec table, closed_door_blocks, MapMetadata::apply_door_blocks}` overlays
  the *closed* block's tiles when the gating events are unset (`CurrentMap.closed_doors`, read from
  `wEventFlags`). Diagnostics that nailed it: `wTileInFrontOfPlayer=0x18` vs. the agent's `0x01`, then a
  live-`wTileMap`-vs-ROM-block comparison showed the ROM block is all-floor → a runtime edit.
- **DONE — Silph Scope leg** (`can_get_silph_scope`, ~16 s, gated behind `slow-tests`, snapshots
  `post-silph-scope.bin`). Route: enter the elevator from **B2F** (its warp is ungated, unlike B1F's
  Rocket-5 door — BFS reroutes there once the B1F door is modelled shut), ride to B4F, beat the two
  Rockets to drop the B4F door wall, beat Giovanni (Grass starter 4× on his Ground/Rock team), grab the
  Scope. Verified end-to-end: "CLAUDE found SILPH SCOPE!", `bag has scope=true`.
  **Not yet folded into `complete_game_steps`** — that's the next increment (append
  `rocket_hideout_entrance_steps` + `silph_scope_steps` after the Rainbow Badge leg; today the full run
  still asserts 4 badges and stops at Celadon).
- **DONE — Poké Flute leg** (`can_get_poke_flute`, ~48 s, `slow-tests`, snapshots `post-poke-flute.bin`).
  From `post-silph-scope.bin`: exit the hideout (elevator → B2F → stairs to B1F → Game Corner → Celadon;
  the B1F elevator warp is behind the shut Rocket-5 door, so ride to **B2F**, not B1F), heal, cross the
  **Route 7–8 Underground Path** to Lavender (reverse of `lavender_to_celadon_steps`), then climb
  **Pokémon Tower** 1F→7F. Channelers engage by line of sight while routing to each floor's up-warp.
  **6F gotcha:** the **Rare Candy ball at (6,8)** blocks the *only* chokepoint into the sub-region
  holding the ghost-Marowak trigger (10,16) and the 7F stairs (9,16) — `CollectItem(RARE_CANDY)` opens
  it (found by dumping the stalled 6F map: the 7F warp was simply not in `actions()`). The Silph-Scope
  makes the scripted lv30 ghost Marowak fightable; on 7F the three Rockets fall and Mr. Fuji warps the
  player to his house, where a second `Interact` hands over the Poké Flute.
- **DONE — field item-use capability + Route 12 Snorlax** (`can_wake_snorlax`, ~14 s, `slow-tests`,
  snapshots `post-snorlax.bin`). New `PolicyStep::UseFieldItem { item, target }` / `FieldMove` /
  `AgentState::UsingFieldItem`: route to face the target sprite, then drive START→ITEM→(bag)→USE; the
  item's field effect takes over (the Poké Flute wakes the Snorlax → a wild battle the normal battle
  handler wins). Completion via the seen-then-gone latch (target sprite removed). `snorlax_steps`:
  Mr. Fuji's house → Lavender → Route 12; the **Route-12 Gate** building blocks the road, so pass
  through it (north warp → gate → **south** warp, disambiguated by the south landing (10,21)) before
  the Snorlax is reachable. The snorlax leg now also **heals at Lavender** first (party had been
  fighting since before the tower; also makes Lavender the fallback center for a low-PP heal-flee).
- **DONE — Soul Badge** (`soul_badge_steps`, `can_get_soul_badge`, ~56 s, `slow-tests`, snapshots
  `post-soul-badge.bin`). Route 12 south → 13 → 14 → 15 → Fuchsia → heal → Koga (Poison; Grass starter
  resists it). Two navigation fixes:
  1. **Targeted connection landing** — `MetaTileMap::connection_action(to_map, to_position)` builds the
     route to a *specific* connection crossing on demand; `EnterMap { to_position }` uses it when the
     nearest crossing (all `actions()` emits) doesn't match, and `OverworldMovement` re-derives it each
     tick to keep tracking. The nearest Route 13→14 crossing drops into a **dead-end pocket** (row 6)
     sealed by a south-facing Bird Keeper; crossing at Route 13 (0,9) lands at the open Route 14 (19,8).
     (An earlier attempt that emitted *every* connection tile from `actions()` regressed the whole-game
     run — it perturbed `route_toward`/grind navigation and the early game blew the time budget at Mt
     Moon — so the specific landing is computed off the hot path instead. `can_start_game` stays green.)
- **DONE — Safari Zone: Surf (HM03) + Strength (HM04)** (`can_get_surf_safari` + `can_get_strength_warden`,
  `slow-tests`; fixtures `post-safari-surf.bin`, `post-safari.bin`). New **Safari battle mechanic**:
  `BattleType::Safari` (wBattleType==2); `battle_options` offers **BALL/BAIT/ROCK/RUN** (new
  `BattleAction::Safari{Ball,Bait,Rock}` + `BattleMenuState` variants mapping the 2×2 Safari menu) so a
  future LLM policy can hunt, while the deterministic policy always **RUNs** (never fails, saves
  balls/steps). Gate entry auto-confirms the "join?" 500 prompt on A-mash. Navigation: the Center is
  split by water, so both in and out go the long way — **Center → East → North → West** (Gold Teeth
  @(19,7); Surf from the Secret-House guru) and reverse to exit. Then the **Warden** (Fuchsia) trades the
  Gold Teeth for HM04 Strength. (HMs are held, not yet taught — Surf will go to the Silph Co Lapras,
  Strength to whoever, when first needed.) **Reusable for a future hunting policy.**
- **DONE — Saffron entry** (`saffron_entry_steps`, `can_enter_saffron`, `slow-tests`, `at-saffron.bin`).
  New **`UseVendingMachine`** step (face the Celadon-roof vending bg-event + A-mash buys the cheapest
  drink; reuses the trash-can face-and-press mechanism) → Fresh Water. Reverse Fuchsia→Celadon trek:
  the soul-badge gates mirrored (Route 15 gate west-door→east-exit; Route 12 gate south-door→north-exit)
  and two connection crossings that jam at the nearest tile taken via `EnterMap { to_position }`
  (Lavender→Route 8 at (59,8); the Route-7 gate east door at (18,10)). Then the **Route-7 guard** takes
  the drink (walk east through the gate over the (3,4) trigger, no push-back with a drink in the bag) →
  **Saffron City**.
- **WIP — Silph Co → Marsh** (the big one; `silph_co_card_key_steps`, `can_get_silph_card_key`,
  `#[ignore]`). **DONE:** **Card Key door modeling** — `MapMetadata::apply_card_key_doors` marks tiles
  **$18/$24** on Silph floors as `Obstacle` while the Card Key is absent (and forces them `Empty` once
  held, since the game opens them on approach); gated by `map_has_card_key_doors` +
  `CurrentMap.card_key_locked` (read from the bag in `read_current_map`), inert on every non-Silph map.
  Agent enters Silph 1F from Saffron. **BLOCKED — root cause found (the hard part):** Silph Co's
  floor-to-floor warps don't fire through the agent's normal warp handling.
  - pokered fires a warp only when the player is **standing on a warp/door tile**
    (`CheckWarpsNoCollision` → `IsPlayerStandingOnDoorTileOrWarpTile`, reading `lda_coord 8,9` — the
    bottom-left standing sub-tile — against the tileset warp-tile list). FACILITY warp tiles = **$43,
    $58, $20**.
  - The **elevator (20,0)=$58** and **2F (26,0)=$43** warp tiles sit on the top wall row; the agent
    reaches the tile just below (e.g. (20,1)) but can't step onto the warp tile. Its BFS marks the tile
    a reachable `Warp` (warp_event overlay) while the game's collision treats it as a wall you can only
    walk *into*. Needs walk-into-warp handling for warp tiles the BFS over-marks reachable.
  - The **3F teleport (16,10)** isn't a warp tile at all (standing tile $01, no `$20` warp-pad tiles on
    1F); that warp fires via **`ExtraWarpCheck`** — the Silph teleport special case — a separate
    mechanic to model.
  **NEXT (large):** teach the agent to fire (a) walk-into warp tiles it's adjacent to and (b)
  `ExtraWarpCheck` teleports; navigate the pad graph to the 5F Card Key; then the key opens the doors so
  the elevator reaches 7F (Lapras + rival) and 11F (Giovanni). Beat Giovanni → Saffron Rockets leave →
  **Sabrina** → Marsh Badge. The Card-Key door overlay (`apply_card_key_doors`) is already in place.
  2. **Route 15 gate** — like the Route 12 gate, a gate building walls off the Fuchsia (west)
     connection; traverse it (east door → west exit landing (7,8)) before Fuchsia is reachable.
  Also: the Lavender heal (in `snorlax_steps`) was needed here — without it the party heal-fled to
  Celadon on Route 13 and broke the EnterMap chain.
  **Next:** Fuchsia (Safari Zone → Surf HM03 + Gold Teeth → Strength HM04); Saffron/Silph Co → Marsh
  Badge (Sabrina); then Cinnabar/Volcano (Blaine) + Viridian/Earth (Giovanni) → Victory Road → E4.

Then per-badge (walkthrough2): each later badge introduces a new field HM where first
required — teaching now works via the real menus (`thunder_badge_steps` shows the pattern:
`TeachMove` → HM-gated `CutTree`-style action). Strength (boulders), Surf (water) and Fly follow the
same shape: add a `MetaTile` + an `AgentState` that drives its field-move menu, one badge at a time.

---

## Guiding principles

1. **No cheating.** No injecting items, levels, badges, HMs, or Pokémon. Everything is obtained by
   playing: walking to it, battling for it, catching it, buying it, teaching HMs from the real item.
   `pimp_out_pokemon` stays **only** for isolated maze tests (`can_navigate_mt_moon`) that prove
   *navigation* in isolation. `can_start_game` plays from a fresh `RedsHouse2F` save with no Pokémon.
2. **Forward navigation is explicit.** Every forward map transition is a `PolicyStep::EnterMap`
   (hard-fails if not reachable). Local, on-map tasks (`Interact`, `BuyFromMart`, `GrindUntilLevel`,
   `CatchPokemon`, `CollectItem`, `DefeatGymLeader`) route over the **incrementally-observed** world
   graph, which only contains sprite-resolved, already-visited territory.
3. **Small, provable stages.** Each stage ends with a green integration test and pushes
   `can_start_game` one milestone further. New agent capabilities get a focused fixture test first,
   then are exercised inside `can_start_game`.
4. **Keep the suite green.** After every stage, `cargo test --release --package gb --bin gb` passes
   (modulo tests explicitly rewritten in that stage).

## Current state (2026-07-05)

- `complete_game_steps` plays legitimately from a fresh `RedsHouse2F` save through the **Rainbow
  Badge** (Boulder → Cascade → Bill/SS-Ticket → Vermilion → S.S. Anne/HM01 → teach Cut → gym tree →
  trash-can puzzle → Lt. Surge → back to Cerulean → Rock Tunnel → Lavender → Underground Path →
  Celadon → Erika), all button-input only. Each leg has a fast focused test; the full
  `can_start_game` is the end-to-end source of truth. **It runs in ~6 min in `--release`** (the
  emulator does ~20× realtime); the "~2 h" seen before was a **debug-mode** run. It is now **opt-in**
  behind the `slow-tests` cargo feature so it doesn't run on a normal `cargo test`:
  `cargo test --release --features slow-tests can_start_game`.
- The Stage 1/2/3 sections below are the historical implementation log for those legs (kept for the
  hard-won reasoning; the mechanics themselves now live in code + tests). **Next: Stage 4 past
  Celadon** — see "For a future agent → NEXT STEP" at the top.

---

## Stage 1 — Restore `can_start_game` (Boulder + Cascade) with explicit navigation

Rewrite `complete_game_steps()` (and `can_navigate_to_pewter_city`) so every cross-map hop is an
explicit `EnterMap`. Reuse `mt_moon_traversal_steps()` for Mt Moon. Discover the **Viridian Forest**
transitions the same way Mt Moon was discovered (ROM warp graph + live reachability). Keep the
existing `Interact`/`Buy`/`Grind`/`Catch` steps for on-map tasks; precede each with the `EnterMap`
chain that reaches (and thereby observes) its map.

Sub-milestones, each verified by running `can_start_game` to a progressively later map:
- **1a** RedsHouse → Pallet → OaksLab starter → back to Pallet (Oak's blocking script).
- **1b** Viridian City: parcel (Mart), Pokédex (OaksLab), heal, Town Map, restock, catch Pidgey, grind.
- **1c** Route 2 → **Viridian Forest** (discover maze transitions) → Pewter City.
- **1d** Beat Brock (Boulder Badge).
- **1e** Route 3 grind → Mt Moon (reuse proven steps) → Cerulean → beat Misty (Cascade Badge).

**Exit test:** `can_start_game` asserts Boulder + Cascade (unchanged assertions, now green).

**Status — DONE ✅.** `can_start_game` passes: the agent plays legitimately from `RedsHouse2F`
through the starter pick + rival battle, Oak's Parcel, Pokédex, Town Map, Viridian shopping/grinding,
Viridian Forest, Brock (Boulder), Route 3 grind, Mt Moon (fossil chokepoint + Super Nerd), and Misty
(Cascade) — with 0 phantom-route pops and self-correcting blackout recovery. Fixes landed:

- **Ledge-jump BFS** (`tile_map.rs`): blocked landings no longer recorded as phantom route steps.
- **Blackout / heal-flee recovery** (`policy.rs`): `EnterMap` falls back to routing over the
  incremental world graph when the direct transition isn't on the current map, but only into
  already-observed territory (still hard-fails forward into unvisited maps).
- **Connection node snapping** (`world_graph.rs`): a connection's geometric `to_position` is ~1 tile
  off (and column-dependent) from the raw landing the node is keyed under; BFS now snaps edge
  targets / start nodes to the nearest observed node of the same map (≤ 8 tiles), so multi-hop
  overworld routing no longer dead-ends. This subsumed the old "column-dependent crossing" blocker.
- **Duplicate-sprite disambiguation** (`policy.rs`): all pokecenter Nurses / mart Clerks are the same
  `MapSprite` value, so `sprite.map()` returned the wrong map; `Interact` now matches the sprite on
  the current map first (the preceding `enter(map)` guarantees the agent is there).
- **Single-hop forward enters** (`policy.rs`): after `DefeatGymLeader` the script exits the gym to the
  city before entering the Pokécenter — every forward `enter` is one direct transition.
- **Battle deadlocks** (`agent.rs`/`menu.rs`/`battle.rs`): (a) turn-result text boxes ("… is fast
  asleep!") re-use the battle-menu geometry with `text_box_id = MessageBox`, hanging `Navigating` —
  it now hands back to `WaitingForMenu` to advance the text; (b) a Disabled move made the policy
  re-pick it forever — the disabled slot (`wPlayerDisabledMove`) is excluded from available moves, the
  move list is recognized by geometry even under `MessageBox`, and the agent backs out (B) when the
  highlighted move is the disabled one.

## Stage 2 — Level-up move learning

When a Pokémon that already knows 4 moves learns a new one, the game prompts
"…wants to learn X / forget which?". Today nothing drives this deliberately.

- Detect the learn-move prompt (game state: `wMoveToLearn` / the move-forget menu).
- Add policy hook: `pick_move_to_forget(species, current_moves, new_move) -> Option<Option<usize>>`
  (`None` = still deciding; `Some(None)` = don't learn; `Some(Some(slot))` = forget that slot).
- Agent drives the forget-move menu (YES/NO + move list) to satisfy the policy.
- Deterministic policy heuristic: keep the 4 highest-value moves (damaging power / needed HMs),
  skip learning a strictly-worse move.

**Exit test:** focused fixture — a 4-move Pokémon levels into a 5th; assert the resulting moveset
matches the policy decision. `can_start_game` stays green (grinding may now trigger this).

**Implementation notes (2026-07-03, from pokered `engine/pokemon/learn_move.asm`):** the in-battle
level-up move-learn flow is: `TryingToLearn` prints "… is trying to learn …", shows a YES/NO
`TWO_OPTION_MENU` ("Delete a move to make room?"); on YES it prints "Which move should be forgotten?"
and opens a **4-move menu** via `HandleMenuInput`, cursor in `wCurrentMenuItem` (0-3),
`wMaxMenuItem = wNumMovesMinusOne`; A picks the slot to forget (HMs are rejected), B cancels →
`AbandonLearning` YES/NO. **The move-forget menu has distinct geometry: `wTopMenuItemX=5,
wTopMenuItemY=8`** (vs the battle move-list at `x=5, y=12` and the disabled-move-list at `x=5,y=12`).
`MenuState::battle_menu_state()` (`src/pokemon/menu.rs`) currently returns `None` for it, so in
`agent.rs::BattleState::WaitingForMenu` the agent A-mashes through and forgets slot 0 (→ loses Tackle).
**Fix path:** (a) recognize the forget-move menu in `menu.rs` (new `BattleMenuState` variant or detect
`x=5,y=8`); (b) in the agent, on that menu call a new policy hook `pick_move_to_forget(current_moves,
new_move) -> Option<Option<usize>>` and either navigate the cursor to the chosen slot + A, or press B
(then confirm the abandon YES/NO) to decline; (c) deterministic heuristic: decline unless `new_move`
beats the worst current move by value (`is_damaging_move`/power), forgetting the weakest slot.
Simplest viable version that fixes the party: **decline any learn that would drop a damaging move for
a status move** (keeps Tackle + Vine Whip). The `expected_damage`/`is_damaging_move` helpers in
`damage.rs` give move value.

**IMPLEMENTED (2026-07-03).** Move-learning + a battle-deadlock fix landed (unit-tested; full
in-battle verification pending a level-up fixture / the regen run):
- `menu.rs`: new `BattleMenuState::ForgetMoveList` detected by the forget-menu geometry (top-left 5,8)
  — unit test `menu::forget_move_menu`.
- `policy.rs`: `Policy::pick_move_to_forget(current_moves, new_move)` hook (default = keep moveset);
  `DeterministicPolicy` heuristic **forgets the weakest non-HM move** (status moves rank below every
  damaging move; HMs are never forgotten), so a mixed moveset keeps its damaging moves — it forgets
  Growl/Leech Seed, never Tackle/Vine Whip. Unit tests `policy::move_learn_tests::*`. (Chose
  "always learn the weakest slot" over true decline to avoid the fragile abandon-YES/NO flow.)
- `mod.rs`: `move_to_learn()` (`wMoveNum`) + `learning_pokemon_index()` (`wWhichPokemon`) readers.
- `agent.rs`: `WaitingForMenu` drives the forget menu — navigates the cursor to the policy's slot + A.
- `policy.rs::pick_battle_action`: when no damaging move is available, **switch to a party member that
  can damage** the enemy instead of random-picking a status move; never fall back to Leech Seed
  (which self-heals into an infinite stalemate — the Nugget-Bridge stall).

**Finding (2026-07-02):** move-learning **does not deadlock** the agent today — the post-Cascade
Ivysaur in `can_start_game` reached lv23 and learned Poisonpowder, and the agent advanced through the
prompt fine. But with nothing driving the choice it **forgot Tackle** (its cursor landed on the
learn/forget default), keeping the status move over a damaging one — a suboptimal outcome that
motivates the heuristic hook. It didn't trigger in the Boulder/Cascade run itself (Ivysaur peaks
~lv18-22 there; next 4-move learn is Poisonpowder@22), so it only matters once deeper grinding for
later gyms pushes past level 22. Not urgent, but needed for a strong party. (A synthetic-party
fixture to force the prompt hit a battle-setup artifact — build the fixture from a real grind state.)

## Stage 3 — HM01 Cut

- Reach Vermilion: Cerulean → Route 5 → Underground Path → Route 6 → Vermilion City.
- Get HM01 (Cut) on the S.S. Anne (Captain), then leave.
- New policy step `TeachMove { hm/tm item, target_slot }`: agent opens the item, drives the
  "teach to which Pokémon / replace which move" menus.
- Make `MetaTile::CutTree` actionable: when the party has Cut **and** the Cascade Badge, emit a
  "Cut tree at (x,y)" action; agent faces the tree and uses Cut via the field-move menu.

**Exit test:** inverse of `test_cut_bush_blocks_fisher_without_cut` — with Cut available the agent
cuts the bush and reaches the Fisher. Then extend `can_start_game` through Vermilion + beat Lt. Surge
(Thunder Badge) — Surge's gym also needs the trash-can switch puzzle.

**Cerulean → Route 5: NOT a navigation bug (fully diagnosed 2026-07-02).** The earlier hypothesis
(a ledge/tile-pair classification bug) is **disproven**. Our collision/ledge model is **100% faithful
to the ROM** — verified three independent ways: (a) re-decoding `CeruleanCity.blk` + `overworld.bst`
+ `Overworld_Coll` from the pokered submodule, (b) rendering the actual map PNG, and (c) a
**real-engine flood-fill** (BFS over save-states driving actual game physics) which reaches the
**exact same 346-tile region** our BFS does. From the Cerulean Pokécenter `(19,18)` raw, the real game
itself **cannot walk to Route 5** (`max_y=28`, the hedge row). Details:

- The south "wall" (raw row 28) is a solid **tree hedge** (tile `0x50`, block `0x6c`), *not* a ledge
  (`LedgeTiles` only lists `0x36/0x37/0x27/0x0D/0x1D`; pokered's `HandleLedges` confirmed). It is
  genuinely impassable. Its only real opening is the **east gap** (raw x36,37); the central "gap"
  (x16,17) dead-ends at the sign tiles `0x55/0x56`.
- Cerulean is split by **one-way south ledges** into disconnected terraces. The Pokécenter's terrace
  (the whole main city: PokéCenter/Mart/Gym/houses) reaches Route 4 (west, upper) and Route 24 (north)
  but **not** Route 5. Route 5 is only reachable from the **lower-west terrace** (west edge rows
  21–35, which borders Route 4's *lower* east edge) or the **east/Route 9 terrace** — neither of which
  is in the Pokécenter's component, and re-entering from the Route 24 north connection does *not*
  reach Route 5 either. (The earlier "ledge-agnostic flood reaches Route 5" was a false positive — a
  ledge-agnostic fill cheats by crossing south ledges *upward*, which is illegal.)

**Real fix (revised Stage 3, step 0): multi-map routing to Route 5.** Reaching Route 5 requires a
legal detour that leaves Cerulean and re-enters a Route-5-reaching terrace (the lower-west terrace,
which borders Route 4's *lower* east edge, or the east/Route 9 terrace). **Exhaustively confirmed
(2026-07-02/03) that none of the obvious detours work from the Pokécenter**, using four independent
tools that all agree:

1. Static ledge-BFS model — no Route 5.
2. Real-engine save-state flood-fill (actual physics) — reaches the same 346-tile main-city region;
   no Route 5.
3. Battle-aware save-state BFS (pimped party, A-spam wins) — still stuck (the Cerulean rival at (20,2)
   is a *script*, not a plain battle, so a raw BFS can't cross it).
4. **`ExplorerPolicy` driving the real agent** (handles the rival script + ledge hops + trainers) —
   reached Route 24 → Route 25 → **Bill's House** (SS Ticket is thus obtainable), but **never Route 5**.
   The world-graph dump shows every observed Cerulean node has the *same* connections (Route 24 north,
   Route 4 west) and **Route 4 only ever re-enters Cerulean at the main terrace (0,18)/(0,19)** —
   Route 4 is itself ledge-terraced, and its main-entry-reachable interior (~123 tiles) does not reach
   the lower Cerulean re-entry.

**Architectural implication.** The connection-based `EnterMap`/`ExplorerPolicy` always takes the
*nearest* connection, so it never walks *within* Route 4 (or Route 9) to a different border row to
discover the lower/east terrace re-entry. Reaching Route 5 needs a **wandering explorer** that
navigates within a map (walking to far tiles / hopping ledges) to observe *all* of its
terraces/connections, after which the incremental world graph can route the multi-map path — **or** a
manual real-engine trace that fully explores Route 4's/Route 9's interior to pin the exact re-entry.
(The east/Route 9 terrace may additionally be gated behind the trashed-house / `EVENT_BEAT_CERULEAN_
ROCKET_THIEF` event, which is post-Misty mainline.) **Independently, `post-cascade.bin` lacks the SS
Ticket** (Bill skipped) — required for the S.S. Anne (HM01 Cut) — so the Route 24 → Route 25 (Bill)
detour must be added to `complete_game_steps` regardless; the ExplorerPolicy proved that leg is
navigable.

**ROUTE 5 PATH SOLVED (2026-07-03, per user + model).** The user confirmed the real route: after Bill,
a guard sprite (`CERULEANCITY_GUARD2` at raw (27,12), initially SHOWN) that blocks the **trashed-house
entrance** (raw (27,11)) is removed, and the **trashed house is the terrace-bridge**: enter Cerulean
warp at (27,11) [main-city terrace] → exit its back door → land at Cerulean (27,9), which **is** in the
Route-5-reaching terrace (model-confirmed: `bfs(27,9)` reaches the Route 5 south edge; `bfs(27,11)`
does not). So the scripted crossing is: `enter(CeruleanTrashedHouse)` → `enter_at(CeruleanCity, 27, 9)`
→ `enter(Route5)`. No wandering explorer needed after all.

**Bill leg — navigation WORKS, blocked by PARTY STRENGTH (2026-07-03).** New test `can_reach_bill`
(Cerulean → Route 24 → Route 25 → Bill). The agent correctly triggers + **wins the Cerulean rival
battle** en route (coord trigger at (20,6)/(21,6); rival is a HIDE-until-trigger missable sprite), then
crosses onto Route 24 — but **stalls on the Nugget Bridge trainers**. Root cause is the fixture party,
not code: `post-cascade.bin` holds a **single Ivysaur "Celina" lv23** whose only damaging move is
**Vine Whip (10 PP, currently 0 PP)** plus three dead status slots (Poisonpowder/Growl/Leech Seed) — it
**forgot Tackle** (the Stage 2 move-learning gap) and there is **no second Pokémon**. Two concrete
problems surface:
1. **Battle deadlock** (`policy.rs::pick_battle_action`): when `pick_best_move` returns `None` (no
   damaging move with PP), the fallback picks a *random* Fight move — often **Leech Seed**, whose drain
   keeps Ivysaur alive forever while it can't KO → infinite stalemate (the observed stall).
2. **Party too weak**: one Pokémon, one 10-PP damaging move can't clear ~14 back-to-back Nugget Bridge
   Pokémon (several resist Grass). Needs the **Stage 2 move-learning heuristic** (keep damaging moves
   like Tackle/RazorLeaf) + a viable second Pokémon, then **regenerate `post-cascade.bin`**.

### Progress log
- **Stage 1 — DONE** (`can_start_game` green: Boulder + Cascade).
- **Stage 2 — DONE & VERIFIED end-to-end (2026-07-03 session 2).** Move-learning detection is now
  robust: the "Which move should be forgotten?" menu is confirmed by BOTH the `(5,8)` cursor geometry
  AND the on-screen prompt text (`menu::is_forget_move_prompt`, gated in `agent.rs` — stale geometry
  can no longer misfire). Verified in a real `can_start_game` run: the lv23 Ivysaur learned
  Poisonpowder and correctly forgot **Growl** (a status move), **keeping Tackle**.
- **Party fix — DONE & VERIFIED. `post-cascade.bin` regenerated with a viable 2-mon party**
  (Ivysaur lv23 "Celina" + Pidgey lv4 "Leslee"). The long-standing lone-Ivysaur fixture was caused by
  a chain of bugs, all fixed in `policy.rs`/`integration_tests.rs`:
  1. **Mart affordability**: only ₽1500 is available at the Viridian shop, but it tried to buy 10
     Poké Balls (₽2000). The game silently rejects an unaffordable order and the mart state machine
     reported false success → 0 balls → Pidgey never caught. Now buys **7** (₽1400); dropped the
     Viridian Potion buy (that mart doesn't sell Potions). Added **verify-and-retry** to `BuyFromMart`
     (pops only once the bag reflects the purchase, else re-opens the shop, capped at 4 tries).
  2. **Battle death-spiral / XP-starve**: the voluntary low-HP switch sent the weak Pidgey out to die
     — during the Route 1 grind (starving slot 0 of XP → grind never finished) and during the Mt Moon
     Super Nerd fight (fossil never collected → stall). Fixed: voluntary switch now requires the bench
     mon be **healthy (>50% HP) AND at least the active mon's level**, and is **skipped entirely during
     `GrindUntilLevel`**. This restores the original lone-Ivysaur behaviour (fight on, blackout-recover)
     while still allowing a genuine switch when a strong team-mate exists.
  3. **Stall threshold**: `CollectItem` (crossing battle-heavy Mt Moon B2F with a real, non-pimped
     party) is now exempt from the per-step stall detector, like grinding/catching.
  4. Added a heal step after the Pidgey catch (catch battles leave both mons hurt).
- **Stage 3 — Bill leg + SS Ticket DONE & VERIFIED.** `can_reach_bill`→`can_get_ss_ticket` is green:
  the strong party clears the **Nugget Bridge** + Cerulean rival, and the new **PC-tile interaction**
  drives the full Bill flow to obtain the **SS Ticket** ("CLAUDE received an S.S.TICKET!").
  - New capability: **`MetaTile::Pc` + `MetaTileMap::pc_locations()` + `PolicyStep::UsePc`**. PCs are
    hidden-object tiles (not sprites), so `pc_locations()` supplies the coord per map (Bill's House PC
    at `(1,4)`) and `actions()` emits a face-and-A route (reusing the sprite-interaction routing). The
    agent handles it via the normal route-execution branch (A → PC textbox → done).
  - `PolicyStep::bill_ss_ticket_steps()`: `Interact(BILL_POKEMON)` (A-mash = default YES) → `UsePc` →
    **8×** `Interact(BILL1)`. The retries matter: Bill's exit-machine is a ~1-2s scripted walk, so an
    `Interact` issued mid-script aborts (reason `Script`); retrying lands one after he settles.
- **Stage 3 → Vermilion — DONE & VERIFIED.** `can_reach_vermilion` is **un-ignored and green**: the
  full leg Nugget Bridge → Bill (SS Ticket) → return → **trashed-house bridge**
  (`enter(CeruleanTrashedHouse)` → `enter_at(CeruleanCity, 27, 9)` → `enter(Route5)`) → Underground
  Path → Route 6 → **Vermilion City**. The trashed-house guard clears after meeting Bill, as expected.
- **S.S. Anne — DONE & VERIFIED (full clear).** `can_clear_ss_anne` is green: board Vermilion →
  `VermilionDock` → `SSAnne1F`, **defeat all 16 trainers** (1F ×4, B1F ×6, 2F ×4, Bow ×2 via 3F),
  beat the **rival** (`SPRITE_BLUE` at SSAnne2F (36,4)), talk to the Captain → **HM01 Cut**, and
  **disembark back to Vermilion City**. The Ivysaur levels **23 → 33** over the sweep. Snapshots
  **`post-ss-anne.bin`** (HM01 in bag) for the next leg. (`can_board_ss_anne` remains as a fast
  boarding-only regression; `at-vermilion.bin` is the pre-board fixture.)
  - **How the combat wall was solved.** The lone starter can't out-attrition the ship (no Pokémon
    Center aboard) — it fainted mid-sweep. Fix: each floor is a **heal → board → sweep → disembark**
    cycle returning to Vermilion, so the party is always fresh; floors are ordered so the Ivysaur is
    ~lv32 before the rival (a single 6-Pokémon battle with no mid-battle healing). Cabins are
    **disconnected rooms** within the `*Rooms` maps, each reached by a distinct warp landing
    (`enter_at`, coords decoded from `data/maps/objects/SSAnne*Rooms.asm`); `Interact(trainer)` walks
    up + A to start each trainer battle. New capability used: **HM01–HM05 in `ItemId`** ($C4–$C8) so
    `Cut` is detectable; the PC-interaction and `bill_ss_ticket_steps`/`ss_anne_steps` helpers.
  - **Thunder Badge (Lt. Surge) — DONE via the real UI (`can_beat_lt_surge` green, 2026-07-05).**
    Cut tree, Vermilion Gym trash-can puzzle, and Lt. Surge all beaten with button input only.
    - **KEY CORRECTION to the "HandleMenuInput input-delay wall" below (it was a misdiagnosis).**
      Driving `HandleMenuInput` menus from the agent's tick model works fine with two fixes: **(a)
      mash** — press a button on one agent tick and `release_all_buttons` on the next, so each nav/
      confirm produces a fresh rising edge every 2 ticks (holding for N ticks = ONE edge); **(b)
      navigate the cursor to the target index using `menu_geometry()`/`menu_state()` THEN press A** —
      never press A blind (it selects whatever's under the cursor). This is the pattern in
      `AgentState::CuttingTree`.
    - **Cut tree — DONE (`can_cut_gym_tree` green).** `AgentState::CuttingTree` routes to face the tree
      then button-drives START→POKéMON→mon→CUT. The static-ROM map won't show the felled tree, so the
      agent records cut positions in `PokemonAgent::cut_tiles` and `observe_state()` overrides them to
      `Empty` for the BFS. Snapshots `in-vermilion-gym.bin`.
    - **Trash-can puzzle + Lt. Surge — DONE (`can_solve_gym_trash_cans`, `can_beat_lt_surge` green).**
      `PolicyStep::SolveTrashCans` + `pick_field_move` read which cans hold the switches from RAM
      (`GameState::trash_cans`: `wFirstLockTrashCanIndex`/`wSecondLockTrashCanIndex` → `trash_can_position`,
      lock events `EVENT_1ST/2ND_LOCK_OPENED` at `wEventFlags[44]` bits 1/0). `AgentState::CheckingTrashCan`
      uses new `MetaTileMap::route_to_face(target)` to walk to each can and mash A. Then retry
      `Interact(VERMILIONGYM_LT_SURGE)` ×8 (junior trainers interrupt the walk by LOS; `Interact` pops
      per attempt). Fixtures: `gym-trash-solved.bin`, `post-thunder-badge.bin`.
    - **Teach Cut — redone button-only (`can_teach_cut` green).** `AgentState::TeachingMove` drives the
      real START→ITEM→bag→USE→"make room?"YES→party menus (see fact 5 for the bag-index + forget-menu
      gotchas); the `teach_move_direct`/`write_cur_item`/`write_menu_cursor` RAM shortcuts are DELETED.
      **No RAM-write shortcuts remain in the play path.**
    - **All folded into `complete_game_steps`** via `cerulean_to_vermilion_steps`, `ss_anne_steps`,
      `thunder_badge_steps` (teach → cut → `SolveTrashCans` → `DefeatGymLeader` for Surge). Integrated
      test `can_get_thunder_badge` runs the Thunder leg from `post-ss-anne.bin`; `can_reach_vermilion`
      now calls the shared helper so test + playthrough stay in lockstep. **Next: Rainbow Badge (top).**

## Stage 4 — Surf, Strength, and the rest of the badges

Introduce each field HM where the mainline first requires it, same pattern as Cut (teach + new
action type), extending `can_start_game` one badge at a time toward the Champion:
- **Strength** (HM04) — boulder pushes (Victory Road; some gyms).
- **Surf** (HM03) — water crossings (needed widely: Fuchsia, Cinnabar, Seafoam, many routes).
- Remaining gyms (Erika, Koga, Sabrina, Blaine, Giovanni), Victory Road, Elite Four + Champion.

Each field move: focused fixture proving the action, then fold into `can_start_game`.

---

Progress is tracked in the task list; `can_start_game` is the single source of truth for how far the
agent can legitimately play.
