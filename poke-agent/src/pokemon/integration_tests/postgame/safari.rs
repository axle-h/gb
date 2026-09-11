
use super::super::*;

use crate::pokemon::postgame::safari;

const FLASH: &[u8] = include_bytes!("../../data/postgame-flash.bin");

/// Slowpoke, the party's Dig holder — the way out of Rock Tunnel and therefore of the fixture.
const DIG_SLOT: u8 = 4;

/// The two cheapest new species in the centre's table, and — with the party at 5 — one catch
/// either side of the party/box boundary.
const CHEAP_PAIR: &[PokemonSpecies] = &[PokemonSpecies::Rhyhorn, PokemonSpecies::Exeggcute];

/// Kangaskhan is in the East and West tables and *not* the centre's
/// (`data/wild/maps/SafariZoneCenter.asm`), so a centre hunt for it can only ever run the budget
/// out — which is precisely what the ejection half of E4 needs to observe.
const KANGASKHAN: &[PokemonSpecies] = &[PokemonSpecies::Kangaskhan];

/// Scratch: the Safari Zone's west area, entered by the *eastern* of its two warp pairs.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn cut_safari_west_shelf_fixture() {
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(120), vec![
        PolicyStep::Dig { target: crate::pokemon::policy::PartyRef::Slot(DIG_SLOT) },
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::enter(Map::SafariZoneGate),
        PolicyStep::enter(Map::SafariZoneCenter),
        PolicyStep::enter(Map::SafariZoneEast),
        PolicyStep::enter(Map::SafariZoneNorth),
        PolicyStep::enter_at(Map::SafariZoneWest, 26, 0),
    ]);
    fixture.step_until_exhausted();
    for _ in 0..50 { fixture.step() }
    let state = fixture.game_state();
    println!("{} @ {} · safari {:?}", state.map.map, state.map.player_position, state.safari);
    assert_eq!(state.map.map, Map::SafariZoneWest);
    fixture.save_state_named("src/pokemon/data/safari-west-shelf.bin").unwrap();
}

#[test]
fn the_safari_wests_rest_house_is_a_row_from_the_shelf_it_is_on() {
    let mut fixture = TestFixture::new(
        include_bytes!("../../data/safari-west-shelf.bin"), Duration::from_mins(2), vec![]);
    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::SafariZoneWest);
    let door = state.map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::Warp { to_map: Map::SafariZoneWestRestHouse, .. }))
        .expect("the rest house door is a row from this shelf");
    println!("{} -> {} in {} steps", door.id(), door.destination, door.route.len());
    assert_eq!(door.id(), "SafariZoneWest:11,11:Warp");
}

/// Tasks E2 + E3 + E4 (walking out) — throw Safari Balls instead of running, catch a species the
/// Safari Zone is the only source of, and leave through the gate under our own steam.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_a_safari_exclusive() {
    let mut steps = vec![PolicyStep::Dig { target: crate::pokemon::policy::PartyRef::Slot(DIG_SLOT) }];
    steps.extend(PolicyStep::safari_hunt_steps(CHEAP_PAIR, 2));
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(90), steps);

    let before = fixture.game_state();
    for species in CHEAP_PAIR {
        assert!(!before.pokedex_owned.contains(species), "entry fixture already owns {species}");
    }
    assert_eq!(before.pokemon.len(), 5, "one free party slot, so the second catch takes the box path");
    let boxed_before = before.boxed_pokemon.len();
    assert!(before.safari.is_none(), "not in the Safari Zone yet");
    let money_before = before.money;

    // Paying is observable in three places at once, and all three matter: the fee leaves the
    // wallet, the game hands over its two budgets, and `EVENT_IN_SAFARI_ZONE` goes up.
    let paid = fixture.run_until(|s| s.safari.is_some());
    let trip = paid.safari.unwrap();
    println!("in the zone: {} steps, {} balls, ¥{}", trip.steps_left, trip.balls_left, paid.money);
    assert_eq!(trip.balls_left, 30, "the gate hands over 30 Safari Balls");
    assert!(trip.steps_left > 490, "the step budget starts at 502, got {}", trip.steps_left);
    assert_eq!(paid.money, money_before - safari::ENTRY_FEE, "the gate charges ¥{}", safari::ENTRY_FEE);

    let caught = fixture.run_until(|s| CHEAP_PAIR.iter().all(|t| s.pokedex_owned.contains(t)));
    let after = caught.safari.expect("still on the clock when the catches land");
    println!("both caught with {} steps and {} balls left · dex owned {} · party {} · box {}",
        after.steps_left, after.balls_left, caught.pokedex_owned.species().len(),
        caught.pokemon.len(), caught.boxed_pokemon.len());
    assert!(after.balls_left < 30, "balls were thrown, not run from");

    // E4, the walk-out: the hunt pops itself, then the two `enter` steps take us back through the
    // gate.
    let out = fixture.run_leg(|s| s.map.map == Map::FuchsiaCity && s.safari.is_none());
    assert!(out.safari.is_none(), "the trip should be over once we are back in Fuchsia");
    for species in CHEAP_PAIR {
        assert!(out.pokedex_owned.contains(species), "{species} should be owned");
    }
    // The party filled, so exactly one of the two went to the box — the wedged path, driven to
    // the end.
    assert_eq!(out.pokemon.len(), 6, "the first catch fills the party");
    assert_eq!(out.boxed_pokemon.len(), boxed_before + 1, "the second is transferred to BILL's PC");
    println!("out of the zone at {} · dex owned {} · ¥{}",
        out.map.map, out.pokedex_owned.species().len(), out.money);
    // Deliberately no fixture: this leg proves the mechanism, `can_sweep_the_safari_zone`
    // produces the state, and an uncommitted-but-written fixture nothing reads is just drift
    // waiting to happen.
}

/// Task E3, at full size — sweep all four areas for every species the Safari Zone adds.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "very slow (381 s, 6× the leg tier's next \
    slowest) — run with --features slow-tests")]
fn can_sweep_the_safari_zone() {
    /// Per area, not for the sweep — and the binding one is the centre's, where Scyther took ten
    /// (4.3 % of encounters, 21 % per encounter).
    const MAX_TRIPS: u32 = 15;

    let mut steps = vec![PolicyStep::Dig { target: crate::pokemon::policy::PartyRef::Slot(DIG_SLOT) }];
    steps.extend(PolicyStep::safari_sweep_steps(MAX_TRIPS));
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(1200), steps);

    let before = fixture.game_state();
    let owned_before = before.pokedex_owned.species().len();
    println!("starting at dex {owned_before} owned, ¥{}, box {} of 20, party {}",
        before.money, before.boxed_pokemon.len(), before.pokemon.len());

    let out = fixture.run_leg(|s| s.map.map == Map::FuchsiaCity && s.safari.is_none());
    let owned = out.pokedex_owned.species().len();
    let missed: Vec<_> = [safari::grounds::CENTRE, safari::grounds::EAST,
                          safari::grounds::NORTH, safari::grounds::WEST]
        .concat().into_iter().filter(|s| !out.pokedex_owned.contains(s)).collect();
    println!("swept: dex {owned_before} → {owned} owned · ¥{} · box {} of 20 · missed {missed:?}",
        out.money, out.boxed_pokemon.len());

    assert!(out.safari.is_none(), "the last trip should be closed out");
    assert!(owned >= 30, "the sweep is worth taking for H3's gate of 30 owned; got {owned}, missing {missed:?}");

    fixture.save_state_named("src/pokemon/data/postgame-safari.bin").unwrap();
}

/// Tasks E1 + E4 (ejection) — the 500-step budget, watched all the way to zero and out.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn runs_the_step_budget_down_and_is_ejected() {
    let mut steps = vec![PolicyStep::Dig { target: crate::pokemon::policy::PartyRef::Slot(DIG_SLOT) }];
    steps.extend(PolicyStep::safari_hunt_steps(KANGASKHAN, 1));
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(120), steps);

    let start = fixture.run_until(|s| s.safari.is_some());
    let opening = start.safari.unwrap();
    // The gate writes `HIGH(502)/LOW(502)` — the counter is 502, not the 500 the signs claim —
    // but the first tick that can observe it is already inside, because paying ends with a
    // scripted three-tile auto-walk north and those tiles are charged like any others.
    assert!((495..=502).contains(&opening.steps_left),
        "the budget starts at 502 less the entrance auto-walk, got {}", opening.steps_left);
    assert!(!opening.game_over);

    // It falls, and it falls because of *walking*: pacing grass for encounters is what spends it.
    let halfway = fixture.run_until(|s| s.safari.is_some_and(|z| z.steps_left < 250));
    println!("halfway: {} steps, {} balls left", halfway.safari.unwrap().steps_left,
        halfway.safari.unwrap().balls_left);

    // `EVENT_SAFARI_GAME_OVER` goes up the instant the counter hits zero, a few ticks before the
    // gate script clears `EVENT_IN_SAFARI_ZONE` — so this is the one state where `safari` is
    // `Some` and the trip is already over.
    let over = fixture.run_until(|s| s.safari.is_some_and(|z| z.game_over));
    println!("game over at {} steps on {}", over.safari.unwrap().steps_left, over.map.map);
    assert_eq!(over.safari.unwrap().steps_left, 0, "the trip ends when the counter reaches 0");

    let ejected = fixture.run_leg(|s| s.map.map == Map::FuchsiaCity && s.safari.is_none());
    assert!(ejected.safari.is_none(), "the gate closes the trip out");
    assert!(!ejected.pokedex_owned.contains(&PokemonSpecies::Kangaskhan),
        "Kangaskhan is not in the centre's table — the point of asking for it");
    println!("ejected and back in Fuchsia · ¥{} · dex owned {}",
        ejected.money, ejected.pokedex_owned.species().len());
}
