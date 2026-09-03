//! Tests for workstream `items` — see `docs/postgame-coverage-plan.md` §8-I and
//! [`crate::pokemon::postgame::items`].
//!
//! Rooted on **H's output**, the chain head, for the reason §8-I gives: `postgame-aides.bin` arrives
//! with the Itemfinder, a PC full of medicine — and, less obviously, a **fainted Venusaur** and an
//! Articuno at 64/259, which is the only fixture in the repo that gives a Revive and a Potion a legal
//! target without arranging one first.
//!
//! Nothing here is debug-seeded. Every item is either in the PC already, on a shelf
//! (`data/items/marts.asm`), or lying on the floor as a hidden item — which turned out to be the
//! cheapest source of all, and the one that makes I7 provable.

#[allow(unused_imports)]
use super::super::*;
use crate::pokemon::item::ItemId;
use crate::pokemon::postgame::items;

/// H's output and the chain head (§9): Route 15, dex 52, bag 20/20, **Venusaur fainted**,
/// Articuno 64/259, ¥5,894.
const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// The three bag rows I1 spends: two TMs nothing in this repo teaches, and a Full Heal for a party
/// that has no status to cure.
const JUNK: &[ItemId] = &[ItemId::Tm29Psychic, ItemId::Tm31Mimic, ItemId::FullHeal];

/// **Task I1** — `ItemUseMedicine` out of battle: a **Revive** on a fainted mon and a **Potion** on a
/// hurt one, then a Potion on a healthy one that must be *refused*. Emulates ≤60 min (≈2 min wall).
///
/// Three assertions and the third is the point. §8-I1 warns that at full HP the ROM prints *"It won't
/// have any effect"* — a text box that reads exactly like success — and keeps the item, so a driver
/// waiting for the item to be consumed waits for ever. The guard is
/// [`crate::pokemon::postgame::items::blocked`], and the way to prove a guard is to hand it the case
/// it guards against and watch the queue drain anyway.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_revive_and_heal_a_party_member() {
    /// Venusaur — the credits left it at 0 HP and nothing since has healed it.
    const FAINTED: u8 = 0;
    /// Articuno at 64/259.
    const HURT: u8 = 1;
    /// Vaporeon at 315/315 — the one the ROM will decline.
    const HEALTHY: u8 = 2;

    let mut steps = vec![
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::enter(Map::FuchsiaPokecenter),
    ];
    // Three rows out for three items in — a withdraw into a 20/20 bag does nothing, quietly.
    steps.extend(JUNK.iter().map(|&it| PolicyStep::deposit_item(it, u8::MAX, Map::FuchsiaPokecenter)));
    steps.extend([
        PolicyStep::withdraw_item(ItemId::Revive, 1, Map::FuchsiaPokecenter),
        PolicyStep::withdraw_item(ItemId::Potion, 1, Map::FuchsiaPokecenter),
        // ⚠️ The PC holds exactly **one** Potion, which the heal below spends — so the declined use
        // needs a *different* item or it pops on "not in the bag" and proves nothing. Six Full
        // Restores are banked; one comes out to be refused.
        PolicyStep::withdraw_item(ItemId::FullRestore, 1, Map::FuchsiaPokecenter),
        PolicyStep::use_medicine(ItemId::Revive, FAINTED),
        PolicyStep::use_medicine(ItemId::Potion, HURT),
        // …and the one that must be declined rather than retried.
        PolicyStep::use_medicine(ItemId::FullRestore, HEALTHY),
        PolicyStep::enter(Map::FuchsiaCity),
    ]);

    let mut fixture = TestFixture::new(AIDES, Duration::from_mins(60), steps);

    let before = fixture.game_state();
    assert_eq!(before.pokemon[FAINTED as usize].current_hp, 0, "I1 needs a fainted target");
    let hurt_hp = before.pokemon[HURT as usize].current_hp;
    assert!(hurt_hp > 0 && hurt_hp < before.pokemon[HURT as usize].stats.hp, "I1 needs a hurt target");
    assert_eq!(before.pokemon[HEALTHY as usize].current_hp, before.pokemon[HEALTHY as usize].stats.hp);

    let revived = fixture.run_until(|s| s.pokemon[FAINTED as usize].current_hp > 0);
    println!("revived: {:?} {}/{} hp", revived.pokemon[FAINTED as usize].species,
        revived.pokemon[FAINTED as usize].current_hp, revived.pokemon[FAINTED as usize].stats.hp);

    let state = fixture.run_leg(|s| s.pokemon[HURT as usize].current_hp > hurt_hp
        && s.map.map == Map::FuchsiaCity);
    assert!(state.pokemon[FAINTED as usize].current_hp > 0, "the Revive should have stuck");
    assert!(state.pokemon[HURT as usize].current_hp > hurt_hp,
        "slot {HURT} should have healed: {} → {}", hurt_hp, state.pokemon[HURT as usize].current_hp);
    // The declined use: the queue drained (so no wedge) and the Potion is still in the bag.
    assert!(fixture.agent.policy_exhausted(), "the full-HP Full Restore should have popped, not stalled");
    assert!(items::bag_quantity(&state, ItemId::FullRestore) > 0,
        "the declined Full Restore should still be in the bag — the ROM does not consume a \
         no-effect item, which is exactly why issuing one is an endless retry without the guard");
    println!("HP {} → {} · Full Restores left {} · ¥{}", hurt_hp, state.pokemon[HURT as usize].current_hp,
        items::bag_quantity(&state, ItemId::FullRestore), state.money);

    fixture.save_state_named("src/pokemon/data/postgame-medicine.bin").unwrap();
}

/// I1's output: Fuchsia City, Venusaur revived, Articuno topped up, two bag rows spare.
const MEDICINE: &[u8] = include_bytes!("../../data/postgame-medicine.bin");

/// **Task I7** — press the **Itemfinder**, both ways. Emulates ≤90 min (≈3 min wall).
///
/// The Itemfinder is the one item in the table with **no RAM observable whatsoever**: it plays four
/// sound effects and prints one of two texts. So the test reads the screen, and it reads it in both
/// places — a run that only ever saw "found nothing" would pass an assertion that merely says
/// *something* was printed, and would be indistinguishable from a driver that opened the bag and
/// pressed A on the wrong row.
///
/// See [`PolicyStep::press_the_itemfinder_steps`] for why the positive answer needs a specific door:
/// `HiddenItemNear`'s window is small, the Fly stop is outside it, and the item it points at happens
/// to be one no one can ever pick up — which is what makes this stable rather than single-use.
///
/// It also buys the **Repel** the next leg spends, because it is standing in the only mart that
/// stocks one (`data/items/marts.asm:17`).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_press_the_itemfinder_both_ways() {
    let mut fixture = TestFixture::new(MEDICINE, Duration::from_mins(90),
        PolicyStep::press_the_itemfinder_steps(Map::VermilionTradeHouse));

    // Collect every distinct Itemfinder text the run prints, in order.
    let mut said: Vec<String> = Vec::new();
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        if let Some(text) = fixture.api().on_screen_text(false) {
            if text.contains("ITEMFINDER") && said.last() != Some(&text) { said.push(text); }
        }
    }
    for line in &said { println!("Itemfinder said: {line}"); }

    let found = said.iter().any(|t| t.contains("indicates"));
    let nothing = said.iter().any(|t| t.contains("indicator is off") || t.contains("Nope"));
    assert!(found,
        "standing next to Vermilion's uncollected Max Ether, the Itemfinder should have said it \
         indicates something nearby. Texts seen: {said:#?}");
    assert!(nothing,
        "in Fuchsia, which has no hidden items at all, it should have said its indicator is off. \
         Texts seen: {said:#?}");

    let state = fixture.game_state();
    assert!(items::bag_quantity(&state, ItemId::Repel) > 0, "the Vermilion mart stocks Repel");
    assert!(items::bag_quantity(&state, ItemId::Itemfinder) > 0, "the Itemfinder is never consumed");
    println!("Repel bought · ¥{} · at {}", state.money, state.map.map);

    fixture.save_state_named("src/pokemon/data/postgame-finder.bin").unwrap();
}

/// I7's output: Fuchsia City, one Repel in the bag, ¥5,544.
const FINDER: &[u8] = include_bytes!("../../data/postgame-finder.bin");

/// Venusaur's move slots on this chain: Solarbeam (5 of 10 PP), Razor Leaf, Cut, Vine Whip.
const SOLARBEAM_SLOT: u8 = 0;
const RAZOR_LEAF_SLOT: u8 = 1;

/// **Task I2** — `ItemUsePPRestore` and `ItemUsePPUp`. Emulates ≤120 min (≈5 min wall).
///
/// The plan calls this the highest-value item in the workstream and the archive says why: a 0-PP
/// battle deadlock is what once made grinding look impossible, and the only cure today is a walk to a
/// Pokémon Center.
///
/// Two items, one ROM routine (`ItemUsePPUp` falls through into `ItemUsePPRestore`), two different
/// observables — and the second is only visible because of a wrinkle worth knowing:
///
/// ⚠️ **`PokemonMove::pp` is the raw PP byte.** `encoding.rs` reads it unmasked and the ROM packs the
/// **PP Up count into bits 6–7**. So an Ether shows up in bits 0–5 and a PP Up shows up as the whole
/// byte jumping by 64 — and any comparison of "PP" that forgets the mask is wrong the moment a single
/// PP Up has ever been spent.
///
/// ⚠️ And the **move menu is 1-indexed** (`MoveSelectionMenu`'s relearn layout). The other three move
/// slots are asserted unchanged, because an off-by-one here restores the wrong move in silence.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_restore_pp_and_raise_it() {
    let mut fixture = TestFixture::new(FINDER, Duration::from_mins(120),
        PolicyStep::pp_restore_steps(ItemId::Ether, 0, SOLARBEAM_SLOT, RAZOR_LEAF_SLOT));
    // ⚠️ Debug tier, deliberately — see `PolicyStep::pp_restore_steps`: nothing in Kanto sells an
    // Ether and every one on the floor is behind a trek this leg is not about.
    //
    // ⚠️ **The PP Up is seeded too, since 2026-09-03.** It used to be dug out of Celadon, and every
    // PP Up in the game is a hidden item — hidden-item collection is gone from the crate, so there
    // is nowhere left to get one. What this leg tests is unchanged: `ItemUsePPUp` falls through into
    // `ItemUsePPRestore`, so one ROM routine produces two different observables, and neither of them
    // cares where the item came from.
    fixture.api().debug_give_item(ItemId::Ether, 1).expect("bag should have a free row for the Ether");

    let before = fixture.game_state();
    let solarbeam = before.pokemon[0].moves[SOLARBEAM_SLOT as usize].clone().expect("move slot 0");
    let razor_leaf = before.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].clone().expect("move slot 1");
    assert!(items::move_pp(&solarbeam) < items::max_pp(&solarbeam),
        "I2 needs a move that is missing PP; {:?} is at {}", solarbeam.name, items::move_pp(&solarbeam));
    assert_eq!(items::pp_ups(&razor_leaf), 0, "the PP Up target should have none spent on it yet");

    // ── the Ether ────────────────────────────────────────────────────────────────────────────────
    let restored = fixture.run_until(|s| s.pokemon[0].moves[SOLARBEAM_SLOT as usize].as_ref()
        .is_some_and(|m| items::move_pp(m) > items::move_pp(&solarbeam)));
    let now = restored.pokemon[0].moves[SOLARBEAM_SLOT as usize].as_ref().unwrap();
    println!("{:?} {} → {} PP (max {})", now.name, items::move_pp(&solarbeam), items::move_pp(now),
        items::max_pp(now));
    assert!(items::move_pp(now) > items::move_pp(&solarbeam), "the Ether should have restored PP");

    // ── the PP Up ────────────────────────────────────────────────────────────────────────────────
    // ⚠️ **Seeded here rather than up with the Ether, because the bag is at its 20-slot cap**, and
    // waited for rather than assumed. H3's output leaves exactly one free row; the Ether takes it
    // and only gives it back when the *item* is consumed, which is a few ticks after the PP moves —
    // so seeding on the PP assertion above still finds a full bag. This is the order the leg ran in
    // when the PP Up was dug out of Celadon, which is why the cap never showed up before.
    fixture.run_until(|s| items::bag_quantity(s, ItemId::Ether) == 0);
    fixture.api().debug_give_item(ItemId::PpUp, 1).expect("the spent Ether should have freed a row");
    let state = fixture.run_leg(|s| s.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].as_ref()
        .is_some_and(|m| items::pp_ups(m) > 0));
    let leaf = state.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].as_ref().unwrap();
    assert_eq!(items::pp_ups(leaf), 1, "one PP Up should have been applied to {:?}", leaf.name);
    // ⚠️ A PP Up raises the **current** PP as well as the maximum — `.PPNotMaxedOut` calls
    // `RestoreBonusPP` right after bumping the count (`item_effects.asm:2008-2010`), and the bonus is
    // `base / 5`. The first draft of this assertion said "the maximum, not the current PP" and was
    // simply wrong; Razor Leaf went 25 → 30 on both.
    let bonus = razor_leaf.name.metadata().pp / 5;
    assert_eq!(items::max_pp(leaf), razor_leaf.name.metadata().pp + bonus,
        "one PP Up should raise the maximum by base/5");
    assert_eq!(items::move_pp(leaf), items::move_pp(&razor_leaf) + bonus,
        "…and RestoreBonusPP hands the same bonus to the current PP");
    println!("{:?}: {} PP Ups, max now {}", leaf.name, items::pp_ups(leaf), items::max_pp(leaf));

    // The 1-indexed move menu: everything the two uses did not target must be untouched.
    for i in [2usize, 3] {
        assert_eq!(before.pokemon[0].moves[i].as_ref().map(|m| m.pp),
                   state.pokemon[0].moves[i].as_ref().map(|m| m.pp),
            "move slot {i} changed — the PP-restore move menu index is off by one");
    }
    assert_eq!(items::bag_quantity(&state, ItemId::Ether), 0, "the Ether should have been spent");
    assert_eq!(items::bag_quantity(&state, ItemId::PpUp), 0, "the PP Up should have been spent");

    fixture.save_state_named("src/pokemon/data/postgame-ether.bin").unwrap();
}

/// I2's output: Celadon City, Solarbeam topped up, one PP Up on Razor Leaf, a Repel in the bag.
const ETHER: &[u8] = include_bytes!("../../data/postgame-ether.bin");

/// **Task I5** — the Repel family. Emulates ≤30 min (≈1½ min wall).
///
/// `ItemUseRepelCommon` writes `wRepelRemainingSteps` and nothing else, so the observable is exactly
/// that byte: 0 → **100** for a Repel (200 for a Super Repel, 250 for a Max Repel), then one less per
/// overworld step. Both halves are asserted — a counter that is set but never decremented would mean
/// the agent was not actually walking, which is the failure mode §10 warns about for every wander.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_set_a_repel_running() {
    let mut fixture = TestFixture::new(ETHER, Duration::from_mins(30),
        PolicyStep::repel_steps(ItemId::Repel, Map::Route7));

    assert_eq!(fixture.game_state().repel_steps, 0, "no Repel should be running yet");

    let lit = fixture.run_until(|s| s.repel_steps > 0);
    assert_eq!(lit.repel_steps, 100, "a plain Repel sets 100 steps (item_effects.asm:1532)");
    println!("Repel running: {} steps", lit.repel_steps);

    let walked = fixture.run_until(|s| s.repel_steps > 0 && s.repel_steps < 95);
    assert!(walked.repel_steps < 95, "the counter should tick down as the agent walks");
    println!("after walking to {}: {} steps left", walked.map.map, walked.repel_steps);
}

/// **Task I6** — ride the Bicycle. Emulates ≤30 min (≈1½ min wall).
///
/// `ItemUseBicycle` toggles `wWalkBikeSurfState` between 0 and 1, and 1 is what doubles overworld
/// speed. Two things this pins that §8-I6 only asserts:
///
/// 1. It is a **toggle**, so getting off is the same item again — which is why
///    [`crate::pokemon::postgame::items::Effect::TogglesBicycle`] completes on "the mount state
///    changed" rather than "we are on the bike". With the latter the dismount step would be
///    satisfied before it started and pop without pressing anything.
/// 2. It really is faster. The same walk is timed on foot and on the bike, in emulated cycles, and
///    §8-I6's "this one may pay for itself in emulated minutes" is either true or it is not.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ride_the_bicycle() {
    // On foot first, so the two numbers are the same walk between the same two maps.
    let walk_cycles = {
        let mut fixture = TestFixture::new(ETHER, Duration::from_mins(30),
            vec![PolicyStep::enter(Map::Route7)]);
        let before = fixture.total_cycles;
        fixture.run_leg(|s| s.map.map == Map::Route7);
        fixture.total_cycles - before
    };

    let mut fixture = TestFixture::new(ETHER, Duration::from_mins(30),
        PolicyStep::ride_bicycle_steps(Map::Route7));

    assert!(!fixture.game_state().on_bicycle, "should start on foot");
    assert!(items::bike_riding_allowed(&fixture.game_state()),
        "Celadon City is an OVERWORLD tileset, so IsBikeRidingAllowed should say yes");

    let mounted = fixture.run_until(|s| s.on_bicycle);
    println!("on the bike at {} @ {}", mounted.map.map, mounted.map.player_position);
    let ride_start = fixture.total_cycles;

    let ridden = fixture.run_until(|s| s.map.map == Map::Route7);
    let ride_cycles = fixture.total_cycles - ride_start;
    assert!(ridden.on_bicycle, "the bike should still be under us on arrival");
    println!("Celadon → Route 7: walked {:?}, cycled {:?}",
        walk_cycles.to_duration(), ride_cycles.to_duration());

    // …and off again — the same item, the same step, the other direction.
    let state = fixture.run_leg(|s| !s.on_bicycle);
    assert!(!state.on_bicycle,
        "using the Bicycle a second time should have dismounted (it toggles wWalkBikeSurfState)");
    println!("dismounted at {} @ {}", state.map.map, state.map.player_position);
}

/// **Tasks I3 + I4** — the seven in-battle stat items and the **Poké Doll**, in one wild battle.
/// Emulates ≤150 min (≈6½ min wall).
///
/// Everything is bought (`data/items/marts.asm`): Celadon Mart 5F's first clerk sells all seven stat
/// items, 4F sells the doll. The battle is a level-3 Route 1 wild against a level-71 lead, so it
/// lasts exactly as long as the shopping list.
///
/// The observables are the two the plan asks for and no animation is watched:
///
/// * `wPlayerMonAttackMod` and friends — **7 is neutral**, so each `ItemUseXStat` shows up as an 8.
/// * `wPlayerBattleStatus2` — X Accuracy, Guard Spec. and Dire Hit are *not* `XStat` entries at all;
///   they set `USING_X_ACCURACY`, `PROTECTED_BY_MIST` and `GETTING_PUMPED` instead.
///
/// Both are battle-scoped and reset when it ends, so they are sampled every tick while it runs
/// rather than asserted afterwards. The Poké Doll's observable is the battle ending on the spot.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_use_the_stat_items_and_a_poke_doll_in_battle() {
    use crate::pokemon::postgame::items::{battle_status2, StatMods, STAT_ITEMS};

    /// Nine rows have to be free before the first purchase, or the clerk says "you can't carry any
    /// more items" and `BuyFromMart` gives up quietly (§10). None of these is needed again: the HMs,
    /// the Bicycle and the Itemfinder all stay.
    /// ⚠️ **Eight, not seven.** Seven is the arithmetic (19 held − 7 shed + 8 bought = 20, exactly
    /// full) and exactly-full is how the first run failed: the eighth purchase, Dire Hit, was refused
    /// four times and the step gave up — the §10 trap, from the one direction that still bites when
    /// you have counted. One spare row costs nothing and removes the whole class.
    const SHED: &[ItemId] = &[ItemId::GreatBall, ItemId::EscapeRope, ItemId::ExpAll,
                              ItemId::PokeFlute, ItemId::TownMap, ItemId::FullRestore,
                              ItemId::Repel, ItemId::SecretKey];
    /// The doll last: it ends the battle.
    const IN_BATTLE: &[ItemId] = &[ItemId::XAttack, ItemId::XDefend, ItemId::XSpeed, ItemId::XSpecial,
                                   ItemId::XAccuracy, ItemId::GuardSpec, ItemId::DireHit,
                                   ItemId::PokeDoll];

    let mut fixture = TestFixture::new(ETHER, Duration::from_mins(240),
        PolicyStep::stat_item_steps(SHED, Map::Route1, IN_BATTLE));

    let before = fixture.game_state();
    let bag_before = fixture.api().mmu().read_pointer(&pokered_symbols::wNumBagItems);
    println!("shopping with ¥{} and {bag_before}/20 bag rows", before.money);

    // The shedding, asserted on its own. Every purchase below needs a row, and a mart sale into a
    // full bag is refused with one text box and no error — so if this is where it goes wrong, it
    // should say so here rather than as a mystery three purchases later.
    let shed = fixture.run_until(|s| SHED.iter().all(|&i| items::bag_quantity(s, i) == 0));
    let bag_after = fixture.api().mmu().read_pointer(&pokered_symbols::wNumBagItems);
    println!("shed {} items: bag {bag_before} → {bag_after}", SHED.len());
    assert!(bag_after as usize + IN_BATTLE.len() <= 20,
        "after shedding, {bag_after}/20 rows are used and {} items still have to be bought — the \
         mart will refuse the last of them silently", IN_BATTLE.len());
    let _ = shed;

    // Everything on the list has to actually arrive, or the battle proves nothing about the items
    // that did not.
    let stocked = fixture.run_until(|s|
        IN_BATTLE.iter().all(|&i| items::bag_quantity(s, i) > 0));
    println!("bought all {} items, ¥{} left", IN_BATTLE.len(), stocked.money);

    // Sample the two battle-scoped observables every tick — they are wiped when the battle ends.
    let mut best = StatMods::NEUTRAL;
    let mut status2 = 0u8;
    let mut battled = false;
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        let mods = { let api = fixture.api(); items::stat_mods(api.mmu()) };
        let flags = { let api = fixture.api(); items::player_battle_status2(api.mmu()) };
        if fixture.agent.in_battle() {
            battled = true;
            best = StatMods {
                attack: best.attack.max(mods.attack), defense: best.defense.max(mods.defense),
                speed: best.speed.max(mods.speed), special: best.special.max(mods.special),
            };
            status2 |= flags;
        }
    }
    assert!(battled, "the step never got into a battle, so nothing was used");

    println!("stat stages at their peak: {best:?} (7 is neutral) · wPlayerBattleStatus2 ${status2:02x}");
    assert!(best.attack > 7, "X Attack should have raised wPlayerMonAttackMod above the neutral 7");
    assert!(best.defense > 7, "X Defend should have raised wPlayerMonDefenseMod");
    assert!(best.speed > 7, "X Speed should have raised wPlayerMonSpeedMod");
    assert!(best.special > 7, "X Special should have raised wPlayerMonSpecialMod");
    assert!(status2 & battle_status2::USING_X_ACCURACY != 0, "X Accuracy sets USING_X_ACCURACY");
    assert!(status2 & battle_status2::PROTECTED_BY_MIST != 0, "Guard Spec. sets PROTECTED_BY_MIST");
    assert!(status2 & battle_status2::GETTING_PUMPED != 0, "Dire Hit sets GETTING_PUMPED");

    let state = fixture.game_state();
    for &item in STAT_ITEMS {
        assert_eq!(items::bag_quantity(&state, item), 0, "{item:?} should have been spent");
    }
    assert_eq!(items::bag_quantity(&state, ItemId::PokeDoll), 0, "the Poké Doll should have been used");
    assert!(state.battle.is_none(), "the Poké Doll ends the battle outright (wEscapedFromBattle)");
    println!("battle over via the Poké Doll · at {} · ¥{}", state.map.map, state.money);

    fixture.save_state_named("src/pokemon/data/postgame-items.bin").unwrap();
}

/// Diagnostic for **I7** — where does a Fly land in Vermilion, and is the hidden Max Ether inside the
/// Itemfinder's ±5-tile window from there?
///
/// `HiddenItemNear` compares **raw** map coordinates, and `MetaTileMap` reports connection-offset
/// ones, so "is it near?" is not a question to answer by eye. Run this before changing
/// `vermilion_item_steps`' landing map.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_itemfinder_range() {
    for approach in [None, Some(Map::VermilionTradeHouse), Some(Map::PokemonFanClub),
                     Some(Map::VermilionMart), Some(Map::VermilionPokecenter)] {
        let mut steps = vec![PolicyStep::Fly { to: Map::VermilionCity }];
        if let Some(map) = approach {
            steps.push(PolicyStep::enter(map));
            steps.push(PolicyStep::enter(Map::VermilionCity));
        }
        let mut fixture = TestFixture::new(MEDICINE, Duration::from_mins(40), steps);
        fixture.step_until_exhausted();
        for _ in 0..60 { fixture.step(); }
        let state = fixture.game_state();
        let here = state.map.player_position;
        // ⚠️ **The item list this used to print is gone with the decoder** (2026-09-03). What is
        // left is the position, which is the half that actually varies: the Itemfinder answers on
        // `HiddenItemNear`'s ±5 x, +5/−4 y box around the player, so where the approach leaves you
        // is the whole question. Run `press_the_itemfinder_steps` to see which text it prints.
        println!("== approach {approach:?}: at {here} on {}", state.map.map);
    }
}

