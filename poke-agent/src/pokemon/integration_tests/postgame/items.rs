use super::super::*;
use crate::pokemon::item::ItemId;
use crate::pokemon::postgame::items;

const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// Three rows to spend: two TMs nothing here teaches, and a Full Heal with no status to cure.
const JUNK: &[ItemId] = &[ItemId::Tm29Psychic, ItemId::Tm31Mimic, ItemId::FullHeal];

/// `ItemUseMedicine` out of battle: a Revive, a Potion, and a refused use on a healthy mon.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_revive_and_heal_a_party_member() {
    /// Venusaur, left at 0 HP by the credits.
    const FAINTED: u8 = 0;
    /// Articuno at 64/259.
    const HURT: u8 = 1;
    /// Vaporeon at 315/315 — the one the ROM will decline.
    const HEALTHY: u8 = 2;

    let mut steps = vec![
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::enter(Map::FuchsiaPokecenter),
    ];
    // Three rows out for three in: a withdraw into a 20/20 bag does nothing, quietly.
    steps.extend(JUNK.iter().map(|&it| PolicyStep::deposit_item(it, u8::MAX, Map::FuchsiaPokecenter)));
    steps.extend([
        PolicyStep::withdraw_item(ItemId::Revive, 1, Map::FuchsiaPokecenter),
        PolicyStep::withdraw_item(ItemId::Potion, 1, Map::FuchsiaPokecenter),
        // The one Potion is spent below, so the refused use needs a different item.
        PolicyStep::withdraw_item(ItemId::FullRestore, 1, Map::FuchsiaPokecenter),
        PolicyStep::use_medicine(ItemId::Revive, FAINTED),
        PolicyStep::use_medicine(ItemId::Potion, HURT),
        // The one that must be declined rather than retried.
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
    // The declined use: the queue drained, and the item is still in the bag.
    assert!(fixture.agent.policy_exhausted(), "the full-HP Full Restore should have popped, not stalled");
    assert!(items::bag_quantity(&state, ItemId::FullRestore) > 0,
        "the declined Full Restore should still be in the bag — the ROM does not consume a \
         no-effect item, which is exactly why issuing one is an endless retry without the guard");
    println!("HP {} → {} · Full Restores left {} · ¥{}", hurt_hp, state.pokemon[HURT as usize].current_hp,
        items::bag_quantity(&state, ItemId::FullRestore), state.money);

    fixture.save_state_named("src/pokemon/data/postgame-medicine.bin").unwrap();
}

/// `can_revive_and_heal_a_party_member`'s output: Fuchsia City, Venusaur revived, two rows spare.
const MEDICINE: &[u8] = include_bytes!("../../data/postgame-medicine.bin");

/// The Itemfinder pressed both ways.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_press_the_itemfinder_both_ways() {
    let mut fixture = TestFixture::new(MEDICINE, Duration::from_mins(90),
        PolicyStep::press_the_itemfinder_steps(Map::VermilionTradeHouse));

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

/// `can_press_the_itemfinder_both_ways`'s output: Fuchsia City, one Repel in the bag.
const FINDER: &[u8] = include_bytes!("../../data/postgame-finder.bin");

/// Venusaur's move slots on this chain: Solarbeam (5 of 10 PP), Razor Leaf, Cut, Vine Whip.
const SOLARBEAM_SLOT: u8 = 0;
const RAZOR_LEAF_SLOT: u8 = 1;

/// `ItemUsePPRestore` and `ItemUsePPUp`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_restore_pp_and_raise_it() {
    let mut fixture = TestFixture::new(FINDER, Duration::from_mins(120),
        PolicyStep::pp_restore_steps(ItemId::Ether, 0, SOLARBEAM_SLOT, RAZOR_LEAF_SLOT));
    // Debug tier: nothing in Kanto sells an Ether (see `PolicyStep::pp_restore_steps`).
    fixture.api().debug_give_item(ItemId::Ether, 1).expect("bag should have a free row for the Ether");

    let before = fixture.game_state();
    let solarbeam = before.pokemon[0].moves[SOLARBEAM_SLOT as usize].clone().expect("move slot 0");
    let razor_leaf = before.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].clone().expect("move slot 1");
    assert!(items::move_pp(&solarbeam) < items::max_pp(&solarbeam),
        "I2 needs a move that is missing PP; {:?} is at {}", solarbeam.name, items::move_pp(&solarbeam));
    assert_eq!(items::pp_ups(&razor_leaf), 0, "the PP Up target should have none spent on it yet");

    // ── the Ether ──
    let restored = fixture.run_until(|s| s.pokemon[0].moves[SOLARBEAM_SLOT as usize].as_ref()
        .is_some_and(|m| items::move_pp(m) > items::move_pp(&solarbeam)));
    let now = restored.pokemon[0].moves[SOLARBEAM_SLOT as usize].as_ref().unwrap();
    println!("{:?} {} → {} PP (max {})", now.name, items::move_pp(&solarbeam), items::move_pp(now),
        items::max_pp(now));
    assert!(items::move_pp(now) > items::move_pp(&solarbeam), "the Ether should have restored PP");

    // ── the PP Up ── Seeded here rather than with the Ether because the bag is at its 20-slot cap.
    fixture.run_until(|s| items::bag_quantity(s, ItemId::Ether) == 0);
    fixture.api().debug_give_item(ItemId::PpUp, 1).expect("the spent Ether should have freed a row");
    let state = fixture.run_leg(|s| s.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].as_ref()
        .is_some_and(|m| items::pp_ups(m) > 0));
    let leaf = state.pokemon[0].moves[RAZOR_LEAF_SLOT as usize].as_ref().unwrap();
    assert_eq!(items::pp_ups(leaf), 1, "one PP Up should have been applied to {:?}", leaf.name);
    let bonus = razor_leaf.name.metadata().pp / 5;
    assert_eq!(items::max_pp(leaf), razor_leaf.name.metadata().pp + bonus,
        "one PP Up should raise the maximum by base/5");
    assert_eq!(items::move_pp(leaf), items::move_pp(&razor_leaf) + bonus,
        "…and RestoreBonusPP hands the same bonus to the current PP");
    println!("{:?}: {} PP Ups, max now {}", leaf.name, items::pp_ups(leaf), items::max_pp(leaf));

    // Everything the two uses did not target is untouched.
    for i in [2usize, 3] {
        assert_eq!(before.pokemon[0].moves[i].as_ref().map(|m| m.pp),
                   state.pokemon[0].moves[i].as_ref().map(|m| m.pp),
            "move slot {i} changed — the PP-restore move menu index is off by one");
    }
    assert_eq!(items::bag_quantity(&state, ItemId::Ether), 0, "the Ether should have been spent");
    assert_eq!(items::bag_quantity(&state, ItemId::PpUp), 0, "the PP Up should have been spent");

    fixture.save_state_named("src/pokemon/data/postgame-ether.bin").unwrap();
}

/// `can_restore_pp_and_raise_it`'s output: Celadon City, Solarbeam topped up, Razor Leaf PP Up.
const ETHER: &[u8] = include_bytes!("../../data/postgame-ether.bin");

/// The Repel family.
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

/// Ride the Bicycle.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ride_the_bicycle() {
    // On foot first, so both numbers are the same walk.
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

    let state = fixture.run_leg(|s| !s.on_bicycle);
    assert!(!state.on_bicycle,
        "using the Bicycle a second time should have dismounted (it toggles wWalkBikeSurfState)");
    println!("dismounted at {} @ {}", state.map.map, state.map.player_position);
}

/// The seven in-battle stat items and the Poké Doll, in one wild battle.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_use_the_stat_items_and_a_poke_doll_in_battle() {
    use crate::pokemon::postgame::items::{battle_status2, StatMods, STAT_ITEMS};

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

    let shed = fixture.run_until(|s| SHED.iter().all(|&i| items::bag_quantity(s, i) == 0));
    let bag_after = fixture.api().mmu().read_pointer(&pokered_symbols::wNumBagItems);
    println!("shed {} items: bag {bag_before} → {bag_after}", SHED.len());
    assert!(bag_after as usize + IN_BATTLE.len() <= 20,
        "after shedding, {bag_after}/20 rows are used and {} items still have to be bought — the \
         mart will refuse the last of them silently", IN_BATTLE.len());
    let _ = shed;

    // Every item on the list has to arrive, or the battle proves nothing about the missing ones.
    let stocked = fixture.run_until(|s|
        IN_BATTLE.iter().all(|&i| items::bag_quantity(s, i) > 0));
    println!("bought all {} items, ¥{} left", IN_BATTLE.len(), stocked.money);

    // Sample the two battle-scoped observables every tick; the battle's end wipes them.
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
