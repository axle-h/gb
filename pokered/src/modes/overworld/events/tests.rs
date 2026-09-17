//! The events played through the game's own loop.

use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::{EVENT_1ST_LOCK_OPENED, EVENT_BEAT_VERMILION_GYM_TRAINER_0,
    EVENT_BEAT_VERMILION_GYM_TRAINER_2, EVENT_FOLLOWED_OAK_INTO_LAB};
use crate::command::{Command, Decision, Reply};
use crate::input::Joypad;
use crate::mode::{Mode, Status};
use crate::modes::overworld::Overworld;
use crate::party::{Named, PartyMon};
use crate::rng::GameRng;
use crate::systems::add_mon::{new_party_mon, Origin};
use crate::systems::overworld::{Direction, Location};
use crate::world::World;
use crate::{Game, Input, Pacing};

fn mon(species: PokemonSpecies, level: u8) -> Named<PartyMon> {
    Named {
        mon: new_party_mon(species, level, 1, &Origin::Trainer, &mut GameRng::tape(vec![])),
        ot: encode("RED").unwrap(),
        nick: encode("MON").unwrap(),
    }
}

fn game(map: Map, x: u8, y: u8, facing: SpriteFacing, setup: impl FnOnce(&mut World)) -> Game {
    let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
    world.location = Location { map, x, y, facing, last_map: Map::PalletTown, ..Location::default() };
    world.party = vec![mon(PokemonSpecies::Pidgey, 40)];
    world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB);
    world.money = [0x00, 0x10, 0x00];
    setup(&mut world);
    let mut game = Game::new(world, GameRng::seeded(9), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::new()));
    play_until(&mut game, 600, &mut |_| 0, free);
    game
}

fn free(game: &Game) -> bool {
    game.status() == Status::Waiting(Decision::Overworld)
}

/// Answers texts with A, and every menu of options with what `choose` says, until `done`.
fn play_until(game: &mut Game, limit: u32, choose: &mut dyn FnMut(&Game) -> u8, done: impl Fn(&Game) -> bool) {
    for _ in 0..limit {
        if done(game) {
            return;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::TwoOption | Decision::CursorMenu | Decision::PartyMenu) => Input::Command(Command::ChooseOption(choose(game))),
            _ => Input::None,
        };
        game.frame(input);
    }
    panic!("never done: {:?} on {:?}", game.status(), game.modes().last().map(std::mem::discriminant));
}

/// A held until something takes it.
fn press_a(game: &mut Game) {
    for _ in 0..4 {
        game.frame(Input::Buttons(Joypad::A));
    }
}

#[test]
fn the_nurse_heals_the_party_and_remembers_the_town() {
    let mut game = game(Map::ViridianPokecenter, 3, 3, SpriteFacing::Up, |world| {
        world.party[0].mon.mon.hp = 1;
        world.party[0].mon.mon.status = 8;
        world.location.last_map = Map::ViridianCity;
    });
    let reply = game.frame(Input::Command(Command::Interact)).reply;
    assert_eq!(reply, Some(Reply::Accepted));
    play_until(&mut game, 3000, &mut |_| 0, |game| free(game) && game.world().used_pokecenter);
    let mon = &game.world().party[0].mon;
    assert_eq!((mon.mon.hp, mon.mon.status), (mon.stats[0], 0));
    assert_eq!(game.world().location.last_blackout_map, Map::ViridianCity);
}

#[test]
fn a_hidden_item_is_found_once() {
    let mut game = game(Map::ViridianForest, 16, 43, SpriteFacing::Up, |_| {});
    press_a(&mut game);
    play_until(&mut game, 1000, &mut |_| 0, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Antidote), 1);
    for _ in 0..40 {
        game.frame(Input::None);
    }
    press_a(&mut game);
    play_until(&mut game, 1000, &mut |_| 0, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Antidote), 1);
}

#[test]
fn poison_takes_a_point_every_fourth_step_and_a_party_that_faints_blacks_out() {
    let mut game = game(Map::PalletTown, 5, 8, SpriteFacing::Right, |world| {
        world.party[0].mon.mon.hp = 2;
        world.party[0].mon.mon.status = 8;
        world.location.last_blackout_map = Map::PalletTown;
    });
    for step in 0..16 {
        let direction = if step % 2 == 0 { Direction::Right } else { Direction::Left };
        let reply = game.frame(Input::Command(Command::Step(direction))).reply;
        assert_eq!(reply, Some(Reply::Accepted), "step {step} at {:?}", game.world().location);
        play_until(&mut game, 3000, &mut |_| 0, |game| free(game));
        if game.world().party[0].mon.mon.hp == 0 || game.world().money[1] == 0x05 {
            break;
        }
    }
    play_until(&mut game, 3000, &mut |_| 0, free);
    let world = game.world();
    assert_eq!(world.money, [0x00, 0x05, 0x00], "halved");
    assert_eq!(world.party[0].mon.mon.hp, world.party[0].mon.stats[0], "healed");
}

#[test]
fn a_vending_machine_sells_a_fresh_water() {
    let mut game = game(Map::CeladonMartRoof, 10, 2, SpriteFacing::Up, |_| {});
    let reply = game.frame(Input::Command(Command::Interact)).reply;
    assert_eq!(reply, Some(Reply::Accepted));
    play_until(&mut game, 3000, &mut |_| 0, |game| free(game) && game.world().bag.quantity_of(ItemId::FreshWater) == 1);
    assert_eq!(game.world().money, [0x00, 0x08, 0x00]);
}

#[test]
fn the_underground_path_girl_trades_a_nidoran_female_for_a_male() {
    let mut game = game(Map::UndergroundPathRoute5, 2, 4, SpriteFacing::Up, |world| {
        world.party.push(mon(PokemonSpecies::NidoranMale, 7));
    });
    let reply = game.frame(Input::Command(Command::Interact)).reply;
    assert_eq!(reply, Some(Reply::Accepted));
    play_until(&mut game, 5000, &mut |game| if game.status() == Status::Waiting(Decision::PartyMenu) { 1 } else { 0 },
        |game| free(game) && game.world().in_game_trades != 0);
    let party = &game.world().party;
    assert_eq!(party.len(), 2);
    assert_eq!((party[1].mon.mon.species, party[1].mon.level), (PokemonSpecies::NidoranFemale, 7));
    assert_eq!((party[1].nick.clone(), party[1].ot.clone()), (encode("SPOT").unwrap(), vec![0x5D]));
    assert!(game.world().pokedex.is_owned(PokemonSpecies::NidoranFemale));
}

#[test]
fn the_day_care_takes_a_mon_and_gives_it_back_grown_for_its_price() {
    let mut game = game(Map::Daycare, 2, 4, SpriteFacing::Up, |world| {
        world.party.push(mon(PokemonSpecies::Rattata, 5));
    });
    game.frame(Input::Command(Command::Interact));
    play_until(&mut game, 5000, &mut |game| if game.status() == Status::Waiting(Decision::PartyMenu) { 1 } else { 0 },
        |game| free(game) && game.world().day_care.is_some());
    assert_eq!(game.world().party.len(), 1);
    let mut world = game.world().clone();
    let growth = world.day_care.as_ref().unwrap().mon.base_stats().growth_rate;
    world.day_care.as_mut().unwrap().mon.exp = crate::systems::experience::calc_experience(growth, 8);
    let mut game = Game::new(world, GameRng::seeded(1), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::new()));
    play_until(&mut game, 600, &mut |_| 0, free);
    game.frame(Input::Command(Command::Interact));
    play_until(&mut game, 5000, &mut |_| 0, |game| free(game) && game.world().day_care.is_none());
    let party = &game.world().party;
    assert_eq!((party[1].mon.mon.species, party[1].mon.level), (PokemonSpecies::Rattata, 8));
    assert_eq!(game.world().money, [0x00, 0x06, 0x00], "¥400 for three levels");
}

/// A machine and the aisle square beside it, which is where `AbleToPlaySlotsCheck` wants the player.
const SLOT_MACHINE: (u8, u8, SpriteFacing) = (17, 12, SpriteFacing::Right);

fn game_corner(coins: [u8; 2], coin_case: bool) -> Game {
    let (x, y, facing) = SLOT_MACHINE;
    game(Map::GameCorner, x, y, facing, |world| {
        world.coins = coins;
        if coin_case {
            world.bag.add(ItemId::CoinCase, 1);
        }
    })
}

fn playing_slots(game: &Game) -> bool {
    game.modes().iter().any(|mode| matches!(mode, Mode::SlotMachine(_)))
}

#[test]
fn a_slot_machine_opens_for_a_player_stood_beside_it_and_gives_the_overworld_back() {
    let mut game = game_corner([0x10, 0x00], true);
    press_a(&mut game);
    play_until(&mut game, 2000, &mut |_| 0, playing_slots);
    // The machine itself is `Busy`: a press of A takes the bet and stops each wheel in turn. The
    // yes/no that opened it is answered; the only one left is another go, refused.
    for frame in 0..20000 {
        if free(&game) && !playing_slots(&game) {
            break;
        }
        let input = match game.status() {
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(1)),
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(_) => Input::Command(Command::ChooseOption(0)),
            _ if frame % 2 == 0 => Input::Buttons(Joypad::A),
            _ => Input::None,
        };
        game.frame(input);
    }
    assert!(free(&game) && !playing_slots(&game), "the machine never let go: {:?}", game.status());
    assert_ne!(game.world().coins, [0x10, 0x00], "a bet is paid for");
}

#[test]
fn a_slot_machine_wants_a_coin_case_coins_and_the_player_beside_it() {
    let mut facing_it = game(Map::GameCorner, 18, 16, SpriteFacing::Up, |world| {
        world.coins = [0x10, 0x00];
        world.bag.add(ItemId::CoinCase, 1);
    });
    press_a(&mut facing_it);
    for _ in 0..60 {
        facing_it.frame(Input::None);
    }
    assert!(free(&facing_it), "a machine is played from beside it");

    for (coins, coin_case) in [([0x10, 0x00], false), ([0x00, 0x00], true)] {
        let mut refused = game_corner(coins, coin_case);
        press_a(&mut refused);
        play_until(&mut refused, 600, &mut |_| 0, |game| game.status() == Status::Waiting(Decision::Text));
        assert!(!playing_slots(&refused));
    }
}

/// The cans are inside the gym's own sight lines, so its trainers have to be done with first.
fn beaten_gym_trainers(world: &mut World) {
    for event in EVENT_BEAT_VERMILION_GYM_TRAINER_0..=EVENT_BEAT_VERMILION_GYM_TRAINER_2 {
        world.events.set(event);
    }
}

#[test]
fn the_vermilion_gym_cans_hold_two_locks_and_a_wrong_can_shuts_the_first_again() {
    // `GymTrashCans` counts the cans along its rows, so can 0 is the one at (1, 7).
    let mut first = game(Map::VermilionGym, 1, 8, SpriteFacing::Up, |world| {
        beaten_gym_trainers(world);
        world.scripts.trash_cans = [0, 0];
    });
    press_a(&mut first);
    play_until(&mut first, 2000, &mut |_| 0, |game| free(game) && game.world().events.is_set(EVENT_1ST_LOCK_OPENED));
    let second = first.world().scripts.trash_cans[1];
    assert_ne!(second, 0, "the second lock is in another can");

    let wrong = if second == 6 { 0 } else { 6 };
    let (x, y) = ((wrong / 3) * 2 + 1, (wrong % 3) * 2 + 7);
    let mut again = game(Map::VermilionGym, x, y + 1, SpriteFacing::Up, |world| {
        beaten_gym_trainers(world);
        world.scripts.trash_cans = [wrong, second];
        world.events.set(EVENT_1ST_LOCK_OPENED);
    });
    press_a(&mut again);
    play_until(&mut again, 2000, &mut |_| 0, |game| free(game) && !game.world().events.is_set(EVENT_1ST_LOCK_OPENED));
}

#[test]
fn the_prize_vendor_sells_a_tm_for_coins_and_refuses_when_they_are_short() {
    for (coins, bought) in [([0x40u8, 0x00u8], true), ([0x10, 0x00], false)] {
        let mut vendor = game(Map::GameCornerPrizeRoom, 6, 3, SpriteFacing::Up, |world| {
            world.coins = coins;
            world.bag.add(ItemId::CoinCase, 1);
        });
        vendor.frame(Input::Command(Command::Interact));
        play_until(&mut vendor, 4000, &mut |_| 0, free);
        let world = vendor.world();
        assert_eq!(world.bag.quantity_of(ItemId::Tm23DragonRage), u8::from(bought));
        assert_eq!(world.coins, if bought { [0x07, 0x00] } else { coins });
    }
}
