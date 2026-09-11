
use super::super::*;

use crate::pokemon::postgame::trades::trade_for;

const NAME_RATER: &[u8] = include_bytes!("../../data/postgame-name-rater.bin");

/// The three party members every leg keeps: Venusaur leads for Cut, Articuno carries Fly (which
/// every leg starts with) and Vaporeon carries Surf.
const KEEP: usize = 3;
/// Slots 3, 4 and 5 — Slowpoke, Aerodactyl, Hitmonlee on the entry fixture.
const BANK: &[u8] = &[3, 4, 5];

/// Task G5 — the trade driver, proved on Abra → Mr. Mime at `Route2TradeHouse`.
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

/// G5's output: Route 2 (outdoors, so the next leg can Fly), party Venusaur / Articuno / Vaporeon
/// / Mr. Mime, dex 13, 8 Great Balls, five mons banked in box 1.
const MR_MIME: &[u8] = include_bytes!("../../data/postgame-mr-mime.bin");

/// Task G6a — Spearow → Farfetch'd at `VermilionTradeHouse`.
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

/// G6a's output: Vermilion City, party Venusaur / Articuno / Vaporeon / Farfetch'd, dex 15.
const FARFETCHD: &[u8] = include_bytes!("../../data/postgame-farfetchd.bin");

/// Task G6b — Nidoran♂ → Nidoran♀ at `UndergroundPathRoute5`, and the third trade.
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

/// G6b's output: Route 5, party Venusaur / Articuno / Vaporeon / Nidoran♀, dex 17.
const NIDORAN: &[u8] = include_bytes!("../../data/postgame-trades.bin");

/// Task G6c — Venonat → Tangela at `CinnabarLabTradeRoom`, the third of G6's three.
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

const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// Task K1 — a sixth in-game trade: Ponyta → Seel in `CinnabarLabFossilRoom`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_a_boxed_ponyta_for_a_seel() {
    /// Where H5 left the Ponyta in the open box (box 3, which is what `wCurrentBoxNum` reads).
    const PONYTA_BOX_SLOT: u8 = 2;
    /// Rhyhorn — a lv25 with one move, the most expendable member of a full party of six.
    const BANK_SLOT: u8 = 5;

    let trade = trade_for(PokemonSpecies::Ponyta);
    let mut fixture = TestFixture::new(AIDES, Duration::from_mins(60),
        PolicyStep::trade_boxed_steps(trade.give, PONYTA_BOX_SLOT, BANK_SLOT, Map::FuchsiaPokecenter));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&trade.get), "entry fixture already owns a Seel");
    assert_eq!(before.pokemon.len(), 6, "the party is full, so the withdraw needs a deposit first");
    assert_eq!(before.boxed_pokemon[PONYTA_BOX_SLOT as usize].species, trade.give,
        "box slot {PONYTA_BOX_SLOT} of the open box should be the Ponyta H5 caught");

    // The withdraw first — it is the half `trade_steps` normally spends a catch on.
    let withdrawn = fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == trade.give));
    println!("Ponyta out of box {} and in the party at {:?}", withdrawn.current_box + 1, withdrawn.map.map);

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == trade.get));
    assert!(!state.pokemon.iter().any(|p| p.species == trade.give), "the Ponyta was not handed over");
    assert!(state.pokedex_owned.contains(&trade.get), "Seel should be in the dex");
    assert_eq!(state.map.map, Map::CinnabarIsland, "the leg ends outdoors so the next Fly is allowed");
    println!("traded {:?} → {:?} · dex owned {}", trade.give, trade.get,
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-seel.bin").unwrap();
}

const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// All nine in-game trades, each with the give-species deliberately *not* the party lead, and
/// each answered by the agent rather than by a `PartyScript`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn every_in_game_trade_can_be_made_by_talking_to_the_trader() {
    /// Where the give-species goes.
    const SEED_SLOT: usize = 3;

    let mut done: Vec<(PokemonSpecies, PokemonSpecies)> = Vec::new();
    for trade in crate::pokemon::postgame::trades::TRADES.iter().copied() {
        let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(30),
            PolicyStep::walk_up_and_trade_steps(trade));

        // Seed before the first tick, exactly as `branch_points` does: the policy sees it only
        // through an ordinary `GameState`.
        let before = fixture.game_state();
        assert!(!before.pokemon.iter().any(|mon| mon.species == trade.give),
            "the snapshot already carries a {:?}; the seed below would be untestable", trade.give);
        let mut party = crate::pokemon::party::PokemonParty::default();
        let mut members: Vec<_> = before.pokemon.iter().cloned().collect();
        members.insert(SEED_SLOT.min(members.len()), crate::pokemon::pokemon::Pokemon::maxed(
            trade.give, "SWAPME",
            [PokemonMoveName::WaterGun, PokemonMoveName::BodySlam,
             PokemonMoveName::Psychic, PokemonMoveName::Blizzard],
            before.name.clone(), before.player_id));
        members.truncate(6);
        for member in members { let _ = party.push(member); }
        fixture.api().debug_set_party(&party).expect("a party can be installed");
        assert_eq!(fixture.game_state().pokemon[0].species, PokemonSpecies::Venusaur,
            "Venusaur has to lead: `CuttingTree` only ever asks slot 0, and Route 2 needs a cut");

        let state = fixture.run_leg(|s| s.pokemon.iter().any(|mon| mon.species == trade.get));

        assert!(!state.pokemon.iter().any(|mon| mon.species == trade.give),
            "the {:?} was not handed over; a trade swaps rather than adds", trade.give);
        // Waited for rather than sampled beside the party.
        assert!(fixture.try_run_until(|s| s.pokedex_owned.contains(&trade.get)).is_some(),
            "{:?} never reached the Pokédex", trade.get);
        assert_eq!(state.map.map, trade.at, "the leg ends where the trader is");
        println!("[trade] {:?} → {:?} at {:?} ({:?} of game time)",
            trade.give, trade.get, trade.at, fixture.total_cycles.to_duration());
        done.push((trade.give, trade.get));
    }

    assert_eq!(done.len(), 9, "all nine trades: {done:?}");
}
