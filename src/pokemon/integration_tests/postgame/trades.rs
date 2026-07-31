//! Tests for workstream `trades` — see `docs/postgame-coverage-plan.md` §6-G5/G6 and
//! [`crate::pokemon::postgame::trades`].
//!
//! Rooted on **G-gifts' output** rather than `postgame-phase0.bin`, for two reasons: the trade driver
//! is a third [`crate::pokemon::postgame::gifts::PartyScript`] variant, and the party has to be
//! banked down before every catch, which needs the box G-gifts has already been using.
//!
//! Each leg is the same shape — bank, catch the give-species in grass, travel, trade — and each is
//! worth **two** dex entries: the mon caught and the mon received.

#[allow(unused_imports)]
use super::super::*;

use crate::pokemon::postgame::trades::trade_for;

/// G-gifts' output (§9): Celadon City, party Venusaur / Articuno / Vaporeon / Slowpoke / Aerodactyl /
/// Hitmonlee, box 1 holding Lapras + Omanyte, dex 11 owned, ¥44,284.
const NAME_RATER: &[u8] = include_bytes!("../../data/postgame-name-rater.bin");

/// The three party members every leg keeps: Venusaur leads for **Cut**, Articuno carries **Fly**
/// (which every leg starts with) and Vaporeon carries **Surf**. Everything else is banked so the
/// catch has somewhere to land.
const KEEP: usize = 3;
/// Slots 3, 4 and 5 — Slowpoke, Aerodactyl, Hitmonlee on the entry fixture. Deposited highest-first
/// by `trade_steps`, so the numbering does not shift underneath itself.
const BANK: &[u8] = &[3, 4, 5];

/// **Task G5** — the trade driver, proved on **Abra → Mr. Mime** at `Route2TradeHouse`.
///
/// Abra first because it makes the strongest test of the *driver* rather than the route: Mr. Mime is
/// one of the five species obtainable **only** by trading, so its dex entry cannot have come from
/// anywhere else.
///
/// ⚠️ The party is banked to three before the catch. With six a caught mon goes to the box, and D's
/// §11 entry records the agent *wedging* on the nickname screen on that path — this leg would
/// otherwise spend its whole budget there.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_an_abra_for_a_mr_mime() {
    let trade = trade_for(PokemonSpecies::Abra);
    let mut fixture = TestFixture::new(NAME_RATER, Duration::from_mins(90),
        PolicyStep::trade_steps(trade.give, Map::Route24, BANK, Map::CeruleanPokecenter));

    let before = fixture.game_state();
    assert_eq!(before.pokemon.len(), 6);
    assert!(!before.pokedex_owned.contains(&trade.give), "entry fixture already owns an Abra");
    assert!(!before.pokedex_owned.contains(&trade.get), "entry fixture already owns a Mr. Mime");

    let caught = fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == trade.give));
    assert_eq!(caught.pokemon.len(), KEEP + 1, "the catch must land in the party, not the box");
    println!("caught {:?} — party {}", trade.give, caught.pokemon.len());

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == trade.get));

    assert!(!state.pokemon.iter().any(|p| p.species == trade.give), "the Abra was not handed over");
    assert_eq!(state.pokemon.len(), KEEP + 1, "a trade swaps, it does not add");
    assert!(state.pokedex_owned.contains(&trade.give), "a traded-away mon stays in the dex");
    assert!(state.pokedex_owned.contains(&trade.get));
    println!("traded {:?} → {:?} · dex owned {}", trade.give, trade.get,
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-mr-mime.bin").unwrap();
}

/// G5's output: Route 2 (outdoors, so the next leg can Fly), party Venusaur / Articuno / Vaporeon / **Mr. Mime**, dex 13,
/// 8 Great Balls, five mons banked in box 1.
const MR_MIME: &[u8] = include_bytes!("../../data/postgame-mr-mime.bin");

/// **Task G6a** — **Spearow → Farfetch'd** at `VermilionTradeHouse`.
///
/// Farfetch'd is a second trade-only species, and Route 22 is the cheapest hunting ground left: it is
/// one map west of a Fly destination and holds *both* remaining give-species, so G6a and G6b share a
/// route and differ only in what they ask `CatchPokemon` for.
///
/// From here on the party arrives at four, so `bank` is a single slot — the previous leg's trophy.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_a_spearow_for_a_farfetchd() {
    let trade = trade_for(PokemonSpecies::Spearow);
    let mut fixture = TestFixture::new(MR_MIME, Duration::from_mins(90),
        PolicyStep::trade_steps(trade.give, Map::Route22, &[3], Map::ViridianPokecenter));

    let before = fixture.game_state();
    assert_eq!(before.pokemon[3].species, PokemonSpecies::MrMime, "slot 3 should be G5's trophy");
    assert!(!before.pokedex_owned.contains(&trade.get), "entry fixture already owns a Farfetch'd");

    let caught = fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == trade.give));
    assert_eq!(caught.pokemon.len(), KEEP + 1, "the catch must land in the party, not the box");

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == trade.get));
    assert!(!state.pokemon.iter().any(|p| p.species == trade.give), "the Spearow was not handed over");
    assert!(state.pokedex_owned.contains(&trade.give) && state.pokedex_owned.contains(&trade.get));
    println!("traded {:?} → {:?} · dex owned {}", trade.give, trade.get,
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-farfetchd.bin").unwrap();
}

/// G6a's output: Vermilion City, party Venusaur / Articuno / Vaporeon / **Farfetch'd**, dex 15.
const FARFETCHD: &[u8] = include_bytes!("../../data/postgame-farfetchd.bin");

/// **Task G6b** — **Nidoran♂ → Nidoran♀** at `UndergroundPathRoute5`, and the third trade.
///
/// The odd one out of the nine: Nidoran♀ is *not* trade-exclusive, so this row is worth taking for
/// the **route** rather than the species. The NPC is underground, in the tunnel whose Route 5 mouth
/// sits in a different corridor from the Day Care's — the map in G8b's §11 entry is the one to read.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_a_nidoran_underground() {
    let trade = trade_for(PokemonSpecies::NidoranMale);
    let mut fixture = TestFixture::new(FARFETCHD, Duration::from_mins(90),
        PolicyStep::trade_steps(trade.give, Map::Route22, &[3], Map::ViridianPokecenter));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&trade.get), "entry fixture already owns a Nidoran♀");

    fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == trade.give));
    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == trade.get));

    assert!(!state.pokemon.iter().any(|p| p.species == trade.give), "the Nidoran♂ was not handed over");
    assert_eq!(state.map.map, Map::Route5, "the leg ends outdoors so the next Fly is allowed");
    assert!(state.pokedex_owned.contains(&trade.give) && state.pokedex_owned.contains(&trade.get));
    println!("traded {:?} → {:?} · dex owned {}", trade.give, trade.get,
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-trades.bin").unwrap();
}

/// G6b's output: Route 5, party Venusaur / Articuno / Vaporeon / **Nidoran♀**, dex 17.
const NIDORAN: &[u8] = include_bytes!("../../data/postgame-trades.bin");

/// **Task G6c** — **Venonat → Tangela** at `CinnabarLabTradeRoom`, the third of G6's three.
///
/// Tangela is the third trade-only species this workstream reaches, and Venonat is the last
/// give-species obtainable without either an evolution grind or the Safari Zone: Route 15's grass,
/// one map east of a Fly destination. What is left after this needs **E** or a levelling detour, and
/// the §11 entry says which is which.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_a_venonat_for_a_tangela() {
    let trade = trade_for(PokemonSpecies::Venonat);
    let mut fixture = TestFixture::new(NIDORAN, Duration::from_mins(90),
        PolicyStep::trade_steps(trade.give, Map::Route15, &[3], Map::FuchsiaPokecenter));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&trade.get), "entry fixture already owns a Tangela");

    fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == trade.give));
    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == trade.get));

    assert!(!state.pokemon.iter().any(|p| p.species == trade.give), "the Venonat was not handed over");
    assert!(state.pokedex_owned.contains(&trade.give) && state.pokedex_owned.contains(&trade.get));
    assert_eq!(state.map.map, Map::CinnabarIsland, "the leg ends outdoors so the next Fly is allowed");
    println!("traded {:?} → {:?} · dex owned {}", trade.give, trade.get,
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-tangela.bin").unwrap();
}

/// Diagnostic for **G6c**: is there grass where Fuchsia lets you onto Route 15?
#[test]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_route15_grass() {
    let mut fixture = TestFixture::new(NIDORAN, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::FuchsiaCity },
        PolicyStep::goto(Map::Route15),
    ]);
    fixture.run_until(|s| s.map.map == Map::Route15);
    for _ in 0..50 { fixture.step(); }
    let state = fixture.game_state();
    println!("== Route15 @ {} ({}x{})", state.map.player_position, state.map.width, state.map.height);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    for y in 0..state.map.height.min(20) as u8 {
        let row: String = (0..state.map.width.min(34) as u8).map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
            Some(MetaTile::Obstacle) => '#',
            Some(MetaTile::Empty) => '.',
            Some(MetaTile::Warp { .. }) => 'W',
            Some(MetaTile::Connection { .. }) => 'C',
            Some(MetaTile::Jump(_)) => 'J',
            Some(MetaTile::Sprite(_)) => 'S',
            Some(MetaTile::Grass) => 'g',
            Some(MetaTile::CutTree) => 'T',
            _ => '?',
        }).collect();
        println!("  y{y:>2} {row}");
    }
}

/// Diagnostic for **G5**: Route 2's north half. The trade house door is at (15,19) and neither end of
/// the route can reach it — the same "stands still" failure `postgame::gifts` records for Route 5.
#[test]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_route2_trade_house() {
    let mut fixture = TestFixture::new(NAME_RATER, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::PewterCity },
        PolicyStep::enter(Map::Route2),
    ]);
    fixture.run_until(|s| s.map.map == Map::Route2);
    for _ in 0..50 { fixture.step(); }
    let state = fixture.game_state();
    println!("== Route2 @ {}", state.map.player_position);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    for y in 0..26u8 {
        let row: String = (0..20u8).map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
            Some(MetaTile::Water) => '~',
            Some(MetaTile::Obstacle) => '#',
            Some(MetaTile::Empty) => '.',
            Some(MetaTile::Warp { .. }) => 'W',
            Some(MetaTile::Connection { .. }) => 'C',
            Some(MetaTile::Jump(_)) => 'J',
            Some(MetaTile::Sprite(_)) => 'S',
            Some(MetaTile::Grass) => 'g',
            Some(MetaTile::CutTree) => 'T',
            _ => '?',
        }).collect();
        println!("  y{y:>2} {row}");
    }
}
