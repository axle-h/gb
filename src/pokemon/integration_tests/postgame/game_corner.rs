//! Tests for workstream `game_corner` — see `docs/postgame-coverage-plan.md` §6-F and
//! [`crate::pokemon::postgame::game_corner`].
//!
//! The chain is `postgame-fly-bike.bin` → `-coin-case` → `-coins` → `postgame-game-corner.bin`. It is
//! rooted on **B's** output rather than `postgame-phase0.bin` because the Diner, the Game Corner and
//! the prize room are all in Celadon and Fly turns that trip into one step — the same reasoning C's
//! row gives for the three fishing rods.

#[allow(unused_imports)]
use super::super::*;

/// Workstream B's output (§9): Fuchsia City, Fly on Articuno, the Bicycle in the bag (16/20), party
/// Venusaur / Articuno / Vaporeon / Slowpoke, ¥41,209, dex 7 owned / 112 seen.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// **Task F1** — the Coin Case, from the gym guide in the Celadon Diner.
///
/// Every coin operation in the game is gated on holding it: the counter clerk, the NPC who hands out
/// ten free coins, and the prize vendors all begin with `ld b, COIN_CASE / call IsItemInBag` and bail
/// out with "you don't have a COIN CASE" otherwise. So this is F's Phase 0.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_coin_case() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(20), PolicyStep::coin_case_steps());

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::CoinCase),
        "entry fixture already has the Coin Case");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::CoinCase));
    // Outdoors, not in the Diner: the leg's last step walks back out so the next leg's `Fly` is not
    // refused for being indoors. See `postgame::fishing::rod_pickup`.
    assert_eq!(state.map.map, Map::CeladonCity);
    println!("Coin Case in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-coin-case.bin").unwrap();
}

/// F1's output: Celadon City, outside the Diner, with the Coin Case in the bag (17/20).
const COIN_CASE: &[u8] = include_bytes!("../../data/postgame-coin-case.bin");

/// **Task F2** — buy coins at the counter: ¥1000 → 50, one conversation each.
///
/// 200 coins is four purchases, which is what makes this a *test* rather than a demonstration: the
/// step re-polls after every conversation and stops on the coin count, so a driver that only landed
/// one purchase (or that answered the clerk's YES/NO with NO) would end on 50 or 0 and the money
/// assertion would catch it either way.
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

/// F2's output: Celadon City with the Coin Case and 200 coins, ¥37,209.
const COINS: &[u8] = include_bytes!("../../data/postgame-coins.bin");

/// **Task F3** — sell to a mart. The other half of the shop the agent has never opened.
///
/// Three junk TMs out of PC storage and over the Viridian Mart counter. Selling is a different menu
/// chain from buying, not a mirror of it: the list is the **bag** rather than the shop's stock, the
/// prices are halved, and a completed sale drops back to the bag list instead of the Buy/Sell/Quit
/// menu, so nothing about "done" is visible on screen. Money is the assertion, at exactly half list
/// price: Mega Drain ¥5000, Fissure ¥5000, Bide ¥2000 → ¥6,000.
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

/// F3's output: Viridian City, ¥43,209, 200 coins, three junk TMs sold.
const SOLD: &[u8] = include_bytes!("../../data/postgame-sold.bin");

/// **Task F4** — redeem a prize: an **Abra** from the first vendor, 180 coins.
///
/// Abra rather than something rarer for two reasons beyond the price. It is dex-new here, so the
/// prize genuinely lands rather than silently overwriting nothing; and it is the give-species for
/// the Abra → Mr. Mime trade in `Route2TradeHouse`, one of the five species obtainable *only* by
/// trading, so workstream G gains a row it could not otherwise fill.
///
/// ⚠️ A prize mon **is** offered a nickname, and the naming screen is the load-bearing part of this
/// leg. `_GivePokemon` → `AddPartyMon` names the mon whenever `wMonDataLocation` is 0, which nothing
/// on this path sets otherwise (`engine/pokemon/add_mon.asm:43-52`) — so the run goes through the
/// same screen a catch does, and the agent's generic naming handler answers it. With a **full**
/// party the prize goes to the box via `SendNewMonToBox` instead, which skips the naming entirely;
/// see the §11 entry, because that is the opposite of the boxed-*catch* wedge D recorded.
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

/// F4's output: Celadon City, an Abra in the party, 20 coins, ¥43,209.
const GAME_CORNER: &[u8] = include_bytes!("../../data/postgame-game-corner.bin");

/// Seed money from the **debug tier** (§3 of the plan) so a test can reach the expensive half of the
/// prize room.
///
/// Same technique, and the same reasoning, as `legendaries::seed_master_ball`: it separates two
/// independent questions that were hiding each other. *Can the agent drive a TM prize?* is a menu
/// question with a real answer. *Can it earn ¥66,000?* is an economy question whose only in-scope
/// answer is grinding the Elite Four, several minutes of emulated time a lap, and it tells you
/// nothing about the mechanism. `debug_set_money` lives in the **test tree**, which the guard
/// `play_path_contains_no_debug_ram_writes` deliberately does not scan (it reads `policy.rs`,
/// `agent.rs` and `postgame/*.rs`), so nothing a `Policy` can reach knows about it.
fn seed_money(fixture: &mut TestFixture, amount: u32) {
    fixture.api().debug_set_money(amount);
}

/// **Task F4, second branch** — a prize **TM**, which is a different code path from a prize mon.
///
/// `HandlePrizeChoice` forks on `wWhichPrizeWindow == 2`: the TM vendor calls `GetItemName` +
/// `GiveItem`, the mon vendors call `GetMonName` + `GivePokemon`. So this exercises the third vendor
/// tile, the item-name menu, the bag-full refusal path, and — the part that actually differs for the
/// driver — a purchase where **no party count moves**, which is why [`prize_tick`] reads completion
/// from the coins rather than from the party.
///
/// ⚠️ **Debug-money-seeded** ([`seed_money`]). 3300 coins is 66 trips through the counter clerk at
/// ¥1000 each, and the entry fixture holds ¥43,209. What is seeded is the *money*; the 66 purchases
/// and the redemption are driven normally, so nothing about the mechanism is short-circuited. The
/// fixture is deliberately **not** committed — F's chain ends at `postgame-game-corner.bin`, which is
/// honestly earned.
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
