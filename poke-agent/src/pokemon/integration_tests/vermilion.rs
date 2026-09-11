//! S.S.

use super::*;

/// Board the S.S.
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
    // Snapshot post-S.S.-Anne (HM01 in bag, party ~lv32) for the next leg (teach Cut → Lt.
    fixture.save_state_named("src/pokemon/data/post-ss-anne.bin").unwrap();
}

/// Teach HM01 Cut via the bag (START → ITEM → HM01 → USE → choose Pokémon), from the
/// post-S.S.-Anne save.
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

/// The two field mechanics that gate Lt.
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

/// The integrated Thunder-Badge leg exactly as folded into `complete_game_steps`: from
/// post-S.S.-Anne (HM01 Cut in the bag, in Vermilion City) run `thunder_badge_steps()` — teach
/// Cut, cut the gym tree, solve the trash-can puzzle, beat Lt.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_thunder_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(20),
        PolicyStep::thunder_badge_steps(),
    );
    // `DefeatGymLeader` never pops on its own, and the trailing `Interact` retries keep talking
    // to Surge after the win — so stop on the badge, not on an empty queue.
    let s = fixture.run_until(|s| s.badges.contains(Badge::ThunderBadge));
    println!("badges={:?} on {}", s.badges, s.map.map);
    fixture.save_state_named("src/pokemon/data/post-thunder-badge.bin").unwrap();
}

/// From the post-Thunder-Badge state (inside the Vermilion Gym), exit the gym, re-cut the
/// enclosure tree (it regrew when the map reloaded), heal, catch and grind the Route 11 Drowzee,
/// teach the starter Dig, and trek back to Cerulean City via the Underground Path (Route 6 →
/// Route 5) — Saffron's Route 6 gate is guard-blocked, so the tunnel is the only way north.
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

/// A door the cartridge draws shut is a wall, and Vermilion Gym's are the only ones a finished
/// save can never show you.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn lt_surge_is_not_a_row_while_his_doors_are_shut() {
    // `post-teach-cut`, and the tree first.
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
    // He is on the map — the row is withheld because the doorway is a wall, not because the
    // sprite is missing, and a test that could not tell those apart would pass on an empty sprite
    // table.
    assert!(
        state.map.sprites.iter().any(|sprite| sprite.name.contains("Surge") && !sprite.hidden),
        "Lt. Surge has to be on the map for this to mean anything: {:?}",
        state.map.sprites.iter().map(|s| s.name).collect::<Vec<_>>(),
    );
    assert!(
        !rows.iter().any(|row| row.contains("Surge")),
        "the doors are shut, so there is no route to Lt. Surge and no row for him: {rows:?}",
    );
    // And the trash cans that open them *are* rows, so the floor is not simply unreachable.
    assert!(
        state.map.actions().len() > 1,
        "the front room has to be reachable: {rows:?}",
    );
}

/// Cut a coverage start that is standing *on* the ship: `on-the-ss-anne.bin`.
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
