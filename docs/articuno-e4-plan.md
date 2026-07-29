# Plan: beat the Elite Four via an Articuno-led team (for a clean agent)

---

## DONE (2026-07-28): **Articuno is caught.** Read this instead of §3 and §4 — both are superseded

`can_catch_articuno` passes end to end and `seafoam_articuno_steps()` is folded into
`complete_game_steps()` between the Volcano and Earth badges. The leg starts and ends on Cinnabar
Island and adds two party members: a **Slowpoke** HM-slave at slot 2 (Strength + Dig) and **Articuno**
at slot 3. `victory_road_1f_steps(2)` therefore became `victory_road_1f_steps(4)`.

Final party out of the leg: Vaporeon lv53, Venusaur lv57, Slowpoke lv30, **Articuno lv50
(Peck/Ice Beam)**. Fixture: `data/post-articuno.bin`. §5 (teach it the Blizzard TM and beat the E4) is
untouched and is the next piece of work.

### What §3 and §4 got wrong

* **§3 (catch a Geodude at Rock Tunnel for Strength) is unnecessary.** The HM-slave is caught *inside*
  Seafoam: **Slowpoke** learns HM04, is one of the two commonest land encounters on 1F (which has the
  highest encounter rate of the five floors and an all-land entry pocket), and it also learns **TM28
  Dig**, which is the way out (below). Ten Great Balls from the Cinnabar Mart cover the catch.
* **§4's multi-floor boulder chain is unnecessary**, but a boulder pair *is* mandatory: `EVENT_SEAFOAM4_*`
  gates both routes onto B4F's west lake, and B3F carries four permanently-visible boulders of its own
  ((5,14), (3,15), (8,14), (9,14)) next to its holes at (3,16)/(6,16). Two drops, one floor.
  `hide_show_data.asm` is what makes this work: B1F's and B2F's boulders all start HIDDEN, B3F's do not.
* **Do not trust a "boulder-free" reading of the scripts.** `CheckBothEventsSet` sets Z when **both**
  flags are set, so `jr z, .playerNotInStrongCurrent` means the current runs *until* the boulders are
  down. Misreading that cost a full debugging cycle; the emulator settled it.
* **Getting out is the hard half, and the answer is DIG.** Every walkable way back east is sealed until
  `EVENT_SEAFOAM3_*` (B2F's boulders — hidden behind the whole 1F→B1F→B2F chain): B4F's (20,17)/(21,17)
  staircases are force-walked away from, B2F's (22,6) hole drops you into B3F's east region only for its
  strong-current script to sweep you back down to B4F (verified — the agent looped B2F→B3F→B4F), and the
  west exit lands on the Fuchsia side of Route 20's x=63 wall. TM28 Dig warps to `wLastBlackoutMap`,
  which the Cinnabar heal at the top of the leg sets to Cinnabar Island. One TM, one step.
  (An Escape Rope would do the same but cannot be bought — see the bag-capacity note below.)

### The route (all hops verified offline first, then in the emulator)

| leg | route |
|---|---|
| in | Route 20 → 1F (26,17) → B1F (23,15) → B2F (25,11) → B3F (25,14) |
| | → B4F (20,17) → B3F (8,6) — the only crossing to B3F's west half |
| SEAFOAM4 | drop two B3F boulders into (3,16) and (6,16) |
| catch | hole (6,16) → B4F (5,14), afloat on the west lake; Master Ball on the bird |
| out | DIG → Cinnabar Island |

### Agent capabilities added along the way (all in `seafoam_articuno_steps`'s doc comment)

1. **`TilePairCollisionsWater` is now loaded** (`map_metadata.rs`/`tile_map.rs`) — it never had been;
   the stale comment claimed the agent never surfs. In Cavern it is `($14,$05)`: inside Seafoam you may
   only get on/off water at a shore tile. `pair_blocked` picks the table by whether either end of the
   edge is water.
2. **`apply_seafoam_holes`** — all four floors' holes as inter-map warps, so BFS routes through them.
3. **`apply_seafoam_currents`** — B3F (15,8) impassable (its script sweeps you to B4F's east water).
4. **`no_surf_mount_table`** — the (7,11) `IsSurfingAllowed` gate, one-way (land→water blocked).
5. **`solve_boulder_push` is now a multi-boulder Sokoban** (state = all boulder positions + the
   player's connected component, capped at 50k layouts). The single-boulder version declared B3F
   unsolvable, because (5,14) seals the only corridor to the tile (3,15) must be pushed from.
6. **`CatchPokemon` takes an explicit `ball`** — `Bag::best_pokeball()` returns the Master Ball, which
   would otherwise be spent on the HM-slave.
7. **`CatchPokemon` routes to a static encounter's sprite** (Articuno, Snorlax, …) instead of pacing.
8. **Cave pacing works** (`PacingForEncounters`, ex-`WanderingInGrass`): a wander action targets an
   untyped floor tile, which `actions()` never lists, so the executor used to abort it as `NoRoute` —
   the policy could ask for a cave wander but the agent could not drive one.
9. **`FieldMove::UseFieldMove { slot, move_index }`** (ex-`UseStrength`) — the party field-move menu
   lists a mon's field moves in move-slot order, so the index is computed per mon (`field_move_index`).
   A slave that knows both Strength and Dig needs this.
10. **`PolicyStep::Dig`** + two agent fixes it needed: field-move states are exempt from the
    `RunningScript` machinery (Dig's warp is a script, and the short rollback restored the field-move
    state with `entered_menu` cleared → infinite retry), and a field-move state also ends when the
    **map changes** (Dig leaves a text box open on the far side; waiting for the overworld there means
    mashing A at whatever NPC you landed in front of).
11. **`PolicyStep::TossItem`** — tosses a bag item (START→ITEM→TOSS→qty→YES) to free a slot. Needed
    because the run reaches Cinnabar with a **full 20-slot bag**, which makes the Great Ball purchase
    fail silently; without it the Master Ball goes on the Slowpoke and Articuno is uncatchable. The
    Nugget is the thing tossed: pure sell-fodder, and not a key item (`data/items/key_items.asm`).
12. `in_victory_road` → **`in_center_less_dungeon`**, extended to the five Seafoam maps.
13. **`DefeatGymLeader` survives losing.** The step always meant to retry after a black-out, but it
    popped on the first failed route, and — the real blocker — a black-out reloads the map, so every
    tree the leg cut has **regrown**: Celadon's gym entrance and its garden maze are both sealed by
    cuttable trees. It now pushes a `CutTree` step back on and resumes, on either side of the door.
    (Found via `can_get_rainbow_badge` failing on drifted fixtures whose only real difference was RNG
    phase: Erika's Victreebel Wrap-locks Venusaur and wins.)
14. Offline ROM-only probes (<1s each) that made this tractable — use them before paying for an
    emulator run: `probe_seafoam_maps_offline`, `probe_seafoam_actions_offline`,
    `probe_seafoam_connectivity_offline`, `probe_seafoam_b4f_articuno_actions`,
    `probe_seafoam_boulder_and_exit_offline`, `probe_stall_state` (`STALL=<file>`),
    `probe_bag_contents`.

## 🏆 THE ELITE FOUR IS BEATEN (2026-07-29)

`probe_e4_gauntlet` clears **Lorelei → Bruno → Agatha → Lance → Champion**. The rival's concession
text — *"You're the new POKéMON LEAGUE champion!"* — is in the log, and `EVENT_BEAT_CHAMPION_RIVAL`
is set. The team that did it, from `at-indigo-articuno.bin`:

```
Articuno lv70 247hp — Peck / Ice Beam / Blizzard / Mist      (leads for Lance + the Champion)
Venusaur lv70 228hp — Solarbeam / Razor Leaf / Cut / VineWhip (leads for Lorelei/Bruno/Agatha)
Vaporeon lv70 310hp — Bite / Blizzard / Surf / Hydro Pump
Slowpoke lv30       — Strength / Dig (HM slave, never fights)
```

**What actually unlocked it** — three things, in the order they mattered:

1. **Articuno.** It alone broke Agatha and Lance, the two documented walls.
2. **A 0-PP battle deadlock.** The policy could re-select a move whose PP it had just spent (its PP
   read is a turn stale), the game answered "No PP left", and the agent confirmed the same move
   forever. Every long grind wedged on it within ten minutes. The agent now backs out with B, exactly
   as it already did for a Disabled move. **This was the real blocker on levelling** — before the fix
   the team gained 1 level in 10 minutes; after it, three mons went to lv70 in 31.
3. **lv70.** `probe_grind_to_70` trains Vaporeon, Venusaur and Articuno in Pokémon Mansion 1F
   (~577 XP/battle, Cinnabar's Center next door for PP). It also picks moves differently while
   grinding — the *highest-PP* move that still one-shots rather than the strongest — because the
   Center round trips, not the fights, are what makes a grind take hours.

Two things fell out of the grind that make earlier plans moot: **Articuno learns Blizzard by itself at
lv55**, so the TM14 detour below is unnecessary at this level, and Venusaur picks up **Solarbeam**.

Also needed: **lead Articuno for Lance** (`MovePokemonToFront`). The battle policy only switches when
the active mon has *no* damaging move, so Vaporeon otherwise stayed in and chipped away with Bite once
Blizzard's 5 PP were gone — Lance's room ate a 120-minute clock that way.

**The credits roll, and the agent rolls them.** `probe_e4_gauntlet` ends on `HallOfFame` and saves
`post-hall-of-fame.bin`.

That needed a real fix, because the agent used to wedge in Oak's post-win chain at script stage 6
(OAK_DISAPPOINTED). `probe_hall_of_fame_bisect` (`MODE=`) is the experiment that explains it:

| arm | what it does | result |
|---|---|---|
| `agent` | the agent drives | wedged |
| `toggle` | no agent; probe toggles A on the *same 20 ms cadence* | Hall of Fame |
| `press` | no agent; hands off 1 s, then one 100 ms A press | Hall of Fame |
| `reads` | that press cycle **plus the agent's per-tick reads** | Hall of Fame |
| `agent-press` | the agent runs, but the probe forces the presses | Hall of Fame |
| `memdiff` | emulate, then run only the agent, diffing RAM each tick | wedged, **no RAM writes** |

So it is not the reads (`read_pointer` takes `&self` and indexes the ROM array — it cannot switch
banks), not the cadence, not the write ordering, and the agent writes nothing but the joypad. Tracing
the A line in a working arm and a wedging arm gives **bit-identical** input.

What differs is the *stray* `release_all_buttons` calls the agent's ordinary machinery makes as it
moves between states. `toggle_button` flips relative to the current joypad, so a release landing
between two toggles turns alternation into two presses in a row — and A held across a tick boundary is
exactly what pokered's `HoldTextDisplayOpen` spins on ("hold the dialogue box open as long as the
player keeps holding down the A button").

The fix is `PokemonAgent::drive_post_champion_cutscene`: recognise the Champion's room with its map
script at or past OAK_ARRIVES and take over — hands off the joypad, one deliberate A press per cycle,
returning early from `update` so none of the usual machinery runs. Gating on OAK_ARRIVES rather than
RIVAL_DEFEATED matters: the stage before it is the rival's concession, and the policy needs one
ordinary tick there to pop its `BattleTrainer` step. With the handler in place even the fast toggle
walks the chain — which is the proof that the early return, not the cadence, was the missing piece.

Note the queue does not empty at the end: the rival battle starts from a map script rather than from a
step, and the cutscene handler stops polling the policy, so the gauntlet's last two steps stay queued.
"Done" is the credits map, not an exhausted queue.

**Also fixed here:** `victory_road_1f_steps` no longer stops at the **Viridian Mart**, which does not
sell Hyper Potions (`data/items/marts.asm`) — that restock walked in, failed four times and walked out,
leaving the Route-22 rival to be fought on Saffron leftovers. The stack is now bought on **Cinnabar**
during the Seafoam leg, at the last mart on the route that stocks them.

### Earlier measurement (2026-07-28, before the grind), kept for the reasoning

**Lorelei → Bruno → Agatha → Lance all fall.** Both of the walls the previous agent hit are gone:
Agatha's Gengar lock and Lance's dragons. The run now dies in the **Champion's room**.

Getting there took one fix beyond catching the bird: **lead Articuno for Lance** (`MovePokemonToFront`
before the room). The battle policy only switches when the active mon has *no* damaging move at all, so
Vaporeon stayed in and chipped away with **Bite** once Blizzard's 5 PP were gone — Lance's room ate the
whole 120-minute clock. With Articuno leading, the same fixture cleared Lance in ~8k agent ticks.

**Why the Champion wins.** The log ends with Articuno, Vaporeon and Venusaur fainted and the **lv30
Slowpoke HM-slave** left in, spamming Strength at 18% HP while Full Restores keep it alive. Three
compounding causes, in the order worth attacking:

1. **Ice PP.** Articuno arrives with Peck + Ice Beam — **10 PP of Ice for five rooms**. TM14 Blizzard is
   now collected in the Mansion and taught to it (+5 PP, 120 power), which is the cheapest real gain.
2. **Levels.** Articuno lv50 / Vaporeon lv56 / Venusaur lv57 against a lv61–65 Champion team. The
   previous agent's finding still stands: every wild area the run can reach is lv22–26, and Victory Road
   has no PP restore, so grinding there deadlocks on the low-PP flee. A grind needs a Pokémon Center
   next to decent wilds, and it must happen **before** the one-way climb to Indigo.
3. **The battle AI feeds the HM-slave into the fight.** A lv30 Slowpoke with Strength counts as "has a
   damaging move", so nothing switches it out and nothing stops it being sent in. A "never lead with a
   mon that cannot meaningfully damage the enemy" rule would at least stop the run burning its items and
   its clock on a fight it has already lost.

Reproduce: `probe_e4_backhalf_articuno` (post-articuno.bin → Indigo, ~20 min, saves
`at-indigo-articuno.bin`) then `probe_e4_gauntlet` (`FIXTURE=` picks the team; ~5 min).

**The Blizzard TM is deliberately NOT in the playthrough.** Collecting it in the Mansion and teaching it
to Articuno works — and `seafoam_articuno_steps` still has the `TeachMove`, which skips harmlessly when
the TM was never taken — but adding the pickup to `complete_game_steps` shifted the RNG line onto the
losing side of the **Route-22 rival fight** and the run stalemated there, twice. That fight is marginal
for a reason worth knowing: `victory_road_1f_steps` tries to restock Hyper Potions at the **Viridian
Mart, which does not stock them** (`data/items/marts.asm` — Poké Ball / Antidote / Parlyz Heal / Burn
Heal only), so `BuyFromMart` gives up and the rival is fought on whatever is left over from Saffron.
With PP gone and nothing that can KO, the agent heals itself in circles until the clock runs out.
Fixing that restock — from a mart that sells them, or by buying more at Saffron — is the prerequisite
for putting Blizzard in the main run.

For the Elite Four chain, take the TM outside the playthrough: `probe_get_blizzard_tm_only` saves
`at-mansion-tm14.bin` (TM unspent), then run `can_catch_articuno` with `FIXTURE=` pointing at it, and
the Seafoam leg teaches Blizzard to the bird.

### Two traps worth remembering

* **The bag is at its 20-slot cap and `state.bag` does not show it.** `Bag`'s reader drops every item
  id `ItemId` cannot name — all the TMs — so a bag that prints 13 entries is really 19. Every mart
  purchase of a *new* item then fails with "You can't carry any more items", the policy retries four
  times and gives up, and nothing says why. That is what killed the Escape Rope plan. `probe_bag_contents`
  dumps the raw slots.
* **Fixture drift is real and it will waste your time.** `data/*.bin` are rewritten by the very tests
  that consume them, so a fast-suite failure can be pure drift. Check by running the leg from HEAD's
  copy (`git show HEAD:src/pokemon/data/at-celadon.bin > /tmp/x.bin`) before believing a regression.

---

## TL;DR

The deterministic playthrough (`PolicyStep::complete_game_steps()` in `src/pokemon/policy.rs`) reaches all 8
badges and Victory Road, but its team — effectively a lone Venusaur plus an underlevelled Vaporeon — **walls
at Lance** (the 4th Elite Four member): his dragons are bulky and our best Ice is Vaporeon's *non-STAB*
Blizzard on 5 PP, so it can't out-attrition him. The fix is a real Ice sweeper: **catch Articuno (Ice/Flying)
in the Seafoam Islands, teach it the Blizzard TM (STAB → OHKOs Lance's dragons and the Champion's threats),
and lead it in the E4 gauntlet.**

**Do this by editing `complete_game_steps()` in place — one continuous playthrough, minimal backtracking — NOT
with the pile of one-off `probe_*` integration tests the previous agent used.** Those probes (and their
`.bin` fixtures) were scaffolding; treat them as reference, not the plan.

Three hard constraints from the project owner:
1. **No big backtracking.** Slot the new work into the existing linear playthrough at the natural point.
2. **Stop using fixed party-slot indices.** Adding mons shifts every hard-coded slot and breaks things.
   Refactor the slot-based `PolicyStep`s to reference party members by **species** (see §2). This is the
   first task and it de-risks everything after it.
3. **Do NOT catch a Pidgey.** (The current policy already doesn't — it's lone-starter. Old `.bin` fixtures
   contain a stray lv4 Pidgey from a previous run; regenerate them, don't add a Pidgey.)

The playthrough MUST stay legitimate — no cheating game state, real catches/battles only.

---

## 0. Orientation: how the playthrough works

- `PolicyStep` (in `policy.rs`) is a scripted step (enter a map, talk to a sprite, catch a mon, teach a move,
  push boulders, defeat a gym leader, …). `complete_game_steps()` returns the full `Vec<PolicyStep>`.
- `DeterministicPolicy` executes them; `PokemonAgent` synthesises joypad input each frame.
- Party references today are **fixed `slot: u8` / `target_slot: u8`** on these steps:
  `MovePokemonToFront{slot}`, `GrindUntilLevel{slot}`, `TeachMove{target_slot}`, `EvolveWithStone{target_slot}`,
  `UseRareCandy{slot}`, `UseStrength{slot}`, and the internal `train_slot`. (grep `slot` in `policy.rs`.)
- Run the full playthrough with the slow-tests feature (see `full_playthrough` in `integration_tests.rs`).
  It's ~15 min in release; each iteration is a long loop, so change deliberately and keep fast unit tests
  (`cargo test --release --package gb --bin gb -- pokemon::integration_tests::`) green (33 pass today).
- **Fixture drift:** many integration tests save `.bin` files. After any run,
  `git checkout -- src/pokemon/data/*.bin pokemon-red.bin` to revert unintended writes (repo is read-only —
  do not commit).

---

## 1. Why Articuno (and not the alternatives)

- **Level to 70?** Not feasible pre-Champion. The best grind spot caps ~lv52 (Cinnabar Mansion wilds are
  lv30-39 → negligible XP for a lv52 mon), and the only high-XP area (Cerulean Cave, lv46-65) unlocks *after*
  the Champion. So the team can't simply out-level Lance.
- **Vaporeon's Blizzard** works but has **no STAB** and only 5 PP → marginal, confirmed to stall at Lance.
- **Lapras** (Silph Co 7F gift) also learns Blizzard with STAB and would work — it's the easy fallback if
  Seafoam proves too costly — but the owner wants full-game coverage, so we build Articuno.
- **Articuno** (Seafoam Islands B4F, a static ~lv50 encounter) is caught with the **Master Ball** we already
  hold (guaranteed), skips grinding entirely, and STAB Blizzard 2×/4× shreds Lance + the Champion.
  It's Ice/Flying — safe vs Lance (Aerodactyl has no Rock move in Gen 1; Lance has no Fire/Electric).

---

## 2. FIRST TASK — kill the party-slot fragility (species-based party refs)

Adding Geodude (Strength slave) and Articuno shifts every hard-coded slot. Fix the mechanism, don't paper
over it. Replace the `slot: u8` / `target_slot: u8` fields on the party-referencing `PolicyStep`s with a
**party reference that resolves to a slot at runtime from the live `GameState`.**

Recommended shape:

```rust
/// Identifies a party member independent of its current slot. `Species` matches an exact species;
/// `Family` matches any member of an evolution line (so "the starter" keeps working across
/// Bulbasaur→Ivysaur→Venusaur). Resolve to a slot with the current party each time it's used.
enum PartyRef { Species(PokemonSpecies), Family(&'static [PokemonSpecies]) }

fn slot_of(state: &GameState, who: &PartyRef) -> Option<u8> { /* first party index whose species matches */ }
```

Then `MovePokemonToFront`, `GrindUntilLevel`, `TeachMove`, `EvolveWithStone`, `UseRareCandy`, `UseStrength`
take a `PartyRef` (or `who: PokemonSpecies`) instead of a raw slot, and their handlers call `slot_of(...)`.
Internal `train_slot: Option<u8>` becomes `Option<PartyRef>` resolved per battle.

Notes / edge cases:
- The **starter** evolves, so reference it as `Family(&[Bulbasaur, Ivysaur, Venusaur])` (there are constants
  in `species.rs` — add a helper if needed). Same for Eevee→Vaporeon if that catch is kept.
- `MovePokemonToFront{slot:1}` before the Route-22 rival was "put Venusaur up" — becomes
  `MovePokemonToFront{Family(bulbasaur_line)}`.
- After this refactor, inserting Geodude/Articuno anywhere no longer disturbs other steps. Land this and keep
  `full_playthrough` green **before** adding new mons.

---

## 3. Strength HM-slave: catch a GEODUDE at Rock Tunnel (replaces the VR Machop)

Neither Venusaur nor Vaporeon can learn Strength (verified — Venusaur's only HM is Cut). Seafoam's boulders
AND Victory Road's boulders both need Strength. Rather than catch a Seel *inside* Seafoam (its wild-encounter
area is a warp-maze pocket — painful, see §4) or a Machop at the very end in VR, **catch a Geodude at Rock
Tunnel** — which the playthrough already traverses (needs Flash), it's a common land encounter, and Geodude
learns Strength (verified). One HM-slave then serves **both** Seafoam (mid-game) and Victory Road (end-game).

Concrete edits to `complete_game_steps()`:
- **In the Rock Tunnel leg**, add `CatchPokemon{ Geodude, on_map: RockTunnel1F }` (needs a Poké Ball in the
  bag — buy a few at the previous mart if none). Geodude is the HM-slave; it never battles.
- **After Fuchsia** (once HM04 Strength is in the bag — it comes from the Safari Zone Warden, before
  Cinnabar), add `TeachMove{ Hm04Strength, who: Geodude }`.
- **Delete the VR Machop catch** (`policy.rs:1288`, `CatchPokemon{Machop, VictoryRoad1F}` and its
  `TeachMove{Hm04Strength, …}`) — Geodude already knows Strength by then. Update the VR steps'
  `UseStrength`/`victory_road_*_steps` to reference Geodude (via `PartyRef`).

(Rock Tunnel is before Fuchsia, so catch Geodude first, teach Strength once HM04 is obtained. If it's simpler
to catch Geodude in Victory Road's own wilds and *also* keep an earlier one for Seafoam, don't — one Geodude,
caught early, is the clean answer.)

---

## 4. The Seafoam Articuno detour (insert at the Cinnabar leg)

Seafoam Islands sit on Route 20, immediately **east of Cinnabar Island** (`CinnabarIsland` has an east
connection to `Route20`; Route 20 warps into `SeafoamIslands1F`). So the detour is: at Cinnabar, Surf east
to Seafoam, solve it, catch Articuno, come back — a short there-and-back, not a cross-map backtrack. Insert it
around the Cinnabar/Volcano-Badge steps (the team already has Surf + Strength by then).

### 4a. Seafoam is a multi-floor boulder-and-current puzzle (the hard part)

You must **push 2 boulders down through the floors' holes to the bottom**; only then does the **strong water
current** on B3F/B4F stop, opening the Surf path to Articuno on B4F. Data pulled from
`pokered/scripts/SeafoamIslands*.asm`:

- **Hole coords** a boulder is pushed onto to fall to the next floor (both boulders, each floor):
  - **1F**: (17,6) and (24,6)   → sets `EVENT_SEAFOAM1_BOULDER{1,2}_DOWN_HOLE`
  - **B1F**: (18,6) and (23,6)  → `SEAFOAM2`
  - **B2F**: (19,6) and (22,6)  → `SEAFOAM3`
  - **B3F**: (3,16) and (6,16)  → `SEAFOAM4` → boulders land on B4F, currents stop.
- **B4F** checks `CheckBothEventsSet SEAFOAM3_*` and `SEAFOAM4_*`; until both pairs are down it runs
  `forceSurfMovement` (a `DecodeRLEList` that shoves the surfing player along the current). **The agent does
  not model currents.** Strategy: push every boulder from dry land BEFORE Surfing the swept water; once both
  are down, currents are gone and normal Surf-nav works. Verify each floor's holes are reachable on foot
  (dry land) without crossing an active current.
- 1F boulders start at Boulder1 (18,10), Boulder2 (26,7). Boulder MapSprites are already defined in `map.rs`
  (`SEAFOAMISLANDS{1F,B1F,B2F}_BOULDERn`, `hidden`).

**Reuse the Victory Road machinery** — this is the same shape as VR2F/3F, just more floors:
- `DropBoulderInHole{ hole }` (`PolicyStep`) drives a Strength push into a hole; `solve_boulder_push` is the
  Sokoban planner; `apply_victory_road_holes` shows how to model a floor hole as an inter-map warp so BFS
  routes onto it. Add a Seafoam analogue: register each floor's holes (a `hole_table` per Seafoam map) and
  model the boulder-fall (a pushed boulder appears on the next floor). If the current-stop reveals tiles via
  `ReplaceTileBlock`, add the Seafoam floors to `map_uses_runtime_blocks` (`map_metadata.rs`).
- Sequence per floor: route to the floor, `UseStrength{who: Geodude}`, `DropBoulderInHole` ×2, take the
  stairs/hole down, repeat 1F→B1F→B2F→B3F, then Surf to Articuno on B4F.

### 4b. Known Seafoam-1F snag (already hit)

Entering Seafoam from Route 20 lands the player at **(26,17)**, a small CLOSED land pocket (walled off from
the main cave by the row-11 wall; its only exits are warps at (23,15) and (27,17)). Getting into the main
cave / down to the boulder area means taking those warps — treat 1F as a warp maze, not one open room. Dump
any Seafoam floor's grid to plan: the previous agent's `probe_seafoam_explore` prints the `MetaTileMap`
(`{}`) — legend `O`=wall `_`=empty `X`=water `W`=warp `S`=sprite `P`=player. (Because Seafoam is a *cave*,
wild encounters are LAND encounters — walking, not surfing — so we do NOT need the Seel water-catch; §3's
Geodude removes that need entirely.)

### 4c. Catch Articuno

Articuno is a static trainer-style encounter on `SeafoamIslandsB4F` (`ld a, ARTICUNO`, ~lv50). Walk into it,
throw the **Master Ball** (in the bag — `best_pokeball()` should pick it; it's a guaranteed catch). Articuno
appends to the party (now referenced by species, so no slot issues).

---

## 5. Blizzard onto Articuno + the E4

- **Blizzard TM (TM14)** is an item ball at **Pokémon Mansion B1F (19,25)** — one-use. The current
  playthrough teaches it to Vaporeon; **redirect it to Articuno** (`TeachMove{Tm14Blizzard, who: Articuno}`).
  `ItemId::Tm14Blizzard = 0xD6` is already added, and `hm_move()` already maps it to `Blizzard` (needed so
  `TeachMove` detects completion). Reaching the Mansion B1F TM uses the same switch/hole route as the Secret
  Key (`mansion_secret_key_steps`): up to 3F → `FlipSwitch(3F 10,5)` → hole-fall to 1F → B1F staircase →
  `FlipSwitch(B1F 18,25)` → `CollectItem(POKEMONMANSIONB1F_TM_BLIZZARD)` → flip (18,25) back → up-staircase
  → out. (The Mansion is already visited for the Secret Key, so fold the TM grab into that visit — no extra
  trip.)
- **Verify tmhm before every teach.** A "teach loop" (the `teach:` state never completes) means the mon
  *can't learn that move* — it's not a menu bug. (That's how we found Venusaur can't learn Strength.)
- **Elite Four gauntlet:** at the Indigo lobby, heal, buy the biggest Full-Restore/Revive stack the money
  allows, and run Lorelei→Bruno→Agatha→Lance→Champion (see `probe_e4_gauntlet` for the pattern:
  `BattleTrainer{leader}` per room). Lead with Articuno for Lance/Champion; Venusaur/Vaporeon cover the rest.
  Two battle-AI improvements from the previous agent are already in and kept (fast tests green): a
  **switch-to-a-stronger-attacker** tactic in `pick_battle_action` (hand off when the active mon can't hit
  hard and a bench mon can), and `BattleTrainer` is **stall-exempt** (`current_step_is_long_running`) so a
  long E4 fight isn't killed by the 10-min stall guard. Consider also a **switch-out-to-clear-confusion**
  tactic (Gen-1: switching cures confusion) for Aerodactyl's Supersonic.
- Fold the E4 gauntlet onto the end of `complete_game_steps()` (today it stops at Victory Road / the badges).

---

## 6. What's already built and reusable (don't redo)

- **Encounter wander:** `MetaTileMap::wander_action()` — paces to the farthest reachable walkable tile so
  CatchPokemon/grind trigger per-step encounters on caves/water with no grass. Used by the CatchPokemon arm.
- **Catch-patience:** `CatchPokemon` waits (bounded, `catch_wander_stuck`) for the tile grid to settle on map
  entry instead of popping instantly (sprites read out-of-bounds for a moment after a warp).
- **Boulders/holes/switches:** `DropBoulderInHole`, `solve_boulder_push`, `apply_victory_road_holes`,
  `apply_mansion_holes`, `FlipSwitch`, `UseStrength`, `map_uses_runtime_blocks`.
- **Blizzard plumbing:** `ItemId::Tm14Blizzard`, `hm_move(Tm14Blizzard)=Blizzard`.
- **E4:** `probe_e4_gauntlet` (reference), switch-to-stronger-attacker tactic, stall-exemption.
- **Fixtures** (reference save states, but prefer regenerating via the edited `complete_game_steps`):
  `at-mansion-grinded.bin` (Cinnabar, Master Ball, Blizzard TM uncollected), `at-indigo-*.bin`,
  `vr1f-strength.bin`. NB some carry a stray Pidgey — regenerate rather than trust.

---

## 7. Suggested order of work (each a checkpoint against `full_playthrough`)

1. **Refactor party refs slot→species** (§2). Keep `full_playthrough` green. *(Enabler — do first.)*
2. **Add the Geodude catch at Rock Tunnel + teach Strength after Fuchsia; delete the VR Machop catch** (§3).
   Re-run the VR leg to confirm Strength/boulders still work via Geodude.
3. **Fold the Blizzard TM grab into the Mansion Secret-Key visit; redirect the teach target** (§5) —
   but hold the teach until Articuno exists, or teach a placeholder and move it (simplest: collect the TM,
   teach after Articuno is caught).
4. **Build the Seafoam boulder/current solver + insert the Cinnabar→Seafoam detour + Master Ball Articuno**
   (§4). This is the big one; validate floor-by-floor. Model holes + boulder-fall, then handle currents by
   dropping-before-surfing.
5. **Fold the E4 gauntlet onto the end; lead Articuno; validate a full clear** (§5).

## 8. Risks / where it will fight you

- **Seafoam currents** are the deepest risk: the agent has no current model, so the boulder-drops must all
  happen from dry land before any swept-water Surf. Confirm reachability at each step from the live map dump.
- **Seafoam 1F warp maze** (§4b) — plan floor navigation from grid dumps, not assumptions.
- **The species-ref refactor** touches many steps; land it isolated and green before anything else.
- **Long feedback loops** (~15 min/full run). Validate legs in isolation where possible before the full run,
  and always revert `.bin` drift afterward.

Good luck. The Articuno line is the right call for full-game coverage — Seafoam is literally the game's
hardest dungeon, so budget accordingly, but every piece here (cave/legendary catching, multi-floor boulder+
current puzzles, HM-slave management, species-based party handling) is a feature the whole game needs anyway.
