//! The PC played through the game: its menu on its own, and turned on from a Pokémon Center.

use poke_core::bag::BagItem;
use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::{EVENT_FOLLOWED_OAK_INTO_LAB, EVENT_GOT_POKEDEX};
use crate::command::{Command, Decision, Reply};
use crate::gfx::ui::UiSurface;
use crate::input::Joypad;
use crate::mode::{Mode, Status};
use crate::modes::overworld::Overworld;
use crate::rng::GameRng;
use crate::systems::inventory::{Inventory, PC_ITEM_CAPACITY};
use crate::systems::overworld::Location;
use crate::world::World;
use crate::{Event, Game, Input, Pacing};
use super::players_pc::PlayerPc;
use super::PcMenu;

const PLAYERS_PC: u8 = 1;
const WITHDRAW: u8 = 0;
const DEPOSIT: u8 = 1;
const TOSS: u8 = 2;

fn world(bag: &[(ItemId, u8)], pc: &[(ItemId, u8)]) -> World {
    World {
        player_name: encode("RED").unwrap(),
        bag: Inventory::bag(bag.iter().map(|&(id, q)| BagItem::new(id, q)).collect()),
        pc_items: Inventory::pc(pc.iter().map(|&(id, q)| BagItem::new(id, q)).collect()),
        ..World::default()
    }
}

fn game(world: World, mode: Mode) -> Game {
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    game.push(mode);
    game
}

fn settle(game: &mut Game) -> Decision {
    for _ in 0..3000 {
        if let Status::Waiting(decision) = game.status() {
            return decision;
        }
        game.frame(Input::None);
    }
    panic!("nothing ever waited: {:?}", game.modes().last());
}

fn answer(game: &mut Game, decision: Decision, command: Command) {
    assert_eq!(settle(game), decision, "before {command:?}");
    assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted), "{command:?}");
    for _ in 0..600 {
        if game.frame(Input::None).events.contains(&Event::CommandDone(command.clone())) {
            return;
        }
    }
    panic!("{command:?} never finished");
}

/// Texts answered until something else waits.
fn read_on(game: &mut Game) -> Decision {
    while settle(game) == Decision::Text {
        answer(game, Decision::Text, Command::Advance);
    }
    settle(game)
}

fn text_row(game: &Game, y: usize) -> Vec<u8> {
    let mut row = game.ui().row(y)[1..18].to_vec();
    while row.last() == Some(&UiSurface::BLANK) {
        row.pop();
    }
    row
}

/// The main menu's rows, read back off the screen.
fn menu_rows(game: &Game) -> Vec<Vec<u8>> {
    (2..12).step_by(2).map(|y| {
        let mut row = game.ui().row(y)[2..15].to_vec();
        while row.last() == Some(&UiSurface::BLANK) {
            row.pop();
        }
        row
    }).filter(|row| !row.is_empty() && row[0] != 0x7A).collect()
}

/// A Pokémon Center's PC, on the player's PC's menu.
fn at_players_pc(world: World) -> Game {
    let mut game = game(world, Mode::PcMenu(PcMenu::new()));
    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(PLAYERS_PC));
    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    game
}

#[test]
fn the_menu_grows_with_the_pokedex_and_the_hall_of_fame() {
    let mut plain = game(world(&[], &[]), Mode::PcMenu(PcMenu::new()));
    read_on(&mut plain);
    let rows = |names: &[&str]| names.iter().map(|name| encode(name).unwrap()).collect::<Vec<_>>();
    assert_eq!(menu_rows(&plain), rows(&["SOMEONE's PC", "RED's PC", "LOG OFF"]));

    let mut with_dex = world(&[], &[]);
    with_dex.events.set(EVENT_GOT_POKEDEX);
    let mut dex = game(with_dex.clone(), Mode::PcMenu(PcMenu::new()));
    read_on(&mut dex);
    assert_eq!(menu_rows(&dex), rows(&["SOMEONE's PC", "RED's PC", "PROF.OAK's PC", "LOG OFF"]));
    assert_eq!(dex.ui().row(9)[0], 0x7D, "the box's bottom corner");

    with_dex.hall_of_fame_teams = 1;
    let mut league = game(with_dex, Mode::PcMenu(PcMenu::new()));
    read_on(&mut league);
    let mut expected = rows(&["SOMEONE's PC", "RED's PC", "PROF.OAK's PC"]);
    expected.push([&[0xE1, 0xE2][..], &encode("LEAGUE").unwrap()].concat());
    expected.push(encode("LOG OFF").unwrap());
    assert_eq!(menu_rows(&league), expected);
}

#[test]
fn a_hall_of_fame_without_the_pokedex_draws_the_tall_box_round_three_rows() {
    let mut world = world(&[], &[]);
    world.hall_of_fame_teams = 1;
    let mut game = game(world, Mode::PcMenu(PcMenu::new()));
    read_on(&mut game);
    assert_eq!(game.ui().row(11)[0], 0x7D);
    assert_eq!(menu_rows(&game).last(), Some(&encode("LOG OFF").unwrap()));
    let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!() };
    assert_eq!(menu.rows(), 3);
}

#[test]
fn a_deposit_asks_how_many_and_a_withdrawal_brings_them_back() {
    let mut game = at_players_pc(world(&[(ItemId::Potion, 5), (ItemId::Antidote, 1)], &[]));
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    answer(&mut game, Decision::Quantity, Command::ChooseQuantity(3));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 16), encode("stored via PC.").unwrap());
    assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 2);
    assert_eq!(game.world().pc_items.items, [BagItem::new(ItemId::Potion, 3)]);
    answer(&mut game, Decision::Text, Command::Advance);
    assert_eq!(settle(&mut game), Decision::List, "the bag again");
    answer(&mut game, Decision::List, Command::CancelList);

    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!() };
    assert_eq!(menu.selected(), DEPOSIT, "the menu reopens on the row chosen");
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    answer(&mut game, Decision::Quantity, Command::ChooseQuantity(3));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("Withdrew").unwrap());
    assert!(game.world().pc_items.items.is_empty());
    assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 5);
}

#[test]
fn a_toss_says_no_a_row_down_and_yes_throws_away() {
    let mut game = at_players_pc(world(&[], &[(ItemId::Potion, 4), (ItemId::Antidote, 2)]));
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(TOSS));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    answer(&mut game, Decision::Quantity, Command::ChooseQuantity(2));
    answer(&mut game, Decision::Text, Command::Advance);
    answer(&mut game, Decision::TwoOption, Command::ChooseOption(1));
    assert_eq!(read_on(&mut game), Decision::List);
    let Some(Mode::ListMenu(list)) = game.modes().last() else { panic!() };
    assert_eq!(list.selected(), 1, "the yes/no's answer is where the list reopens");
    assert_eq!(game.world().pc_items.quantity_of(ItemId::Potion), 4);

    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    answer(&mut game, Decision::Quantity, Command::ChooseQuantity(2));
    answer(&mut game, Decision::Text, Command::Advance);
    answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("Threw away").unwrap());
    assert_eq!(game.world().pc_items.quantity_of(ItemId::Potion), 2);
}

#[test]
fn a_key_item_skips_the_count_and_is_too_important_to_toss() {
    let mut game = at_players_pc(world(&[(ItemId::Bicycle, 1)], &[(ItemId::Hm01Cut, 1)]));
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(TOSS));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("That's too impor-").unwrap());
    answer(&mut game, Decision::Text, Command::Advance);
    answer(&mut game, Decision::List, Command::CancelList);
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    assert_eq!(settle(&mut game), Decision::Text, "stored with no count asked");
    assert!(game.world().bag.items.is_empty());
    assert_eq!(game.world().pc_items.quantity_of(ItemId::Bicycle), 1);
}

#[test]
fn nothing_stored_and_no_room_are_said_and_change_nothing() {
    let mut game = at_players_pc(world(&[], &[]));
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("There is nothing").unwrap());
    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("You have nothing").unwrap());

    let full: Vec<_> = (0..PC_ITEM_CAPACITY).map(|i| (ItemId::from_repr(i + 1).unwrap(), 1)).collect();
    let mut game = at_players_pc(world(&[(ItemId::Ether, 1)], &full));
    answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
    answer(&mut game, Decision::List, Command::ChooseListEntry(0));
    answer(&mut game, Decision::Quantity, Command::ChooseQuantity(1));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("No room left to").unwrap());
    assert_eq!(game.world().bag.quantity_of(ItemId::Ether), 1);
    assert_eq!(game.world().pc_items.items.len(), PC_ITEM_CAPACITY as usize);
}

#[test]
fn logging_off_the_player_s_pc_goes_back_to_the_menu_and_off_the_menu_closes_it() {
    let mut game = at_players_pc(world(&[], &[]));
    assert!(game.menu().no_menu_button_sound, "the player's PC is silent too");
    // Pressed rather than commanded: the main menu is up in the frame LOG OFF is taken.
    for button in [Joypad::DOWN, Joypad::empty(), Joypad::DOWN, Joypad::empty(), Joypad::DOWN, Joypad::empty(), Joypad::A] {
        assert_eq!(settle(&mut game), Decision::CursorMenu);
        game.frame(Input::Buttons(button));
    }
    game.frame(Input::None);
    assert_eq!(settle(&mut game), Decision::CursorMenu);
    let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!() };
    assert_eq!((menu.rows(), menu.selected()), (3, 0), "the main menu, from the top");
    assert!(game.menu().no_menu_button_sound, "the main menu is silent");
    assert_eq!(menu_rows(&game)[1], encode("RED's PC").unwrap());
    answer(&mut game, Decision::CursorMenu, Command::CancelOption);
    for _ in 0..200 {
        game.frame(Input::None);
    }
    assert!(game.modes().is_empty());
    assert!(!game.menu().no_menu_button_sound);
    assert_eq!(game.world().no_text_delay, false);
}

#[test]
fn the_player_s_own_pc_turns_itself_on_and_off() {
    let mut game = game(world(&[], &[(ItemId::Potion, 1)]), Mode::PlayerPc(PlayerPc::direct()));
    assert_eq!(settle(&mut game), Decision::Text);
    assert_eq!(text_row(&game, 14), encode("RED turned on").unwrap());
    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    answer(&mut game, Decision::CursorMenu, Command::CancelOption);
    for _ in 0..200 {
        game.frame(Input::None);
    }
    assert!(game.modes().is_empty());
}

#[test]
fn a_pokemon_center_pc_hides_the_player_under_its_menu() {
    let mut world = world(&[], &[(ItemId::Potion, 1)]);
    world.location = Location { map: Map::ViridianPokecenter, x: 13, y: 4, facing: SpriteFacing::Up, last_map: Map::ViridianCity,
        ..Location::default() };
    world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB);
    world.events.set(EVENT_GOT_POKEDEX);
    let mut game = Game::new(world, GameRng::seeded(9), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::new()));
    assert_eq!(settle(&mut game), Decision::Overworld);
    let player = |game: &Game| match &game.modes()[0] {
        Mode::Overworld(overworld) => overworld.sprites()[0].image_index,
        _ => unreachable!(),
    };
    assert_ne!(player(&game), 0xFF);
    for _ in 0..4 {
        game.frame(Input::Buttons(Joypad::A));
    }
    assert_eq!(read_on(&mut game), Decision::CursorMenu);
    assert!(matches!(game.modes()[1], Mode::PcMenu(_)));
    assert_eq!(player(&game), 0xFF, "the box covers the player's square");
    answer(&mut game, Decision::CursorMenu, Command::CancelOption);
    assert_eq!(read_on(&mut game), Decision::Overworld);
    assert_ne!(player(&game), 0xFF);
}

#[test]
fn a_save_mid_deposit_resumes_identically() {
    let mut whole = at_players_pc(world(&[(ItemId::Potion, 5)], &[]));
    answer(&mut whole, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
    answer(&mut whole, Decision::List, Command::ChooseListEntry(0));
    whole.frame(Input::Buttons(Joypad::A));
    for _ in 0..3 {
        whole.frame(Input::None);
    }
    let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
    for frame in 0..300 {
        let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
        assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
    }
    assert_eq!(restored.world(), whole.world());
}

mod bills {
    use poke_core::species::PokemonSpecies;
    use crate::party::{BoxMon, Named, PartyMon, MONS_PER_BOX, PARTY_LENGTH};
    use crate::systems::add_mon::{deposit, new_party_mon, Origin};
    use super::super::bills_pc::{BillsPc, CHANGE_BOX, DEPOSIT, MOVE, RELEASE, SEE_YA, STATS, WITHDRAW};
    use super::*;

    const BILLS_PC: u8 = 0;

    fn party_mon(species: PokemonSpecies, level: u8) -> Named<PartyMon> {
        let mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: encode("RED").unwrap(), nick: encode(&species.to_string().to_uppercase()).unwrap() }
    }

    fn box_mon(species: PokemonSpecies, level: u8) -> Named<BoxMon> {
        let named = party_mon(species, level);
        Named { mon: deposit(named.mon), ot: named.ot, nick: named.nick }
    }

    fn trainer(party: &[(PokemonSpecies, u8)], boxed: &[(PokemonSpecies, u8)]) -> World {
        World {
            party: party.iter().map(|&(species, level)| party_mon(species, level)).collect(),
            boxes: vec![boxed.iter().map(|&(species, level)| box_mon(species, level)).collect()],
            ..world(&[], &[])
        }
    }

    /// A Pokémon Center's PC, on BILL's PC's menu.
    fn at_bills_pc(world: World) -> Game {
        let mut game = game(world, Mode::PcMenu(PcMenu::new()));
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(BILLS_PC));
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert!(matches!(game.modes()[game.modes().len() - 2], Mode::BillsPc(_)));
        game
    }

    fn selected(game: &Game) -> u8 {
        let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!("{:?}", game.modes().last()) };
        menu.selected()
    }

    fn row_at(game: &Game, x: usize, y: usize, text: &str) -> Vec<u8> {
        let expected = encode(text).unwrap();
        assert_eq!(game.ui().row(y)[x..x + expected.len()], expected[..], "{text} at ({x}, {y})");
        expected
    }

    /// The text after a choice, read on to the menu.
    fn said(game: &mut Game, first_line: &str) {
        assert_eq!(settle(game), Decision::Text);
        assert_eq!(text_row(game, 14), encode(first_line).unwrap());
        assert_eq!(read_on(game), Decision::CursorMenu);
    }

    #[test]
    fn the_menu_shows_its_rows_and_the_box_number() {
        let mut world = trainer(&[(PokemonSpecies::Pidgey, 5)], &[]);
        world.current_box = 11;
        let game = at_bills_pc(world);
        row_at(&game, 2, 2, "WITHDRAW");
        assert_eq!(game.ui().row(2)[11..13], [0xE1, 0xE2], "<PKMN>");
        row_at(&game, 2, 6, "RELEASE");
        row_at(&game, 2, 8, "CHANGE BOX");
        row_at(&game, 2, 10, "SEE YA!");
        row_at(&game, 10, 16, "BOX No.12");
        assert_eq!(game.ui().row(14)[1..6], encode("What?").unwrap()[..]);
        let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!() };
        assert_eq!((menu.rows(), menu.selected()), (5, WITHDRAW));
    }

    #[test]
    fn a_deposit_goes_to_the_end_of_the_box_and_a_withdrawal_to_the_end_of_the_party() {
        let world = trainer(&[(PokemonSpecies::Pidgey, 5), (PokemonSpecies::Rattata, 3)], &[(PokemonSpecies::Spearow, 4)]);
        let mut game = at_bills_pc(world);
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
        assert_eq!(settle(&mut game), Decision::List);
        row_at(&game, 6, 6, "RATTATA");
        assert_eq!(game.ui().row(7)[14..16], [0x6E, 0xF9], "<LV>3");
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        row_at(&game, 11, 12, "DEPOSIT");
        row_at(&game, 11, 14, "STATS");
        row_at(&game, 11, 16, "CANCEL");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(MOVE));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("RATTATA was").unwrap());
        assert_eq!(text_row(&game, 16), encode("stored in Box 1.").unwrap());
        let species = |mons: Vec<PokemonSpecies>| mons;
        assert_eq!(species(game.world().party.iter().map(|m| m.mon.mon.species).collect()), [PokemonSpecies::Pidgey]);
        let boxed = &game.world().boxes[0];
        assert_eq!(boxed.iter().map(|m| m.mon.species).collect::<Vec<_>>(), [PokemonSpecies::Spearow, PokemonSpecies::Rattata]);
        assert_eq!(boxed[1].mon.box_level, 3);
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert_eq!(selected(&game), DEPOSIT, "the menu reopens on the row chosen");

        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        row_at(&game, 11, 12, "WITHDRAW");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(MOVE));
        said(&mut game, "SPEAROW is");
        let party: Vec<_> = game.world().party.iter().map(|m| (m.mon.mon.species, m.mon.level)).collect();
        assert_eq!(party, [(PokemonSpecies::Pidgey, 5), (PokemonSpecies::Spearow, 4)]);
        assert_eq!(game.world().boxes[0].len(), 1);
    }

    #[test]
    fn every_refusal_is_said_and_changes_nothing() {
        let mut game = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &[]));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
        said(&mut game, "You can't deposit");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        said(&mut game, "What? There are");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(RELEASE));
        said(&mut game, "What? There are");

        let six = [(PokemonSpecies::Pidgey, 5); PARTY_LENGTH];
        let full_box = [(PokemonSpecies::Rattata, 2); MONS_PER_BOX];
        let mut game = at_bills_pc(trainer(&six, &full_box));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        said(&mut game, "You can't take");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(DEPOSIT));
        said(&mut game, "Oops! This Box is");
        assert_eq!((game.world().party.len(), game.world().boxes[0].len()), (PARTY_LENGTH, MONS_PER_BOX));
    }

    #[test]
    fn a_release_says_no_back_to_the_list_and_yes_lets_it_go() {
        let boxed = [(PokemonSpecies::Rattata, 2), (PokemonSpecies::Spearow, 4), (PokemonSpecies::Zubat, 6)];
        let mut game = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &boxed));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(RELEASE));
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Once released,").unwrap());
        assert_eq!(read_on(&mut game), Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(1));
        assert_eq!(settle(&mut game), Decision::List);
        let Some(Mode::ListMenu(list)) = game.modes().last() else { panic!() };
        assert_eq!(list.selected(), 1, "the list reopens where it was");
        assert_eq!(game.world().boxes[0].len(), 3);

        answer(&mut game, Decision::List, Command::ChooseListEntry(2));
        assert_eq!(read_on(&mut game), Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        said(&mut game, "ZUBAT was");
        let left: Vec<_> = game.world().boxes[0].iter().map(|m| m.mon.species).collect();
        assert_eq!(left, [PokemonSpecies::Rattata, PokemonSpecies::Spearow]);
        assert_eq!(selected(&game), RELEASE);
    }

    #[test]
    fn stats_show_the_mon_and_ask_again_on_stats() {
        let mut game = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &[(PokemonSpecies::Spearow, 4)]));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        let before: Vec<Vec<u8>> = (0..18).map(|y| game.ui().row(y).to_vec()).collect();
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(STATS));
        assert_eq!(settle(&mut game), Decision::StatusScreen);
        answer(&mut game, Decision::StatusScreen, Command::Advance);
        answer(&mut game, Decision::StatusScreen, Command::Advance);
        assert_eq!(settle(&mut game), Decision::CursorMenu);
        assert_eq!(selected(&game), STATS);
        let after: Vec<Vec<u8>> = (0..18).map(|y| game.ui().row(y).to_vec()).collect();
        assert_eq!(after[..10], before[..10], "the list is back");
        row_at(&game, 11, 12, "WITHDRAW");
        answer(&mut game, Decision::CursorMenu, Command::CancelOption);
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert_eq!(selected(&game), WITHDRAW);
        assert_eq!(game.world().boxes[0].len(), 1);
    }

    #[test]
    fn changing_the_box_asks_first_marks_full_boxes_and_moves_the_box_number() {
        let mut world = trainer(&[(PokemonSpecies::Pidgey, 5)], &[(PokemonSpecies::Spearow, 4)]);
        world.boxes.resize(3, Vec::new());
        world.boxes[2].push(box_mon(PokemonSpecies::Zubat, 6));
        let mut game = at_bills_pc(world);
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(CHANGE_BOX));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("When you change a").unwrap());
        assert_eq!(read_on(&mut game), Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(1));
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert_eq!(selected(&game), CHANGE_BOX);

        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(CHANGE_BOX));
        assert_eq!(read_on(&mut game), Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::CursorMenu);
        let Some(Mode::CursorMenu(menu)) = game.modes().last() else { panic!() };
        assert_eq!((menu.rows(), menu.selected()), (12, 0));
        row_at(&game, 13, 1, "BOX 1");
        row_at(&game, 13, 12, "BOX12");
        row_at(&game, 1, 2, "BOX No. 1");
        assert_eq!(text_row(&game, 14), encode("Choose a").unwrap());
        let balls: Vec<_> = (1..13).filter(|&y| game.ui().get(18, y) == 0x78).collect();
        assert_eq!(balls, [1, 3]);
        assert_eq!(game.ui().get(12, 1), 0xED, "the cursor a row per box");

        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(9));
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert_eq!(game.world().current_box, 9);
        row_at(&game, 10, 16, "BOX No.10");
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        said(&mut game, "What? There are");
    }

    #[test]
    fn see_ya_goes_back_to_the_pc_menu() {
        let mut game = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &[]));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(SEE_YA));
        assert_eq!(settle(&mut game), Decision::CursorMenu);
        assert!(matches!(game.modes()[game.modes().len() - 2], Mode::PcMenu(_)));
        assert_eq!(menu_rows(&game)[0], encode("SOMEONE's PC").unwrap());
        assert!(game.menu().no_menu_button_sound, "the main menu sets it again");
    }

    #[test]
    fn bill_s_own_pc_switches_itself_on_and_off() {
        let mut game = game(trainer(&[(PokemonSpecies::Pidgey, 5)], &[]), Mode::BillsPc(BillsPc::direct()));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Switch on!").unwrap());
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        assert!(!game.menu().no_menu_button_sound);
        answer(&mut game, Decision::CursorMenu, Command::CancelOption);
        for _ in 0..200 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        assert!(!game.world().no_text_delay);
    }

    /// `BillsPCMenu` prints "What?" with the background transfer off, so it is never half written
    /// on the screen, however slow the letters are.
    #[test]
    fn what_is_never_seen_half_written() {
        let mut game = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &[(PokemonSpecies::Spearow, 4)]));
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(WITHDRAW));
        assert_eq!(settle(&mut game), Decision::List);
        // `ExitListMenu` clears `BIT_NO_TEXT_DELAY`, so this "What?" is printed a letter at a time.
        game.frame(Input::Buttons(Joypad::B));
        assert!(!game.world().no_text_delay);
        let whole = encode("What?").unwrap();
        let mut frames = 0;
        while game.status() != Status::Waiting(Decision::CursorMenu) {
            let shown = game.ui().row(14)[1..6].to_vec();
            assert!(shown[0] != whole[0] || shown == whole, "half written: {shown:?}");
            game.frame(Input::None);
            frames += 1;
            assert!(frames < 300, "the menu never came back");
        }
        assert!(frames > 5, "the letters took their delays: {frames} frames");
        assert_eq!(game.ui().row(14)[1..6], whole[..]);
    }

    #[test]
    fn a_save_mid_release_resumes_identically() {
        let mut whole = at_bills_pc(trainer(&[(PokemonSpecies::Pidgey, 5)], &[(PokemonSpecies::Spearow, 4)]));
        answer(&mut whole, Decision::CursorMenu, Command::ChooseOption(RELEASE));
        answer(&mut whole, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(read_on(&mut whole), Decision::TwoOption);
        whole.frame(Input::Buttons(Joypad::A));
        for _ in 0..3 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..400 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
        assert_eq!(restored.world(), whole.world());
        assert!(restored.world().boxes[0].is_empty());
    }
}

mod oaks_and_league {
    use poke_core::species::PokemonSpecies;
    use crate::systems::hall_of_fame::HallOfFameMon;
    use super::*;

    const OAKS_PC: u8 = 2;
    const LEAGUE_PC: u8 = 3;
    const YES: u8 = 0;
    const NO: u8 = 1;

    /// A player with the Pokédex, `owned` mons owned and one more seen.
    fn rated(owned: u8) -> World {
        let mut world = world(&[], &[]);
        world.events.set(EVENT_GOT_POKEDEX);
        for dex in 1..=owned {
            world.pokedex.owned[(dex as usize - 1) / 8] |= 1 << ((dex - 1) % 8);
            world.pokedex.seen[(dex as usize - 1) / 8] |= 1 << ((dex - 1) % 8);
        }
        world.pokedex.seen[owned as usize / 8] |= 1 << (owned % 8);
        world
    }

    fn mon(species: PokemonSpecies, level: u8) -> HallOfFameMon {
        HallOfFameMon { species, level, nick: encode(&species.to_string().to_uppercase()).unwrap() }
    }

    /// A Pokémon Center's PC, on the row given.
    fn at_pc(world: World, row: u8) -> Game {
        let mut game = game(world, Mode::PcMenu(PcMenu::new()));
        assert_eq!(read_on(&mut game), Decision::CursorMenu);
        answer(&mut game, Decision::CursorMenu, Command::ChooseOption(row));
        game
    }

    /// The PC's text answered until the first mon is on the screen.
    fn to_the_first_mon(game: &mut Game) {
        for _ in 0..4 {
            if game.ui().row(15)[1..16] == encode("HALL OF FAME No").unwrap()[..] {
                return;
            }
            answer(game, Decision::Text, Command::Advance);
        }
        panic!("no mon was ever shown");
    }

    #[test]
    fn oaks_pc_rates_the_dex_on_a_yes_and_closes_the_link_either_way() {
        let mut game = at_pc(rated(12), OAKS_PC);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Accessed PROF.").unwrap());
        assert_eq!(read_on(&mut game), Decision::TwoOption);
        assert_eq!(text_row(&game, 14), encode("Want to get your").unwrap());

        answer(&mut game, Decision::TwoOption, Command::ChooseOption(YES));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14)[4..], encode("DEX comp-").unwrap()[..], "the ligature is four tiles");
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14)[..3], encode("13 ").unwrap()[..], "13 seen, one more than owned");
        assert_eq!(text_row(&game, 14)[7..], encode("MON seen").unwrap()[..]);
        assert_eq!(text_row(&game, 16)[..3], encode("12 ").unwrap()[..]);
        assert_eq!(text_row(&game, 16)[7..], encode("MON owned").unwrap()[..]);
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14)[..4], encode("PROF").unwrap()[..], "the rating's own paragraph");
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("You're on the").unwrap(), "the rating for 10 to 19 owned");
        // Its two `cont`s, each a line scrolled up under a `▼`.
        for _ in 0..2 {
            assert_eq!(game.ui().row(16)[18], encode("▼").unwrap()[0]);
            answer(&mut game, Decision::Text, Command::Advance);
            assert_eq!(settle(&mut game), Decision::Text);
        }
        // The rating's `done` ends it, and `WaitForTextScrollButtonPress` holds it with no `▼`.
        assert_eq!(text_row(&game, 16), encode("from my AIDE!").unwrap());
        assert_eq!(game.ui().row(16)[18], UiSurface::BLANK, "no arrow");
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Closed link to").unwrap());
        assert_eq!(game.ui().row(16)[18], UiSurface::BLANK, "`text_waitbutton` has none either");
        assert_eq!(read_on(&mut game), Decision::CursorMenu, "back on the PC's menu");
        assert_eq!(menu_rows(&game).len(), 4);

        let mut no = at_pc(rated(12), OAKS_PC);
        assert_eq!(read_on(&mut no), Decision::TwoOption);
        answer(&mut no, Decision::TwoOption, Command::ChooseOption(NO));
        assert_eq!(settle(&mut no), Decision::Text);
        assert_eq!(text_row(&no, 14), encode("Closed link to").unwrap(), "no rating");
        assert_eq!(read_on(&mut no), Decision::CursorMenu);
    }

    fn with_teams(teams: Vec<Vec<HallOfFameMon>>, recorded: u8) -> World {
        let mut world = rated(12);
        world.hall_of_fame = teams;
        world.hall_of_fame_teams = recorded;
        world
    }

    #[test]
    fn the_league_pc_shows_every_mon_of_every_team_in_turn() {
        let teams = vec![
            vec![mon(PokemonSpecies::Pidgey, 7), mon(PokemonSpecies::Rattata, 9)],
            vec![mon(PokemonSpecies::Spearow, 11)],
        ];
        let mut game = at_pc(with_teams(teams, 2), LEAGUE_PC);
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14)[..9], encode("Accessed ").unwrap()[..]);
        to_the_first_mon(&mut game);

        for (number, nick, level) in [(1, "PIDGEY", 7), (1, "RATTATA", 9), (2, "SPEAROW", 11)] {
            assert_eq!(settle(&mut game), Decision::Text, "{nick}");
            assert!(matches!(game.modes()[game.modes().len() - 2], Mode::PcMenu(_)), "{nick}");
            assert_eq!(game.ui().row(15)[1..16], encode("HALL OF FAME No").unwrap()[..], "{nick}");
            assert_eq!(game.ui().row(15)[18], encode(&number.to_string()).unwrap()[0], "team {number}");
            assert_eq!(game.ui().row(4)[1..1 + nick.len()], encode(nick).unwrap()[..]);
            assert_eq!(game.ui().row(6)[2..8], encode("LEVEL/").unwrap()[..]);
            let level = encode(&level.to_string()).unwrap();
            assert_eq!(game.ui().row(7)[8..8 + level.len()], level[..]);
            assert_eq!(game.ui().get(12, 5), 0, "the picture's first tile");
            answer(&mut game, Decision::Text, Command::Advance);
        }
        assert_eq!(read_on(&mut game), Decision::CursorMenu, "back on the PC's menu");
        assert!(!game.world().no_text_delay);
    }

    #[test]
    fn b_leaves_the_league_pc_where_it_is() {
        let teams = vec![vec![mon(PokemonSpecies::Pidgey, 7), mon(PokemonSpecies::Rattata, 9)]];
        let mut game = at_pc(with_teams(teams, 1), LEAGUE_PC);
        to_the_first_mon(&mut game);
        assert_eq!(game.ui().row(4)[1..7], encode("PIDGEY").unwrap()[..]);
        game.frame(Input::Buttons(Joypad::B));
        assert_eq!(read_on(&mut game), Decision::CursorMenu, "B left without the second mon");
        assert_eq!(menu_rows(&game).len(), 5);
    }

    #[test]
    fn the_numbers_start_where_the_record_does_once_the_oldest_are_gone() {
        let teams = vec![vec![mon(PokemonSpecies::Pidgey, 7)]; 50];
        let mut game = at_pc(with_teams(teams, 53), LEAGUE_PC);
        to_the_first_mon(&mut game);
        assert_eq!(game.ui().row(15)[18], encode("4").unwrap()[0], "53 teams, 50 kept");
    }

    #[test]
    fn a_save_mid_league_pc_resumes_identically() {
        let teams = vec![vec![mon(PokemonSpecies::Pidgey, 7), mon(PokemonSpecies::Rattata, 9)]];
        let mut whole = at_pc(with_teams(teams, 1), LEAGUE_PC);
        to_the_first_mon(&mut whole);
        whole.frame(Input::Buttons(Joypad::A));
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..400 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
        assert_eq!(restored.world(), whole.world());
    }
}
