//! Vermilion: the S.S. Anne, Cut, the trash cans and the Thunder Badge.

use super::*;

/// Board the S.S. Anne from `at-vermilion.bin` and leave with HM01 Cut.
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
    // Snapshot after the S.S. Anne for the next leg, teaching Cut.
    fixture.save_state_named("src/pokemon/data/post-ss-anne.bin").unwrap();
}

/// Teach HM01 Cut from the bag, from `post-ss-anne.bin`.
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
    // Snapshot with Cut taught, for the gym tree and the trash cans.
    fixture.save_state_named("src/pokemon/data/post-teach-cut.bin").unwrap();
}

/// From `post-teach-cut.bin`, cut the gym tree and solve the trash cans that gate Lt. Surge.
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

/// `thunder_badge_steps()` as `complete_game_steps` runs it, from `post-ss-anne.bin` to the badge.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_thunder_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(20),
        PolicyStep::thunder_badge_steps(),
    );
    // `DefeatGymLeader` never pops, and trailing `Interact` retries keep talking to Surge, so stop
    // on the badge.
    let s = fixture.run_until(|s| s.badges.contains(Badge::ThunderBadge));
    println!("badges={:?} on {}", s.badges, s.map.map);
    fixture.save_state_named("src/pokemon/data/post-thunder-badge.bin").unwrap();
}

/// From the Vermilion Gym: re-cut the tree, a Drowzee for Dig, the Underground Path to Cerulean.
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

/// Lt. Surge is not a row while the cartridge draws his doors shut.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn lt_surge_is_not_a_row_while_his_doors_are_shut() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-teach-cut.bin"), Duration::from_mins(10),
        vec![
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::VermilionGym),
        ],
    );
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::VermilionGym);

    let rows: Vec<String> = state.map.actions().iter().map(|a| format!("{:?}", a.tile)).collect();
    println!("in the gym on {} badges: {}", state.badges.bits().count_ones(), rows.join(", "));
    // He is on the map: the row is withheld because the doorway is a wall, not because the sprite
    // is missing.
    assert!(
        state.map.sprites.iter().any(|sprite| sprite.name.contains("Surge") && !sprite.hidden),
        "Lt. Surge has to be on the map for this to mean anything: {:?}",
        state.map.sprites.iter().map(|s| s.name).collect::<Vec<_>>(),
    );
    assert!(
        !rows.iter().any(|row| row.contains("Surge")),
        "the doors are shut, so there is no route to Lt. Surge and no row for him: {rows:?}",
    );
    // The trash cans that open them are rows, so the floor is reachable.
    assert!(
        state.map.actions().len() > 1,
        "the front room has to be reachable: {rows:?}",
    );
}

/// Cut a coverage start standing on the ship: `on-the-ss-anne.bin`.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts on-the-ss-anne.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_on_the_ss_anne_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-vermilion.bin"),
        Duration::from_mins(20),
        vec![PolicyStep::enter(Map::VermilionDock), PolicyStep::enter(Map::SSAnne1F)],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::SSAnne1F, "the start has to be standing on the ship");
    fixture.save_state_named("src/pokemon/data/on-the-ss-anne.bin").unwrap();
}
