# Covering the rest of Pokémon Red — plan **and** work record

The agent finishes the game. It had barely *played* it. This document is how that gap was closed, and
what is still open.

> ## Where this stands — 2026-08-04
>
> **Phase 0 and workstreams A–H are done** ([§5](#5-phase-0--foundation-) and [§6](#6-workstreams-ah-)). Every mechanic
> in the original scope has a `PolicyStep`, a driver and a test; the dex went **7 → 52 owned /
> 111 → 117 seen** as a side effect. Chain head: `postgame-aides.bin`.
>
> **Four workstreams are open** ([§8](#8-open--workstreams-il)), all agreed with Alex on 2026-08-04:
> **I** the rest of the ROM's item-use table, **J** fast fixtures (options written straight to RAM),
> **K** one of the five unproven in-game trades, **L** a visit to every visitable map.
>
> ⚠️ **Two things are closed by decision and must not be reopened**: winning the legendary fights
> without a Master Ball, and any exhaustive sweep (dex, hidden items, gift TMs). Both are in
> [§3](#3-scope)'s "Out" table with the reasoning.
>
> This file was condensed from a 2,500-line working document on 2026-08-04. The raw findings log —
> probe dumps, ROM citations, every wrong assumption and what it cost — is
> **[`postgame-findings-archive.md`](postgame-findings-archive.md)**. It is history, not a to-do list,
> but it is where the *evidence* for everything below lives. New findings still get appended there.

---

## 1. Definition of done

**Mechanism coverage.** Every distinct mechanic the game offers has a `PolicyStep`, a driver, and a
focused test proving it works. Pokédex count rises as a side effect and is **not** the target; there
is no exhaustive catching sweep here.

The yardstick for "every mechanic" is the ROM, not a walkthrough. For items it is
`engine/items/item_effects.asm`'s `ItemUsePtrTable` — one entry per distinct effect, which is exactly
the checklist workstream **I** works through. For maps it is the `Map` enum, which is workstream **L**.

---

## 2. Ground truth

Measured, never assumed. Re-read any fixture with the coverage probe:

```bash
cargo test --release --bin gb -- pokemon::integration_tests::fixture::probe_coverage --exact --ignored --nocapture
```

It prints map, badges, money, coins, dex owned/seen, `wBoxCount`/`wCurrentBoxNum`, the open box, the
party, a **raw** bag read (`GameState::bag` silently drops every id `ItemId` cannot name, i.e. most
TMs), PC item storage and the Safari step budget, for every committed `postgame-*.bin`.

**Where it started**, `post-hall-of-fame.bin`: badges 255, ¥37,774, party 4, dex 7/111, bag **20/20 —
full**, `wBoxCount=0`. Two facts drove the whole ordering: the full bag meant the agent physically
could not pick up HM02, a rod or the Itemfinder until item PC storage existed (hence Phase 0), and
`wBoxCount=0` meant the party could never exceed 6 (hence workstream A first).

**Where it is now**, `postgame-aides.bin`: Route 15, ¥5,894, party 6 (Venusaur 71, Articuno 73,
Vaporeon 71, Tangela, Slowpoke, Rhyhorn), box 3 holding 6, **dex 52 / 117**, bag 20/20 with all five
HMs, the Itemfinder and the Exp.All.

⚠️ **The streams are siblings off Phase 0, not one chain** — so no single save holds everything. The
three legendaries live on `postgame-legendaries.bin`; the dex-52 state lives on `postgame-aides.bin`.
Merging them was considered and dismissed (§3).

---

## 3. Scope

### In

Mechanism coverage for: item + Pokémon PC storage, Fly, the Bicycle and Cycling Road, fishing, the
three remaining legendaries, Safari Zone catching, the Game Corner prize economy, fossil revival,
in-game trades, gift Pokémon, the dex-gated Oak's-aide items (all **done**, §5–§6), and — added
2026-08-04 — the rest of the item-use table, RAM-set options, one more trade, and a full map visit
(**open**, §8).

### Out — decided, do not relitigate

| Out | Why |
|---|---|
| Link-cable content | `Colosseum` / `TradeCenter` / Cable Club, and the 4 trade evolutions (Alakazam, Machamp, Golem, Gengar) need a second cartridge. |
| Glitches & exploits | Missingno, the Mew glitch, item duplication, the Old Man trick. Anything relying on ROM/emulator bugs rather than intended play. |
| The MCP text interface | `CLAUDE.md` names it as the project goal and it does not exist yet. It gets its own plan; this one is about game coverage. |
| The slot machine minigame | Coins are purchasable at the Game Corner counter (¥1000 → 50 coins), which reaches every prize without driving an RNG-heavy reel minigame. |
| An exhaustive dex sweep | Mechanism coverage is the goal. See [§7](#7-the-dex-ceiling-for-reference) for the ceiling if this is ever revisited. |
| **Winning the legendary fights honestly** | Decided 2026-08-04, after it had eaten a good deal of effort. The three catches use a debug-seeded **Master Ball** and the tests prove the **routes**, which is the mechanism. Beating a catch-rate-3 encounter that must not be KO'd, fled or lost is *battle tactics*, and in deployment those are the **LLM's** decisions, not the deterministic policy's. This retired sub-steps D2a and D4. The archive entries that chased it are history. |
| The **OPTION menu driver** | Decided 2026-08-04. The options are worth *setting* but not worth *driving* — workstream **J** writes `wOptions` in the debug tier instead. |
| The **Pokédex screen** and the **Town Map item** | Dex state is read from RAM; the town-map screen is already driven by Fly (B5). Opening either from the menu adds no mechanism. |
| Uncollected **NPC-gift TMs** | Swift (`Route12Gate2F`), Metronome (`CinnabarLabMetronomeRoom`), Selfdestruct (`SilphCo2F`), the three Celadon Mart roof drink-for-TM prizes, Dream Eater (Viridian), Dig (Cerulean), Softboiled (Celadon). The gift mechanism is proven by G8a; these are content. |
| A **hidden-item sweep** | All 54 are mapped in `MetaTileMap::hidden_items` and H4 proved collection. Collecting the rest buys nothing. |
| A merged **"everything" save** | Nice to look at, expensive to maintain, and it would turn the sibling fixtures back into a chain where one leg's failure invalidates the rest. |

### The RAM-write rule

The repo's existing claim — *"no RAM-write shortcuts remain in the play path"* — **stands**. RAM writes
are allowed, but only in an explicitly-named debug tier:

- **Play path** (anything reachable from `Policy::pick_*` during a legitimate run): button input only.
- **Debug tier** (`PokemonApi::debug_*`): free to write RAM. Used *only* for fixture construction, test
  seeding and diagnostics. `postgame::debug::play_path_contains_no_debug_ram_writes` greps for it and
  fails if it appears in `policy.rs`, `agent.rs` or `postgame/`.

`PolicyStep::MovePokemonToFront` is the one pre-existing violation (it writes party order directly).
Leave it; don't add more. Workstreams **J** and **K** both use the debug tier deliberately and say so.

---

## 4. Rules of engagement

### 4.1 File ownership

`policy.rs` and `agent.rs` are the conflict hotspots. The seam that makes parallel work possible:
**each workstream owns two new files and touches shared files on exactly four lines.**

```
src/pokemon/postgame/<stream>.rs                   ← owned: step-list constructors + driver logic
src/pokemon/integration_tests/postgame/<stream>.rs ← owned: the tests
```

Rust allows `impl` blocks for a type in any module of the same crate, so a workstream adds its step
constructors as `impl PolicyStep { … }` **in its own file**. The four shared-file lines are one
`PolicyStep` variant, one `AgentState` variant, and one match arm each — **delegating in one line,
with no logic inline**:

```rust
PolicyStep::UsePcBox { .. } => return postgame::pc_box::pick(self, state, world_graph),
AgentState::UsingPcBox(s)   => return postgame::pc_box::tick(self, api, s),
```

One-line arms merge cleanly. Inline bodies do not. This is the single most important rule here.

### 4.2 Fixtures

- Root at `postgame-phase0.bin`, or at another stream's output where that is cheaper — most chose
  **B's** `postgame-fly-bike.bin`, because Fly makes every trip one step. Name your output
  `postgame-<stream>.bin`.
- `complete_game_steps` and `full_playthrough` are **frozen**. Do not insert side content into the
  mainline; a later backport pass can move things earlier once each is individually green.
- Fixture writes stay gated behind `--features regen-fixtures`. If a leg test fails, check
  `git status src/pokemon/data/` **first** — drift is the usual cause, not your code.

### 4.3 Tests

- New tests go in the `slow-tests` tier unless they emulate under ~30 s of game time.
- `--release` always. The crate has no lib target: `--bin gb`, never `--lib`.
- Full path in the filter, e.g.
  `cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests::postgame::pc_box --nocapture`
- Wall clock ≈ emulated-minutes ÷ 23. Budget accordingly and say so in the test doc comment.
- Failure artifacts (save state + screenshot) land in `target/test-artifacts/`. Read the screenshot
  before theorising; it identified the boxed-catch wedge in one look.

### 4.4 Working protocol

- **Claim your row** in [§9](#9-status) before starting. Work in small increments, each ending in
  something *observable* — a passing test, a probe dump, a fixture — not "code written".
- **Record what you learn** in [`postgame-findings-archive.md`](postgame-findings-archive.md),
  especially an assumption here that turned out **wrong**: that is worth more to the next agent than a
  finished task. Prefer evidence over assertion — a ROM `file:line` or a probe dump settles an
  argument, a recollection does not. This project has a track record of confident misdiagnosis.
- **Done** means: sub-steps ticked, the test green in the `slow-tests` tier, the fixture committed, the
  §9 row reading `✅ done`, and at least one archive entry — even if it only says "the plan was right".

---

## 5. Phase 0 — foundation ✅

**Phase 0.** The coverage probe, the `postgame` module skeleton,
`MetaTileMap::pc_locations` (every Pokémon Center's PC is a hidden object at **(13,3)**, facing up),
walking out of the Hall of Fame, `PolicyStep::UsePc`, `DepositItem`/`WithdrawItem`, and the `debug_*`
tier with its guard test. ⚠️ `post-hall-of-fame.bin` is a **cutscene, not a playable state** — A–H root
at `postgame-phase0.bin`. ⚠️ A PC can only be used **from below**. ⚠️ Phase 0 banked the bag down to
14/20 and the **Silph Scope** went to the PC with it, which is why every chain rooted here reads every
Pokémon Tower wild as an uncatchable GHOST.

---

## 6. Workstreams A–H ✅

The record, not the plan: what was built, what is reusable, and the warnings worth carrying. Evidence
for every claim is in the archive. Section numbering here is load-bearing — source comments cite
`§6-B`, `§6-G` and so on.

### A — Pokémon storage (PC boxes) ✅

`GameState::boxed_pokemon` + `.current_box`; `PolicyStep::deposit_pokemon` / `withdraw_pokemon` /
`change_box` / `release_pokemon`. Box data is in **WRAM** and only
the *open* box is readable. ⚠️ The PC menu's labels vary with progress but **entry 0 is always the
player's PC** — detect menus by on-screen text, never geometry. ⚠️ `change_box` **saves the game**.

### B — Fly, the Bicycle, Cycling Road ✅

The biggest quality-of-life win here:
**`PolicyStep::Fly { to }` reaches any of the 11 towns from any outdoor map**, and every later stream
is built on it. Bike Voucher → Bicycle → HM02 → teach Fly (**Articuno was the only compatible party
member**). ⚠️ The town map is a **bespoke screen**: no cursor in RAM, no flag — it is identified by its
**broken font**. ⚠️ HM02 depends on the Bicycle: `Route16Gate1F` is two corridors and the guard between
them wants to see it. Nothing has to *use* the Bicycle — owning it opens the gate (see **I6**). Two
shared fixes landed here: field-move menu detection, and `can_surf` false on Cycling Road.

### C — Fishing ✅

All three rods; `PolicyStep::Fish { rod, map, goal }`. The *policy* picks the water
tile and owns the repetition, one cast per driver invocation. ⚠️ The cast animation must be **mashed
through, not waited out** — its flag clears only after a `prompt`. Everything fishable is catch rate
155–255, so no weakening pass.

### D — Legendaries ✅

Routes to Moltres (`VictoryRoad2F`, a **third separately-sealed region** reached
only via VR3F's (2,0) warp), Zapdos (Power Plant) and Mewtwo (Cerulean Cave, reached only by **Route
24's left river seam**, a `ConnectionWater`; 1F's B1F ladder is behind a tile-pair elevation boundary
passable only via 2F). ⚠️ **Never `BattleAction::Run` against one** — they are `trainer`-flagged map
objects and `EndTrainerBattle` hides them on any exit but a blackout, so running deletes the legendary
from the save. ⚠️ **Never weaken one** either: all three are catch rate 3, where the ball's status
subtraction is the only lever that moves at all. All three catches are **Master-Ball-seeded on
purpose** (§3). One general fix survived that effort: `CatchPokemon`'s static branch matched sprite
names exactly, and a map with several of a species numbers them (`Electrode 1`/`2`) — `sprite_is_species`
strips the number.

### E — Safari Zone ✅

`GameState::safari` (steps, balls, game-over) keyed off `EVENT_IN_SAFARI_ZONE`;
⚠️ the counter is **502**, not 500. A real catch policy scoped to a live `SafariHunt` step, plus
`safari_sweep_steps` — all twelve targets, **dex 19 → 31**, 21 trips, ¥9,000. ⚠️ **BAIT and ROCK are
never worth throwing** (worked through the ROM's turn in a unit test; both lose to a plain ball).
⚠️ The two exits are not symmetric — `EVENT_IN_SAFARI_ZONE` lingers a few ticks past the ejection warp,
so a hunt that keeps routing there pays a second ¥500. Three shared fixes: the battle executor **could
not press BALL**; the **boxed-catch wedge** (START at a prompt that only takes A) is fixed, retiring
the old "leave a party slot free" rule; `can_surf` is false in the zone.

### F — Game Corner ✅

Coin Case (from the **gym guide** in `CeladonDiner`), `BuyGameCoins`,
`SellToMart` (the mart's sell half, which nothing had), `RedeemPrize` with all nine prizes pinned
against the ROM. ⚠️ Selling is **not** buying in reverse: different list, halved prices, and **no
screen shows that it worked**. Also new: `ItemId::is_key_item()` / `is_hm()`.

### G — Gifts, trades, one-off rooms ✅

Omanyte, Aerodactyl, Lapras, Hitmonlee, four in-game trades
(**dex 11 → 19**), the five skipped Silph floors (**ten** items, not eight), the Saffron TM gifts, the
Day Care and the Name Rater. Most reusable thing here: **`PolicyStep::PartyScript { script, slot }`** —
any script-opened party menu, because those open on a *stale* cursor and an A-mash hands over an
arbitrary mon. ⚠️ Fossil revival is **two visits**; the *walk away* is what clears the flag.
⚠️ Taking Hitmonlee `HideObject`s Hitmonchan for this cartridge. ⚠️ A **full party does not skip the
naming screen**. ⚠️ `UseElevator` **rides you back where you came from** if issued while still on the
lift tile. ⚠️ `deposit_item` on a partial stack frees no bag row — pass `u8::MAX` for "all of it".

### H — Oak's aides ✅

HM05 Flash at 10 owned (Route 2 Gate), Itemfinder at 30 (`Route11Gate2F`),
Exp.All at 50 (`Route15Gate2F`) — **check the gate with the probe before travelling**. `UseFlash` +
`GameState::map_is_dark` (the ROM's own observable: `wMapPalOffset = 6` on entering `RockTunnel1F`, and
Flash is the only thing that clears it). ⚠️ **The Itemfinder is not a prerequisite for hidden items** —
it only detects; `FieldMove::CheckTrashCan` drives trash cans, switches and hidden items alike, because
the ROM dispatches all three from `CheckForHiddenObject`. New: **`MetaTileMap::hidden_items`** (all 54,
ROM-derived, connection-offset applied) and **`crate::pokemon::wild`** — the ROM's encounter tables
decoded, so "what lives here and how often" is a lookup — plus `PolicyStep::SweepDex`, which took the
dex **31 → 52**.

---

## 7. The dex ceiling, for reference

Not a target, recorded so nobody re-derives it. On a single Red cartridge with no link cable,
**26 species are unobtainable**: Mew (1); the 11 Blue exclusives (Sandshrew, Sandslash, Vulpix,
Ninetales, Meowth, Persian, Bellsprout, Weepinbell, Victreebel, Magmar, Pinsir); the 4 trade evolutions
(Alakazam, Machamp, Golem, Gengar); the two unchosen starter lines (6); the two unchosen Eeveelutions
(Jolteon, Flareon); and the unchosen fossil line (Kabuto/Kabutops).

**Max = 125. Current = 52.** `docs/pokemon-locations-and-evolutions.txt` is a per-species location
index — use it to plan any catching, rather than reading walkthroughs.

---

## 8. Open — workstreams I–L

Agreed with Alex on 2026-08-04. Same rules as A–H: own two files, four shared lines, an archive entry
when you finish.

### I — the rest of the item-use table ☐

The checklist is the ROM's own dispatch table, `ItemUsePtrTable` in `engine/items/item_effects.asm`.
Everything below is an entry in that table with **no driver**. Already covered, so don't redo:
`ItemUseBall`, `ItemUseEvoStone`, `ItemUseCardKey`, `ItemUsePokeFlute`, `ItemUseOldRod/GoodRod/SuperRod`,
`ItemUseCoinCase`, `ItemUseOaksParcel`, `ItemUseSurfboard`, `ItemUseBait`/`Rock` (Safari), medicine
**in** battle, and `ItemUseVitamin` / `ItemUseEscapeRope` (same ROM paths as `UseRareCandy` and `Dig`,
both driven). `ItemUsePokedex` / `ItemUseTownMap` are out (§3).

All of I1/I2/I7 ride the existing START → ITEM → bag → USE chain that `FieldMove::TeachMove`,
`EvolveWithStone` and `UseRareCandy` already drive — **read that driver before writing a new one**; the
work is mostly the extra menu each item opens, not the chain.

- [ ] **I1 — `ItemUseMedicine` out of battle.** Potion / status heal / Revive onto a party member.
      New: `PolicyStep::UseMedicine { item, slot }`. Observable: that slot's HP or status changes.
      ⚠️ At full HP the ROM prints *"it won't have any effect"* — **a text box that reads exactly like
      success**, the same family as the full-bag trap. Assert on HP, never on the conversation. A Revive
      needs a fainted target, so the test has to arrange one.
- [ ] **I2 — `ItemUsePPRestore`** (Ether / Max Ether / Elixer / Max Elixer) **and `ItemUsePPUp`**.
      ⚠️ Ether opens a **move submenu** after the mon is picked — one more menu than the teach chain
      has. Observable: a move's PP rises (the party read already exposes PP). This is the highest-value
      item in the workstream: a **0-PP battle deadlock** is the failure that once made grinding look
      impossible (archive, articuno/E4 work), and today the only cure is a walk to a Pokémon Center.
- [ ] **I3 — in-battle stat items.** X Attack / X Defend / X Speed / X Special (`ItemUseXStat`),
      X Accuracy, Guard Spec., Dire Hit. `BattleAction::UseItem` already expresses them; what is
      missing is a policy branch that *chooses* one and a test that proves the effect — assert the
      stat-stage RAM (`wPlayerMonAttackMod` and friends), not the animation.
- [ ] **I4 — `ItemUsePokeDoll`.** Ends a wild battle outright; a second escape route for when Run keeps
      failing. ⚠️ It is still an **exit**, so the legendary rule applies: never against a
      `trainer`-flagged static object, or the object is hidden for good.
- [ ] **I5 — the Repel family.** No target, no party menu; sets `wRepelRemainingSteps`. Observable is
      the counter plus no encounters while it lasts. Useful to any leg that has to cross grass it does
      not want battles in.
- [ ] **I6 — ride the Bicycle** (`ItemUseBicycle`). Today the bike is owned but never mounted; only
      Cycling Road force-mounts it. It toggles `wWalkBikeSurfState`, and it **doubles overworld speed**,
      so this one may pay for itself in emulated minutes on long legs. ⚠️ Refused indoors and while
      surfing (`ItemUseNotTime`) — the driver must not wait forever on a mount that will never happen.
- [ ] **I7 — use the Itemfinder** (`ItemUseItemfinder`). Collected in H3, never pressed. Not needed to
      *collect* anything (H4); this covers the item's own effect. Observable is its text box.

**Test:** one leg per sub-step, `postgame::items`. Root at `postgame-aides.bin` (it has the Itemfinder;
its PC holds Revives, Full Restores, an X Accuracy, Carbos and Calcium — withdraw rather than buy).
⚠️ Bag is 20/20 there: shed rows to the PC before any purchase, or the buy silently no-ops.

### J — fast fixtures: options written to RAM ☐

Alex, 2026-08-04: don't drive the OPTION menu, but **do** cut emulated time by setting the options
directly. This is a debug-tier RAM write and is exactly what that tier is for.

From `constants/ram_constants.asm`: `wOptions` low three bits are the text delay
(`TEXT_DELAY_MASK = %111`, `TEXT_DELAY_FAST = %001`) and bit 7 is `BIT_BATTLE_ANIMATION`, where
**set means animations are OFF** (`engine/battle/animations.asm:422` — `bit BIT_BATTLE_ANIMATION, a` /
`jr nz, .animationsDisabled`). So the value wanted is **`0b1000_0001`**.

- [ ] **J1** `PokemonApi::debug_set_options` in `postgame/debug.rs`.
- [ ] **J2** Apply it in the **integration-test fixture loader**, not by regenerating fixtures. One
      place, every tier, no churn across the 27 committed `.bin`s and no chance of a half-regenerated
      chain.
- [ ] **J3** ⚠️ `wOptions` sits inside the SRAM-saved main data block, so a **soft reset → CONTINUE
      restores whatever was saved** — and Phase 0's Hall-of-Fame walk-out does exactly that, as does
      `change_box`. Re-apply after any reset (or write the SRAM copy too) and prove it with a probe
      that reads `wOptions` back after `can_walk_out_of_the_hall_of_fame`.
- [ ] **J4** **Measure it.** Time the default tier and one representative slow leg before and after,
      and record both numbers in the archive. If the win is not real, say so and revert — the point of
      this workstream is wall clock, so an unmeasured version of it has not been done.
- ⚠️ **Watch the text driver.** It reads text from VRAM and mashes A; faster printing means fewer ticks
  per box. Anything that *counts ticks* rather than watching a flag may need its bound revisited — and
  `AGENT_RESOLUTION` is tuned, so do not touch it to fix a driver bug.
- **Battle style** (bit 6, `BIT_BATTLE_SHIFT`) is deliberately **left alone**: SET would drop the
  "will you switch?" prompt in trainer battles, which is a real saving but changes the battle flow every
  driver was tuned against. Only with a measured before/after and a full slow tier.

### K — prove one of the five unproven in-game trades ☐

Four of the nine trades are done (G5/G6). The other five each need a give-species the save does not
have in hand, which is why they were skipped. Alex, 2026-08-04: prove one **can** be done; catching the
give mon or seeding it in the debug tier are both fine.

| Give | Get | Map |
|---|---|---|
| Nidorino | Nidorina | `Route11Gate2F` |
| **Ponyta** | **Seel** | **`CinnabarLabFossilRoom`** |
| Poliwhirl | Jynx | `CeruleanTradeHouse` |
| Raichu | Electrode | `CinnabarLabTradeRoom` |
| Slowbro | Lickitung | `Route18Gate2F` (needs the Bicycle) |

- [ ] **K1** Take **Ponyta → Seel**. It needs **no seeding at all**: `postgame-aides.bin`'s box 3 slot 2
      already holds a lv32 Ponyta. Withdraw it (workstream A's `withdraw_pokemon`), Fly to Cinnabar,
      run `PolicyStep::trade_steps` with `PartyScript::Trade`. No new driver — a trade NPC opens the
      same stale-cursor party menu the Day Care does. Observable: dex **owned +1 (Seel)** and the slot's
      species changes. Fall back to `debug_set_party` only if that Ponyta has been spent.
- [ ] **K2** Archive entry recording whether the give-species really was the only obstacle for the
      other four, so the table above can be trusted or corrected.
- ⚠️ **The trade NPC is never the one you would guess** — read the script that sets `wWhichTrade`, not
  the object list. Cinnabar Lab is the Gramps and the Beauty, not the Super Nerd.
- ⚠️ A traded mon **cannot be renamed** by the Name Rater (it checks OT name *and* ID), and it obeys
  only at your badge level — irrelevant here, but it has surprised a leg before.

### L — visit every visitable map ☐

Alex, 2026-08-04: *visit ALL visitable rooms to check there are no broken map mechanics we haven't
covered in the agent.* The deliverable is as much the **list of maps that could not be entered, and
why**, as the green test.

Of the 248 `Map` variants, ~220 are visitable: strike the 22 `UnusedMap*`, `Colosseum` and
`TradeCenter` (link cable, §3), and the four duplicate slots (`CeruleanTrashedHouseCopy`,
`CinnabarMartCopy`, `UndergroundPathRoute6Copy`, `UndergroundPathRoute7Copy`). About 60 of them have
never been referenced by any step list — regenerate that list any time with:

```bash
comm -13 <(grep -ohr "Map::[A-Za-z0-9_]*" src/pokemon/policy.rs src/pokemon/postgame/ | sed 's/Map:://' | sort -u) \
         <(sed -n '7,300p' src/pokemon/map.rs | grep -oE "^    [A-Za-z0-9_]+ =" | sed 's/ =//;s/^ *//' | sort -u)
```

⚠️ That list over-reports: gyms are reached through `DefeatGymLeader` and Silph floors through
`UseElevator`, so they never appear as a literal `Map::`. Confirm with the world graph, not the grep.

- [ ] **L1 — the static audit first, no emulation.** For every visitable map assert the agent's own
      tables answer: the header resolves, the tileset is known, warps decode, and the sprite table is
      non-empty wherever the ROM has objects. Default tier, seconds, and it finds missing metadata
      without burning a single emulated minute. Do this **before** L2 — anything it catches would
      otherwise show up as a silent stall an hour into a tour.
- [ ] **L2 — the emulated tour, sliced per Fly hub.** For each town: enter every building in the
      cluster and every connected route, and per map assert the `MetaTileMap` builds, `actions()` is
      non-empty, and an exit action exists. One test per hub so a failure costs one town, not the tour.
- [ ] **L3 — the awkward set**, which is where the real findings will be. Check reachability *before*
      budgeting: the four **Safari rest houses** (behind the ¥500 gate and the 502-step budget, and the
      zone is a chain, not a hub), **Museum 2F**, **Route 19/20** water, **Route 8 Gate**, **Cerulean
      Badge House**, the **Cinnabar Lab** rooms, and the **SS Anne**'s 15 maps — ⚠️ the ship **sails**
      once the mainline is done, so check `EVENT_SS_ANNE_LEFT` before planning a visit. Two known
      one-way doors from the archive: **Pokémon Tower 6F/7F cannot be climbed** on a save that already
      finished the tower, and the **Hall of Fame** needs another Champion run.
- [ ] **L4** Archive entry: every map entered, every map that could not be, and every mechanic the tour
      turned up that the agent does not model.
- ⚠️ Expect this to be the most expensive workstream here in emulated time. Slice it, bound every wait,
  and remember that `EnterMap` with no matching action **does nothing at all** — a tour that "passes"
  while standing still is the failure mode to design against.

---

## 9. Status

Claim your row before you start: `☐ unclaimed` · `🔵 in progress` · `🟡 blocked` · `✅ done`.

| Stream | Entry fixture | Test module | Status | Output + what to know |
|---|---|---|---|---|
| 0 — foundation | `post-hall-of-fame.bin` | `postgame::phase0` | ✅ done | `postgame-phase0.bin` — bag 14/20, healed, at the Viridian PC. ⚠️ A–H root here, and the **Silph Scope is in the PC**. |
| A — PC boxes | `postgame-phase0.bin` | `postgame::pc_box` | ✅ done | `postgame-pc-box.bin`. `deposit_pokemon` / `withdraw_pokemon` / `change_box` / `release_pokemon`. |
| B — Fly / Bike | `postgame-phase0.bin` | `postgame::fly_bike` | ✅ done | `postgame-fly-bike.bin` (Fuchsia). **`PolicyStep::Fly { to }` is available to everyone.** Heal first — Solarbeam is at 0 PP. |
| C — Fishing | `postgame-fly-bike.bin` | `postgame::fishing` | ✅ done | `postgame-fishing.bin`, all three rods. `PolicyStep::fish(rod, map, goal)`. |
| D — Legendaries | `postgame-fly-bike.bin` | `postgame::legendaries` | ✅ done | `postgame-legendaries.bin`, all three caught (Master-Ball-seeded, §3). ⚠️ Never `Run`, never weaken. |
| E — Safari Zone | `postgame-flash.bin` | `postgame::safari` | ✅ done | `postgame-safari.bin`, dex 31, all twelve species. ⚠️ Saved inside Rock Tunnel, so every leg opens with a `Dig`. |
| F — Game Corner | `postgame-fly-bike.bin` | `postgame::game_corner` | ✅ done | `postgame-game-corner.bin`. Reusable: `SellToMart`, `RedeemPrize`, `ItemId::is_key_item()`. |
| G — Gifts | `postgame-fly-bike.bin` | `postgame::gifts` | ✅ done | `postgame-name-rater.bin`. Reusable: **`PolicyStep::PartyScript { script, slot }`**. |
| G — Trades | `postgame-name-rater.bin` | `postgame::trades` | ✅ done | `postgame-tangela.bin`, dex 19. `TRADES` is the nine-row table, ROM-pinned. |
| H — Oak's aides | `postgame-tangela.bin` · `postgame-safari.bin` | `postgame::aides` | ✅ done | **`postgame-aides.bin`**, dex 52 — the chain head. Reusable: `SweepDex`, `SearchHiddenItem`, `UseFlash`, `MetaTileMap::hidden_items`, `crate::pokemon::wild`. |
| **I — item-use table** | `postgame-aides.bin` | `postgame::items` | ☐ unclaimed | Medicine / PP / stat items / Poké Doll / Repel / Bicycle / Itemfinder. |
| **J — fast fixtures** | n/a (test harness) | `postgame::phase0` or its own | ☐ unclaimed | `debug_set_options`, applied at fixture load. Must be **measured**. |
| **K — one more trade** | `postgame-aides.bin` | `postgame::trades` | ☐ unclaimed | Ponyta → Seel; the Ponyta is already in box 3. |
| **L — visit every map** | `postgame-aides.bin` | `postgame::maps` | ☐ unclaimed | Static audit first, then a tour per Fly hub. The unreachable list is the deliverable. |

---

## 10. Known traps

The living version is the `postgame-recurring-traps` memory; the evidence for each is in the archive.

- **Almost every failure here is silent.** The agent walks somewhere plausible, or stands still, and
  the leg dies minutes later somewhere else. `EnterMap` with no matching action does nothing at all;
  `enter_at`'s position is a *preference* that falls through to any other crossing; `goto` cannot see a
  gate building. Probe the arrival tile with `state.map.actions()` **before** writing a route.
- **Maps are often several sealed regions**, not one room: Route 5 (Day Care in the middle), Route 2,
  Silph 7F (three pockets), Victory Road 2F (three), Cerulean (cut in two), the Safari Zone (a chain).
- **Menu indices shift.** The PC main menu gains entries with the Pokédex and post-Champion; the
  forget-move menu sits at a different origin in battle vs. the overworld. Detect menus by on-screen
  **text**, not geometry.
- **Bag rows:** use `api.bag_item_position()` (raw `wBagItems`), *not* `GameState::bag` — the latter
  drops every id `ItemId` can't name (all the TMs) and so shifts indices.
- **Menu driving:** mash — press one agent tick, `release_all_buttons` the next, so each input is a
  fresh rising edge. Holding for N ticks is ONE edge. Navigate the cursor to the target index, *then*
  press A; never press A blind.
- **A full bag reads exactly like a successful conversation** — gift givers, aides and clerks all print
  one box and move on. Free rows *before* you arrive. The same shape as medicine at full HP (I1).
- **Raw pokered coordinates are one tile out on any connected outdoor map** — each connection widens
  the map by a strip. `MetaTileMap::new` is the one place that offset is applied; put new raw tables
  there with the others.
- **Cut trees regrow after any battle** (the battle reloads the map). `PokemonAgent::cut_tiles` is
  cleared on map change; a battle on the same map invalidates it.
- **Every wait needs a bound**, and a wait whose escape depends on the agent *moving* will never end.
  If the flag you are waiting on cannot change without a button, pressing the *right* button is the fix
  (fishing's cast animation; the naming screen taking only A or B).
- **A spinning log is not a wedge until you have counted the ticks.** The one confirmed "driver bug"
  filed from this project — *"`UseRareCandy` only works on slot 0"* — was a **misdiagnosis**; the chain
  had worked and the log was the idle loop afterwards. Pinned now by
  `mechanics::rare_candy_works_on_a_late_party_slot`.
- **`AGENT_RESOLUTION` (20 ms) is tuned.** Don't change it to fix a driver bug.
- **Don't optimise the agent.** It is only ~11 % of runtime; the emulator is the cost. And
  `target-cpu=native` measured *slower*.

---

## 11. The findings archive

[`postgame-findings-archive.md`](postgame-findings-archive.md) — the append-only log from Phase 0 and
A–H, kept verbatim. Everything in §5, §6 and §10 above is a summary of it, so when a summary and the archive
disagree, **the archive has the evidence**. Keep appending: a corrected wrong assumption is the single
most useful thing you can leave the next agent.
