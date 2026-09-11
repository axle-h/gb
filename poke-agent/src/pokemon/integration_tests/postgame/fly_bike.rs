use super::super::*;

const ENTRY: &[u8] = include_bytes!("../../data/postgame-entry.bin");

/// The Bike Voucher from the Pokémon Fan Club chairman in Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_bike_voucher() {
    let mut fixture = TestFixture::new(ENTRY, Duration::from_mins(45), PolicyStep::bike_voucher_steps());

    let state = fixture.game_state();
    assert!(!state.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "entry fixture already has a voucher");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::BikeVoucher));
    assert_eq!(state.map.map, Map::PokemonFanClub);
    println!("bike voucher in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-bike-voucher.bin").unwrap();
}

/// `can_get_the_bike_voucher`'s output: in the Fan Club with the Bike Voucher.
const BIKE_VOUCHER: &[u8] = include_bytes!("../../data/postgame-bike-voucher.bin");

/// The voucher traded for the Bicycle at the Cerulean Bike Shop.
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

/// `can_trade_the_voucher_for_a_bicycle`'s output: in the Bike Shop with the Bicycle.
const BICYCLE: &[u8] = include_bytes!("../../data/postgame-bicycle.bin");

/// HM02 Fly from the girl in the Route 16 house.
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

/// `can_get_hm02_fly`'s output: `Route16FlyHouse`, HM02 in the bag; only Articuno can learn it.
const HM02: &[u8] = include_bytes!("../../data/postgame-hm02.bin");

/// The party slot Fly goes on, from `can_get_hm02_fly` onward.
const FLY_SLOT: u8 = 1;

/// Fly taught to Articuno.
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

/// Teach Fly, step outside, and fly from Route 16 to Pewter City.
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

    // Let the bird animation finish.
    for _ in 0..200 {
        fixture.step();
    }
    fixture.save_state_named("src/pokemon/data/postgame-fly.bin").unwrap();
}

/// `can_fly_between_towns`'s output: Fly taught, standing in Pewter City.
const FLY: &[u8] = include_bytes!("../../data/postgame-fly.bin");

/// Wake the Route 16 Snorlax, then ride Cycling Road from Celadon to Fuchsia.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ride_cycling_road_to_fuchsia() {
    let mut fixture = TestFixture::new(FLY, Duration::from_mins(60), PolicyStep::cycling_road_steps());

    let snorlax_present = |f: &mut TestFixture| f.game_state().map.sprites.iter()
        .any(|s| !s.hidden && s.name == MapSprite::ROUTE16_SNORLAX.name);

    fixture.run_until(|s| s.map.map == Map::Route16);
    assert!(snorlax_present(&mut fixture), "the Route 16 Snorlax should still be asleep on the road");

    let state = fixture.run_until(|s| s.map.map == Map::FuchsiaCity);
    assert!(state.pokedex_seen.contains(&PokemonSpecies::Snorlax), "the Snorlax battle should have happened");
    println!("rode Cycling Road to {} @ {}", state.map.map, state.map.player_position);

    fixture.save_state_named("src/pokemon/data/postgame-fly-bike.bin").unwrap();
}
