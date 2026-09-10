//! Pokémon Tower → Poké Flute → Snorlax → Soul Badge → Safari Zone (Surf + Strength).

use super::*;

/// From the hideout (post-Silph-Scope), leave, travel to Lavender, climb Pokémon Tower (Channelers +
/// the Scope-revealed ghost Marowak), beat the 7F Rockets and rescue Mr. Fuji, who hands over the
/// **Poké Flute**.
///
/// The elevator was never the problem, though this test spent a long time `#[ignore]`d saying it was.
/// The ride out of Giovanni's B4F room works; what wedged was the step after it. **Rocket Hideout B1F
/// is two disconnected halves**, split by the full-width wall at row 16, and B2F has a staircase into
/// each: (21,22) → B1F (21,24) in the south, (27,8) → B1F (23,2) in the north. Only the north half
/// holds the Game Corner staircase out. A bare `enter(RocketHideoutB1F)` takes the *nearest* warp —
/// from the elevator that is the southern one, 10 steps against 33 — and the south half's only other
/// exit is the elevator itself, behind the still-shut Rocket-5 door. So the run reached B1F fine and
/// then stalled on `EnterMap { to_map: GameCorner }` with no reachable Game Corner warp on the map.
/// The fix is to name the landing (`policy.rs`, `poke_flute_steps`); `probe_hideout_b1f_halves` below
/// dumps both halves. Worth remembering generally: a map is one node in the world graph, so **the graph
/// cannot see an intra-map partition** — only an explicit `to_position` can.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_poke_flute() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-silph-scope.bin"),
        Duration::from_mins(60),
        PolicyStep::poke_flute_steps(),
    );
    let s = fixture.run_leg(|s| s.bag.contains(&ItemId::PokeFlute));
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/post-poke-flute.bin").unwrap();
}

/// Use the Poké Flute to wake the **Route 12 Snorlax** (the field item-use capability), beating it in
/// the wild battle to clear the road south.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_wake_snorlax() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-poke-flute.bin"),
        Duration::from_mins(30),
        PolicyStep::snorlax_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    // The Snorlax sprite is gone once beaten; the step completes on that. Confirm we're on Route 12
    // and the blocker is cleared.
    let snorlax_present = s.map.sprites.iter().any(|sp| !sp.hidden && sp.name == "Snorlax");
    println!("ended on {} @ {} snorlax_present={}", s.map.map, s.map.player_position, snorlax_present);
    assert_eq!(s.map.map, Map::Route12, "should be on Route 12 after waking the Snorlax");
    assert!(!snorlax_present, "the Route 12 Snorlax should be defeated and gone");
    fixture.save_state_named("src/pokemon/data/post-snorlax.bin").unwrap();
}

/// From Route 12 (post-Snorlax), travel Route 13 → 14 → 15 → Fuchsia City and beat Koga for the
/// **Soul Badge**.
///
/// Two navigation fixes made this work: (1) `actions()` emits *every* reachable connection tile per
/// adjacent map (not just the nearest), so an `EnterMap { to_position }` can pick a landing — the
/// nearest Route 13→14 crossing drops into a trainer-sealed dead-end pocket (row 6), so we cross at
/// (0,9) to land at the open Route 14 (19,8); (2) Route 15 has a gate building walling off the Fuchsia
/// connection, traversed like the Route 12 gate (east door → west exit (7,8)).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_soul_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-snorlax.bin"),
        Duration::from_mins(60),
        PolicyStep::soul_badge_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges);
    assert!(s.badges.contains(Badge::SoulBadge), "should win the Soul Badge from Koga");
    fixture.save_state_named("src/pokemon/data/post-soul-badge.bin").unwrap();
}

/// Safari Zone run for HM03 Surf + the Gold Teeth (exercises the Safari battle handling — the agent
/// RUNs from every encounter).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_surf_safari() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-soul-badge.bin"),
        Duration::from_mins(45),
        PolicyStep::safari_zone_surf_steps(),
    );
    let s = fixture.run_leg(|s| s.bag.contains(&ItemId::Hm03Surf));
    println!("ended on {} @ {} gold_teeth={}", s.map.map, s.map.player_position,
        s.bag.contains(&ItemId::GoldTeeth));
    fixture.save_state_named("src/pokemon/data/post-safari-surf.bin").unwrap();
}

/// Exit the Safari Zone and give the Gold Teeth to the Warden for HM04 Strength.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_strength_warden() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-safari-surf.bin"),
        Duration::from_mins(30),
        PolicyStep::safari_zone_strength_steps(),
    );
    let s = fixture.run_leg(|s| s.bag.contains(&ItemId::Hm04Strength));
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/post-safari.bin").unwrap();
}
