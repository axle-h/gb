use super::super::*;

const TANGELA: &[u8] = include_bytes!("../../data/postgame-tangela.bin");

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_hm05_and_light_rock_tunnel() {
    /// Slowpoke, box slot 4, one of two Flash learners this save has.
    const SLOWPOKE_BOX_SLOT: u8 = 4;
    const FLASH_SLOT: u8 = 4;
    /// The bag is 20/20; Slowpoke knows Dig, so the Escape Rope goes.
    const SHED: ItemId = ItemId::EscapeRope;

    let mut fixture = TestFixture::new(TANGELA, Duration::from_mins(90),
        PolicyStep::flash_steps(Some(SLOWPOKE_BOX_SLOT), SHED, FLASH_SLOT));

    let before = fixture.game_state();
    assert!(before.pokedex_owned.species().len() >= 10, "H1's aide wants 10 species owned");
    assert!(!before.bag.iter().any(|i| i.id == ItemId::Hm05Flash), "entry fixture already has HM05");
    assert_eq!(before.boxed_pokemon[SLOWPOKE_BOX_SLOT as usize].species, PokemonSpecies::Slowpoke,
        "box slot {SLOWPOKE_BOX_SLOT} should be the Flash learner");

    let with_hm = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::Hm05Flash));
    println!("HM05 in the bag at {:?}", with_hm.map.map);

    let taught = fixture.run_until(|s| s.pokemon.get(FLASH_SLOT as usize).is_some_and(|p|
        p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Flash)));
    assert_eq!(taught.pokemon[FLASH_SLOT as usize].species, PokemonSpecies::Slowpoke);
    println!("Flash taught to slot {FLASH_SLOT}");

    // Entering Rock Tunnel makes the map dark; assert that, or "lit" proves nothing.
    let dark = fixture.run_until(|s| s.map.map == Map::RockTunnel1F && s.map_is_dark);
    assert!(dark.map_is_dark, "Rock Tunnel 1F should be dark on arrival (wMapPalOffset = 6)");

    let state = fixture.run_leg(|s| s.map.map == Map::RockTunnel1F && !s.map_is_dark);
    assert!(!state.map_is_dark, "Flash should have cleared wMapPalOffset");
    println!("Rock Tunnel lit · dex owned {}", state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-flash.bin").unwrap();
}

const SAFARI: &[u8] = include_bytes!("../../data/postgame-safari.bin");

/// The Itemfinder from the `Route11Gate2F` aide, at a dex gate of 30 owned.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_itemfinder() {
    /// The least useful things in a full bag whose party is past level 71.
    const SHED: &[ItemId] = &[ItemId::Calcium, ItemId::Carbos];

    let mut fixture = TestFixture::new(SAFARI, Duration::from_mins(60),
        PolicyStep::itemfinder_steps(SHED));

    let before = fixture.game_state();
    assert!(before.pokedex_owned.species().len() >= 30, "H3's aide wants 30 species owned");
    assert!(!before.bag.iter().any(|i| i.id == ItemId::Itemfinder), "entry fixture already has it");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Itemfinder));
    for shed in SHED {
        assert!(!state.bag.iter().any(|i| i.id == *shed), "{shed:?} should have gone to the PC");
    }
    assert_eq!(state.map.map, Map::Route11, "the leg ends outdoors so the next Fly is allowed");
    println!("Itemfinder in the bag · dex owned {} · named bag items {}",
        state.pokedex_owned.species().len(), state.bag.iter().count());

    fixture.save_state_named("src/pokemon/data/postgame-itemfinder.bin").unwrap();
}

/// `can_get_the_itemfinder`'s output: Route 11 by the gate, Itemfinder in the bag, a slot free.
const ITEMFINDER: &[u8] = include_bytes!("../../data/postgame-itemfinder.bin");

/// The share floor every sweep uses: chase a map's fat slots, take anything rarer, move on.
const MIN_SHARE: u8 = 20;

/// The sweep's outfitting, then the two grounds either side of Route 11.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_vermilion_grounds() {
    const SHED: &[ItemId] = &[ItemId::RareCandy];
    /// Box 1 holds the Safari catches; box 2 is empty.
    const BOX: u8 = 1;

    let mut steps = PolicyStep::dex_sweep_outfit_steps(SHED, 99, BOX);
    steps.extend(PolicyStep::dex_sweep_vermilion_steps(MIN_SHARE));
    let mut fixture = TestFixture::new(ITEMFINDER, Duration::from_mins(300), steps);

    let before = fixture.game_state();
    let owned_before = before.pokedex_owned.species().len();
    assert_eq!(owned_before, 31, "H3's output");

    let stocked = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::PokeBall && i.quantity > 50));
    println!("stocked: {} Poké Balls, ¥{}, box {}",
        stocked.bag.iter().find(|i| i.id == ItemId::PokeBall).unwrap().quantity, stocked.money,
        stocked.current_box + 1);
    assert_eq!(stocked.current_box, BOX, "the sweep needs an empty box open");

    let state = fixture.run_leg(|s| s.pokedex_owned.species().len() >= owned_before + 3);
    for species in [PokemonSpecies::Ekans, PokemonSpecies::Drowzee, PokemonSpecies::Diglett] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    assert_eq!(state.map.map, Map::Route11, "the leg ends outdoors so the next Fly is allowed");
    println!("dex owned {} · {} balls left · ¥{}", state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity), state.money);

    fixture.save_state_named("src/pokemon/data/postgame-sweep-vermilion.bin").unwrap();
}

/// `can_sweep_the_vermilion_grounds`'s output: Route 11, box 2 open with its catches.
const SWEEP_VERMILION: &[u8] = include_bytes!("../../data/postgame-sweep-vermilion.bin");

/// Route 1 and Viridian Forest: the cheapest seven species in the game.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_viridian_grounds() {
    let mut fixture = TestFixture::new(SWEEP_VERMILION, Duration::from_mins(300),
        PolicyStep::dex_sweep_viridian_steps(MIN_SHARE));

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    let state = fixture.run_leg(|s| s.map.map == Map::Route2 && s.pokedex_owned.species().len() > owned_before);
    for species in [PokemonSpecies::Pidgey, PokemonSpecies::Rattata,
                    PokemonSpecies::Weedle, PokemonSpecies::Kakuna] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    println!("dex owned {} (was {owned_before}) · {} balls left",
        state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity));

    fixture.save_state_named("src/pokemon/data/postgame-sweep-viridian.bin").unwrap();
}

/// `can_sweep_the_viridian_grounds`'s output.
const SWEEP_VIRIDIAN: &[u8] = include_bytes!("../../data/postgame-sweep-viridian.bin");

/// The three Lavender grounds: Route 8, Pokémon Tower 7F, and Rock Tunnel.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_lavender_grounds() {
    let mut fixture = TestFixture::new(SWEEP_VIRIDIAN, Duration::from_mins(400),
        PolicyStep::dex_sweep_lavender_steps(MIN_SHARE));

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    let state = fixture.run_leg(|s| s.map.map == Map::Route10 && s.pokedex_owned.species().len() > owned_before);
    for species in [PokemonSpecies::Gastly, PokemonSpecies::Zubat,
                    PokemonSpecies::Geodude, PokemonSpecies::Machop] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    println!("dex owned {} (was {owned_before}) · {} balls left",
        state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity));

    fixture.save_state_named("src/pokemon/data/postgame-sweep-lavender.bin").unwrap();
}

/// `can_sweep_the_lavender_grounds`'s output.
const SWEEP_LAVENDER: &[u8] = include_bytes!("../../data/postgame-sweep-lavender.bin");

/// Route 7's Oddish and the Pokémon Mansion, then the Exp.All aide.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_exp_all() {
    /// Box 2 carries the earlier sweeps' catches; the Mansion's would overflow it.
    const BOX: u8 = 2;
    /// Free bag slots, or the aide refuses.
    const SHED: &[ItemId] = &[ItemId::MaxRevive, ItemId::SilphScope];

    let mut steps = PolicyStep::dex_sweep_outfit_steps(SHED, 57, BOX);
    steps.extend(PolicyStep::dex_sweep_celadon_steps());
    steps.extend(PolicyStep::dex_sweep_mansion_steps());
    steps.extend(PolicyStep::exp_all_steps());
    let mut fixture = TestFixture::new(SWEEP_LAVENDER, Duration::from_mins(400), steps);

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    println!("entering with dex owned {owned_before}");

    let fifty = fixture.run_until(|s| s.pokedex_owned.species().len() >= 50);
    println!("dex owned {} — Exp.All's gate cleared at {:?}",
        fifty.pokedex_owned.species().len(), fifty.map.map);

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::ExpAll));
    assert!(state.pokedex_owned.species().len() >= 50, "H5's aide wants 50 species owned");
    println!("Exp.All in the bag · dex owned {}", state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-aides.bin").unwrap();
}
