# Covering the rest of Pokémon Red — plan **and** work record

The agent finishes the game. It has barely *played* it. This document closes the gap, and is written
so that **several agents can work on it at once** without fighting over the same files or fixtures.

> ## ⚠️ THIS DOCUMENT IS THE COORDINATION MECHANISM
>
> It is **not** a plan you read once and leave behind. It is the **shared work record**. Agents
> coordinate through this file and nothing else — there is no other channel between you.
>
> **Every agent must, in this order:**
> 1. **Read** §0 (protocol), §4 (rules of engagement), §11 (findings log) and your own workstream
>    section — *before writing any code*. §11 is where previous agents recorded what turned out to be
>    wrong. Reading it is the cheapest work you will ever do.
> 2. **Claim** your workstream in the §9 status table before starting.
> 3. **Update as you go** — tick sub-steps the moment they are green, not at the end.
> 4. **Record what you learned** in §11, especially assumptions in this document that turned out to
>    be **wrong**. A corrected wrong assumption is worth more to the next agent than a finished task.
>
> An agent that finishes its code but leaves this document untouched **has not finished**.

---

## 0. Working protocol

### 0.1 Claiming work

Before your first edit, set your row in [§9 Status](#9-status) to `🔵 in progress` and put your
identifier in the Owner column. If the row is already claimed, **pick a different one** — do not
start a second effort on the same stream. If every unblocked row is claimed, say so and stop.

### 0.2 Editing this document without conflicting

Several agents will have this file open at once. The rules that keep that safe:

- **Only ever edit: your own workstream's section, your own row in §9, and by *appending* to §11.**
- **Never** rewrite, reword, reorder or "tidy" another workstream's section, another agent's row, or
  an existing §11 entry. If you believe another section is wrong, **append a §11 entry saying so** —
  do not edit it yourself.
- Append to §11, never insert into the middle. Newest entries go at the bottom.
- Make many small edits rather than one big rewrite at the end. A small edit rarely conflicts; a
  whole-file rewrite always does.

### 0.3 The step rhythm

Work in small, verifiable increments. Every sub-step in §6 is sized so that:

- it is **one focused change**, typically under an hour,
- it ends in something **observable** — a passing test, a probe dump, a fixture — not "code written",
- and it can be ticked on its own, so a stopped agent leaves clean, resumable state.

Do not batch several sub-steps and tick them together at the end. If you find a sub-step is actually
three, split it in the document first, then do them.

### 0.4 Definition of done

- **A sub-step** is done when its observable result exists and its box is ticked.
- **A workstream** is done when every sub-step is ticked, its test is green in the `slow-tests` tier,
  its fixture is committed, its §9 row reads `✅ done`, and it has at least one §11 entry (even if
  that entry is only "everything in the plan was accurate").

### 0.5 If you get blocked

Do **not** silently give up, and do **not** expand scope to route around it. Instead:

1. Finish every sub-step that does *not* depend on the blocker.
2. Set your §9 row to `🟡 blocked` and write the blocker in the Notes column.
3. Append a §11 entry describing what you tried and what you'd try next.

A half-finished workstream with an accurate record beats a finished one nobody can build on.

---

## 1. Definition of done for the whole effort

**Mechanism coverage.** Every distinct mechanic the game offers has a `PolicyStep`, a driver, and a
focused test proving it works. Pokédex count rises as a side effect of that work but is *not* the
target — there is no exhaustive catching sweep here.

---

## 2. Ground truth

Measured, not assumed — from `post-hall-of-fame.bin`:

```
badges 255 (all 8) · ¥37,774 · party 4 · DEX OWNED 7 / SEEN 111
party: Articuno 73, Venusaur 70, Vaporeon 71, Slowpoke 30
bag:   20/20 — FULL
       TownMap, TM34, HelixFossil, SSTicket, HM01 Cut, LiftKey, SilphScope,
       PokeFlute, HM03 Surf, HM04 Strength, CardKey, SecretKey,
       GreatBall×9, FullRestore×6, Revive×4
raw:   wBoxCount=0   wCurrentBoxNum=0   wPlayerCoins=0
```

Two consequences drive the whole ordering of this plan:

1. **The bag is full.** The agent physically cannot pick up HM02, a fishing rod, or the Itemfinder
   until item PC storage exists. This is why item storage is Phase 0 and not a workstream.
2. **`wBoxCount=0`.** The Pokémon storage system has never been opened, so the party can never
   exceed 6 and no workstream can bank a caught mon.

**96 of the 248 maps are never referenced anywhere in `policy.rs`.** Regenerate that list any time
with:

```bash
comm -13 <(grep -o "Map::[A-Za-z0-9_]*" src/pokemon/policy.rs | sed 's/Map:://' | sort -u) \
         <(sed -n '7,300p' src/pokemon/map.rs | grep -oE "^    [A-Za-z0-9_]+ =" | sed 's/ =//;s/^ *//' | sort -u)
```

Notable absences: `PowerPlant`, `CeruleanCave{1F,2F,B1F}`, `Route16/17/18` (all of Cycling Road),
`BikeShop`, `PokemonFanClub`, `Route16FlyHouse`, `CinnabarLab*` (4 rooms), `FightingDojo`,
`GameCornerPrizeRoom`, `Daycare`, `Museum1F/2F`, `NameRatersHouse`, `CopycatsHouse1F/2F`, and
`SilphCo{2,4,6,8,10}F`.

---

## 3. Scope

### In

Mechanism coverage for: item + Pokémon PC storage, Fly, the Bicycle and Cycling Road, fishing, the
three remaining legendaries, Safari Zone catching, the Game Corner prize economy, fossil revival,
in-game trades, gift Pokémon, and the dex-gated Oak's-aide items.

### Out — decided, do not relitigate

| Out | Why |
|---|---|
| Link-cable content | `Colosseum` / `TradeCenter` / Cable Club, and the 4 trade-evolutions (Alakazam, Machamp, Golem, Gengar) need a second cartridge. |
| Glitches & exploits | Missingno, the Mew glitch, item duplication, the Old Man trick. Anything relying on ROM/emulator bugs rather than intended play. |
| The MCP text interface | `CLAUDE.md` names it as the project goal and it does not exist yet. It gets its own plan; this one is about game coverage. |
| The slot machine minigame | Coins are purchasable at the Game Corner counter (¥1000 → 50 coins), which reaches every prize without driving an RNG-heavy reel minigame. |
| An exhaustive dex sweep | Mechanism coverage is the goal. See [§7](#7-the-dex-ceiling-for-reference) for the ceiling if this is ever revisited. |

### The RAM-write rule

The repo's existing claim — *"no RAM-write shortcuts remain in the play path"* — **stands**. RAM
writes are allowed, but only in an explicitly-named debug tier:

- **Play path** (anything reachable from `Policy::pick_*` during a legitimate run): button input
  only. No exceptions.
- **Debug tier** (`PokemonApi::debug_*`, added in Phase 0): free to write RAM. Used *only* for
  fixture construction, test seeding, and diagnostics.

`PolicyStep::MovePokemonToFront` is the one pre-existing violation (it writes party order directly).
Leave it; don't add more.

---

## 4. Rules of engagement

### 4.1 File ownership

`policy.rs` (3 114 lines) and `agent.rs` (2 286 lines) are the conflict hotspots. The seam that makes
parallel work possible:

**Each workstream owns two new files and touches shared files on exactly four lines.**

```
src/pokemon/postgame/<stream>.rs             ← owned: step-list constructors + driver logic
src/pokemon/integration_tests/postgame/<stream>.rs ← owned: the tests
```

Rust allows `impl` blocks for a type in any module of the same crate. So a workstream adds its step
constructors as `impl PolicyStep { pub fn my_steps() -> Vec<PolicyStep> { … } }` **in its own file**,
not in `policy.rs`.

The four shared-file lines a workstream is allowed to add:

| File | Allowed edit |
|---|---|
| `policy.rs` | one `PolicyStep` enum variant |
| `policy.rs` | one match arm in `pick_overworld_action`/`pick_field_move`, **delegating in one line** |
| `agent.rs` | one `AgentState` enum variant |
| `agent.rs` | one match arm, **delegating in one line** |

Delegation means literally this — no logic inline:

```rust
// policy.rs
PolicyStep::UsePcBox { .. } => return postgame::pc_box::pick(self, state, world_graph),
// agent.rs
AgentState::UsingPcBox(s) => return postgame::pc_box::tick(self, api, s),
```

One-line arms merge cleanly. Inline bodies do not. This is the single most important rule here.

### 4.2 Fixtures

- **Root every workstream at `post-hall-of-fame.bin`.** It has all 8 badges, Surf and Strength in
  hand, and no remaining main-quest obligations. Workstreams are siblings off that root, not a chain.
- `complete_game_steps` and `full_playthrough` are **frozen**. Do not insert side content into the
  mainline. A later backport pass can move things earlier once each is individually green.
- Fixture writes stay gated behind `--features regen-fixtures`. Never commit a fixture another
  workstream owns. If a leg test fails, check `git status src/pokemon/data/` **first** — drift is the
  usual cause, not your code.
- Name your output fixture `postgame-<stream>.bin`.

### 4.3 Tests

- New tests go in the `slow-tests` tier unless they emulate under ~30 s of game time.
- `--release` always. The crate has no lib target: `--bin gb`, never `--lib`.
- Full path in the filter, e.g.
  `cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests::postgame::pc_box --nocapture`
- Wall clock ≈ emulated-minutes ÷ 23. Budget accordingly and say so in the test doc comment.

### 4.4 Before you start a workstream

Run the coverage probe (added in Phase 0) to see the live state of your entry fixture:

```bash
cargo test --release --bin gb -- pokemon::integration_tests::fixture::probe_coverage --exact --ignored --nocapture
```

---

## 5. Phase 0 — foundation (one agent, blocks everything else)

Nothing else can start until this lands. Keep it to **one** agent; it is almost entirely shared-file
work, which is exactly what §4.1 exists to stop happening more than once.

Do these in order — each builds on the last, and 0.5 is the tool you'll use to check 0.3.

- [x] **0.1 — Coverage probe.** *Do this first: it costs 20 minutes and tells you the truth about
      every fixture.* Add a permanent `#[ignore]`d `probe_coverage` to
      `integration_tests/fixture.rs` printing map, badges, money, party, bag, **dex owned/seen
      counts**, `wBoxCount`, `wCurrentBoxNum`, `wPlayerCoins`.
      *Observable:* it reproduces the numbers in [§2](#2-ground-truth) from `post-hall-of-fame.bin`.
- [x] **0.2 — `postgame` module skeleton.** `src/pokemon/postgame/mod.rs` and
      `src/pokemon/integration_tests/postgame/mod.rs`, one empty file per workstream A–H, wired in.
      *Observable:* `cargo test --release` still green, no behaviour change.
- [x] **0.3 — Pokémon Center PC locations.** `MetaTileMap::pc_locations()` knows only `BillsHouse`
      (1,4). Every Pokémon Center's PC is a hidden object at **(13,3)** facing up — one constant
      covers all of them (verified across `data/events/hidden_objects.asm`; a few non-centre maps
      differ, e.g. `RedsHouse2F` at **(0,1)**).
      *Observable:* a unit test asserting `pc_locations()` is non-empty for a Pokémon Center map.
- [x] **0.35 — Walk out of the Hall of Fame.** *Not in the original plan; added because 0.4 cannot be
      tested without it.* `post-hall-of-fame.bin` is captured on **arrival** in the Hall of Fame, so it
      is a cutscene, not a playable state — see the §11 entry. Drive the ceremony → credits → soft
      reset → CONTINUE (a plain A-mash does all of it) and commit the result as the real postgame root.
      *Observable:* `postgame-post-credits.bin` committed — Pallet Town (5,7), Overworld, badges/party/
      money/dex all intact. Test `postgame::phase0::can_walk_out_of_the_hall_of_fame`, ~7 s wall clock.
- [x] **0.4 — Reach the PC.** Get `PolicyStep::UsePc` to open the PC menu in a Pokémon Center and
      log the on-screen text. No storage logic yet — just prove the agent can stand there and open it.
      *Observable:* a test that asserts the PC menu text appears.
- [x] **0.5 — Item deposit.** `PolicyStep::DepositItem { item, qty }`. Menu chain: PC →
      `SOMEONE's PC` → `WITHDRAW ITEM` / `DEPOSIT ITEM` / `TOSS ITEM` / `LOG OFF` (indices 0–3, from
      `engine/menus/players_pc.asm:243`). Model on `AgentState::TossingItem` — same mash +
      navigate-then-A pattern.
      *Observable:* bag count drops by one; the probe confirms it.
- [x] **0.6 — Item withdraw.** `PolicyStep::WithdrawItem { item, qty }`, same driver, other branch.
      *Observable:* the item round-trips out and back; bag count returns to where it started.
- [x] **0.7 — Debug tier.** A `PokemonApi::debug_*` namespace for RAM writes (give item, set dex bit,
      set money, place party mon), plus a test that greps the play path for `debug_` and fails if it
      appears, so the boundary can't erode.
      *Observable:* the guard test is green, and deliberately calling `debug_` from a policy fails it.
- [x] **0.8 — Reserve the seams.** Land the `PolicyStep` / `AgentState` / `FieldMove` enum variants
      for **all** workstreams A–H as stubs in one commit, so later agents only ever add match arms.
      *Observable:* it compiles; each stub is `todo!()` with the owning workstream named in a comment.
- [x] **0.9 — Ship the entry fixture.** Free enough bag space for the other streams to pick things up.
      *Observable:* `postgame-phase0.bin` committed, bag well under 20, probe output pasted into §11.

**Phase 0 is done when** every box above is ticked and `postgame-phase0.bin` exists with a bag under
20 items. **Announce it by appending a §11 entry** — that entry is the signal other agents wait on.

---

## 6. Workstreams

All are independent and can run concurrently once Phase 0 lands, except **H**, which needs dex
count from the others.

**Each sub-step below is one focused change ending in something observable.** Tick as you go; append
what you learn to §11. If a sub-step turns out to be three, split it here first.

### A — Pokémon storage (PC boxes)

**Why first among equals:** unblocks holding more than 6 Pokémon, which C, D, E, F and G all want.

- [x] **A1 — Read box state.** Expose `GameState.boxed_pokemon` (read `wBoxCount` + the SRAM box data
      via `encoding.rs`). *Observable:* the probe prints box contents, empty at first.
      *Done:* `GameState.boxed_pokemon` + `.current_box`, reader in `postgame::pc_box`. The box data is
      in **WRAM**, not SRAM — see §11. Probe prints `box1: empty`; decode pinned by the default-tier
      test `postgame::pc_box::reads_a_boxed_pokemon_out_of_wram`.
- [x] **A2 — Open Bill's PC.** Navigate the parent PC menu to the `BILL's PC` entry. ⚠️ The parent
      menu's entry *list varies* — `PROF.OAK's PC` only appears once the Pokédex is owned, and a
      `<PKMN>LEAGUE` entry appears post-Champion — so **read the on-screen text, don't hard-code the
      index**. Same trap as the forget-move menu (`menu::is_forget_move_prompt`).
      *Observable:* a test asserting the `WITHDRAW/DEPOSIT/RELEASE` submenu text is on screen.
      *Done:* `postgame::pc_box::can_open_bills_pc` — asserts the parent menu appears **before** the
      submenu, so a deliberate selection is distinguished from falling into entry 0. Index 0 turns out
      to be safe; it is the *label* that varies. See §11.
- [x] **A3 — Deposit.** `PolicyStep::DepositPokemon { slot }`. Submenu entries are `WITHDRAW <PKMN>` /
      `DEPOSIT <PKMN>` / `RELEASE <PKMN>` / `CHANGE BOX` / `SEE YA!`, indices 0–4
      (`engine/pokemon/bills_pc.asm:341`). *Observable:* party count drops, `wBoxCount` rises.
      *Done:* `PolicyStep::deposit_pokemon(slot, map)`; test `postgame::pc_box::can_deposit_a_pokemon`.
      The submenu transcription in the plan is exactly right.
- [x] **A4 — Withdraw.** `PolicyStep::WithdrawPokemon { box_slot }`. *Observable:* the same mon
      round-trips back into the party.
      *Done:* `PolicyStep::withdraw_pokemon(box_slot, map)`; test
      `postgame::pc_box::pokemon_round_trips_through_the_box`.
- [x] **A5 — Change box.** `PolicyStep::ChangeBox { n }`. 12 boxes of 20. ⚠️ Changing box **saves the
      game** — expect a confirmation prompt and a pause. *Observable:* `wCurrentBoxNum` changes.
      *Done:* `PolicyStep::change_box(n, map)`; test `postgame::pc_box::can_change_box` switches away
      and back and finds the banked mon still there. The first change also wipes SRAM — see §11.
- [x] **A6 — Release.** `PolicyStep::ReleasePokemon { box_slot }`. *Observable:* `wBoxCount` drops.
      *Done:* `PolicyStep::release_pokemon(box_slot, map)`; test `postgame::pc_box::can_release_a_pokemon`.
- [x] **A7 — Full round-trip test + fixture.** Deposit, change box, withdraw; party/box counts match
      at every stage. *Observable:* `postgame-pc-box.bin` committed, test green.
      *Done:* `postgame::pc_box::can_round_trip_a_pokemon_through_two_boxes` — deposit → box 2 → box 1
      → withdraw, asserting `(party, box, open box)` between each. Fixture committed and added to
      `probe_coverage`; it restores the party, so its value is the *capability*, not the arrangement.

### B — Fly, the Bicycle, and Cycling Road

The biggest quality-of-life win in the plan: Fly collapses cross-Kanto travel, which every other
workstream pays for in emulated minutes. **Land this early if agents are scarce.**

- [x] **B1 — Bike Voucher.** `PokemonFanClub` (Vermilion), talk to the chairman.
      *Observable:* voucher in bag.
      *Done:* `PolicyStep::bike_voucher_steps()`, test `postgame::fly_bike::can_get_the_bike_voucher`
      (~8 s). The chairman's YES/NO needs no driver — `YesNoChoice` opens on YES and the generic A-mash
      answers it. Viridian → Vermilion goes through **Diglett's Cave**, not the Mt Moon loop.
      Fixture `postgame-bike-voucher.bin`.
- [x] **B2 — Bicycle.** `BikeShop` (Cerulean), trade the voucher. *Observable:* Bicycle in bag.
      *Done:* `PolicyStep::bicycle_steps()`, test `can_trade_the_voucher_for_a_bicycle` (~4 s). One
      `Interact`: with the voucher held the clerk gives the bike and removes it, no menu.
      Fixture `postgame-bicycle.bin`.
- [x] **B3 — HM02.** `Route16FlyHouse`, reached from Celadon via `Route16` (needs Cut).
      *Observable:* HM02 in bag.
      *Done:* `PolicyStep::hm02_steps()`, test `can_get_hm02_fly` (~5 s). ⚠️ **B3 depends on B2**, which
      §6-B does not say: `Route16Gate1F` is two separate corridors and the guard between them wants to
      see the Bicycle. Fixture `postgame-hm02.bin`.
- [x] **B4 — Teach Fly.** `PolicyStep::TeachMove { item: Hm02Fly, .. }` already works — just use it.
      *Observable:* a party mon knows Fly.
      *Done:* test `can_teach_fly` (~1 s). **Articuno is the only compatible party member.** The plan was
      right that this is free — and the "`TeachMove` wedges on a deep-bag HM" suspicion did **not**
      reproduce at bag index 15; see §11.
- [x] **B5 — The Fly driver.** `PolicyStep::Fly { to: Map }` + `AgentState::Flying`, driving
      START → POKéMON → mon → FLY → town-map cursor. ⚠️ The town map is a **bespoke screen**, not a
      `HandleMenuInput` list — budget real time for this one and record what you find in §11.
      *Observable:* the agent Flies between two towns.
      *Done:* [`postgame::fly_bike::tick`] + `FlyState`, test `can_fly_between_towns` (Route 16 → Pewter,
      ~1 s). The warning was an understatement — the cursor is not in RAM at all and there is no flag for
      the screen. Three §11 entries came out of it. Fixture `postgame-fly.bin`.
- [x] **B6 — Cycling Road.** `Route17` is bike-gated; `Route16/17/18` then connect Celadon → Fuchsia.
      *Observable:* the agent walks Celadon → Fuchsia via Cycling Road.
      *Done:* `PolicyStep::cycling_road_steps()`, test `can_ride_cycling_road_to_fuchsia` (~16 s).
      Nothing has to *use* the Bicycle: owning it opens the gate and Route 16 (17,10)/(17,11) mount it.
      Two blockers found on the way — see §11 (`can_surf` on Cycling Road, and Route 18's water flanks).
- [x] **B7 — Route 16 Snorlax.** The **second** Snorlax; `UseFieldItem { PokeFlute }` already exists.
      *Observable:* Snorlax gone, route passable. Then commit `postgame-fly-bike.bin`.
      *Done:* same test (the two share a map, and the Snorlax battle regrows the cut tree the Cycling
      Road entrance is behind). It turns out the Route 16 Snorlax does **not** block the road — the road
      is two tiles tall and it sits on only one of them — so this is dex coverage (SEEN 112), not a gate.
      Fixture `postgame-fly-bike.bin`.

### C — Fishing

- [x] **C1 — Old Rod.** `VermilionOldRodHouse`. *Observable:* rod in bag.
      *Done:* `PolicyStep::old_rod_steps()`, test `postgame::fishing::can_get_the_old_rod` (~1 s). One
      `Interact`; the guru's YES/NO needs no driver, exactly like B1's chairman. ⚠️ The leg ends with an
      extra `enter(town)` so the fixture is saved **outdoors** — see §11.
- [x] **C2 — The fishing driver.** `PolicyStep::Fish { rod, at }` + `AgentState::Fishing`: face a
      water tile, use the rod from the bag (same START → ITEM → USE chain as `UsingFieldItem`),
      handle the "not even a nibble" / "hooked" text, drop into the normal battle handler on a bite.
      *Observable:* a wild battle starts from a water tile.
      *Done:* [`postgame::fishing::tick`] + `FishState`, test `can_fish_a_wild_battle_out_of_the_water`
      (~2 s). The shape changed: the step is `Fish { rod, map, goal }` and the *policy* picks the water
      tile and owns the repetition, one cast per driver invocation. ⚠️ The one real trap is that the
      cast animation **must be mashed through, not waited out** — see §11.
- [x] **C3 — Catch from a bite.** *Observable:* one water species in the dex.
      *Done:* `FishGoal::Catch`, test `can_catch_a_magikarp_on_the_old_rod` (~2 s), fixture
      `postgame-magikarp.bin`. No weakening pass: everything fishable is catch rate 155–255.
- [x] **C4 — Good Rod.** `FuchsiaGoodRodHouse`. *Observable:* rod in bag, different encounter table.
      *Done:* `PolicyStep::good_rod_steps()`, test `can_get_the_good_rod_and_catch_a_goldeen` (~10 s),
      fixture `postgame-good-rod.bin`. **Goldeen** is the proof — it is in the Good Rod's table and not
      the Old Rod's (which is Magikarp and nothing else).
- [x] **C5 — Super Rod.** `Route12SuperRodHouse`. *Observable:* as above; commit
      `postgame-fishing.bin`.
      *Done:* `PolicyStep::super_rod_steps()`, test `can_get_the_super_rod_and_catch_a_tentacool`
      (~16 s). **Tentacool** is the proof (Pallet's Super Rod group only). ⚠️ Route 12's road is
      blocked by its gate building and the house is at (11,77), 56 tiles south of it. The party is
      banked down to four at the Viridian PC first, to keep the catch off the boxed-catch path that
      wedges the agent (§11, D/D5).

Opens the whole water encounter table — Magikarp/Goldeen/Poliwag/Tentacool/Krabby/Horsea/Staryu.
**Dex after C: 10 owned / 113 seen** (from 7/112).

### D — Legendaries: Zapdos, Moltres, Mewtwo

~~Cheapest workstream by far~~ — **that assumption was wrong and the sub-steps below have been
re-cut.** `CatchPokemon`'s static-encounter branch does route to a map sprite named after the species
and press A, and that part is free. What is not free is the *fight*: Articuno was caught with the
**Master Ball**, which is spent, and all three remaining legendaries have **catch rate 3**. See the
2026-07-30 §11 entries for the arithmetic; the short version is that a status ailment is the only lever
that matters and Thunder Wave is the only one the party can get.

⚠️ **Read the "trapping moves and the one-shot legendary" §11 entry before touching D1b or D3.** Two
rules it establishes, both load-bearing and both counter-intuitive:

1. **Never `BattleAction::Run` against one of these.** `EndTrainerBattle` hides the object and sets its
   event on *any* exit but a blackout. Running deletes the legendary from the save.
2. A slower Pokémon **cannot act at all** while the target is mid-Fire-Spin, so the paralyser has to
   **outspeed** the target.

Rule 2 is why the order below is not the order in the original plan: the party has no fast Thunder Wave
user, so the Power Plant (which has one) has to come first.

- [x] **D1a — the toolkit.** TM45 Thunder Wave (a free pickup at Route 24 (10,5)) taught to Slowpoke,
      plus the ball and potion stack every catch spends. *Observable:* `postgame-thunder-wave.bin`.
      *Done:* `PolicyStep::arm_for_legendaries_steps()`, test
      `postgame::legendaries::can_arm_for_the_legendaries` (~8 s). **Slowpoke is the only party member
      TM45 is compatible with**, and TM45 is one-per-cartridge — see §11 before spending it again.
- [ ] **D2 — Reach the Power Plant.** `PowerPlant`, entered by Surfing east off Route 10. Unvisited,
      so the route needs `EnterMap` steps. *Observable:* the agent stands in the Power Plant.
- [ ] **D2a — Catch an Electrode** (new). The Poké-Ball sprites on the Power Plant floor are disguised
      Voltorbs and Electrodes; Electrode is lv43 with base speed **140**, the only obtainable Pokémon
      that outspeeds Moltres and Zapdos *and* learns TM45. It is catch rate 60, i.e. an ordinary catch.
      ⚠️ Its map sprite is named `Electrode 1`/`Electrode 2`, not `Electrode`, so the static-encounter
      branch's exact-name match will not find it — engage it with `Interact` or relax that match.
      *Observable:* Electrode in the party knowing Thunder Wave.
- [x] **D1b — Moltres.** `VictoryRoad2F`. **Not** "already traversed": Moltres sits in a third,
      separately-sealed region of 2F reached only via VR3F's (2,0) warp, and Victory Road can only be
      entered from the *bottom*. *Observable:* Moltres in the dex.
      *Done (⚠️ Master-Ball-seeded):* `PolicyStep::moltres_steps()`, test
      `postgame::legendaries::can_catch_moltres` (~22 s), fixture `postgame-moltres.bin`, dex 8/114.
      The **route** is legitimate and is what the test proves; the **fight** is still open — see below.
- [x] **D2 + D3 — the Power Plant and Zapdos.** *Observable:* Zapdos in the dex.
      *Done (⚠️ Master-Ball-seeded):* `PolicyStep::zapdos_steps()`, test `can_catch_zapdos` (~14 s),
      fixture `postgame-zapdos.bin`, dex 9/117. Dig out of Victory Road, Fly to Cerulean, cross to the
      Route-9 terrace through the **trashed house** (Fly's landing terrace cannot reach Route 9), Cut
      Route 9's (5,8) tree, then Route 10 — the BFS Surfs to the Power Plant door on its own. Zapdos at
      lv50 knows only Thundershock and Drill Peck, i.e. **no trapping move**, so it is the right place
      to prove the honest paralyse-then-throw loop when D2a lands.
- [ ] **D2a — Catch an Electrode** (new) — see above. Still the unblock for the honest fights.
- [ ] **D4 — Stock Ultra Balls.** Mewtwo is lv70 and will need them. *Observable:* balls in bag.
- [x] **D5 + D6 — Cerulean Cave and Mewtwo.** *Observable:* Mewtwo in the dex; `postgame-legendaries.bin`.
      *Done (⚠️ Master-Ball-seeded):* `PolicyStep::mewtwo_steps()`, test `can_catch_mewtwo` (~18 s),
      **dex 10 owned / 121 seen**. The approach is three walls in a row and none of them is the fight —
      Cerulean is cut in two and the cave is on the far half, the way across is **Route 24's left river
      seam** (a `ConnectionWater`, which needed a new `MetaTileMap::water_connection_action`), and 1F's
      B1F ladder is behind a Cavern **tile-pair elevation boundary** reachable only via 2F. Full write-up
      in §11.
      ⚠️ Still open honestly: Mewtwo is lv70 with base speed 130 — it outspeeds even Electrode, so a
      paralyser has to *survive* a hit instead. No trapping move, and half its moveset (Barrier, Recover,
      Swift) is not a one-shot.

### E — Safari Zone proper

The Safari Zone is currently entered only to grab HM03 and the Gold Teeth, and
`pick_battle_action` **hard-codes RUN on every Safari encounter** (`policy.rs:2484`).

- [ ] **E1 — Model the step budget.** The **500-step** counter and the ejection back to the gate.
      Without this a run ends mid-hunt with no warning. *Observable:* the step count is in
      `GameState` and the probe prints it.
- [ ] **E2 — Replace the blanket RUN.** `BattleAction::SafariBall/Bait/Rock` already exist and are
      already offered — write a real catch policy. Rock raises catch rate *and* flee rate; Bait does
      the inverse. *Observable:* the agent throws a ball instead of running.
- [ ] **E3 — Catch a Safari-exclusive.** Chansey, Scyther, Kangaskhan, Tauros, Dratini, Exeggcute,
      Rhyhorn, Parasect, Venomoth. (Pinsir is Blue-only.) *Observable:* one of them in the dex.
- [ ] **E4 — Exit cleanly.** Both ways: walking out, and being ejected at 0 steps.
      *Observable:* test green both ways; commit `postgame-safari.bin`.

### F — Game Corner economy

No slot machines (out of scope). Coins are bought with money instead.

- [x] **F1 — Coin Case.** From a man in the **`CeladonDiner`** (verified: `scripts/CeladonDiner.asm`
      is one of only two files referencing `COIN_CASE`). The Diner is on the unvisited-maps list.
      *Observable:* Coin Case in bag.
      *Done:* `PolicyStep::coin_case_steps()`, test `postgame::game_corner::can_get_the_coin_case`
      (~2 s), fixture `postgame-coin-case.bin`. The giver is the **gym guide**, not the "middle aged
      man" the name suggests; one `Interact` and no menu, exactly like B1's chairman.
- [x] **F2 — Buy coins.** `GameCorner` counter clerk, ¥1000 → 50 coins. *Observable:* `wPlayerCoins`
      rises; the probe confirms it.
      *Done:* `PolicyStep::BuyGameCoins { target }` + `buy_coins_steps(target)`, test
      `can_buy_game_coins` (~2 s), fixture `postgame-coins.bin` (200 coins for ¥4,000). No driver: the
      clerk's YES/NO opens on YES like B1's chairman, so the step just re-issues one `Interact` per
      purchase and stops on the coin count. Every *refusal* is a pre-check — see §11.
- [x] **F3 — Sell to a mart.** The mart driver only implements Buy today. Needed because Porygon is
      9999 coins ≈ ¥200 000. *Observable:* money rises after selling junk.
      *Done:* `PolicyStep::SellToMart { map, item }` + `AgentState::SellingToMart`, driver
      [`postgame::game_corner::sell_tick`], test `can_sell_junk_to_a_mart` (~3 s), fixture
      `postgame-sold.bin` (+¥6,000 from three banked TMs). Selling is **not** a mirror of buying —
      different list, halved prices, and no screen shows that it worked. See §11.
- [x] **F4 — Redeem a prize.** `GameCornerPrizeRoom`: Abra, Clefairy, Nidorina, **Dratini**,
      **Scyther**, **Porygon**, plus prize TMs. *Observable:* a prize Pokémon in the party; commit
      `postgame-game-corner.bin`.
      *Done:* `PolicyStep::RedeemPrize { prize }` + `AgentState::RedeemingPrize`, driver
      [`postgame::game_corner::prize_tick`], test `can_redeem_a_prize_pokemon` (~2 s) — an **Abra**,
      180 coins, **dex 8 owned**. All nine prizes are modelled as `Prize`, with the table pinned
      against ROM by `prize_table_matches_the_rom`. **Both** branches of `HandlePrizeChoice` are
      covered: the mon one honestly, and the `GiveItem` one by `can_redeem_a_prize_tm` (~15 s) — TM23
      Dragon Rage, 3300 coins, i.e. 66 trips through the counter clerk, with the **money**
      debug-seeded per §3 and everything else driven normally. Its fixture is deliberately not
      committed, so F's chain still ends on an honestly-earned state.

### G — Gifts, trades, and one-off rooms

The long tail. Sub-tasks are independent — **a second agent can take G-trades while the first takes
G-gifts**, provided you claim separate rows in §9.

- [ ] **G1 — Fossil revival.** `CinnabarLabFossilRoom`. The agent already carries a **Helix Fossil**,
      so Omanyte is one interaction away. *Observable:* Omanyte in the party.
- [ ] **G2 — Old Amber.** `Museum1F/2F` (Pewter), behind a Cut tree → Aerodactyl at the same lab.
      *Observable:* Aerodactyl in the party.
- [ ] **G3 — Lapras.** `SilphCo7F`, from the rescued employee. Silph is already traversed; the gift
      was simply never taken. *Observable:* Lapras in the party.
- [ ] **G4 — Fighting Dojo.** Saffron: beat the Karate Master, choose Hitmonlee or Hitmonchan.
      *Observable:* the chosen mon in the party.
- [ ] **G5 — The trade driver.** `PolicyStep::TradePokemon { give_slot, at }` driving the
      offer/accept flow. *Observable:* one trade completes.
- [ ] **G6 — Three more trades.** From the table below. *Observable:* dex count rises by three.
- [ ] **G7 — Skipped Silph floors.** `2F/4F/6F/8F/10F` — item pickups only. *Observable:* items in bag.
- [ ] **G8 — Colour rooms.** `Daycare` (Route 5), `NameRatersHouse`, `CopycatsHouse1F/2F` (TM31 Mimic
      for a Poké Doll), `MrPsychicsHouse` (TM29), `ViridianSchoolHouse`,
      `CeladonDiner`/`Hotel`/`ChiefHouse`. *Observable:* commit `postgame-gifts.bin`.

**In-game trades** — there are exactly **9** usable ones (a 10th, Butterfree→Beedrill, is unused
code). Verified from `data/events/trades.asm` + the scripts that reference `TRADE_FOR_*`:

| Give | Get | Map |
|---|---|---|
| Nidorino | Nidorina | `Route11Gate2F` |
| Abra | Mr. Mime | `Route2TradeHouse` |
| Ponyta | Seel | `CinnabarLabFossilRoom` |
| Spearow | Farfetch'd | `VermilionTradeHouse` |
| Slowbro | Lickitung | `Route18Gate2F` |
| Poliwhirl | Jynx | `CeruleanTradeHouse` |
| Raichu | Electrode | `CinnabarLabTradeRoom` |
| Venonat | Tangela | `CinnabarLabTradeRoom` |
| Nidoran♂ | Nidoran♀ | `UndergroundPathRoute5` |

⚠️ **Each trade requires already owning the give-species**, so G5/G6 depend on catching those nine
first — this is not the cheap dex win it looks like. Farfetch'd, Mr. Mime, Lickitung, Jynx and Tangela
are *only* obtainable this way. `Route18Gate2F` additionally needs the Bicycle, so that one row
depends on **B**.

### H — Oak's aides (depends on A–G)

Three items gated on **dex owned**, currently 7. **Check the gate with the probe before travelling —
don't guess.**

| Item | Gate | Where |
|---|---|---|
| **HM05 Flash** | 10 owned | Route 2 Gate |
| **Itemfinder** | 30 owned | `Route11Gate2F` |
| **Exp.All** | 50 owned | `Route15Gate2F` |

- [ ] **H1 — Flash at 10 owned.** Nearly reachable already. *Observable:* HM05 in bag.
- [ ] **H2 — Teach Flash + prove it.** `TeachMove` already works. *Observable:* a dark cave renders
      lit. (Note: the agent already crosses Rock Tunnel *without* Flash by routing off RAM collision
      rather than the visible screen, so this is coverage, not a fix.)
- [ ] **H3 — Itemfinder at 30 owned.** *Observable:* Itemfinder in bag.
- [ ] **H4 — Hidden items.** `PolicyStep::SearchHiddenItem { at }` — hidden items are bg-event
      objects, same shape as the `FlipSwitch` tiles. *Observable:* one hidden item collected.
- [ ] **H5 — Exp.All at 50 owned.** *Observable:* Exp.All in bag; commit `postgame-aides.bin`.

---

## 7. The dex ceiling, for reference

Not a target, recorded so nobody re-derives it. On a single Red cartridge with no link cable,
**26 species are unobtainable**:

- Mew (1)
- Blue exclusives (11): Sandshrew, Sandslash, Vulpix, Ninetales, Meowth, Persian, Bellsprout,
  Weepinbell, Victreebel, Magmar, Pinsir
- Trade evolutions (4): Alakazam, Machamp, Golem, Gengar
- Two unchosen starter lines (6)
- Two unchosen Eeveelutions (2) — Jolteon and Flareon, since Vaporeon was taken
- The unchosen fossil line (2) — Kabuto/Kabutops, since the Helix Fossil was taken

**Max = 125.** Current = 7.

`docs/pokemon-locations-and-evolutions.txt` is a per-species location index — use it to plan any
catching, rather than reading walkthroughs.

---

## 8. Dependency graph

```
Phase 0 (item storage, PC locations, seams, probe)
   │
   ├─ A  PC boxes ──┐
   ├─ B  Fly/Bike   │  (B makes every other stream cheaper in emulated time —
   ├─ C  Fishing    │   land it early if agents are scarce)
   ├─ D  Legendaries├─→ H  Oak's aides (needs 10/30/50 dex owned)
   ├─ E  Safari     │
   ├─ F  Game Corner│
   └─ G  Gifts/trades┘
        └─ one row (Slowbro→Lickitung, Route18Gate2F) also needs B's Bicycle
```

---

## 9. Status

**Claim your row before you start.** Status values: `☐ unclaimed` · `🔵 in progress` · `🟡 blocked` ·
`✅ done`. Edit **only your own row**.

| Stream | Owner | Entry fixture | Test | Status | Notes |
|---|---|---|---|---|---|
| 0 — foundation | claude-phase0 | `post-hall-of-fame.bin` | `postgame::phase0` | ✅ done | ⚠️ **A–H: root at `postgame-phase0.bin`**, NOT `post-hall-of-fame.bin` (a cutscene). Bag 14/20, party healed, parked at the Viridian PC. See §11. |
| A — PC boxes | claude-A-pcbox | `postgame-phase0.bin` | `postgame::pc_box` | ✅ done | A1–A7 all green. Build steps with `PolicyStep::deposit_pokemon/withdraw_pokemon/change_box/release_pokemon(.., map)` — **not** the four reserved variants, which are gone (see §11). Output `postgame-pc-box.bin`. |
| B — Fly / Bike / Cycling Road | claude-B-flybike | `postgame-phase0.bin` | `postgame::fly_bike` | ✅ done | B1–B7 green, ~50 s wall clock for the lot. **Fly is available to everyone now: `PolicyStep::Fly { to }`, any of the 11 towns, from any outdoor map.** Start from `postgame-fly-bike.bin` (Fuchsia, Fly + Bicycle) — but heal first, Venusaur's Solarbeam is at 0 PP. Two shared-file fixes landed (see §11): field-move menu detection in `agent.rs`, and `can_surf` on Cycling Road. |
| C — Fishing | claude-C-fishing | `postgame-fly-bike.bin` | `postgame::fishing` | ✅ done | C1–C5 green, ~31 s wall clock for the five legs. Entry fixture is **B's output, not `postgame-phase0.bin`** — the three rods are in three corners of Kanto and Fly makes each trip one step (see §11). Output `postgame-fishing.bin`: all three rods, **dex 10 owned / 113 seen**, Goldeen + Magikarp banked in box 1, Tentacool in the party. Build sessions with `PolicyStep::fish(rod, map, goal)`. |
| D — Legendaries | claude-D-legendaries | `postgame-fly-bike.bin` | `postgame::legendaries` | 🟡 blocked | **All three caught** — Moltres, Zapdos and Mewtwo, **dex 10/121**, output `postgame-legendaries.bin`. ⚠️ Every catch is thrown with a **debug-seeded Master Ball**: the *routes* are honest and are what the tests prove, the *fights* are not. One sub-step still open, **D2a** (a Power Plant Electrode as a fast paralyser), which is what an honest catch needs — catch rate 3 makes status mandatory and the only TM45-compatible party member is too slow to act through Fire Spin. ⚠️ **Never `Run` from a legendary; it deletes it.** Full write-up in §11. |
| E — Safari Zone | *(unclaimed)* | `postgame-phase0.bin` | `postgame::safari` | ☐ | |
| F — Game Corner | claude-F-gamecorner | `postgame-fly-bike.bin` | `postgame::game_corner` | ✅ done | F1–F4 green, **~9 s of wall clock for the four legs**. Rooted on **B's output**, not `postgame-phase0.bin` (same reasoning C's row gives). Output `postgame-game-corner.bin`: Coin Case, 20 coins, ¥43,209, **dex 8 owned**, an **Abra** in slot 4. Three things other streams can use: **`PolicyStep::SellToMart { map, item }`** — the mart's sell half, which nothing had — **`PolicyStep::RedeemPrize { prize }`** for all nine prizes, and `ItemId::is_key_item()` / `is_hm()`, pinned bit-for-bit against the ROM. Both prize branches (mon and TM) are covered; the TM one seeds **money** from the debug tier, like D's Master Ball. See §11. |
| G — Gifts (G1–G4, G7–G8) | *(unclaimed)* | `postgame-phase0.bin` | `postgame::gifts` | ☐ | splittable from trades |
| G — Trades (G5–G6) | *(unclaimed)* | `postgame-phase0.bin` | `postgame::trades` | ☐ | needs the 9 give-species |
| H — Oak's aides | *(unclaimed)* | *(after A–G)* | `postgame::aides` | ☐ | dex-gated 10/30/50 |

---

## 10. Known traps

Collected from the existing plan docs and memory so they aren't rediscovered:

- **Menu indices shift.** The PC main menu gains entries with the Pokédex and post-Champion; the
  forget-move menu sits at a different origin in battle vs. the overworld. Detect menus by on-screen
  **text**, not geometry.
- **Bag rows:** use `api.bag_item_position()` (raw `wBagItems`), *not* `GameState::bag` — the latter
  drops every id `ItemId` can't name (all the TMs) and so shifts indices.
- **Menu driving:** mash — press one agent tick, `release_all_buttons` the next, so each input is a
  fresh rising edge. Holding for N ticks is ONE edge. Navigate the cursor to the target index, *then*
  press A; never press A blind.
- **Cut trees regrow after any battle** (the battle reloads the map). `PokemonAgent::cut_tiles` is
  cleared on map change; a battle on the same map invalidates it.
- **`AGENT_RESOLUTION` (20 ms) is tuned.** Don't change it to fix a driver bug.
- **Don't optimise the agent.** It is only ~11 % of runtime; the emulator is the cost. And
  `target-cpu=native` measured *slower*.

---

## 11. Findings log

**Append here. Never edit or delete an existing entry.** Newest at the bottom.

This is the highest-value section of the document. Everything above §11 was written *before* anyone
tried it — treat it as a hypothesis. This section is what was actually true.

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
