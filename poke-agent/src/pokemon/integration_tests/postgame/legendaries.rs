use super::super::*;

/// The legendaries' root: each of their three caves is one `Fly` step from it.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// The shared toolkit: Thunder Wave on Slowpoke, 10 Ultra and 40 Poké Balls, 10 Hyper Potions.
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

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::PokeBall && i.quantity >= 40));
    assert!(state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .any(|m| m.name == PokemonMoveName::ThunderWave), "Thunder Wave never landed on the paralyser");

    let held = |id: ItemId| state.bag.iter().find(|i| i.id == id).map_or(0, |i| i.quantity);
    println!("armed: {} Ultra Balls, {} Poké Balls, {} Hyper Potions, ¥{} left",
        held(ItemId::UltraBall), held(ItemId::PokeBall), held(ItemId::HyperPotion), state.money);
    assert_eq!(held(ItemId::UltraBall), 10);
    assert_eq!(held(ItemId::PokeBall), 40);
    assert_eq!(held(ItemId::HyperPotion), 10);
    // Slowpoke is the HM slave as well as the paralyser; Confusion is what should have gone.
    let moves: Vec<_> = state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .map(|m| m.name).collect();
    println!("slot {PARALYSER_SLOT} now knows {moves:?}");
    assert!(moves.contains(&PokemonMoveName::Strength) && moves.contains(&PokemonMoveName::Dig),
        "teaching Thunder Wave must not cost the HM slave its field moves");

    fixture.save_state_named("src/pokemon/data/postgame-thunder-wave.bin").unwrap();
}

/// `can_arm_for_the_legendaries`'s output: Cerulean City, armed, party healed.
const ARMED: &[u8] = include_bytes!("../../data/postgame-thunder-wave.bin");

/// Debug tier: a Master Ball, so the catch cannot fail.
fn seed_master_ball(fixture: &mut TestFixture) {
    fixture.api().debug_give_item(ItemId::MasterBall, 1).expect("bag should have room");
}

/// Moltres on Victory Road 2F.
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

/// `can_catch_moltres`'s output: Victory Road 2F's north strip, Moltres in the party at lv50.
const MOLTRES: &[u8] = include_bytes!("../../data/postgame-moltres.bin");

/// Reach the Power Plant and catch Zapdos.
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

/// `can_catch_zapdos`'s output: the Power Plant, Zapdos in the party at lv50, a party of 6.
const ZAPDOS: &[u8] = include_bytes!("../../data/postgame-zapdos.bin");

/// Cerulean Cave and Mewtwo.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_mewtwo() {
    // The Route 24 river and Cerulean Cave's ladder maze are all encounter territory.
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
