
use super::super::*;

const PHASE0: &[u8] = include_bytes!("../../data/postgame-phase0.bin");

/// Task B1 — the Bike Voucher, from the Pokémon Fan Club chairman in Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_bike_voucher() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(45), PolicyStep::bike_voucher_steps());

    let state = fixture.game_state();
    assert!(!state.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "entry fixture already has a voucher");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::BikeVoucher));
    assert_eq!(state.map.map, Map::PokemonFanClub);
    println!("bike voucher in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-bike-voucher.bin").unwrap();
}

/// B1's output: standing in the Vermilion Pokémon Fan Club with the Bike Voucher in the bag.
const BIKE_VOUCHER: &[u8] = include_bytes!("../../data/postgame-bike-voucher.bin");

/// Task B2 — trade the voucher for the Bicycle at the Cerulean Bike Shop.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_trade_the_voucher_for_a_bicycle() {
    let mut fixture = TestFixture::new(BIKE_VOUCHER, Duration::from_mins(20), PolicyStep::bicycle_steps());

    let state = fixture.game_state();
    assert!(state.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "B1's fixture should hold the voucher");
    assert!(!state.bag.iter().any(|i| i.id == ItemId::Bicycle));

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Bicycle));
    assert_eq!(state.map.map, Map::BikeShop);
    assert!(!state.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "the voucher should have been spent");
    println!("bicycle in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-bicycle.bin").unwrap();
}

/// B2's output: inside the Cerulean Bike Shop with the Bicycle in the bag.
const BICYCLE: &[u8] = include_bytes!("../../data/postgame-bicycle.bin");

/// Task B3 — HM02 Fly, from the girl in the Route 16 house.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_hm02_fly() {
    let mut fixture = TestFixture::new(BICYCLE, Duration::from_mins(40), PolicyStep::hm02_steps());

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::Hm02Fly));

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Hm02Fly));
    assert_eq!(state.map.map, Map::Route16FlyHouse);
    println!("HM02 in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-hm02.bin").unwrap();
}

/// B3's output: inside `Route16FlyHouse` with HM02 in the bag (at bag index 15 of 16) and the
/// party rotated to Venusaur / Articuno / Vaporeon / Slowpoke — Articuno is the only party member
/// HM02 is compatible with (`pokered/data/pokemon/base_stats/articuno.asm:17-20`).
const HM02: &[u8] = include_bytes!("../../data/postgame-hm02.bin");

/// The party slot Fly goes on, in every fixture from B3 onward.
const FLY_SLOT: u8 = 1;

/// Task B4 — teach Fly to Articuno. Measured: well under a minute emulated, <1 s wall clock.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_teach_fly() {
    let mut fixture = TestFixture::new(HM02, Duration::from_mins(10), vec![
        PolicyStep::TeachMove { item: ItemId::Hm02Fly, target: PartyRef::Slot(FLY_SLOT) },
    ]);

    let state = fixture.game_state();
    let flyer = &state.pokemon[FLY_SLOT as usize];
    assert_eq!(flyer.species, PokemonSpecies::Articuno, "slot {FLY_SLOT} should be the Fly candidate");
    assert!(!flyer.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly));
    println!("bag index of HM02: {:?}", fixture.api().bag_item_position(ItemId::Hm02Fly));

    let state = fixture.run_until(|s| s.pokemon[FLY_SLOT as usize].moves.iter().flatten()
        .any(|m| m.name == PokemonMoveName::Fly));
    println!("{:?} now knows {:?}", state.pokemon[FLY_SLOT as usize].species,
        state.pokemon[FLY_SLOT as usize].moves.iter().flatten().map(|m| m.name).collect::<Vec<_>>());
}

/// Task B5 — the Fly driver: teach Fly, step outside, and fly Route 16 → Pewter City.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_fly_between_towns() {
    const DESTINATION: Map = Map::PewterCity;
    let mut fixture = TestFixture::new(HM02, Duration::from_mins(15),
        PolicyStep::teach_and_use_fly_steps(DESTINATION));

    println!("flyable towns: {:?}", crate::pokemon::postgame::fly_bike::visited_towns(fixture.api().mmu()));

    let state = fixture.run_until(|s| s.map.map == DESTINATION);
    println!("landed on {} @ {}", state.map.map, state.map.player_position);
    assert_eq!(state.badges.bits(), 255, "badges lost in flight");
    assert_eq!(state.pokemon.len(), 4);
    assert!(state.pokemon[FLY_SLOT as usize].moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly));

    // Let the bird animation finish before snapshotting.
    for _ in 0..200 {
        fixture.step();
    }
    fixture.save_state_named("src/pokemon/data/postgame-fly.bin").unwrap();
}

/// B5's output: Fly taught and proven, standing in Pewter City — i.e. every town is one step
/// away.
const FLY: &[u8] = include_bytes!("../../data/postgame-fly.bin");

/// Tasks B7 + B6 — wake the Route 16 Snorlax, then ride Cycling Road from Celadon to Fuchsia.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ride_cycling_road_to_fuchsia() {
    let mut fixture = TestFixture::new(FLY, Duration::from_mins(60), PolicyStep::cycling_road_steps());

    // Route 16's Snorlax is a *sprite* on the map, hidden once it has been woken and beaten.
    let snorlax_present = |f: &mut TestFixture| f.game_state().map.sprites.iter()
        .any(|s| !s.hidden && s.name == MapSprite::ROUTE16_SNORLAX.name);

    fixture.run_until(|s| s.map.map == Map::Route16);
    assert!(snorlax_present(&mut fixture), "the Route 16 Snorlax should still be asleep on the road");

    let state = fixture.run_until(|s| s.map.map == Map::FuchsiaCity);
    assert!(state.pokedex_seen.contains(&PokemonSpecies::Snorlax), "the Snorlax battle should have happened");
    println!("rode Cycling Road to {} @ {}", state.map.map, state.map.player_position);

    fixture.save_state_named("src/pokemon/data/postgame-fly-bike.bin").unwrap();
}
