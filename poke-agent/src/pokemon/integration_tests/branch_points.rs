
use super::*;

use crate::pokemon::integration_tests::godmode::{Intent, ScriptedBrain};
use crate::pokemon::integration_tests::llm_harness::LlmRun;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::Pokemon;
use std::sync::{Arc, Mutex};

// ── The snapshots
// ────────────────────────────────────────────────────────────────────────────────

/// Oak's lab with his speech over and the three Poké Balls still on the table: the starter
/// branch, cut one decision before it.
const OAKS_LAB: &[u8] = include_bytes!("../data/branch-oaks-lab.bin");

/// Mt Moon B2F standing at the mouth of the fossil chamber, both fossils still on the floor and
/// the Super Nerd still guarding them.
const MT_MOON: &[u8] = include_bytes!("../data/branch-mt-moon-fossils.bin");

/// The Fighting Dojo with the Karate Master beaten, both prize balls still there and a party slot
/// free so either lands in the party rather than the box.
const DOJO: &[u8] = include_bytes!("../data/branch-dojo-prize.bin");

/// Inside the Cerulean Bike Shop holding the Bike Voucher, one decision from the clerk.
const BIKE_SHOP: &[u8] = include_bytes!("../data/branch-bike-shop.bin");

/// How long an arm will wait on the wall clock before giving up and asserting.
const PATIENCE: Duration = Duration::from_secs(180);

// ── Driving an arm
// ───────────────────────────────────────────────────────────────────────────────

/// Start `fixture` with a brain that carries out `intents` and nothing else.
fn arm(fixture: &'static [u8], name: &'static str, intents: Vec<Intent>) -> (LlmRun, Stuck) {
    let brain = ScriptedBrain::new(intents);
    let stuck = Arc::clone(&brain.stuck);
    let run = LlmRun::builder(fixture)
        .named(name)
        // Six game-hours for a decision that takes seconds, and the margin is not slack.
        .game_time(Duration::from_secs(6 * 3600))
        // The watchdog is armed, and every arm asserts it never fired.
        .stuck_timeout(Duration::from_secs(60))
        .start(Box::new(brain));
    (run, stuck)
}

type Stuck = Arc<Mutex<Option<String>>>;

/// Whether any turn this run put to the model contained `needle`.
fn told(run: &LlmRun, needle: &str) -> bool {
    run.endpoint.requests().iter().any(|request| request.situation().contains(needle))
}

/// Whether the watchdog ever had to nudge this run.
fn was_stuck(run: &LlmRun) -> bool {
    run.endpoint.requests().iter().any(|request| request.is_stuck())
}

/// `stuck`'s message, or a note that the arm simply ran out of game time with the list
/// unfinished.
fn why(stuck: &Stuck) -> String {
    stuck.lock().expect("not poisoned").clone()
        .unwrap_or_else(|| "the intent list resolved but the game never got there".to_string())
}

/// Every row the model was offered on the first overworld turn of a run, as ids.
fn first_menu(run: &LlmRun) -> Vec<String> {
    run.endpoint
        .requests()
        .into_iter()
        .find(|request| request.has_tool("choose_action"))
        .map(|request| request.menu_ids())
        .unwrap_or_default()
}

/// Every row the model was offered on the last overworld turn of a run.
fn last_menu(run: &LlmRun) -> Vec<String> {
    run.endpoint
        .requests()
        .into_iter()
        .filter(|request| request.has_tool("choose_action"))
        .next_back()
        .map(|request| request.menu_ids())
        .unwrap_or_default()
}

// ── 1.

/// The lab's table, as the branch: the row, what taking it puts in the party, and what the rival
/// then takes.
const STARTERS: [(&str, PokemonSpecies, PokemonSpecies); 3] = [
    ("BulbasaurPokeBall", PokemonSpecies::Bulbasaur, PokemonSpecies::Charmander),
    ("CharmanderPokeBall", PokemonSpecies::Charmander, PokemonSpecies::Squirtle),
    ("SquirtlePokeBall", PokemonSpecies::Squirtle, PokemonSpecies::Bulbasaur),
];

/// Wait for `species` to be registered in the Pokédex, and say so if it never is.
fn assert_owned(run: &mut LlmRun, species: PokemonSpecies) {
    let owned = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.pokedex_owned.contains(&species))
    });
    assert!(owned, "{species:?} never reached the Pokédex");
}

/// One arm of the starter branch: take `ball`, and check the game agreed about both halves.
fn starter_arm(name: &'static str, ball: &'static str) {
    let (_, mine, rivals) = STARTERS.iter().copied().find(|(row, _, _)| *row == ball)
        .expect("a row in the table above");
    // The rival's battle is a *coordinate* trigger on the lab door, and neither waiting for it
    // nor talking to him reaches it.
    let (mut run, stuck) = arm(OAKS_LAB, name,
        vec![Intent::Row(ball), Intent::Enter("PalletTown")]);

    // `try_game_state`, not `game_state`.
    let chosen = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.pokemon.len() == 1)
    });

    // All three at once, before anything is asserted about the one that was taken.
    let menu = first_menu(&run);
    for (row, _, _) in STARTERS {
        assert!(
            menu.iter().any(|id| id.ends_with(row)),
            "the lab offered no `{row}` row, so a model could not have chosen it.\n  menu: {menu:?}",
        );
    }
    assert!(chosen, "nothing reached the party; the `{ball}` row was never carried out.\n  {}", why(&stuck));

    let state = run.fixture().game_state();
    assert_eq!(state.pokemon[0].species, mine,
        "the `{ball}` row put a {:?} in the party", state.pokemon[0].species);
    assert_owned(&mut run, mine);

    // And the other half of the branch, which is the half that lasts.
    let fought = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.battle.is_some())
    });
    let state = run.fixture().game_state();
    let battle = state.battle.as_ref();
    assert!(fought, "the rival never challenged, so the branch's other half never ran");
    assert_eq!(battle.map(|b| b.enemy.species), Some(rivals),
        "the rival took the wrong starter for this arm");
    println!("[{name}] took {mine:?}, the rival took {rivals:?}");
}

/// The Bulbasaur arm, which no test in this suite had ever taken.
#[test]
fn the_starter_branch_can_be_taken_to_bulbasaur() {
    starter_arm("branch-starter-bulbasaur", "BulbasaurPokeBall");
}

/// The Charmander arm.
#[test]
fn the_starter_branch_can_be_taken_to_charmander() {
    starter_arm("branch-starter-charmander", "CharmanderPokeBall");
}

/// The Squirtle arm — the one the scripted route takes, and the reason the other two are worth
/// having: everything downstream of `at-cerulean.bin` is a Squirtle save, so this arm is the only
/// one the fixture chain can speak for.
#[test]
fn the_starter_branch_can_be_taken_to_squirtle() {
    starter_arm("branch-starter-squirtle", "SquirtlePokeBall");
}

// ── 2.

/// Mt Moon B2F's two fossils: the row, and what taking it puts in the bag.
const FOSSILS: [(&str, ItemId, &str); 2] = [
    ("DomeFossil", ItemId::DomeFossil, "HelixFossil"),
    ("HelixFossil", ItemId::HelixFossil, "DomeFossil"),
];

/// One arm of the fossil branch: take `row`, and check the other one has gone with it.
fn fossil_arm(name: &'static str, row: &'static str) {
    let (_, item, other) = FOSSILS.iter().copied().find(|(id, _, _)| *id == row)
        .expect("a row in the table above");
    let (_, other_item, _) = FOSSILS.iter().copied().find(|(id, _, _)| *id == other)
        .expect("the other row in the table above");
    // The row is asked for twice, because the Super Nerd interrupts the first attempt.
    let (mut run, stuck) = arm(MT_MOON, name, vec![Intent::Row(row), Intent::Row(row)]);

    let taken = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.bag.iter().any(|i| i.id == item))
    });

    let menu = first_menu(&run);
    for (id, _, _) in FOSSILS {
        assert!(menu.iter().any(|row| row.ends_with(id)),
            "B2F offered no `{id}` row, so a model could not have chosen it.\n  menu: {menu:?}");
    }
    assert!(taken, "the `{row}` row never put a {item:?} in the bag.\n  {}", why(&stuck));

    // The other fossil has to be gone from the menu, and it is the whole branch. The cartridge
    // hides it the moment this one is taken, so a menu that still offered it would be promising a
    // second fossil this save can never have.
    let cleared = run.tick_until(PATIENCE, |run| !last_menu(run).iter().any(|id| id.ends_with(other)));
    let menu = last_menu(&run);
    assert!(cleared, "`{other}` is still a row after `{row}` was taken.\n  menu: {menu:?}");
    assert!(!run.fixture().game_state().bag.iter().any(|entry| entry.id == other_item),
        "both fossils are in the bag, which no save out of Mt Moon can hold");
    println!("[{name}] took {item:?}; {other} is gone");
}

/// The Dome Fossil arm — Kabuto's half of the branch, and the half no save in this repo has.
#[test]
fn the_fossil_branch_can_be_taken_to_the_dome_fossil() {
    fossil_arm("branch-fossil-dome", "DomeFossil");
}

/// The Helix Fossil arm, which is the one the scripted route takes and every committed fixture
/// downstream of Mt Moon carries.
#[test]
fn the_fossil_branch_can_be_taken_to_the_helix_fossil() {
    fossil_arm("branch-fossil-helix", "HelixFossil");
}

// ── 3.

/// The Fighting Dojo's two prize balls: the row, what taking it puts in the party, and the other
/// row — which, unlike Mt Moon's, is *still there* afterwards and answers a refusal.
const DOJO_PRIZE: [(&str, PokemonSpecies, &str); 2] = [
    ("HitmonleePokeBall", PokemonSpecies::Hitmonlee, "HitmonchanPokeBall"),
    ("HitmonchanPokeBall", PokemonSpecies::Hitmonchan, "HitmonleePokeBall"),
];

/// What the cartridge says to a player who comes back for the second ball.
const GREEDY: &str = "Better not get";

/// One arm of the dojo branch: take `row`, then try the other one and be told no.
fn dojo_arm(name: &'static str, row: &'static str) {
    let (_, species, other) = DOJO_PRIZE.iter().copied().find(|(id, _, _)| *id == row)
        .expect("a row in the table above");
    let (_, other_species, _) = DOJO_PRIZE.iter().copied().find(|(id, _, _)| *id == other)
        .expect("the other row in the table above");
    let (mut run, stuck) = arm(DOJO, name, vec![Intent::Row(row), Intent::Row(other)]);

    let got = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == species))
    });

    let menu = first_menu(&run);
    for (id, _, _) in DOJO_PRIZE {
        assert!(menu.iter().any(|row| row.ends_with(id)),
            "the dojo offered no `{id}` row, so a model could not have chosen it.\n  menu: {menu:?}");
    }
    assert!(got, "the `{row}` row never put a {species:?} in the party.\n  {}", why(&stuck));

    // And now the other ball, which is where this branch is different from the other three.
    let refused = run.tick_until(PATIENCE, |run| told(run, GREEDY));

    let state = run.fixture().game_state();
    assert!(!state.pokemon.iter().any(|mon| mon.species == other_species),
        "both dojo prizes are in the party, which the cartridge allows no save to hold");
    assert!(!state.pokedex_owned.contains(&other_species),
        "{other_species:?} is registered as owned; only one of the two can ever be");
    assert!(refused,
        "the model was never told why the second ball gave it nothing. `FightingDojoBetterNotGet\
         GreedyText` is the cartridge's own sentence for it and it never reached a turn.\n  {}",
        why(&stuck));

    assert_eq!(state.pokemon.len(), 6, "the snapshot banks a slot so the gift lands in the party");
    assert!(state.pokedex_owned.contains(&species));
    assert!(!was_stuck(&run), "the watchdog fired on the dojo branch");
    println!("[{name}] took {species:?}; the {other} answered \"Better not get greedy...\"");
}

/// The Hitmonlee arm, which `postgame::gifts::can_beat_the_karate_master_and_take_a_hitmonlee`
/// takes as part of the postgame chain — repeated here because a branch is only a branch with
/// both arms beside it, and because that test never went back for the second ball.
#[test]
fn the_dojo_branch_can_be_taken_to_hitmonlee() {
    dojo_arm("branch-dojo-hitmonlee", "HitmonleePokeBall");
}

/// The Hitmonchan arm.
#[test]
fn the_dojo_branch_can_be_taken_to_hitmonchan() {
    dojo_arm("branch-dojo-hitmonchan", "HitmonchanPokeBall");
}

// ── 4.

/// The voucher arm — the Cerulean clerk hands the Bicycle over as a gift.
#[test]
fn the_bike_branch_can_be_taken_with_the_voucher() {
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-bike-voucher", vec![Intent::Row("Clerk")]);

    let got = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.bag.iter().any(|entry| entry.id == ItemId::Bicycle))
    });
    assert!(got, "the clerk never handed the Bicycle over.\n  {}", why(&stuck));

    let state = run.fixture().game_state();
    assert!(!state.bag.iter().any(|entry| entry.id == ItemId::BikeVoucher),
        "the voucher is what pays for it and should have gone");
    // And no menu at all.
    assert!(!told(&run, "1000000"), "the voucher arm was quoted a price; it is a gift, not a sale");
    assert!(!was_stuck(&run), "the watchdog fired on the voucher arm");
    println!("[branch-bike-voucher] Bicycle received, voucher spent, no shop opened");
}

/// The no-voucher arm — the same clerk, the same row, and a shop at ¥1,000,000.
#[test]
fn the_bike_branch_can_be_taken_without_the_voucher() {
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-bike-shop", vec![Intent::Row("Clerk")]);
    run.fixture().api().debug_take_item(ItemId::BikeVoucher).expect("the snapshot carries one");
    let money = run.fixture().game_state().money;

    // The cartridge's own sentence for a purchase it will not make, quoted into a turn.
    let refused = run.tick_until(PATIENCE, |run| told(run, "Sorry! You can"));
    assert!(refused,
        "the model was never told the shop refused. `BikeShopCantAffordText` is the cartridge's own \
         sentence for it and it never reached a turn.\n  {}", why(&stuck));
    assert!(told(&run, "1000000"),
        "the price never reached a turn, so a model reading this cannot tell why ¥{money} was not \
         enough");

    let state = run.fixture().game_state();
    assert!(!state.bag.iter().any(|entry| entry.id == ItemId::Bicycle),
        "¥{money} bought a ¥1,000,000 bicycle");
    assert_eq!(state.money, money, "nothing should have been spent");
    // And the run is still playing.
    assert!(!was_stuck(&run), "the watchdog fired: the no-voucher shop wedged the agent");
    assert!(!run.endpoint.requests().iter().any(|request| request.has_tool("buy_item")),
        "this arm is a text box rather than a mart; a `buy_item` turn here means the agent's mart \
         driver has started claiming it, and the assertions above are then about the wrong thing");
    println!("[branch-bike-shop] the shop asked ¥1,000,000 of ¥{money} and refused, in one text box");
}

// ── 5.

/// Put `species` in party slot `at`, keeping whatever is already there around it.
fn party_with(run: &mut LlmRun, species: PokemonSpecies, moves: [PokemonMoveName; 4], at: usize) {
    let state = run.fixture().game_state();
    let mut members: Vec<Pokemon> = state.pokemon.iter().cloned().collect();
    let seeded = Pokemon::maxed(species, "SWAPME", moves, state.name.clone(), state.player_id);
    members.insert(at.min(members.len()), seeded);
    members.truncate(6);
    let mut party = PokemonParty::default();
    for member in members {
        let _ = party.push(member);
    }
    run.fixture().api().debug_set_party(&party).expect("a party can be installed");
}

/// A Poliwhirl good enough to be a party member and irrelevant in every other way.
const SWAP_MOVES: [PokemonMoveName; 4] = [
    PokemonMoveName::WaterGun, PokemonMoveName::BodySlam,
    PokemonMoveName::Psychic, PokemonMoveName::Blizzard,
];

/// The three sentences a trader has for a Pokémon that is not the one it asked for —
/// `_WrongMon1Text`, `_WrongMon2Text`, `_WrongMon3Text`, one per text-pointer set.
const WRONG_MON: [&str; 3] = ["What? That's not", "Hmmm? This isn", "...This is no"];

/// An in-game trade through the deployed stack — Poliwhirl → Jynx at `CeruleanTradeHouse`.
#[test]
fn an_in_game_trade_can_be_reached_through_the_llm_path() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-trade", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
        Intent::Row("Gambler"),
    ]);
    party_with(&mut run, trade.give, SWAP_MOVES, 0);

    let traded = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == trade.get))
    });

    let state = run.fixture().game_state();
    println!("[branch-trade] party {:?}", state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());
    assert!(traded, "the gambler never handed a {:?} over.\n  {}", trade.get, why(&stuck));
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.give),
        "the {:?} was not handed over; a trade swaps rather than adds", trade.give);
    assert_owned(&mut run, trade.get);
    assert!(!was_stuck(&run), "the watchdog fired on the trade");
    println!("[branch-trade] traded {:?} -> {:?}", trade.give, trade.get);
}

/// The trade finds its own Pokémon, wherever in the party it is.
#[test]
fn a_trade_finds_the_give_species_wherever_it_is_in_the_party() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-trade-benched", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
    ]);
    // Three slots back, and that is the entire difference from the test above.
    party_with(&mut run, trade.give, SWAP_MOVES, 3);

    let traded = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == trade.get))
    });

    let state = run.fixture().game_state();
    assert!(traded,
        "a benched {:?} was not traded. The cursor opens where the last party menu left it, so \
         without a driver this menu hands over slot {} instead.\n  party: {:?}\n  {}",
        trade.give,
        run.endpoint.requests().len().min(1),
        state.pokemon.iter().map(|mon| mon.species).collect::<Vec<_>>(),
        why(&stuck));
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.give),
        "the {:?} is still in the party; something else was handed over", trade.give);
    assert_owned(&mut run, trade.get);
    assert!(!WRONG_MON.iter().any(|words| told(&run, words)),
        "the trader was handed the wrong Pokémon at some point in this run");
    assert!(!was_stuck(&run), "the watchdog fired on the trade");
    println!("[branch-trade-benched] a {:?} in slot 4 was found and traded for a {:?}",
        trade.give, trade.get);
}

/// A trade the party cannot make is backed out of, and the model is told why.
#[test]
fn a_trade_with_nothing_to_give_backs_out_and_says_so() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, _) = arm(BIKE_SHOP, "branch-trade-empty", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
        Intent::Row("Gambler"),
    ]);
    let before = run.fixture().game_state();
    assert!(!before.pokemon.iter().any(|mon| mon.species == trade.give),
        "this test is about a party with no {:?} in it", trade.give);

    // Read out of the *turn*, not out of the event stream.
    let spoke = run.tick_until(PATIENCE, |run| told(run, "there is none in the party"));

    let state = run.fixture().game_state();
    assert!(spoke, "the model was never told why the trade did not happen");
    assert!(!WRONG_MON.iter().any(|words| told(&run, words)),
        "the trader was offered a Pokémon it had not asked for; the menu should have been backed \
         out of instead");
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.get),
        "a {:?} arrived from a trade the party could not make", trade.get);
    assert!(!was_stuck(&run), "the watchdog fired: an unmakeable trade wedged the party menu");
    println!("[branch-trade-empty] the trade wanted a {:?}, the party had none, and it said so",
        trade.give);
}

// ── The snapshots, cut
// ───────────────────────────────────────────────────────────────────────────

/// Cut Oak's lab one A press before the starter branch.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts the Oak's Lab fixture; needs GB_REGEN_FIXTURES=1"]
fn regen_oaks_lab_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::RedsHouse1F),
            PolicyStep::enter(Map::PalletTown),
            // Oak stops you on the way to Route 1 and marches you into the lab.
            PolicyStep::soft_goto(Map::Route1),
        ],
    );
    fixture.step_until_exhausted();
    // Oak stops the player in the grass and marches them into the lab; the walk that triggers it
    // is still in flight when the queue empties, so the arrival is waited for rather than
    // assumed.
    fixture.run_until(|s| s.map.map == Map::OaksLab);
    // And then his speech, which the agent advances by itself.
    for _ in 0..3000 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::OaksLab, "the branch is in the lab");
    assert_eq!(s.pokemon.len(), 0, "the starter has not been chosen yet");
    fixture.save_state_named("src/pokemon/data/branch-oaks-lab.bin").unwrap();
}

/// Cut Mt Moon B2F with both fossils still on the floor.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts the Mt Moon fossils fixture; needs GB_REGEN_FIXTURES=1"]
fn regen_mt_moon_fossils_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_mins(40),
        vec![
            PolicyStep::enter_at(Map::MtMoonB1F, 5, 5),
            PolicyStep::enter_at(Map::MtMoonB2F, 21, 17),
        ]
        .into_iter()
        // These two walks are what make the snapshot restorable, and neither is the branch. The
        // Super Nerd is a *coordinate* trigger, so walking at him starts no battle — but it
        // carries the player off the B2F landing and up to the chamber, and that route crosses
        // `ROCKET1`'s line of sight.
        .chain(std::iter::repeat_n(
            PolicyStep::InteractIfReachable(MapSprite::MTMOONB2F_SUPER_NERD), 4))
        .chain(std::iter::repeat_n(
            PolicyStep::InteractIfReachable(MapSprite::MTMOONB2F_ROCKET1), 3))
        .collect(),
    );
    fixture.pimp_pokemon();
    fixture.step_until_exhausted();
    // Out of whatever fight the last interact left running, and then a second of quiet.
    fixture.run_until(|s| s.battle.is_none());
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::MtMoonB2F, "the branch is on B2F");
    assert!(s.battle.is_none(), "a snapshot cut mid-battle restores into one, and the branch is a menu");
    assert!(s.map.actions().iter().any(|a| format!("{:?}", a.tile).contains("Dome Fossil")),
        "the Dome Fossil has to still be a row");
    assert!(s.map.actions().iter().any(|a| format!("{:?}", a.tile).contains("Helix Fossil")),
        "the Helix Fossil has to still be a row");
    assert!(!s.bag.iter().any(|i| matches!(i.id, ItemId::HelixFossil | ItemId::DomeFossil)),
        "neither fossil has been taken yet");
    fixture.save_state_named("src/pokemon/data/branch-mt-moon-fossils.bin").unwrap();
}

/// Cut the Fighting Dojo with the Karate Master beaten and both prize balls still on the floor.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts the dojo-prize fixture; needs GB_REGEN_FIXTURES=1"]
fn regen_dojo_prize_fixture() {
    /// Omanyte, which `postgame-lapras.bin` carries in slot 4 — dex-registered, so banking it is
    /// free.
    const BANK_SLOT: u8 = 4;
    let mut steps = PolicyStep::hitmonlee_steps(BANK_SLOT);
    steps.truncate(steps.len() - 2);
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-lapras.bin"),
        Duration::from_mins(45),
        steps,
    );
    fixture.step_until_exhausted();
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::FightingDojo, "the branch is in the dojo");
    assert_eq!(s.pokemon.len(), 5, "a slot has to be free or both arms go to the box");
    assert!(!s.pokedex_owned.contains(&PokemonSpecies::Hitmonlee));
    assert!(!s.pokedex_owned.contains(&PokemonSpecies::Hitmonchan));
    fixture.save_state_named("src/pokemon/data/branch-dojo-prize.bin").unwrap();
}

/// Cut the Cerulean Bike Shop with the Bike Voucher in the bag, one A press from the clerk.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts the bike-shop fixture; needs GB_REGEN_FIXTURES=1"]
fn regen_bike_shop_fixture() {
    let mut steps = PolicyStep::bicycle_steps();
    steps.retain(|step| !matches!(step, PolicyStep::Interact(_)));
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-bike-voucher.bin"),
        Duration::from_mins(45),
        steps,
    );
    fixture.step_until_exhausted();
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — money {}", s.map.map, s.map.player_position, s.money);
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::BikeShop, "the branch is in the shop");
    assert!(s.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "the voucher is what one arm spends");
    assert!(!s.bag.iter().any(|i| i.id == ItemId::Bicycle), "the bike has not been collected yet");
    fixture.save_state_named("src/pokemon/data/branch-bike-shop.bin").unwrap();
}
