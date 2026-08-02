//! Tests for workstream `safari` — see `docs/postgame-coverage-plan.md` §6-E and
//! [`crate::pokemon::postgame::safari`].
//!
//! Rooted on **H's output** (`postgame-flash.bin`), the head of the postgame chain: it has Fly, dex
//! 19 owned, ¥44,564 and — the reason E is worth taking — eleven of the twelve species the Safari
//! Zone would add are exactly what H3's Itemfinder gate of 30 is short of.
//!
//! ⚠️ That fixture is saved **inside Rock Tunnel 1F**, which `FlyState::blocked_by` refuses to fly
//! out of, so every leg here opens with a `Dig` off Slowpoke (party slot 4). Silent if forgotten: the
//! `Fly` pops with a reason and the rest of the queue is discarded for want of a route.

#[allow(unused_imports)]
use super::super::*;

use crate::pokemon::postgame::safari;

/// H's output (§9): Rock Tunnel 1F, party Venusaur / Articuno / Vaporeon / Tangela / Slowpoke, seven
/// mons in box 1, **dex 19 owned**, ¥44,564.
const FLASH: &[u8] = include_bytes!("../../data/postgame-flash.bin");

/// Slowpoke, the party's Dig holder — the way out of Rock Tunnel and therefore of the fixture.
const DIG_SLOT: u8 = 4;

/// The two cheapest new species in the centre's table, and — with the party at 5 — one catch either
/// side of the party/box boundary.
///
/// **Rhyhorn** is encounter slot 1 (19.9 %) with catch rate 120 and a lv25 Speed stat around 17, i.e.
/// ~69 % per encounter; **Exeggcute** holds slots 3 and 5 (9.8 % each) at catch rate 90. Nothing about
/// the mechanism depends on which species these are — only the wall clock does, and *that* they are
/// two is the point: the second one fills the party and the third catch is the box path.
const CHEAP_PAIR: &[PokemonSpecies] = &[PokemonSpecies::Rhyhorn, PokemonSpecies::Exeggcute];

/// **Kangaskhan** is in the East and West tables and *not* the centre's
/// (`data/wild/maps/SafariZoneCenter.asm`), so a centre hunt for it can only ever run the budget out
/// — which is precisely what the ejection half of E4 needs to observe.
const KANGASKHAN: &[PokemonSpecies] = &[PokemonSpecies::Kangaskhan];

/// Diagnostic — what the centre looks like from the tile the entrance auto-walk drops you on.
///
/// Kept in the tree because this is the question that cost E its first run: with Surf in the party the
/// BFS treats water as pass-through, so the nearest grass can sit **across the centre's pond** — and
/// the mount is then refused ("No SURFing here!"), leaving the policy re-issuing the same walk for the
/// whole budget. The dump prints the reachable action set and the meta-tile grid around the player, so
/// "is the grass on our side of the water" is answerable in ~25 s instead of a 90-minute timeout.
#[test]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_safari_centre_from_the_entrance() {
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(60), vec![
        PolicyStep::Dig { slot: DIG_SLOT },
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::enter(Map::SafariZoneGate),
        PolicyStep::enter(Map::SafariZoneCenter),
    ]);
    fixture.run_until(|s| s.map.map == Map::SafariZoneCenter);
    for _ in 0..50 { fixture.step(); } // let the entrance auto-walk and the sprite list settle
    let state = fixture.game_state();

    println!("== {} @ {} · can_surf {} · safari {:?}",
        state.map.map, state.map.player_position, state.map.can_surf, state.safari);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    for y in 0..state.map.height as u8 {
        let row: String = (0..state.map.width as u8).map(|x| {
            let p = Point8 { x, y };
            if p == state.map.player_position { return '@'; }
            match state.map.tile_at(p) {
                MetaTile::Grass => ',',
                MetaTile::Water => '~',
                MetaTile::Empty => '.',
                MetaTile::Warp { .. } => 'W',
                MetaTile::Sprite(_) => 'S',
                _ => '#',
            }
        }).collect();
        println!("   {y:2} {row}");
    }
}

/// Diagnostic — walk the zone's land chain and dump what each area can reach on foot.
///
/// The centre's own probe answers "which way out is walkable" (only east, once water is a wall); this
/// one answers the same question for the rest of the chain, and in particular **which of the north's
/// four west-warps a hunt should take**. They land on two shelves that one-way ledges seal off from
/// each other, and only one has grass: switch the last step to `enter_at(SafariZoneWest, 21, 0)` and
/// it reports `grass: None` — a Tauros hunt there would stand still on a bare shelf for its whole
/// budget — against `grass: Some(((6, 20), 44))` from the (26,0) pair this drives.
#[test]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_safari_areas() {
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(120), vec![
        PolicyStep::Dig { slot: DIG_SLOT },
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::enter(Map::SafariZoneGate),
        PolicyStep::enter(Map::SafariZoneCenter),
        PolicyStep::enter(Map::SafariZoneEast),
        PolicyStep::enter(Map::SafariZoneNorth),
        PolicyStep::enter_at(Map::SafariZoneWest, 26, 0),
    ]);
    for area in [Map::SafariZoneEast, Map::SafariZoneNorth, Map::SafariZoneWest] {
        fixture.run_until(|s| s.map.map == area);
        for _ in 0..50 { fixture.step(); }
        let state = fixture.game_state();
        let grass = state.map.actions().into_iter().find(|a| a.tile == MetaTile::Grass);
        println!("== {} @ {} · safari {:?}", state.map.map, state.map.player_position, state.safari);
        println!("   grass: {:?}", grass.map(|a| (a.destination, a.route.len())));
        for action in state.map.actions().iter().filter(|a| !matches!(a.tile, MetaTile::Grass)) {
            println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
        }
    }
}

/// **Tasks E2 + E3 + E4 (walking out)** — throw Safari Balls instead of running, catch a species the
/// Safari Zone is the only source of, and leave through the gate under our own steam.
///
/// ~4 minutes of emulated time, i.e. ~10 s of wall clock: Rhyhorn is a fifth of the centre's
/// encounter table and a ~73 % catch, so the hunt normally ends a long way inside its first ¥500 trip.
///
/// What this proves that nothing before it did:
///
/// - `pick_battle_action` no longer hard-codes RUN (§6-E2). The old behaviour is still there for
///   legs that merely cross the zone — it is now scoped to "no `SafariHunt` is running".
/// - The agent can actually *press* BALL. It could not before: every Safari option is terminal (there
///   is no list to confirm), and the battle executor only treated RUN that way, so a hunt would have
///   sat on the opening BALL cursor for ever. See §11.
/// - E4's deliberate exit: the gate answers "leaving early?" with a `YesNoChoice` that opens on YES,
///   so the generic A-mash walks us out and the fixture lands outdoors in Fuchsia.
/// - **A catch that goes to the box.** The party starts at 5, so the first catch fills it and the
///   second is sent to `SendNewMonToBox` — the path D reported as wedging the agent and G-gifts could
///   not reproduce. It does wedge, it is now fixed in the naming driver, and this is the regression
///   test: without the fix the leg dies on "<name> was transferred to BILL's PC!" with START being
///   pressed at a prompt that only takes A. See §11.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_a_safari_exclusive() {
    let mut steps = vec![PolicyStep::Dig { slot: DIG_SLOT }];
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

    // Paying is observable in three places at once, and all three matter: the fee leaves the wallet,
    // the game hands over its two budgets, and `EVENT_IN_SAFARI_ZONE` goes up.
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

    // E4, the walk-out: the hunt pops itself, then the two `enter` steps take us back through the gate.
    let out = fixture.run_leg(|s| s.map.map == Map::FuchsiaCity && s.safari.is_none());
    assert!(out.safari.is_none(), "the trip should be over once we are back in Fuchsia");
    for species in CHEAP_PAIR {
        assert!(out.pokedex_owned.contains(species), "{species} should be owned");
    }
    // The party filled, so exactly one of the two went to the box — the wedged path, driven to the end.
    assert_eq!(out.pokemon.len(), 6, "the first catch fills the party");
    assert_eq!(out.boxed_pokemon.len(), boxed_before + 1, "the second is transferred to BILL's PC");
    println!("out of the zone at {} · dex owned {} · ¥{}",
        out.map.map, out.pokedex_owned.species().len(), out.money);
    // Deliberately no fixture: this leg proves the mechanism, `can_sweep_the_safari_zone` produces the
    // state, and an uncommitted-but-written fixture nothing reads is just drift waiting to happen.
}

/// **Task E3, at full size** — sweep all four areas for every species the Safari Zone adds.
///
/// This is the leg that pays for the workstream twice over. The twelve targets in
/// [`safari::grounds`] take **dex 19 → 31 owned**, which clears H3's Itemfinder gate of 30 — and four
/// of them (Chansey, Scyther, Kangaskhan, Tauros) exist nowhere else on a single Red cartridge.
///
/// **~6.5 minutes of wall clock — the longest leg in the `slow-tests` tier.** Measured: 21 paid trips
/// and ¥9,000, and where the time goes is entirely the four species in 4.3 % encounter slots at 18–22 %
/// per encounter (centre 10 trips for Scyther, east 1, north 7 for Chansey, west 3 for Tauros). It is
/// bounded three ways — `max_trips` per area, the wallet, and the test's own cycle cap.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_safari_zone() {
    /// Per area, not for the sweep — and the binding one is the centre's, where **Scyther** took ten
    /// (4.3 % of encounters, 21 % per encounter). Sixty trips would be ¥30,000 of a ~¥44,000 wallet;
    /// the wallet, `max_trips` and the test's cycle cap are three independent bounds on this leg.
    const MAX_TRIPS: u32 = 15;

    let mut steps = vec![PolicyStep::Dig { slot: DIG_SLOT }];
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

/// **Tasks E1 + E4 (ejection)** — the 500-step budget, watched all the way to zero and out.
///
/// ~20 minutes of emulated time, ~50 s of wall clock: one whole ¥500 trip, deliberately spent. The
/// hunt asks for **Kangaskhan**, which is not in the centre's table, so no catch can end it early and
/// the budget is what ends the trip — the ejection path, which no test could reach before because
/// nothing ever stayed in the zone long enough to run it down.
///
/// The three things asserted are the three the plan's E1 is about: the counter is readable, it
/// *falls* while the agent paces, and hitting zero puts the player back at the gate with the trip
/// closed rather than leaving a hunt running against a budget it no longer has.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn runs_the_step_budget_down_and_is_ejected() {
    let mut steps = vec![PolicyStep::Dig { slot: DIG_SLOT }];
    steps.extend(PolicyStep::safari_hunt_steps(KANGASKHAN, 1));
    let mut fixture = TestFixture::new(FLASH, Duration::from_mins(120), steps);

    let start = fixture.run_until(|s| s.safari.is_some());
    let opening = start.safari.unwrap();
    // The gate writes `HIGH(502)/LOW(502)` — the counter is 502, not the 500 the signs claim — but the
    // first tick that can observe it is already inside, because paying ends with a scripted three-tile
    // auto-walk north and those tiles are charged like any others. 500 on arrival is the ROM being
    // consistent, not the read being wrong.
    assert!((495..=502).contains(&opening.steps_left),
        "the budget starts at 502 less the entrance auto-walk, got {}", opening.steps_left);
    assert!(!opening.game_over);

    // It falls, and it falls because of *walking*: pacing grass for encounters is what spends it.
    let halfway = fixture.run_until(|s| s.safari.is_some_and(|z| z.steps_left < 250));
    println!("halfway: {} steps, {} balls left", halfway.safari.unwrap().steps_left,
        halfway.safari.unwrap().balls_left);

    // `EVENT_SAFARI_GAME_OVER` goes up the instant the counter hits zero, a few ticks before the gate
    // script clears `EVENT_IN_SAFARI_ZONE` — so this is the one state where `safari` is `Some` and the
    // trip is already over. `safari::pick` sits on its hands here for exactly that reason.
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
