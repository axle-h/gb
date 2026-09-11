//! Rock Tunnel → Lavender → Celadon → Rainbow Badge → Rocket Hideout → Silph Scope.

use super::*;

/// From the main Cerulean terrace after the Thunder Badge, cross to Lavender Town.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_lavender() {
    // Animations on: `TestFixture::with_original_battle_timing`.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/back-in-cerulean.bin"),
        Duration::from_mins(60),
        PolicyStep::cerulean_to_lavender_steps(),
    ).with_original_battle_timing();
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} party_lv={:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>());
    assert_eq!(s.map.map, Map::LavenderTown, "should cross Rock Tunnel to Lavender Town");
    fixture.save_state_named("src/pokemon/data/at-lavender.bin").unwrap();
}

/// Lavender to Celadon by the Route 7-8 Underground Path, bypassing the drink-gated Saffron gates.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_celadon() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-lavender.bin"),
        Duration::from_mins(30),
        PolicyStep::lavender_to_celadon_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CeladonCity, "should reach Celadon City via the Underground Path");
    fixture.save_state_named("src/pokemon/data/at-celadon.bin").unwrap();
}

/// Cut into the Celadon Gym and beat Erika for the Rainbow Badge.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_rainbow_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-celadon.bin"),
        Duration::from_mins(90),
        PolicyStep::celadon_rainbow_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::RainbowBadge));
    println!("badges={:?} on {} party_lv={:?}", s.badges, s.map.map,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>());
    fixture.save_state_named("src/pokemon/data/post-rainbow-badge.bin").unwrap();
}

/// From Celadon: heal, flip the Game Corner poster switch, and descend to Rocket Hideout B1F.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_rocket_hideout() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-rainbow-badge.bin"),
        Duration::from_mins(20),
        PolicyStep::rocket_hideout_entrance_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::RocketHideoutB1F, "should descend into the Rocket Hideout (B1F)");
    fixture.save_state_named("src/pokemon/data/at-rocket-hideout.bin").unwrap();
}

/// From Hideout B1F: the Lift Key, the elevator to Giovanni's B4F room, and the Silph Scope.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_silph_scope() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-rocket-hideout.bin"),
        Duration::from_mins(40),
        PolicyStep::silph_scope_steps(),
    );
    let s = fixture.run_leg(|s| s.bag.contains(&ItemId::SilphScope));
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert!(s.bag.contains(&ItemId::LiftKey), "should have picked up the Lift Key on the way");
    fixture.save_state_named("src/pokemon/data/post-silph-scope.bin").unwrap();
}

/// A route four wandering pets stand on is waited out, not called routeless.
#[test]
fn a_route_a_wandering_pet_is_standing_on_is_waited_out_rather_than_disputed() {
    use gb::geometry::Point8;
    const STAIRS: Point8 = Point8 { x: 7, y: 1 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/celadon-mansion-pets-in-the-way.bin"), Duration::from_mins(2),
        vec![PolicyStep::EnterMap { to_map: Map::CeladonMansion2F, to_position: None }]);
    let start = fixture.game_state();
    assert_eq!(start.map.map, Map::CeladonMansion1F);
    assert!(
        !start.map.actions().iter().any(|action| action.destination == STAIRS),
        "the state has to be dropped on a tick where the stairs are *not* a row, or it proves \
         nothing: {:?}",
        start.map.actions().iter().map(|a| a.tile).collect::<Vec<_>>(),
    );

    let end = fixture.run_until(|state| state.map.map == Map::CeladonMansion2F);
    println!("ended on {} @ {}", end.map.map, end.map.player_position);
    assert_eq!(end.map.map, Map::CeladonMansion2F);
}

/// A room whose corridors are both blocked by people is waited out, not called routeless.
#[test]
fn a_room_whose_corridors_are_both_blocked_is_waited_out_rather_than_called_routeless() {
    use gb::geometry::Point8;
    use crate::pokemon::tile::MetaTile;
    const BOXED_IN: Point8 = Point8 { x: 0, y: 5 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/celadon-chief-house-both-corridors-blocked.bin"),
        Duration::from_mins(3),
        vec![PolicyStep::EnterMap { to_map: Map::CeladonCity, to_position: None }]);
    let start = fixture.game_state();
    assert_eq!(start.map.map, Map::CeladonChiefHouse);
    assert_eq!(start.map.player_position, BOXED_IN);
    let rows: Vec<MetaTile> = start.map.actions().iter().map(|a| a.tile).collect();
    assert!(
        !rows.iter().any(|tile| matches!(tile, MetaTile::Sprite("Sailor") | MetaTile::Warp { .. })),
        "the state has to be dropped on a tick where neither the Sailor nor the way out is a row, \
         or it proves nothing: {rows:?}",
    );

    // Both disputed rows are missing because of the two people, and come back when they are lifted.
    assert!(start.map.row_blocked_by_people(MetaTile::Sprite("Sailor")),
        "the Sailor is unreachable because the Rocket and the Chief are standing in the two \
         corridors, and lifting them has to bring the row back");
    assert!(start.map.row_blocked_by_people(
            MetaTile::Warp { to_map: Map::CeladonCity, to_position: Point8 { x: 35, y: 27 } }),
        "so is the door out, which is the row the sweep disproved for itself one turn later");

    // And the other direction, or the predicate is just "are there people on this map".
    assert!(!start.map.row_blocked_by_people(MetaTile::Sprite("Nurse")),
        "a row nobody is blocking is a row that is simply not there");

    let end = fixture.run_until(|state| state.map.map == Map::CeladonCity);
    println!("left the room at ({}, {})", end.map.player_position.x, end.map.player_position.y);
}
