//! The bag from the start menu's `ITEM` row, against the cartridge: a toss, and a Potion used on a
//! hurt mon, with the whole screen compared wherever both are waiting for a button.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::bag::BagItem;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::start_menu::StartMenu;
use pokered::party::{Named, PartyMon};
use pokered::rng::GameRng;
use pokered::systems::add_mon::{new_party_mon, Origin};
use pokered::systems::inventory::Inventory;
use pokered::world::{TextSpeed, World, NUM_EVENTS};
use pokered::{Game, Input, Pacing};
use crate::pokemon::item::ItemId;
use crate::pokemon::options::{self, GameOptionsReader};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::{assert_late, breakpoint, cartridge_cursor_to, cartridge_until_polling, hurried_letter, joypad,
            recreation_until_polling, tile_row, to_vblank, ARROW, BOX, CURSOR, DELAY3, LIST, LIST_REDRAWN};

/// `ITEM` is the third row once the player has the Pokédex.
const ITEM_ROW: u8 = 2;
const PARTY_STRUCT: u16 = 0x2C;
const NAME_LENGTH: u16 = 11;

fn name_at(gb: &GameBoy, at: u16) -> Vec<u8> {
    (0..NAME_LENGTH).map(|i| gb.core().mmu().read(at + i)).take_while(|&byte| byte != 0x50).collect()
}

/// The party as the menus and the items read it. What none of them reads is built rather than copied.
pub(super) fn the_party(gb: &GameBoy) -> Vec<Named<PartyMon>> {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&sym::wPartyCount);
    (0..count as u16).map(|slot| {
        let at = sym::wPartyMon1.address + PARTY_STRUCT * slot;
        let word = |offset| u16::from_be_bytes([mmu.read(at + offset), mmu.read(at + offset + 1)]);
        let species = PokemonSpecies::from_repr(mmu.read(at)).expect("the fixture's party is well formed");
        let level = mmu.read(at + 33);
        let mut mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.hp = word(1);
        mon.mon.status = mmu.read(at + 4);
        mon.mon.moves = [0, 1, 2, 3].map(|i| PokemonMoveName::from_repr(mmu.read(at + 8 + i)));
        mon.mon.pp = [0, 1, 2, 3].map(|i| mmu.read(at + 0x1D + i));
        mon.level = level;
        for stat in 0..5 {
            mon.stats[stat] = word(34 + 2 * stat as u16);
            mon.mon.stat_exp[stat] = word(17 + 2 * stat as u16);
        }
        mon.mon.exp = u32::from_be_bytes([0, mmu.read(at + 14), mmu.read(at + 15), mmu.read(at + 16)]);
        mon.mon.dvs = pokered::systems::stats::Dvs([mmu.read(at + 27), mmu.read(at + 28)]);
        Named { mon, ot: Vec::new(), nick: name_at(gb, sym::wPartyMonNicks.address + NAME_LENGTH * slot) }
    }).collect()
}

pub(super) fn the_bag(gb: &GameBoy) -> Inventory {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&sym::wNumBagItems) as u16;
    let at = sym::wBagItems.address;
    Inventory::bag((0..count).map(|i| {
        BagItem::new(ItemId::from_repr(mmu.read(at + 2 * i)).expect("an item"), mmu.read(at + 2 * i + 1))
    }).collect())
}

/// Everything these screens read: the name, the events, the text speed, the money and the coins,
/// the bag, the party. Only those: a test comparing anything else, such as the badges, the rival's
/// name, the Pokedex or a party member's OT, has to fill it itself, or the difference it sees is
/// this world's rather than what it is testing. The party here carries no OT; `status_screen`'s does.
pub(super) fn the_world(gb: &GameBoy) -> World {
    let mmu = gb.core().mmu();
    let mut world = World {
        player_name: name_at(gb, sym::wPlayerName.address),
        party: the_party(gb),
        bag: the_bag(gb),
        money: mmu.read_slice(sym::wPlayerMoney.address, 3).try_into().unwrap(),
        coins: mmu.read_slice(sym::wPlayerCoins.address, 2).try_into().unwrap(),
        ..World::default()
    };
    world.options.text_speed = match mmu.read_game_options().expect("the fixture's options").text_speed {
        options::TextSpeed::Fast => TextSpeed::Fast,
        options::TextSpeed::Medium => TextSpeed::Medium,
        options::TextSpeed::Slow => TextSpeed::Slow,
    };
    let events = sym::wEventFlags.address;
    for event in 0..NUM_EVENTS as u16 {
        if mmu.read(events + event / 8) & 1 << (event % 8) != 0 {
            world.events.set(event);
        }
    }
    world
}

/// The recreation standing where the cartridge stands: its world, its cursor globals, and the
/// screen as the cartridge has it, so everything behind the menus compares as well.
pub(super) fn the_game(gb: &GameBoy) -> Game {
    let mut game = Game::new(the_world(gb), GameRng::seeded(0), Pacing::Faithful);
    let mmu = gb.core().mmu();
    let menu = game.menu_mut();
    menu.battle_and_start = mmu.read_pointer(&sym::wBattleAndStartSavedMenuItem);
    menu.bag_saved = mmu.read_pointer(&sym::wBagSavedMenuItem);
    menu.list_scroll = mmu.read_pointer(&sym::wListScrollOffset);
    for y in 0..18 {
        for (x, tile) in tile_row(gb, y).into_iter().enumerate() {
            game.screen_mut().ui.set(x, y as usize, tile);
        }
    }
    game
}

pub(super) fn screen(gb: &GameBoy) -> Vec<Vec<u8>> {
    (0..18).map(|y| tile_row(gb, y)).collect()
}

pub(super) fn recreated(game: &Game) -> Vec<Vec<u8>> {
    (0..18).map(|y| game.ui().row(y).to_vec()).collect()
}

/// A press into both, and then both until they are waiting again, compared there: the same screen,
/// and the cartridge late by the `loading` the recreation leaves out on the way.
pub(super) fn step(gb: &mut GameBoy, game: &mut Game, press: Joypad, then: Decision, loading: u32, what: &str) {
    let (cartridge, recreation) = press_and_wait(gb, game, press, then, what);
    assert_late(cartridge, recreation, loading, what);
}

/// A press into both and both until they wait again, with the same screen there.
pub(super) fn press_and_wait(gb: &mut GameBoy, game: &mut Game, press: Joypad, then: Decision, what: &str) -> (u32, u32) {
    gb.hold_buttons(joypad(press));
    game.frame(Input::Buttons(press));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let cartridge = cartridge_until_polling(gb);
    let recreation = recreation_until_polling(game, then);
    assert_eq!(screen(gb), recreated(game), "{what}");
    (cartridge, recreation)
}

/// B out of the bag, and the start menu back. With the LCD on, `LoadTextBoxTilePatterns` copies its
/// tiles through `CopyVideoData`, eight a frame and a frame to finish, before the cursor's `Delay3`.
fn back_to_the_start_menu(gb: &mut GameBoy, game: &mut Game) {
    let tiles = (sym::TextBoxGraphicsEnd.address - sym::TextBoxGraphics.address) as u32 / 16;
    let (cartridge, recreation) = press_and_wait(gb, game, Joypad::B, Decision::StartMenu, "back to the start menu");
    assert_late(cartridge, recreation, tiles / 8 + 1 + CURSOR, "back to the start menu");
}

/// As `step`, for a screen that turns the LCD off on the way, so there are no frames to count.
fn step_through_lcd_off(gb: &mut GameBoy, game: &mut Game, press: Joypad, then: Decision, what: &str) {
    gb.hold_buttons(joypad(press));
    game.frame(Input::Buttons(press));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let (stop, _) = gb.run_until(&[poll], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(poll), "{what}: the cartridge never polled");
    to_vblank(gb);
    recreation_until_polling(game, then);
    assert_eq!(screen(gb), recreated(game), "{what}");
}

/// START held until the menu opens, with `before` run on the machine first.
fn open_the_start_menu(before: impl FnOnce(&mut GameBoy)) -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/at-celadon.bin")).unwrap();
    before(&mut gb);
    gb.run(MachineCycles::PER_FRAME * 60);
    gb.hold_buttons(JoypadButtonState { start: true, ..Default::default() });
    let menu = breakpoint(sym::DisplayStartMenu);
    let (stop, _) = gb.run_until(&[menu], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(menu), "START never opened the menu");
    gb.hold_buttons(JoypadButtonState::default());
    gb
}

/// Both at the start menu, the cursor on `ITEM`.
fn both_on_item(before: impl FnOnce(&mut GameBoy)) -> (GameBoy, Game) {
    let mut gb = open_the_start_menu(before);
    cartridge_cursor_to(&mut gb, ITEM_ROW);
    let mut game = the_game(&gb);
    game.menu_mut().battle_and_start = ITEM_ROW;
    game.push(Mode::StartMenu(StartMenu::new()));
    recreation_until_polling(&mut game, Decision::StartMenu);
    assert_eq!(screen(&gb), recreated(&game), "the start menu");
    (gb, game)
}

/// The bag written as given, `$FF` to the end of it.
fn write_bag(gb: &mut GameBoy, slots: &[(ItemId, u8)]) {
    let mut bytes = vec![0xFF; 41];
    for (i, &(item, quantity)) in slots.iter().enumerate() {
        bytes[2 * i] = item as u8;
        bytes[2 * i + 1] = quantity;
    }
    let mmu = gb.core_mut().mmu_mut();
    mmu.write(sym::wNumBagItems.address, slots.len() as u8);
    mmu.write_slice(sym::wBagItems.address, &bytes);
}

#[test]
fn tossing_two_of_an_item_shows_what_the_cartridge_shows_at_every_poll() {
    let bag = [(ItemId::TownMap, 1), (ItemId::Potion, 5), (ItemId::Antidote, 2), (ItemId::Nugget, 1),
               (ItemId::Repel, 3), (ItemId::Ether, 1)];
    let (mut gb, mut game) = both_on_item(|gb| write_bag(gb, &bag));
    let hurry = hurried_letter(&game);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    // The Town Map first: a key item is refused without being asked how many.
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS on the Town Map");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::UseToss, CURSOR, "TOSS");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "too important");
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag again");
    // The cursor reaches the third row in two, and the list scrolls under it after that.
    for press in 0..4 {
        let loading = if press < 2 { CURSOR } else { LIST_REDRAWN };
        step(&mut gb, &mut game, Joypad::DOWN, Decision::List, loading, &format!("down {press}"));
    }
    step(&mut gb, &mut game, Joypad::UP, Decision::List, CURSOR, "and up to the Nugget");
    step(&mut gb, &mut game, Joypad::UP, Decision::List, CURSOR, "the Antidotes");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS on the Antidotes");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::UseToss, CURSOR, "TOSS");
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, 0, "how many");
    step(&mut gb, &mut game, Joypad::UP, Decision::Quantity, 0, "two");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "is it OK");
    step(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "yes or no");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "threw away");
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag without them");
    assert_eq!(game.world().bag, the_bag(&gb));
    back_to_the_start_menu(&mut gb, &mut game);
}

#[test]
fn a_potion_on_a_hurt_mon_shows_what_the_cartridge_shows_at_every_poll() {
    const HURT: u16 = 7;
    let (mut gb, mut game) = both_on_item(|gb| {
        write_bag(gb, &[(ItemId::Potion, 3), (ItemId::Antidote, 1)]);
        let hp = sym::wPartyMon1.address + 1;
        gb.core_mut().mmu_mut().write_slice(hp, &HURT.to_be_bytes());
    });
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::PartyMenu, "the party");
    // The bar's last `Delay3`, `ClearScreen`'s, the box's and `RedrawPartyMenu_`'s, before the 50.
    step(&mut gb, &mut game, Joypad::A, Decision::Text, 4 * DELAY3, "recovered");
    assert_eq!(game.world().party[0].mon.mon.hp, the_party(&gb)[0].mon.mon.hp);
    assert!(game.world().party[0].mon.mon.hp > HURT);
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::List, "the bag with one fewer");
    assert_eq!(game.world().bag, the_bag(&gb));
    back_to_the_start_menu(&mut gb, &mut game);
}

/// An Ether: the party, then a move chosen from the single-spaced menu after a wrap each way.
#[test]
fn an_ether_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| {
        write_bag(gb, &[(ItemId::Ether, 2)]);
        let pp = sym::wPartyMon1.address + 0x1D;
        let mmu = gb.core_mut().mmu_mut();
        mmu.write(pp, mmu.read(pp) & 0xC0);
    });
    let hurry = hurried_letter(&game);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::PartyMenu, "the party");
    step(&mut gb, &mut game, Joypad::A, Decision::MoveMenu, BOX + CURSOR + hurry, "which technique");
    step(&mut gb, &mut game, Joypad::UP, Decision::MoveMenu, CURSOR, "up, round to the last move");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::MoveMenu, CURSOR, "down, round to the first");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "restored");
    assert_eq!(game.world().party[0].mon.mon.pp, the_party(&gb)[0].mon.mon.pp);
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::List, "the bag with one fewer");
    assert_eq!(game.world().bag, the_bag(&gb));
    back_to_the_start_menu(&mut gb, &mut game);
}


/// The fixture's Oddish, second in the party, with two moves and eight levels to its evolution.
const ODDISH: u8 = 1;

fn to_the_oddish(gb: &mut GameBoy, game: &mut Game) {
    step_through_lcd_off(gb, game, Joypad::A, Decision::PartyMenu, "the party");
    step(gb, game, Joypad::DOWN, Decision::PartyMenu, CURSOR, "down to the Oddish");
}

#[test]
fn a_tm_taught_to_a_mon_with_room_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| write_bag(gb, &[(ItemId::Tm06Toxic, 1), (ItemId::Potion, 1)]));
    let hurry = hurried_letter(&game);
    assert_eq!(game.world().party[ODDISH as usize].mon.mon.species, PokemonSpecies::Oddish);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "booted up a TM");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "it contained TOXIC");
    step(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "teach it?");
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::PartyMenu, "ABLE and NOT ABLE");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::PartyMenu, CURSOR, "down to the Oddish");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + hurry, "learned TOXIC");
    assert_eq!(game.world().party[ODDISH as usize].mon.mon.moves, the_party(&gb)[ODDISH as usize].mon.mon.moves);
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::List, "the bag without the TM");
    assert_eq!(game.world().bag, the_bag(&gb));
}

#[test]
fn a_rare_candy_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| write_bag(gb, &[(ItemId::RareCandy, 2)]));
    let hurry = hurried_letter(&game);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    to_the_oddish(&mut gb, &mut game);
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + hurry, "grew to level 14");
    // `RedrawPartyMenu_`'s `Delay3`, after the text and before the box.
    step(&mut gb, &mut game, Joypad::A, Decision::Text, DELAY3, "the level-up stats box");
    assert_eq!(game.world().party[ODDISH as usize].mon, the_party(&gb)[ODDISH as usize].mon);
    step_through_lcd_off(&mut gb, &mut game, Joypad::A, Decision::List, "the bag with one fewer");
    assert_eq!(game.world().bag, the_bag(&gb));
}

#[test]
fn a_leaf_stone_on_a_gloom_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| {
        write_bag(gb, &[(ItemId::LeafStone, 1)]);
        let mmu = gb.core_mut().mmu_mut();
        mmu.write(sym::wPartySpecies.address + ODDISH as u16, PokemonSpecies::Gloom as u8);
        mmu.write(sym::wPartyMon1.address + PARTY_STRUCT * ODDISH as u16, PokemonSpecies::Gloom as u8);
    });
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    to_the_oddish(&mut gb, &mut game);
    step_back_to_the_bag(&mut gb, &mut game, "evolved");
    let (ours, theirs) = (&game.world().party[ODDISH as usize].mon, &the_party(&gb)[ODDISH as usize].mon);
    assert_eq!((ours.mon.species, ours.mon.hp, ours.stats, ours.mon.moves), (theirs.mon.species, theirs.mon.hp, theirs.stats, theirs.mon.moves));
    assert_eq!(ours.mon.species, PokemonSpecies::Vileplume);
    assert_eq!(game.world().bag, the_bag(&gb));
}

/// A press, and both until the bag is back and waiting: an evolution reads the pad in its own
/// windows, which are not the places a lockstep agrees to compare.
fn step_back_to_the_bag(gb: &mut GameBoy, game: &mut Game, what: &str) {
    gb.hold_buttons(joypad(Joypad::A));
    game.frame(Input::Buttons(Joypad::A));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let list = breakpoint(sym::DisplayListMenuID);
    let (stop, _) = gb.run_until(&[list], MachineCycles::PER_FRAME * 3000);
    assert_eq!(stop, Stop::Breakpoint(list), "{what}: the bag never came back");
    super::status_screen::cartridge_until_polling(gb);
    super::status_screen::recreation_until(game, Decision::List);
    assert_eq!(screen(gb), recreated(game), "{what}");
}

/// A Super Repel, which sets the steps as the text prints and is spent at the press after it.
#[test]
fn a_repel_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| {
        write_bag(gb, &[(ItemId::SuperRepel, 2), (ItemId::Potion, 1)]);
        gb.core_mut().mmu_mut().write(sym::wRepelRemainingSteps.address, 7);
    });
    let hurry = hurried_letter(&game);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + hurry, "used SUPER REPEL");
    let steps = gb.core().mmu().read_pointer(&sym::wRepelRemainingSteps);
    assert_eq!((game.world().location.repel_steps, steps), (200, 200));
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag with one fewer");
    assert_eq!(game.world().bag, the_bag(&gb));
    back_to_the_start_menu(&mut gb, &mut game);
}

/// The Coin Case counting the fixture's coins, which are written in first.
#[test]
fn the_coin_case_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = both_on_item(|gb| {
        write_bag(gb, &[(ItemId::CoinCase, 1)]);
        gb.core_mut().mmu_mut().write_slice(sym::wPlayerCoins.address, &[0x04, 0x56]);
    });
    let hurry = hurried_letter(&game);
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::UseToss, CURSOR, "USE/TOSS");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW + hurry, "the coins");
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag again");
    back_to_the_start_menu(&mut gb, &mut game);
}

/// `OLD_ROD`, in the bag's first slot, with the start menu's cursor left on `ITEM`.
fn old_rod_on_top(cartridge: &mut super::scripts::Cartridge) {
    cartridge.write(sym::wBagItems.address, ItemId::OldRod as u8);
    cartridge.write(sym::wBagItems.address + 1, 1);
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 2);
    cartridge.write(sym::wBagSavedMenuItem.address, 0);
    cartridge.write(sym::wListScrollOffset.address, 0);
}

/// The Old Rod cast into Pallet Town's pond, where the fixture stands on the shore facing the water:
/// the text, the cast, the shakes and the bubble, up to the bite.
#[test]
fn the_old_rod_hooks_a_magikarp_as_the_cartridge_does() {
    use super::scripts::{lockstep, Action, PROMPT, WAIT};
    const SCRIPT: &[(Action, &str)] = &[
        (Action::Press(Joypad::START, 2), "START"), (PROMPT, "ITEM"), (PROMPT, "the Old Rod"),
        (PROMPT, "USE: the text, the cast and the shakes, to the bubble"), (WAIT, "the bubble, to the bite"),
    ];
    lockstep(include_bytes!("../pokemon/data/postgame-fishing.bin"), old_rod_on_top, move |i, _| SCRIPT.get(i).copied());
}
