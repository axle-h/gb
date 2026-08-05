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

/// Probe (not a sub-step): **Rocket Hideout B1F is two disconnected halves**, and which of B2F's two
/// staircases you take decides which half you land in.
///
/// The wall at row y=16 runs the full width of the map. The Game Corner staircase (21,2) is in the
/// north half, reached from B2F (27,8); B2F (21,22) lands at B1F (21,24) in the south half, whose only
/// exits are back down to B2F and — behind the Rocket-5 door at column x=23 — the elevator. Nothing on
/// the map connects the two. That is why the exit leg must name its landing explicitly: a bare
/// `enter(RocketHideoutB1F)` takes the *nearest* B1F warp, which from the elevator is the wrong one.
///
/// Rides the elevator out of Giovanni's B4F room exactly as `poke_flute_steps` does, then dumps B2F's
/// reachable actions (both B1F landings should appear), crosses at the northern one, and dumps B1F.
#[test]
#[ignore = "probe — run with --ignored --nocapture"]
fn probe_hideout_b1f_halves() {
    fn dump(fixture: &mut TestFixture) {
        for _ in 0..50 { fixture.step(); }
        let state = fixture.game_state();
        println!("{} @ {} ({}x{})", state.map.map, state.map.player_position,
            state.map.width, state.map.height);
        for y in 0..state.map.height as u8 {
            let row: String = (0..state.map.width as u8)
                .map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
                    Some(MetaTile::Obstacle) => '#',
                    Some(MetaTile::Empty) => '.',
                    Some(MetaTile::Warp { .. }) => 'W',
                    Some(MetaTile::Sprite(_)) => 'S',
                    Some(MetaTile::Counter) => 'n',
                    Some(_) => '?',
                    None => ' ',
                })
                .collect();
            println!("   y={y:3} {row}");
        }
        for action in state.map.actions() {
            println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
        }
    }

    const RIDE: [PolicyStep; 2] = [
        PolicyStep::EnterMap { to_map: Map::RocketHideoutElevator, to_position: None },
        PolicyStep::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 1 },
    ];
    const SCOPE: &[u8] = include_bytes!("../data/post-silph-scope.bin");

    let mut b2f = TestFixture::new(SCOPE, Duration::from_mins(30), RIDE.to_vec());
    b2f.run_until(|s| s.map.map == Map::RocketHideoutB2F);
    println!("=== B2F (off the elevator) — both B1F landings should be listed ===");
    dump(&mut b2f);

    let mut steps = RIDE.to_vec();
    steps.push(PolicyStep::EnterMap {
        to_map: Map::RocketHideoutB1F,
        to_position: Some(Point8 { x: 23, y: 2 }),
    });
    let mut b1f = TestFixture::new(SCOPE, Duration::from_mins(30), steps);
    b1f.run_until(|s| s.map.map == Map::RocketHideoutB1F);
    println!("=== B1F (north half, via B2F (27,8)) — the Game Corner warp should be reachable ===");
    dump(&mut b1f);
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
