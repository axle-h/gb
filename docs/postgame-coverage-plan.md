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

- [ ] **B1 — Bike Voucher.** `PokemonFanClub` (Vermilion), talk to the chairman.
      *Observable:* voucher in bag.
- [ ] **B2 — Bicycle.** `BikeShop` (Cerulean), trade the voucher. *Observable:* Bicycle in bag.
- [ ] **B3 — HM02.** `Route16FlyHouse`, reached from Celadon via `Route16` (needs Cut).
      *Observable:* HM02 in bag.
- [ ] **B4 — Teach Fly.** `PolicyStep::TeachMove { item: Hm02Fly, .. }` already works — just use it.
      *Observable:* a party mon knows Fly.
- [ ] **B5 — The Fly driver.** `PolicyStep::Fly { to: Map }` + `AgentState::Flying`, driving
      START → POKéMON → mon → FLY → town-map cursor. ⚠️ The town map is a **bespoke screen**, not a
      `HandleMenuInput` list — budget real time for this one and record what you find in §11.
      *Observable:* the agent Flies between two towns.
- [ ] **B6 — Cycling Road.** `Route17` is bike-gated; `Route16/17/18` then connect Celadon → Fuchsia.
      *Observable:* the agent walks Celadon → Fuchsia via Cycling Road.
- [ ] **B7 — Route 16 Snorlax.** The **second** Snorlax; `UseFieldItem { PokeFlute }` already exists.
      *Observable:* Snorlax gone, route passable. Then commit `postgame-fly-bike.bin`.

### C — Fishing

- [ ] **C1 — Old Rod.** `VermilionOldRodHouse`. *Observable:* rod in bag.
- [ ] **C2 — The fishing driver.** `PolicyStep::Fish { rod, at }` + `AgentState::Fishing`: face a
      water tile, use the rod from the bag (same START → ITEM → USE chain as `UsingFieldItem`),
      handle the "not even a nibble" / "hooked" text, drop into the normal battle handler on a bite.
      *Observable:* a wild battle starts from a water tile.
- [ ] **C3 — Catch from a bite.** *Observable:* one water species in the dex.
- [ ] **C4 — Good Rod.** `FuchsiaGoodRodHouse`. *Observable:* rod in bag, different encounter table.
- [ ] **C5 — Super Rod.** `Route12SuperRodHouse`. *Observable:* as above; commit
      `postgame-fishing.bin`.

Opens the whole water encounter table — Magikarp/Goldeen/Poliwag/Tentacool/Krabby/Horsea/Staryu.

### D — Legendaries: Zapdos, Moltres, Mewtwo

Cheapest workstream by far — the machinery already exists. `CatchPokemon`'s **static-encounter
branch** (`policy.rs:1946`) routes to a map sprite named after the species and presses A; this is
exactly how Articuno was caught. Expect mostly navigation work, not new mechanics.

- [ ] **D1 — Moltres.** `VictoryRoad2F`. The map is already traversed by `complete_game_steps` — the
      agent walks straight past it. Do this first; it should be near-trivial and it validates the
      whole approach cheaply. *Observable:* Moltres in the dex.
- [ ] **D2 — Reach the Power Plant.** `PowerPlant`, entered by Surfing east off Route 10. Unvisited,
      so the route needs `EnterMap` steps. *Observable:* the agent stands in the Power Plant.
- [ ] **D3 — Zapdos.** Also the only Pikachu/Raichu/Voltorb/Magneton source, so consider catching
      those while there. *Observable:* Zapdos in the dex.
- [ ] **D4 — Stock Ultra Balls.** Mewtwo is lv70 and will need them. *Observable:* balls in bag.
- [ ] **D5 — Cerulean Cave.** `CeruleanCave1F/2F/B1F`, post-Champion only, Surf required.
      *Observable:* the agent reaches Mewtwo's chamber.
- [ ] **D6 — Mewtwo.** *Observable:* Mewtwo in the dex; commit `postgame-legendaries.bin`.

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

- [ ] **F1 — Coin Case.** From a man in the **`CeladonDiner`** (verified: `scripts/CeladonDiner.asm`
      is one of only two files referencing `COIN_CASE`). The Diner is on the unvisited-maps list.
      *Observable:* Coin Case in bag.
- [ ] **F2 — Buy coins.** `GameCorner` counter clerk, ¥1000 → 50 coins. *Observable:* `wPlayerCoins`
      rises; the probe confirms it.
- [ ] **F3 — Sell to a mart.** The mart driver only implements Buy today. Needed because Porygon is
      9999 coins ≈ ¥200 000. *Observable:* money rises after selling junk.
- [ ] **F4 — Redeem a prize.** `GameCornerPrizeRoom`: Abra, Clefairy, Nidorina, **Dratini**,
      **Scyther**, **Porygon**, plus prize TMs. *Observable:* a prize Pokémon in the party; commit
      `postgame-game-corner.bin`.

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
| B — Fly / Bike / Cycling Road | *(unclaimed)* | `postgame-phase0.bin` | `postgame::fly_bike` | ☐ | land early if agents are scarce |
| C — Fishing | *(unclaimed)* | `postgame-phase0.bin` | `postgame::fishing` | ☐ | |
| D — Legendaries | *(unclaimed)* | `postgame-phase0.bin` | `postgame::legendaries` | ☐ | cheapest; good first pick |
| E — Safari Zone | *(unclaimed)* | `postgame-phase0.bin` | `postgame::safari` | ☐ | |
| F — Game Corner | *(unclaimed)* | `postgame-phase0.bin` | `postgame::game_corner` | ☐ | |
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
