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
- [ ] **D2a — Catch an Electrode** (new) — see above. Still the unblock for the honest fights, and
      **attempted 2026-08-03: the route works, the fight does not.** A lv43 Electrode knows
      Selfdestruct and uses it ~1 turn in 4; the Power Plant's disguised Poké Balls are `trainer`-
      flagged, so losing hides them for the save. Both were lost. ~31 % per Electrode with a Great
      Ball at full HP, ~52 % across the two — a coin flip, not a tuning problem. Full arithmetic and
      the three things to try next are in the 2026-08-03 §11 entry;
      `PolicyStep::electrode_steps` and an `#[ignore]`d leg are committed to test a fix against.
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

- [x] **E1 — Model the step budget.** The **500-step** counter and the ejection back to the gate.
      Without this a run ends mid-hunt with no warning. *Observable:* the step count is in
      `GameState` and the probe prints it.
      *Done:* `GameState::safari` — `Option<SafariState>` carrying steps, balls and the game-over flag,
      keyed off `EVENT_IN_SAFARI_ZONE` rather than the map (they disagree exactly where it matters, see
      §11). The counter is **502**, not 500, and `probe_coverage` prints it for every fixture. Test
      `postgame::safari::runs_the_step_budget_down_and_is_ejected` (~25 s).
- [x] **E2 — Replace the blanket RUN.** `BattleAction::SafariBall/Bait/Rock` already exist and are
      already offered — write a real catch policy. Rock raises catch rate *and* flee rate; Bait does
      the inverse. *Observable:* the agent throws a ball instead of running.
      *Done:* [`postgame::safari::pick_battle_action`], scoped to a live `SafariHunt` step so the legs
      that merely *cross* the zone keep the old always-RUN behaviour. ⚠️ **BAIT and ROCK are never
      thrown**, and that is an evidence-backed decision, not a shortcut — worked exactly through the
      ROM's turn in `bait_and_rock_are_never_worth_throwing`, both lose to a plain ball. One shared-file
      bug had to be fixed first: the agent physically **could not press BALL**. See §11.
- [x] **E3 — Catch a Safari-exclusive.** Chansey, Scyther, Kangaskhan, Tauros, Dratini, Exeggcute,
      Rhyhorn, Parasect, Venomoth. (Pinsir is Blue-only.) *Observable:* one of them in the dex.
      *Done:* `PolicyStep::safari_hunt_steps(targets, max_trips)`, test `can_catch_a_safari_exclusive`
      (~4 s) — a **Rhyhorn**, dex 19 → 20, caught on the second ball of the first trip.
- [x] **E4 — Exit cleanly.** Both ways: walking out, and being ejected at 0 steps.
      *Observable:* test green both ways; commit `postgame-safari.bin`.
      *Done:* both, in the two tests above — walking out crosses the gate's "leaving early?"
      (`YesNoChoice`, opens on YES, no driver needed); the ejection is the ROM warping the player to the
      gate at 0 steps. ⚠️ The two are **not symmetric**, and the gap is a trap: `EVENT_IN_SAFARI_ZONE`
      stays set for a few ticks *after* the ejection warp, so a hunt that keeps routing there pays a
      second ¥500. See §11.
- [x] **E5 — Sweep all four areas** (added; E3 at full size). *Observable:* dex past H3's gate of 30.
      *Done:* `PolicyStep::safari_sweep_steps(max_trips)` + `safari::grounds`, test
      `can_sweep_the_safari_zone` — **dex 19 → 31 owned, all twelve targets, 21 trips, ¥9,000, ~6.5 min
      of wall clock**. Fixture `postgame-safari.bin`. Which *area* each species is hunted in is the
      whole cost of this leg — the same species sits in a 4.3 % slot on one map and a 1.2 % slot on
      another. ➡️ **H3 (Itemfinder, 30 owned) and H4 are unblocked.**

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

- [x] **G1 — Fossil revival.** `CinnabarLabFossilRoom`. The agent already carries a **Helix Fossil**,
      so Omanyte is one interaction away. *Observable:* Omanyte in the party.
      *Done:* `PolicyStep::fossil_revival_steps()`, test `postgame::gifts::can_revive_the_helix_fossil`
      (~3 s), fixture `postgame-omanyte.bin`, **Omanyte lv30, dex 8 owned**. Not "one interaction" —
      it is **two visits**: the scientist takes the fossil and asks you to go for a walk, and the walk
      is `CinnabarIsland_Script` clearing `EVENT_LAB_STILL_REVIVING_FOSSIL` on map load. Nothing else
      needed a driver; see §11.
- [x] **G2 — Old Amber.** `Museum1F/2F` (Pewter), behind a Cut tree → Aerodactyl at the same lab.
      *Observable:* Aerodactyl in the party.
      *Done:* `PolicyStep::old_amber_steps()`, test `can_get_the_old_amber_and_revive_it` (~5 s),
      fixture `postgame-aerodactyl.bin`, **dex 9 owned**. The Cut tree **is** load-bearing (measured by
      dropping it — see §11), and the giver is `MUSEUM1F_SCIENTIST2`, not the `SPRITE_OLD_AMBER` object
      standing beside him, which is scenery.
- [x] **G3 — Lapras.** `SilphCo7F`, from the rescued employee. Silph is already traversed; the gift
      was simply never taken. *Observable:* Lapras in the party.
      *Done:* `PolicyStep::lapras_steps()`, test
      `a_full_party_sends_the_silph_lapras_to_the_box` (~4 s), fixture `postgame-lapras.bin`,
      **dex 10 owned**. Two corrections in §11: the **lift does not reach the worker** (7F is cut into
      pockets; he is in the rival one, behind 3F's pad — measured by the `probe_silph_7f_pockets`
      diagnostic left in the test file), and a **full party does *not* skip the naming screen** —
      `SendNewMonToBox` runs its own `AskName`. Run deliberately at party 6 to cover that branch.
- [x] **G4 — Fighting Dojo.** Saffron: beat the Karate Master, choose Hitmonlee or Hitmonchan.
      *Observable:* the chosen mon in the party.
      *Done:* `PolicyStep::hitmonlee_steps(bank_slot)`, test
      `can_beat_the_karate_master_and_take_a_hitmonlee` (~16 s, five dojo battles), fixture
      `postgame-hitmonlee.bin`, **dex 11 owned**. ⚠️ **Hitmonchan is now gone for this cartridge** —
      taking either ball `HideObject`s the other. A slot is banked at the Saffron PC first so this
      lands in the *party*, covering the branch G3 does not. ➡️ **dex owned is now 11, so H1 (Flash,
      gate 10) is unblocked.**
- [x] **G5 — The trade driver.** ~~`PolicyStep::TradePokemon { give_slot, at }`~~ driving the
      offer/accept flow. *Observable:* one trade completes.
      *Done:* **Abra → Mr. Mime** at `Route2TradeHouse`, test
      `postgame::trades::can_trade_an_abra_for_a_mr_mime` (~18 s), fixture `postgame-mr-mime.bin`.
      ⚠️ **The reserved `TradePokemon` / `AgentState::Trading` seams are gone.** A trade NPC opens the
      same stale-cursor party menu the Day Care does, so a trade is a third
      `postgame::gifts::PartyScript` variant and needed no driver at all. Build with
      `PolicyStep::trade_steps(give, catch_on, bank, bank_at)`, which banks a party slot, catches the
      give-species, travels and trades.
- [x] **G6 — Three more trades.** From the table below. *Observable:* dex count rises by three.
      *Done:* **Spearow → Farfetch'd** (`VermilionTradeHouse`), **Nidoran♂ → Nidoran♀**
      (`UndergroundPathRoute5`) and **Venonat → Tangela** (`CinnabarLabTradeRoom`); fixtures
      `postgame-farfetchd.bin`, `postgame-trades.bin`, `postgame-tangela.bin`. Four trades in all,
      **dex 11 → 19 owned** — each is worth two entries, the mon caught and the mon received, and
      three of the four received species (Mr. Mime, Farfetch'd, Tangela) are obtainable no other way
      on one cartridge. The remaining five need an evolution grind or the Safari Zone; see §11.
- [x] **G7 — Skipped Silph floors.** `2F/4F/6F/8F/10F` — item pickups only. *Observable:* items in bag.
      *Done:* `PolicyStep::silph_floors_steps(bank)`, test `can_clear_the_skipped_silph_floors`
      (~14 s), fixture `postgame-silph-floors.bin`. **Ten** items, not eight — 7F's Calcium and TM03
      are on the *lift* side, which G3's route could not reach. Against five free bag slots, so this
      is also the first leg to compose Phase 0's item PC with pickup. Three findings in §11, two of
      them bugs: `UseElevator` **rides you back where you came from** if issued while still on the
      lift tile, and `deposit_item` **hangs** when asked for more of a stack than is held (fixed —
      the quantity is now clamped to the live stack, so a caller can pass `u8::MAX` for "all of it").
**G8 was three sub-steps, not one** (§0.3: split it here first, then do them). The TM gifts and the
Day Care are unrelated mechanics with unrelated failure modes, and the Day Care needed a driver:

- [x] **G8a — The TM gifts.** `MrPsychicsHouse` (TM29) and `CopycatsHouse1F/2F` (TM31 Mimic for a
      Poké Doll). *Observable:* both TMs in the bag; commit `postgame-gifts.bin`.
      *Done:* `PolicyStep::saffron_tm_gifts_steps(bank)`, test `can_collect_the_saffron_tm_gifts`
      (~7 s). The Copycat is the only **conditional** gift in the plan — no doll, no refusal message,
      just one indistinguishable text box — so the leg buys the ¥1000 doll at `CeladonMart4F` first
      and TM31 is the proof it was held. `ItemId` gained `Tm29Psychic` / `Tm31Mimic`.
- [x] **G8b — The Day Care.** `Daycare` (Route 5): leave a mon, collect it, pay.
      *Observable:* party count down then up, and the bill paid; commit `postgame-daycare.bin`.
      *Done:* `PolicyStep::UseDaycare { slot }` + `AgentState::UsingDaycare` +
      [`postgame::gifts::tick`], test `can_leave_a_pokemon_at_the_day_care` (~3 s). **The only
      sub-step in G that needed a driver**, for one reason: the gentleman's `DisplayPartyMenu` opens
      on a *stale* cursor, so an A-mash hands over an arbitrary mon and he refuses every HM carrier
      for ever. Route 5 is also three one-way corridors — see §11. ➡️ **The driver is the reusable
      part**: G5/G6's trades and the Name Rater open the same script-driven party menu.
- [x] **G8c — The remaining colour rooms.** `NameRatersHouse`, `ViridianSchoolHouse`,
      `CeladonDiner`/`Hotel`/`ChiefHouse`. *Observable:* a renamed party mon; the rooms visited.
      *Done:* `PolicyStep::name_rater_and_rooms_steps(slot)`, test
      `can_rename_a_pokemon_and_visit_the_last_rooms` (~4 s), fixture `postgame-name-rater.bin`. The
      Name Rater is G8b's driver with a different completion test, which is why `PartyScript` is an
      enum — **G5/G6's trades should be its third variant, not a fourth driver**. The wrinkle was
      real and sharper than expected: the nickname picker is re-seeded *per leg*, so five of the six
      party members are already called what the first draw returns and only **Articuno** can be
      observably renamed at all. `CeladonDiner` is not in this leg — F already opens it for the Coin
      Case.

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

- [x] **H1 — Flash at 10 owned.** Nearly reachable already. *Observable:* HM05 in bag.
- [x] **H2 — Teach Flash + prove it.** `TeachMove` already works. *Observable:* a dark cave renders
      lit. (Note: the agent already crosses Rock Tunnel *without* Flash by routing off RAM collision
      rather than the visible screen, so this is coverage, not a fix.)
      *Done (both):* `PolicyStep::flash_steps(withdraw, shed, flash_slot)`, test
      `postgame::aides::can_get_hm05_and_light_rock_tunnel` (~6 s), fixture `postgame-flash.bin`. The
      aide's own text confirms the gate — *"You have caught 19 kinds of POKéMON!"*. H2's observable is
      the ROM's: entering `RockTunnel1F` sets `wMapPalOffset = 6` and Flash is the only thing that
      clears it, now readable as **`GameState::map_is_dark`** and driven by the new
      `PolicyStep::UseFlash { slot }`. ⚠️ **Only Slowpoke and Mr. Mime can learn Flash** of everything
      this save has owned; see §11.
- [x] **H3 — Itemfinder at 30 owned.** *Observable:* Itemfinder in bag.
      *Done:* [`PolicyStep::itemfinder_steps(shed)`], test `postgame::aides::can_get_the_itemfinder`
      (~10 s), fixture `postgame-itemfinder.bin`, entered from **E's** `postgame-safari.bin` (dex 31).
      The aide's own text confirms the gate — *"You have caught 31 kinds of POKéMON!"*. `shed` is two
      bag slots deposited on the way past, because the bag is 20/20 and a full bag is the one failure
      that reads exactly like success.
- [x] **H4 — Hidden items.** ~~`PolicyStep::SearchHiddenItem { at }` — hidden items are bg-event
      objects, same shape as the `FlipSwitch` tiles.~~ *Observable:* one hidden item collected.
      *Done:* `PolicyStep::SearchHiddenItem { map, item }` + `hidden_item_steps`, test
      `postgame::aides::can_collect_a_hidden_item` (~0.4 s), fixture `postgame-hidden-item.bin`.
      **"Same shape as `FlipSwitch`" was an understatement — it is the *same field move*.** The
      reserved `FieldMove::SearchHiddenItem` and `AgentState::SearchingHiddenItem` seams are deleted;
      `FieldMove::CheckTrashCan` drives all three. New and reusable: **`MetaTileMap::hidden_items`**
      (all 54, ROM-derived, connection-offset applied) and
      `postgame::aides::hidden_items(map) -> Vec<HiddenItem>`. ⚠️ **The Itemfinder is not a
      prerequisite** — see §11.
- [x] **H5 — Exp.All at 50 owned.** *Observable:* Exp.All in bag; commit `postgame-aides.bin`.
      *Done:* **dex 31 → 52 owned**, Exp.All collected, `postgame-aides.bin`. Unlike H1–H4 this is not
      a mechanism, it is a **catching errand**, split into four legs of a Fly stop each — ~110 s of
      wall clock for the lot once the routes were right:
      - [x] **H5a — outfit + the Vermilion grounds** (~10 s, 31 → 34). 99 Poké Balls, an empty box,
            then Route 11 (Ekans, Drowzee) and Diglett's Cave (Diglett 94.5 %).
            `postgame-sweep-vermilion.bin`.
      - [x] **H5b — Route 1 + Viridian Forest** (~38 s, 34 → 41). Pidgey, Rattata, Weedle, Kakuna, and
            at a 5 % floor Caterpie, Metapod and **Pikachu** too. `postgame-sweep-viridian.bin`.
      - [x] **H5c — the Lavender grounds** (~17 s, 41 → 46). Pokémon Tower **3F** (Gastly 89.5 %) and
            Rock Tunnel 1F (Zubat, Geodude, Machop). `postgame-sweep-lavender.bin`.
      - [x] **H5d/e — Route 7, the Mansion, then the aide** (~43 s, 46 → 52). `postgame-aides.bin`.

      New and reusable, and the reason the grounds above are *these* grounds: **`crate::pokemon::wild`**
      — the ROM's own encounter tables, decoded (`WildDataPointers` + `WildMonEncounterSlotChances`), so
      "what lives here and how often" is a lookup rather than a walkthrough. Plus
      `PolicyStep::SweepDex { on_map, min_share, ball }`, which catches everything the dex is missing on
      a map and leaves once its slots above `min_share` are owned. ⚠️ **Five traps, four of them silent,
      all in §11** — the two that will catch anyone are the **Silph Scope** (in the PC, so every
      Pokémon Tower wild is an uncatchable GHOST) and **pacing into an obstacle** (no step, so no
      encounter roll, so nothing happens at all).

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
| D — Legendaries | claude-D-legendaries | `postgame-fly-bike.bin` | `postgame::legendaries` | 🟡 blocked — **D2a**, and the blocker is the *fight*, not the route (see the 2026-08-03 §11 entry) | **All three caught** — Moltres, Zapdos and Mewtwo, **dex 10/121**, output `postgame-legendaries.bin`. ⚠️ Every catch is thrown with a **debug-seeded Master Ball**: the *routes* are honest and are what the tests prove, the *fights* are not. One sub-step still open, **D2a** (a Power Plant Electrode as a fast paralyser), which is what an honest catch needs — catch rate 3 makes status mandatory and the only TM45-compatible party member is too slow to act through Fire Spin. ⚠️ **Never `Run` from a legendary; it deletes it.** Full write-up in §11. |
| E — Safari Zone | claude-E-safari | `postgame-flash.bin` | `postgame::safari` | ✅ done | E1–E4 green plus an **E5** the plan lacked (the four-area sweep) — **~7 min of wall clock across 3 legs**, output `postgame-safari.bin`: **dex 31 owned / 116 seen**, all twelve Safari species, party 6, box **18 of 20**, ¥35,564. Rooted on **H's output** (the chain head), not `postgame-phase0.bin` — same reasoning C/F/G's rows give; ⚠️ it is saved inside Rock Tunnel, so every leg opens with a `Dig`. **Three shared-file fixes, all in §11 and all of them other people's problems too:** the battle executor **could not press BALL** (only RUN was treated as terminal); the **boxed-catch wedge D reported is fixed** (START at a prompt that only takes A — so "leave a party slot free before a catch" is retired); and `can_surf` is false in the zone (Surf is refused there, and the BFS was routing across the pond). New for anyone: `GameState::safari`, `BattleState::enemy_catch_rate`, `PolicyStep::{safari_hunt_steps, safari_sweep_steps, SafariExit}`. ⚠️ **BAIT and ROCK are never thrown** — worked exactly through the ROM, both lose to a plain ball. ➡️ **H3 + H4 unblocked.** |
| F — Game Corner | claude-F-gamecorner | `postgame-fly-bike.bin` | `postgame::game_corner` | ✅ done | F1–F4 green, **~9 s of wall clock for the four legs**. Rooted on **B's output**, not `postgame-phase0.bin` (same reasoning C's row gives). Output `postgame-game-corner.bin`: Coin Case, 20 coins, ¥43,209, **dex 8 owned**, an **Abra** in slot 4. Three things other streams can use: **`PolicyStep::SellToMart { map, item }`** — the mart's sell half, which nothing had — **`PolicyStep::RedeemPrize { prize }`** for all nine prizes, and `ItemId::is_key_item()` / `is_hm()`, pinned bit-for-bit against the ROM. Both prize branches (mon and TM) are covered; the TM one seeds **money** from the debug tier, like D's Master Ball. See §11. |
| G — Gifts (G1–G4, G7–G8) | claude-G-gifts | `postgame-fly-bike.bin` | `postgame::gifts` | ✅ done | All of G1–G4, G7 and G8a–c green — **~57 s of wall clock across 8 legs**, output `postgame-name-rater.bin`: **dex 11 owned**, party Venusaur / Articuno / Vaporeon / Slowpoke / Aerodactyl / **Hitmonlee**, box 1 holding **Lapras** + Omanyte, bag 19/20 with TM29/TM31/TM03/TM26 and the ten Silph items. Rooted on **B's output** (same reasoning C's and F's rows give). Three things other streams can use: **`PolicyStep::PartyScript { script, slot }`**, the first driver here that can pick a slot in a **script-opened party menu** — **G5/G6's trades should be a third `PartyScript` variant, not a new driver** — a `deposit_item` fix (it **hung** on any full stack), and `probe_coverage` now printing **PC item storage**. Two shared-file bugs found, one fixed; five §11 entries. ➡️ **H1 is unblocked** (dex 11 > Flash's gate of 10). |
| G — Trades (G5–G6) | claude-G-trades | `postgame-name-rater.bin` | `postgame::trades` | ✅ done | **Four** trades green — Abra→Mr. Mime, Spearow→Farfetch'd, Nidoran♂→Nidoran♀, Venonat→Tangela — **~55 s of wall clock across 4 legs**, output `postgame-tangela.bin`: **dex 19 owned**, party Venusaur / Articuno / Vaporeon / Tangela, eight mons banked in box 1. Rooted on **G-gifts' output**: a trade is a third `PartyScript` variant, so the reserved `TradePokemon` / `AgentState::Trading` seams are **deleted**. `TRADES` is the nine-row table, ROM-pinned. The other five trades are not blocked, just expensive — Slowbro/Poliwhirl/Nidorino/Raichu need evolution grinds and Ponyta is in the Pokémon Mansion; see §11. |
| H — Oak's aides | claude-H-aides | `postgame-tangela.bin` (H1/H2) · `postgame-safari.bin` (H3+) | `postgame::aides` | ✅ done | **All of H1–H5.** H1/H2 HM05 Flash + a lit Rock Tunnel; **H3** the Itemfinder (~10 s), rooted on **E's** `postgame-safari.bin` whose dex 31 cleared the gate of 30; **H4** a hidden Escape Rope on Route 11 (~0.4 s); **H5** the dex sweep, **31 → 52 owned**, Exp.All in the bag (~110 s across four legs). Output `postgame-aides.bin`. ⚠️ **H4 needed neither H3's Itemfinder nor a driver** — the Itemfinder only *detects*, and a hidden item is `FieldMove::CheckTrashCan`, so the reserved `FieldMove::SearchHiddenItem` / `AgentState::SearchingHiddenItem` seams are **deleted**. New and reusable: `PolicyStep::{UseFlash, SearchHiddenItem { map, item }, SweepDex { on_map, min_share, ball }}`, `GameState::map_is_dark`, **`MetaTileMap::hidden_items`** (all 54, ROM-derived, offset-corrected) and **`crate::pokemon::wild`** (the ROM's encounter tables — species, level and share per map). ⚠️ **Four shared-file bug fixes, all of them silent failures other streams could hit:** a ball thrown at a **trainer's** Pokémon loops forever; `adjacent_grass` ignored `pair_blocked`; the pacer had no stall guard, so walking into a sprite farmed zero encounters; and `MetaTile::Water` matched **impassable** shore-id tiles, putting six phantom water tiles in Viridian Forest. See §11.

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
