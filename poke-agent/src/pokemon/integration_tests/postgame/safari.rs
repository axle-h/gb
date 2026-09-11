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

/// **Scratch: the Safari Zone's west area, entered by the *eastern* of its two warp pairs.**
///
/// ⭐ `docs/coverage-plan.md` step 1.4. `SafariZoneWest` is two shelves that one-way ledges seal off
/// from each other, and which pair of doors a walk comes in by decides which shelf it stands on: the
/// western pair (North (2, 35) / (3, 35) → West (20, 0) / (21, 0)) lands on the Gold Teeth plateau,
/// and the **eastern** pair (North (8, 35) / (9, 35) → West (26, 0) / (27, 0)) lands on the shelf the
/// **rest house** is on. Every coverage sweep has taken the western pair and left the eastern one
/// `unreached`, which is why `SafariZoneWestRestHouse` has never been in a union.
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

/// **The Safari Zone's west rest house is a row from the shelf it is on** —
/// `docs/coverage-plan.md` step 1.4, and the last map any sweep had never entered.
///
/// ⚠️ **The finding is not that a row was missing; it is that the shelf decides.** `SafariZoneWest`
/// is cut in two by one-way ledges, and `SafariZoneNorth` has *four* doors into it in two pairs that
/// land on opposite sides of them. `actions()` mints a row per unique destination, so all four are on
/// the menu — but the frontier counts a way out **per crossing**, `SafariZoneNorth → SafariZoneWest`,
/// so once either pair is taken the crossing is "done" and the other pair waits behind every other
/// exit on a very large map. Ten walks of six game-hours each took the western pair every time.
///
/// So this pins the half that is a fact about the game rather than about the walk: from the eastern
/// landing the door at (11, 11) is offered and routable, and from the western one it is not there at
/// all. A map that is only reachable through one of two doors is exactly the shape §2.1's ⚠️ about
/// Seafoam's two holes warns about, one level up.
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
/// ⚠️ **Its own tier (`very-slow-tests`), because it emulates more game time than the whole of
/// `slow-tests` put together.** Re-measured 2026-08-05: **381 s** standalone against **65 s** for the
/// next slowest leg (`early_game::can_reach_vermilion`) and ≤65 s for all 115 others — so on the leg
/// tier's 24-way run it *was* the wall clock, six minutes of it, with nothing on screen to say which
/// test was still going. ~190 min of emulated game time: 21 paid trips and ¥9,000, spent almost
/// entirely on the four species in 4.3 % encounter slots at 18–22 % per encounter (centre 10 trips for
/// Scyther, east 1, north 7 for Chansey, west 3 for Tauros).
///
/// **There is no waste to reclaim** — the pacing pair is grass↔grass so every safari step rolls for an
/// encounter, animations are off and text is Fast via [`FAST_FIXTURE_OPTIONS`], each species is hunted
/// where its slot is fattest ([`safari::grounds`]), and BAIT/ROCK are worked out to be losing plays in
/// [`safari::pick_battle_action`]. It is long because the game is: the only lever left is agent-side
/// battle latency, and changing that re-rolls every RNG stream in the suite (see
/// `TestFixture::with_original_battle_timing`), which is not a trade worth making for test wall clock.
///
/// Nothing else in the tier depends on it running: `postgame-safari.bin` is committed, and every
/// *mechanism* it uses — paying, pacing, throwing balls, a catch that overflows to the box, ejection
/// at zero steps and the deliberate walk-out — is covered in `slow-tests` by
/// [`can_catch_a_safari_exclusive`] (10 s) and [`runs_the_step_budget_down_and_is_ejected`] (22 s).
/// What moves out with it is the *route* claim: that the twelve grounds add up to dex 30.
///
/// It is bounded three ways — `max_trips` per area, the wallet, and the test's own cycle cap.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "very slow (381 s, 6× the leg tier's next \
    slowest) — run with --features slow-tests")]
fn can_sweep_the_safari_zone() {
    /// Per area, not for the sweep — and the binding one is the centre's, where **Scyther** took ten
    /// (4.3 % of encounters, 21 % per encounter). Sixty trips would be ¥30,000 of a ~¥44,000 wallet;
    /// the wallet, `max_trips` and the test's cycle cap are three independent bounds on this leg.
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
    let mut steps = vec![PolicyStep::Dig { target: crate::pokemon::policy::PartyRef::Slot(DIG_SLOT) }];
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
