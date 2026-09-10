//! Tests for workstream `trades` — see `docs/postgame-coverage-plan.md` §6-G5/G6 and
//! [`crate::pokemon::postgame::trades`].
//!
//! Rooted on **G-gifts' output** rather than `postgame-phase0.bin`, for two reasons: the trade driver
//! is a third [`crate::pokemon::postgame::gifts::PartyScript`] variant, and the party has to be
//! banked down before every catch, which needs the box G-gifts has already been using.
//!
//! Each leg is the same shape — bank, catch the give-species in grass, travel, trade — and each is
//! worth **two** dex entries: the mon caught and the mon received.

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

/// H's output and the chain head (§9): Route 15, party of six, **box 3 slot 2 is a lv32 Ponyta**,
/// dex 52 owned.
const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// **Task K1** — a **sixth** in-game trade: **Ponyta → Seel** in `CinnabarLabFossilRoom`. Emulates
/// ≤60 min (≈2½ min wall clock).
///
/// Four of the nine trades were done in G5/G6 and the other five were skipped for one reason: each
/// wants a give-species the save did not have *in hand*. K asks whether that was the only obstacle,
/// and Ponyta is the cheapest way to find out — H5's Pokémon Mansion sweep boxed one, so this leg
/// needs no catching and no debug seeding at all, just a swap at a PC
/// ([`PolicyStep::trade_boxed_steps`]).
///
/// Two things worth carrying away, both of which would have been silent failures:
///
/// 1. ⚠️ **The trader is in the fossil room, not the trade room.** `CinnabarLabTradeRoom` holds two
///    trades (Raichu, Venonat) and this is neither; `scripts/CinnabarLabFossilRoom.asm:102` sets
///    `TRADE_FOR_SAILOR` from the *second scientist* in the room where fossils are revived. Talking to
///    the wrong NPC does not error — it just talks.
/// 2. ⚠️ **The party is full**, so the withdraw needs a deposit first, and the deposit has to come
///    first in the queue for the same reason.
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

/// Workstream B's output (§9): Fuchsia City, Fly on Articuno, the Bicycle, party Venusaur /
/// Articuno / Vaporeon / Slowpoke — and, the property this test needs, **all nine trades unspent**.
/// Every fixture after `postgame-name-rater.bin` has G5 onwards already done.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// ⭐ **All nine in-game trades, each with the give-species deliberately *not* the party lead, and
/// each answered by the agent rather than by a `PartyScript`.**
///
/// The four legs above prove a trade under `DeterministicPolicy`, whose `PolicyStep::PartyScript`
/// navigates the party menu itself. That driver is not on the deployed path: a model takes a
/// `talk to` row, and until 2026-09-10 the party menu the trader then opened was answered by the
/// agent's ordinary A-mash — which selects whatever `wCurrentMenuItem` was left on, because
/// `InGameTrade_DoTrade` calls `DisplayPartyMenu` without resetting it. So a trade went through
/// only when the give-species happened to be sitting under the cursor, and `branch_points` measured
/// exactly that. `PokemonAgent`'s `PartyMenuAnswer` is the fix and this is its breadth test.
///
/// Each trade gets its **own fixture** from the same snapshot rather than a chain: a trade is
/// one-shot per save, and nine independent runs are also nine independent walks, so a route that
/// breaks cannot take the eight after it with it.
///
/// ⚠️ **The give-species is seeded by the driver** ([`PokemonApi::debug_set_party`]) at slot 3, which
/// is `docs/coverage-plan.md` §3's line — a cheat between ticks, reaching the policy only through an
/// ordinary `GameState` — and the only affordable way to hold a Nidorino, a Slowbro, a Poliwhirl and
/// a Raichu, none of which is catchable as itself anywhere this save has been. What is *not* cheated
/// is the trade: the walk, the conversation and the menu are all played.
///
/// ⚠️ Slot 3 also keeps **Venusaur in front**, which the Route 2 leg needs for `CutTree` — and being
/// off the front is the whole point, so the two constraints agree.
///
/// **14 min of emulated time, about 10 s of wall clock** — a walk apiece and no catching at all,
/// which is why nine of them cost less than one of the legs above.
///
/// [`PokemonApi::debug_set_party`]: crate::pokemon::PokemonApi::debug_set_party
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn every_in_game_trade_can_be_made_by_talking_to_the_trader() {
    /// Where the give-species goes. ⚠️ **Not 0**: the whole question is whether the agent finds it.
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
        // ⚠️ **Waited for rather than sampled beside the party.** `_AddPartyMon` increments
        // `wPartyCount` before it sets the `wPokedexOwned` bit (`engine/pokemon/add_mon.asm`), so
        // the two are not true on the same tick and asserting them together is a race.
        assert!(fixture.try_run_until(|s| s.pokedex_owned.contains(&trade.get)).is_some(),
            "{:?} never reached the Pokédex", trade.get);
        assert_eq!(state.map.map, trade.at, "the leg ends where the trader is");
        println!("[trade] {:?} → {:?} at {:?} ({:?} of game time)",
            trade.give, trade.get, trade.at, fixture.total_cycles.to_duration());
        done.push((trade.give, trade.get));
    }

    assert_eq!(done.len(), 9, "all nine trades: {done:?}");
}
