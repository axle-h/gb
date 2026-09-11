
use super::super::*;

/// Workstream D roots at B's output, not `postgame-phase0.bin`: D is three trips to opposite
/// corners of Kanto (Victory Road, the Power Plant, Cerulean Cave) and `postgame-fly-bike.bin` is
/// the first fixture from which each of those is one `Fly` step away.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// Task D1a — the shared toolkit: Thunder Wave on Slowpoke, 10 Ultra + 40 Poké Balls, 10 Hyper
/// Potions.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_arm_for_the_legendaries() {
    use crate::pokemon::postgame::legendaries::PARALYSER_SLOT;

    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(45),
        PolicyStep::arm_for_legendaries_steps());

    let state = fixture.game_state();
    assert_eq!(state.pokemon[PARALYSER_SLOT as usize].species, PokemonSpecies::Slowpoke,
        "slot {PARALYSER_SLOT} should be the only Thunder Wave-compatible party member");
    assert!(!state.bag.iter().any(|i| i.id == ItemId::PokeBall || i.id == ItemId::UltraBall));

    // The Poké Balls are the last thing bought, so they are what "armed" means.
    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::PokeBall && i.quantity >= 40));
    assert!(state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .any(|m| m.name == PokemonMoveName::ThunderWave), "Thunder Wave never landed on the paralyser");

    let held = |id: ItemId| state.bag.iter().find(|i| i.id == id).map_or(0, |i| i.quantity);
    println!("armed: {} Ultra Balls, {} Poké Balls, {} Hyper Potions, ¥{} left",
        held(ItemId::UltraBall), held(ItemId::PokeBall), held(ItemId::HyperPotion), state.money);
    assert_eq!(held(ItemId::UltraBall), 10);
    assert_eq!(held(ItemId::PokeBall), 40);
    assert_eq!(held(ItemId::HyperPotion), 10);
    // Slowpoke is the party's HM slave as well as its paralyser — Confusion is what should have
    // gone.
    let moves: Vec<_> = state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .map(|m| m.name).collect();
    println!("slot {PARALYSER_SLOT} now knows {moves:?}");
    assert!(moves.contains(&PokemonMoveName::Strength) && moves.contains(&PokemonMoveName::Dig),
        "teaching Thunder Wave must not cost the HM slave its field moves");

    fixture.save_state_named("src/pokemon/data/postgame-thunder-wave.bin").unwrap();
}

/// D1a's output: Cerulean City, Slowpoke holding Thunder Wave / Strength / Headbutt / Dig, 10
/// Ultra + 40 Poké Balls, 10 Hyper Potions, ¥6,209, party healed.
const ARMED: &[u8] = include_bytes!("../../data/postgame-thunder-wave.bin");

/// Debug-tier seed — a Master Ball, so the catch itself cannot fail.
fn seed_master_ball(fixture: &mut TestFixture) {
    fixture.api().debug_give_item(ItemId::MasterBall, 1).expect("bag should have room");
}

/// Task D1b — catch Moltres on Victory Road 2F.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_moltres() {
    let mut fixture = TestFixture::new(ARMED, Duration::from_mins(90), PolicyStep::moltres_steps());
    seed_master_ball(&mut fixture);

    assert!(!fixture.game_state().pokedex_owned.contains(&PokemonSpecies::Moltres));

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Moltres));
    println!("caught Moltres — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-moltres.bin").unwrap();
}

/// D1b's output: Victory Road 2F's north strip, Moltres in the party at lv50, dex 8 owned / 114
/// seen.
const MOLTRES: &[u8] = include_bytes!("../../data/postgame-moltres.bin");

/// Tasks D2 + D3 — reach the Power Plant and catch Zapdos.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_zapdos() {
    let mut fixture = TestFixture::new(MOLTRES, Duration::from_mins(90), PolicyStep::zapdos_steps());
    seed_master_ball(&mut fixture);

    assert!(!fixture.game_state().pokedex_owned.contains(&PokemonSpecies::Zapdos));

    fixture.run_until(|s| s.map.map == Map::PowerPlant);
    println!("reached the Power Plant");

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Zapdos));
    println!("caught Zapdos — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-zapdos.bin").unwrap();
}

/// D3's output: the Power Plant, Zapdos in the party at lv50, dex 9 owned / 117 seen, party of 6.
const ZAPDOS: &[u8] = include_bytes!("../../data/postgame-zapdos.bin");

/// Tasks D5 + D6 — Cerulean Cave and Mewtwo, the last of the four.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_mewtwo() {
    // A generous budget: this is much the longest leg in D — the Route 24 river, then Cerulean
    // Cave's three-floor ladder maze, whose Surf legs and 128-step walks are all wild-encounter
    // territory.
    let mut fixture = TestFixture::new(ZAPDOS, Duration::from_mins(150), PolicyStep::mewtwo_steps());
    seed_master_ball(&mut fixture);

    let state = fixture.game_state();
    assert!(!state.pokedex_owned.contains(&PokemonSpecies::Mewtwo));
    assert_eq!(state.pokemon.len(), 6, "the leg starts with a full party and banks Moltres to make room");

    fixture.run_until(|s| s.map.map == Map::CeruleanCaveB1F);
    println!("reached Mewtwo's chamber");

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Mewtwo));
    println!("caught Mewtwo — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    println!("box{}[{}]: {:?}", state.current_box + 1, state.boxed_pokemon.len(),
        state.boxed_pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-legendaries.bin").unwrap();
}
