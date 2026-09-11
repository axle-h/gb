use super::super::*;

const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// The Coin Case from the gym guide in the Celadon Diner.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_coin_case() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(20), PolicyStep::coin_case_steps());

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::CoinCase),
        "entry fixture already has the Coin Case");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::CoinCase));
    // Outdoors: the leg walks back out so the next leg's `Fly` is not refused.
    assert_eq!(state.map.map, Map::CeladonCity);
    println!("Coin Case in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-coin-case.bin").unwrap();
}

/// `can_get_the_coin_case`'s output: Celadon City outside the Diner, with the Coin Case.
const COIN_CASE: &[u8] = include_bytes!("../../data/postgame-coin-case.bin");

/// Buy coins at the counter, ¥1000 for 50, one conversation each.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_buy_game_coins() {
    const TARGET: u16 = 200;
    let mut fixture = TestFixture::new(COIN_CASE, Duration::from_mins(20), PolicyStep::buy_coins_steps(TARGET));

    let before = fixture.game_state();
    assert_eq!(before.coins, 0, "entry fixture should have no coins");
    let money_before = before.money;

    let state = fixture.run_leg(|s| s.coins >= TARGET);
    assert_eq!(state.coins, TARGET, "50 coins per purchase, so four purchases land exactly on 200");
    assert_eq!(money_before - state.money, 4 * 1000, "four purchases at ¥1000 each");
    assert_eq!(state.map.map, Map::CeladonCity, "the leg ends outdoors so the next Fly is allowed");
    println!("coins: {} · money: ¥{} → ¥{}", state.coins, money_before, state.money);

    fixture.save_state_named("src/pokemon/data/postgame-coins.bin").unwrap();
}

/// `can_buy_game_coins`'s output: Celadon City with 200 coins.
const COINS: &[u8] = include_bytes!("../../data/postgame-coins.bin");

/// Sell to a mart.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sell_junk_to_a_mart() {
    let mut fixture = TestFixture::new(COINS, Duration::from_mins(30), PolicyStep::sell_junk_tms_steps());

    let money_before = fixture.game_state().money;
    let state = fixture.run_leg(|s| s.money >= money_before + 6_000);

    assert_eq!(state.money - money_before, 6_000, "half of ¥5000 + ¥5000 + ¥2000");
    assert_eq!(state.map.map, Map::ViridianCity, "the leg ends outdoors so the next Fly is allowed");
    println!("money: ¥{money_before} → ¥{}", state.money);

    fixture.save_state_named("src/pokemon/data/postgame-sold.bin").unwrap();
}

/// `can_sell_junk_to_a_mart`'s output: Viridian City, 200 coins, three junk TMs sold.
const SOLD: &[u8] = include_bytes!("../../data/postgame-sold.bin");

/// Redeem an Abra from the first prize vendor for 180 coins.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_redeem_a_prize_pokemon() {
    use crate::pokemon::postgame::game_corner::Prize;

    let mut fixture = TestFixture::new(SOLD, Duration::from_mins(30),
        PolicyStep::redeem_prize_steps(Prize::Abra));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Abra), "entry fixture already owns an Abra");
    let party_before = before.pokemon.len();
    let coins_before = before.coins;

    let state = fixture.run_leg(|s| s.pokemon.len() > party_before);

    assert_eq!(state.pokemon[party_before].species, PokemonSpecies::Abra);
    assert_eq!(state.pokemon[party_before].level, 9, "Red's prize Abra is lv9 (`PrizeMonLevelDictionary`)");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Abra));
    assert_eq!(coins_before - state.coins, Prize::Abra.cost(), "180 coins, taken after the mon is handed over");
    assert_eq!(state.map.map, Map::CeladonCity, "the leg ends outdoors so the next Fly is allowed");
    println!("party is now {:?} · {} coins left",
        state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>(), state.coins);

    fixture.save_state_named("src/pokemon/data/postgame-game-corner.bin").unwrap();
}

/// `can_redeem_a_prize_pokemon`'s output: Celadon City, an Abra in the party, 20 coins.
const GAME_CORNER: &[u8] = include_bytes!("../../data/postgame-game-corner.bin");

fn seed_money(fixture: &mut TestFixture, amount: u32) {
    fixture.api().debug_set_money(amount);
}

/// A prize TM, a different code path from a prize mon.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_redeem_a_prize_tm() {
    use crate::pokemon::postgame::game_corner::Prize;

    let mut fixture = TestFixture::new(GAME_CORNER, Duration::from_mins(60),
        PolicyStep::redeem_prize_steps(Prize::DragonRage));
    seed_money(&mut fixture, 100_000);

    let before = fixture.game_state();
    assert!(!before.bag.iter().any(|i| i.id == ItemId::Tm23DragonRage), "entry fixture already has TM23");
    let party_before = before.pokemon.len();

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Tm23DragonRage));

    assert_eq!(state.pokemon.len(), party_before, "a TM prize must not touch the party");
    assert_eq!(state.coins, 20, "66 purchases take 20 coins to 3320, and TM23 costs 3300");
    assert_eq!(100_000 - state.money, 66 * 1_000, "66 trips through the counter at ¥1000 each");
    println!("TM23 Dragon Rage in the bag · {} coins · ¥{}", state.coins, state.money);
}
