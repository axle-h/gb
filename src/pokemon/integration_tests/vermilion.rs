//! S.S. Anne → HM01 Cut → the Vermilion Gym → Thunder Badge → back to Cerulean.

use super::*;

/// Board the S.S. Anne, defeat all 16 cabin/bow trainers (leveling the party), beat the rival, get
/// HM01 Cut from the captain, and disembark back to Vermilion. Each floor is a heal → board → sweep
/// → disembark cycle (no Pokémon Center on the ship). The longest single leg in the chain, and the
/// one that bounds the wall clock of `--features slow-tests`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_clear_ss_anne() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-vermilion.bin"),
        Duration::from_mins(90),
        PolicyStep::ss_anne_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended {} @ {} party_lv={:?} bag={:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>(), s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::Hm01Cut), "should have HM01 Cut after clearing the S.S. Anne");
    assert_eq!(s.map.map, Map::VermilionCity, "should have disembarked back to Vermilion City");
    // Snapshot post-S.S.-Anne (HM01 in bag, party ~lv32) for the next leg (teach Cut → Lt. Surge).
    fixture.save_state_named("src/pokemon/data/post-ss-anne.bin").unwrap();
}

/// Teach HM01 Cut via the bag (START → ITEM → HM01 → USE → choose Pokémon), from the post-S.S.-Anne
/// save.
///
/// ⚠️ **To the Oddish, not the starter, and that is the point of the test now.**
/// `data/pokemon/base_stats/wartortle.asm` has no CUT, so a `Slot(0)` teach is *correctly* refused by
/// `learnset::can_learn` and skipped — which looks exactly like the driver failing. The route catches
/// an Oddish on Route 25 to hold Cut; this proves the teach lands on a mon that is not the lead and
/// that `can_use_cut` (which asks the whole party) then goes true.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_teach_cut() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(5),
        vec![PolicyStep::TeachMove { item: ItemId::Hm01Cut,
                                    target: PartyRef::Species(PokemonSpecies::Oddish) }],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    for p in s.pokemon.iter() { println!("{} moves: {:?}", p.species, p.moves); }
    assert!(s.can_use_cut, "the Cut carrier should know Cut (can_use_cut true) after TeachMove");
    // Snapshot with Cut taught (at Vermilion) for the next leg (cut the gym tree → trash cans).
    fixture.save_state_named("src/pokemon/data/post-teach-cut.bin").unwrap();
}

/// The two field mechanics that gate Lt. Surge, isolated from the fight itself.
///
/// **Cut**: the `CuttingTree` state drives START→POKéMON→mon→CUT with plain button mashing (cursor to
/// each target index, then A). The agent's `MetaTileMap` is decoded from static ROM, so it still shows
/// the felled tree — it records what it cut and treats it as `Empty` for routing (`observe_state`).
///
/// **The trash cans**: the agent reads which cans hold the two switches (`GameState::trash_cans`, from
/// RAM), walks to each and presses A, unlocking the door to Surge. Junior trainers that engage en route
/// are fought and beaten normally.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_gym_trash_cans() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-teach-cut.bin"),
        Duration::from_mins(15),
        vec![
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::VermilionGym),
            PolicyStep::SolveTrashCans,
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    assert_eq!(s.map.map, Map::VermilionGym, "should cut the gym tree and enter the Vermilion Gym");
    let tc = s.trash_cans.clone().expect("trash-can puzzle state in the gym");
    println!("player@{} first_opened={} second_opened={}", s.map.player_position, tc.first_opened, tc.second_opened);
    assert!(tc.second_opened, "both trash-can switches should be flipped (door to Lt. Surge unlocked)");
    fixture.save_state_named("src/pokemon/data/gym-trash-solved.bin").unwrap();
}

/// The integrated Thunder-Badge leg exactly as folded into `complete_game_steps`: from post-S.S.-Anne
/// (HM01 Cut in the bag, in Vermilion City) run `thunder_badge_steps()` — teach Cut, cut the gym tree,
/// solve the trash-can puzzle, beat Lt. Surge — and confirm the badge. Keeps the helper and the full
/// playthrough in lockstep, and subsumes the two mechanic tests above.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_thunder_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(20),
        PolicyStep::thunder_badge_steps(),
    );
    // `DefeatGymLeader` never pops on its own, and the trailing `Interact` retries keep talking to
    // Surge after the win — so stop on the badge, not on an empty queue.
    let s = fixture.run_until(|s| s.badges.contains(Badge::ThunderBadge));
    println!("badges={:?} on {}", s.badges, s.map.map);
    fixture.save_state_named("src/pokemon/data/post-thunder-badge.bin").unwrap();
}

/// From the post-Thunder-Badge state (inside the Vermilion Gym), exit the gym, re-cut the enclosure
/// tree (it regrew when the map reloaded), heal, **catch and grind the Route 11 Drowzee**, teach the
/// starter Dig, and trek back to Cerulean City via the Underground Path (Route 6 → Route 5) —
/// Saffron's Route 6 gate is guard-blocked, so the tunnel is the only way north. Snapshots
/// `back-in-cerulean.bin` for the Rock Tunnel leg.
///
/// ⚠️ **Four hours of game time rather than thirty minutes, because this leg now contains a grind.**
/// The Drowzee arrives at lv9-13 and is taken to **26**, where it becomes a Hypno; a grind's cost is
/// measured in encounters rather than in steps, and the trainee is handed off to a tank on turn one
/// of every one of them, so it earns the halved participation share.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_return_to_cerulean() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-thunder-badge.bin"),
        Duration::from_mins(240),
        PolicyStep::back_to_cerulean_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CeruleanCity, "should trek back to Cerulean City");
    fixture.save_state_named("src/pokemon/data/back-in-cerulean.bin").unwrap();
}
