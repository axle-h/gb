//! Rock Tunnel → Lavender → Celadon → Rainbow Badge → Rocket Hideout → Silph Scope.

use super::*;

/// From the main Cerulean terrace (post-Thunder), cross to Lavender Town.
///
/// The Pokécenter terrace only connects to Route 4 (west) and Route 24 (north); Route 9 (east) is on a
/// separate terrace, reached — like Route 5 — through the trashed house's back door at (27,9). Route 9
/// then boxes the west-entry pocket behind a Cut tree at (5,8). Beyond that is the Rock Tunnel warp
/// maze, which the agent routes from RAM tile collision rather than the darkened screen, so no Flash.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_lavender() {
    // ⚠️ Pinned to the pre-**J** battle timing. With animations off the RNG stream shifts, a wild
    // battle interrupts the walk at a different tile, and the agent ends up in Route 10's *southern*
    // pocket — from which Lavender is not reachable and the leg stalls at (12,20). See
    // `TestFixture::with_original_battle_timing`.
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

/// Lavender Town → Celadon City via the Route 7–8 Underground Path (bypassing the drink-gated Saffron
/// gates). Snapshots `at-celadon.bin` for the Rainbow-Badge leg.
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

/// Cut into the Celadon Gym and beat Erika for the **Rainbow Badge**.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_rainbow_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-celadon.bin"),
        Duration::from_mins(45),
        PolicyStep::celadon_rainbow_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::RainbowBadge));
    println!("badges={:?} on {} party_lv={:?}", s.badges, s.map.map,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>());
    fixture.save_state_named("src/pokemon/data/post-rainbow-badge.bin").unwrap();
}

/// From Celadon City, reach the Rocket Hideout: heal, walk to the Game Corner, flip the poster switch
/// (`FlipSwitch` + the `found_rocket_hideout` event), and descend to B1F.
///
/// Started from the pre-gym Celadon state so the entrance mechanic is exercised on its own: the hideout
/// needs Cut, not the Rainbow Badge, so this is valid before Erika.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_rocket_hideout() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-celadon.bin"),
        Duration::from_mins(20),
        PolicyStep::rocket_hideout_entrance_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::RocketHideoutB1F, "should descend into the Rocket Hideout (B1F)");
    fixture.save_state_named("src/pokemon/data/at-rocket-hideout.bin").unwrap();
}

/// The full Silph Scope leg — from inside the hideout (B1F), get the Lift Key, take the **elevator**
/// (entered from B2F, whose warp is not gated by the Rocket-5 door) to Giovanni's split B4F room, beat
/// the two Rockets to drop the door wall, beat Giovanni, and grab the **Silph Scope**.
///
/// Relies on the runtime `ReplaceTileBlock` door-block modelling (`MetaTileMap::apply_door_blocks`) so
/// BFS avoids the event-gated B1F/B4F door walls that the static ROM map shows as open floor. The Lift
/// Key pickup is the first half of `silph_scope_steps`, so it needs no separate test.
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
