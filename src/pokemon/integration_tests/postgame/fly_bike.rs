//! Tests for workstream `fly_bike` — see `docs/postgame-coverage-plan.md` §6 and
//! [`crate::pokemon::postgame::fly_bike`].

#[allow(unused_imports)]
use super::super::*;

/// Workstream B's entry fixture (§9): all 8 badges, party of 4 (Articuno / Venusaur / Vaporeon /
/// Slowpoke), bag 14/20, standing in the Viridian Pokémon Center. Not `post-hall-of-fame.bin`, which
/// is a cutscene — see the Phase 0 §11 entries.
const PHASE0: &[u8] = include_bytes!("../../data/postgame-phase0.bin");

/// **Task B1** — the Bike Voucher, from the Pokémon Fan Club chairman in Vermilion.
///
/// Most of the cost is the walk: Viridian → Route 2 → Diglett's Cave → Route 11 → Vermilion, which is
/// `saffron_to_cinnabar_steps`' crossing in reverse and the only short land route between the two
/// (the alternative is the Pewter/Mt Moon loop). Measured: ~3 min emulated, **8 s wall clock**.
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

/// **Task B2** — trade the voucher for the Bicycle at the Cerulean Bike Shop.
///
/// Vermilion → Cerulean via the Underground Path, then one `Interact` with the clerk. Measured: ~2 min
/// emulated, **4 s wall clock**. The interesting property is that the voucher is *consumed*: the clerk's
/// no-voucher branch opens a ¥1,000,000 BICYCLE/CANCEL menu and gives nothing, so a run that landed
/// there would still print "bag unchanged" rather than failing loudly — hence both assertions.
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

/// **Task B3** — HM02 Fly, from the girl in the Route 16 house.
///
/// Cerulean → Saffron → Celadon → Route 16, then one `Interact`. Measured: ~2 min emulated, **5 s wall
/// clock**.
/// The route's one non-obvious link is `Route16Gate1F`: its two corridors (Route 16 y=10/11 and y=4/5)
/// are joined only past a guard who wants to see the **Bicycle**, so this leg genuinely depends on B2.
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

/// B3's output: inside `Route16FlyHouse` with HM02 in the bag (at bag index 15 of 16) and the party
/// rotated to Venusaur / **Articuno** / Vaporeon / Slowpoke — Articuno is the only party member HM02
/// is compatible with (`pokered/data/pokemon/base_stats/articuno.asm:17-20`).
const HM02: &[u8] = include_bytes!("../../data/postgame-hm02.bin");

/// The party slot Fly goes on, in every fixture from B3 onward.
const FLY_SLOT: u8 = 1;

/// **Task B4** — teach Fly to Articuno. Measured: well under a minute emulated, **<1 s wall clock**.
///
/// Kept as its own test, even though `can_fly_between_towns` teaches Fly too, because B4 is a sub-step
/// with its own observable and this isolates it for under a second.
///
/// The plan expected this to be free ("`TeachMove` already works — just use it") and it was, which is
/// worth recording: HM02 lands at bag **index 15 of 16**, and three tests are `#[ignore]`d blaming
/// `TeachMove` for wedging on "an HM deep in the bag" (HM04 at index 11). It taught first try in 0.6 s,
/// so bag depth is not what ails those tests. See the §11 entry.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_teach_fly() {
    let mut fixture = TestFixture::new(HM02, Duration::from_mins(10), vec![
        PolicyStep::TeachMove { item: ItemId::Hm02Fly, target_slot: FLY_SLOT },
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

/// **Task B5** — the Fly driver: teach Fly, step outside, and fly Route 16 → **Pewter City**.
///
/// Measured: ~30 s emulated, **1 s wall clock** — which is the point, since the same trip on foot is
/// Celadon → Saffron → Cerulean → Mt Moon → Pewter.
///
/// Pewter is chosen deliberately. The town-map cursor starts on **Pallet** (map id 0) and only walks
/// *forward* through `wFlyLocationsList`, so Pewter (id 2) proves the driver moves the cursor and
/// lands on the right entry — a target the cursor already happens to sit on would prove nothing, and a
/// wrong-by-one cursor would land in Viridian.
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

    // Let the bird animation finish before snapshotting. `run_until` returns the tick the map changes,
    // which is *mid-flight*: `BIT_USED_FLY` is still set, and a fixture captured there hands the next
    // leg a save that looks like a flight already in progress.
    for _ in 0..200 {
        fixture.step();
    }
    fixture.save_state_named("src/pokemon/data/postgame-fly.bin").unwrap();
}

/// B5's output: Fly taught and proven, standing in Pewter City — i.e. every town is one step away.
const FLY: &[u8] = include_bytes!("../../data/postgame-fly.bin");

/// **Tasks B7 + B6** — wake the Route 16 Snorlax, then ride Cycling Road from Celadon to Fuchsia.
/// Measured: ~6 min emulated, **16 s wall clock**; the budget is left far above that because how much of
/// Route 17 goes on biker battles depends on which lines of sight the ride happens to cross. Commit
/// target: `postgame-fly-bike.bin`.
///
/// Both halves in one run because they share a map and interfere: the Snorlax battle reloads Route 16,
/// which regrows the cut tree the Cycling Road entrance is behind.
///
/// What the Snorlax half proves is the **second** Snorlax, not the mechanism — `UseFieldItem` with the
/// Poké Flute was already how Route 12's was cleared. What Cycling Road proves is that owning the
/// Bicycle is enough: the gate guard checks the bag, and the tiles past the gate mount the bike
/// themselves, so the agent never needs to use it.
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
