# Plan: Driving Pokémon Red to completion with the deterministic policy

The `DeterministicPolicy` exists to **prove an agent can finish the game** end-to-end. It is scripted
(a fixed `Vec<PolicyStep>`), but it must play **legitimately** — the same way the live LLM policy
will have to. The live policy is separate and reasons per-map with runtime sprite resolution.

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

## Current state (start of this effort)

- Incremental world graph + `EnterMap` + `CollectItem` landed; `can_navigate_mt_moon` **passes**
  (proves the hardest early maze, including the fossil chokepoint + Super Nerd battle).
- **Stage 1 complete: `can_start_game` passes** (Boulder + Cascade), playing legitimately end-to-end
  from `RedsHouse2F`. See the Stage 1 status section below for the fixes that landed.

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
- **Stage 3 — navigation SOLVED, party blocked.** Route 5 crossing (trashed-house bridge) is
  understood and scriptable; rival battle works; `can_reach_bill` reaches Route 24 then stalls on the
  Nugget Bridge due to the weak single-Ivysaur fixture party. **Next (now on the critical path):**
  Stage 2 move-learning heuristic + battle-deadlock fix + a viable party, then regenerate
  `post-cascade.bin`, then finish Bill → SS Ticket → trashed-house bridge → Route 5 → Vermilion.
- **Stage 2 — CODE DONE (2026-07-03), verification pending**: move-learning heuristic (keep damaging
  moves) + battle-deadlock fix implemented and unit-tested (see "IMPLEMENTED" above). Still needs an
  end-to-end run to confirm the in-battle menu driving.
- **NEXT (critical path):** regenerate `post-cascade.bin` with a viable party — upgrade
  `complete_game_steps` to grind the starter higher (the move-learning now keeps Tackle/adds Razor
  Leaf @lv30) and reliably catch + keep a 2nd Pokémon, heal, then re-run `can_start_game` to produce
  the fixture. Then finish the Bill leg: Route 24 → Route 25 → **talk to Bill** (multi-step: talk to
  Bill's Pokémon → use the PC cell-separator → talk to Bill for the SS Ticket) → return → the
  trashed-house bridge (`enter(CeruleanTrashedHouse)` → `enter_at(CeruleanCity, 27, 9)`) →
  `enter(Route5)` → Underground Path → Route 6 → Vermilion. (`can_reach_bill` reaches Route 24 today.)

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
