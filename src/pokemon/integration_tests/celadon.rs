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
///
/// ⚠️ **Ninety minutes, because the garden is eight separate cuts before the fight even starts.**
/// The gym's paths are real cuttable trees and `CutTree` clears one chokepoint at a time as the last
/// one opens access to the next, with the junior trainers engaging by line of sight in between — so
/// most of this leg's cost is over before Erika is reached.
///
/// This is also the fight the Route 11 Drowzee exists for: Blastoise's Water is resisted by all three
/// of Erika's mons, and measured here it fell asleep to a Sleep Powder and fainted — after which
/// **Confusion one-shot the Victreebel** and carried the room. Grass/Poison takes 2× from Psychic.
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

/// From Celadon City, reach the Rocket Hideout: heal, walk to the Game Corner, flip the poster switch
/// (`FlipSwitch` + the `found_rocket_hideout` event), and descend to B1F.
///
/// ⚠️ **Started from `post-rainbow-badge.bin`, and starting it before Erika instead was a hole in the
/// whole chain.** The hideout needs Cut rather than the badge, so `at-celadon.bin` is a *valid* seed
/// for this leg — and it is the seed everything downstream then inherited, so the badge was never
/// picked up. Nothing noticed until the starter changed: **Strength's field use is gated on the
/// Rainbow Badge**, and the Seafoam leg opened the party menu on a Blastoise that knows Strength and
/// got STATS / SWITCH / CANCEL back, then drove it for the rest of its budget. Erika is one step and
/// the leg above already proves her; a chain that skips a badge is a chain that is not the run.
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

/// ⭐ **"There is no route" about a route four pets are standing on is a claim about the wrong
/// thing.**
///
/// Every row is a BFS from where the player is standing, so a person anywhere on the path makes the
/// square unreachable and the row is simply not in `actions()` on that tick. Celadon Mansion 1F is
/// the sharpest case in the game: a room barely two squares wide with a Meowth, a Clefairy, a
/// Nidoran and their owner all wandering across the one corridor to the stairs. The coverage walk of
/// 2026-09-10 took `CeladonMansion1F:7,1:Warp`, one of them stepped into the gap, and the walk was
/// abandoned with *"there is no route to the warp to CeladonMansion2F"* — from a square where, two
/// ticks later, there was.
///
/// The state below is the one the walk dropped at that moment, and the action list on it holds four
/// sprites and **neither** the stairs nor the door out, which is what says this is a blocked
/// corridor rather than anything about stairs. `MAX_ROUTE_LOST_TICKS` is what waits it out; the
/// abort still happens, five seconds of game time later, for a row that really has gone.
///
/// ⚠️ The same shape scored `CeruleanMart:CooltrainerFemale` on three earlier sweeps and never twice
/// in the same region, which is the signature of a wanderer rather than of a pathfinder.
// Default tier: the state is a few ticks from the answer and the whole test is milliseconds.
#[test]
fn a_route_a_wandering_pet_is_standing_on_is_waited_out_rather_than_disputed() {
    use crate::geometry::Point8;
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

/// **A corridor with a person in it needs longer than a wanderer's pause, and 5 s was the wrong
/// bound.**
///
/// The sibling above waits `MAX_ROUTE_LOST_TICKS` — 5 s of game time — for the row to come back,
/// sized against one Gen-1 NPC taking a step. Celadon's Chief House is what that does not cover: two
/// corridors, each one tile wide, with a Rocket in one and the Chief in the other, so a row on the
/// far side of the room needs *both* of them to be somewhere else at once. The `celadon` walk of
/// 2026-09-10 said *"there is no route to Sailor"*, then *"there is no route to the warp to
/// CeladonCity"* — and then took that same warp on the very next turn, which is the whole proof that
/// neither sentence was true.
///
/// So past the short bound the agent stops counting and asks
/// [`MetaTileMap::row_blocked_by_people`](crate::pokemon::tile_map::MetaTileMap::row_blocked_by_people):
/// put everybody else back on the floor they are standing on, and does the row come back? A `yes`
/// buys `MAX_ROUTE_BLOCKED_TICKS`, 30 s. A `no` is a real absence — a `; inaccessible` warp — and is
/// still answered at 5 s exactly as before, which is why this is a question rather than a bigger
/// number.
///
/// ⚠️ **The question is what is pinned here, not the bound, and that is a limit of the evidence
/// rather than a choice.** Restoring the dropped state re-rolls how the two wanderers walk, so the
/// jam that lasted longer than five seconds in the sweep clears in well under one on a replay —
/// §6.2's "which id fails is not reproducible by re-running" reaching one layer further down than
/// usual, since even the state does not reproduce it. What *is* on the state, exactly as the walk
/// met it, is a room where the row is missing and the reason is two people; so that is asserted in
/// both directions, because a predicate that answered "yes, people" for everything absent would buy
/// the longer bound for the `; inaccessible` warps too.
///
/// The state is the one the walk dropped: the player boxed into the west column at (0, 5) by the
/// Rocket at (1, 5), with the Chief at (4, 2) plugging the only other way round.
// Default tier: the state is standing on the answer; the walk out is a couple of seconds.
#[test]
fn a_room_whose_corridors_are_both_blocked_is_waited_out_rather_than_called_routeless() {
    use crate::geometry::Point8;
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

    // ⭐ Both of the rows the walk disputed are missing *because of the two people*, and come back
    // the moment they are lifted.
    assert!(start.map.row_blocked_by_people(MetaTile::Sprite("Sailor")),
        "the Sailor is unreachable because the Rocket and the Chief are standing in the two \
         corridors, and lifting them has to bring the row back");
    assert!(start.map.row_blocked_by_people(
            MetaTile::Warp { to_map: Map::CeladonCity, to_position: Point8 { x: 35, y: 27 } }),
        "so is the door out, which is the row the sweep disproved for itself one turn later");

    // ⚠️ **And the other direction, or the predicate is just "are there people on this map".** There
    // is no Nurse in the Rocket chief's house, so no amount of standing aside produces a row for
    // one, and a row that is absent for a reason other than people must not buy the longer bound.
    assert!(!start.map.row_blocked_by_people(MetaTile::Sprite("Nurse")),
        "a row nobody is blocking is a row that is simply not there");

    let end = fixture.run_until(|state| state.map.map == Map::CeladonCity);
    println!("left the room at ({}, {})", end.map.player_position.x, end.map.player_position.y);
}
