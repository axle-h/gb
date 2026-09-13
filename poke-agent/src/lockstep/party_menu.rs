//! The party menu the start menu's `POKéMON` row opens, drawn from the fixture's own party.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::party_menu::{PartyMenu, PartyMenuType};
use pokered::party::{Named, PartyMon};
use pokered::rng::GameRng;
use pokered::systems::add_mon::{new_party_mon, Origin};
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, cartridge_cursor_to, cartridge_until_polling, joypad,
            open_the_start_menu, press_a_and_settle, recreation_until_polling, tile_row, to_vblank};

/// `POKéMON` is the second row once the player has the Pokédex.
const POKEMON_ROW: u8 = 1;
const PARTY_STRUCT: u16 = 0x2C;
const NAME_LENGTH: u16 = 11;

/// The party as the menu reads it: a nickname, a status, the HP either side and a level. The rest
/// of a mon never reaches this screen, so it is built rather than copied.
fn the_party(gb: &GameBoy) -> Vec<Named<PartyMon>> {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&pokered_symbols::wPartyCount);
    (0..count as u16).map(|slot| {
        let at = pokered_symbols::wPartyMon1.address + PARTY_STRUCT * slot;
        let word = |offset| u16::from_be_bytes([mmu.read(at + offset), mmu.read(at + offset + 1)]);
        let species = PokemonSpecies::from_repr(mmu.read(pokered_symbols::wPartySpecies.address + slot))
            .expect("the fixture's party is well formed");
        let level = mmu.read(at + 33);
        let mut mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.hp = word(1);
        mon.mon.status = mmu.read(at + 4);
        mon.level = level;
        mon.stats[0] = word(34);
        let nicks = pokered_symbols::wPartyMonNicks.address + NAME_LENGTH * slot;
        let nick = (0..NAME_LENGTH).map(|i| mmu.read(nicks + i)).take_while(|&byte| byte != 0x50).collect();
        Named { mon, ot: Vec::new(), nick }
    }).collect()
}

#[test]
fn the_party_menu_shows_what_the_cartridge_shows_at_every_poll() {
    let mut gb = open_the_start_menu();
    cartridge_cursor_to(&mut gb, POKEMON_ROW);
    gb.hold_buttons(joypad(Joypad::A));
    let party_menu = breakpoint(pokered_symbols::DisplayPartyMenu);
    let (stop, _) = gb.run_until(&[party_menu], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(party_menu), "POKéMON never opened the party");
    gb.hold_buttons(JoypadButtonState::default());

    // Opening the party menu turns the LCD off to load the icon graphics, so VBlank stops for a
    // stretch and a poll cannot be waited for in frames. The first one is waited for outright.
    let poll = breakpoint(pokered_symbols::JoypadLowSensitivity);
    let (stop, _) = gb.run_until(&[poll], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(poll), "the party menu never polled");
    to_vblank(&mut gb);

    let party = the_party(&gb);
    assert!(party.len() > 1, "the fixture's party has to have somewhere to move to");
    let mut game = Game::new(World { party, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
    game.menu_mut().party_and_bills =
        gb.core().mmu().read_pointer(&pokered_symbols::wPartyAndBillsPCSavedMenuItem);
    game.push(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)));

    // The rows only: the mon icons are OAM sprites, and the message box is the text engine's.
    let cartridge = |gb: &GameBoy| (0..12).map(|y| tile_row(gb, y)).collect::<Vec<_>>();
    let recreation = |game: &Game| (0..12).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>();

    for (step, press) in [Joypad::DOWN, Joypad::UP, Joypad::UP].into_iter().enumerate() {
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::PartyMenu);
        assert_eq!(cartridge(&gb), recreation(&game), "polling before press {step}");
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
    }
}

/// `SWITCH` sits below the mon's field moves and `STATS`. The count comes from the list itself,
/// because `DisplayFieldMoveMonMenu` zeroes `wNumFieldMoves` before it prints the names.
fn switch_row(gb: &GameBoy) -> u8 {
    let mmu = gb.core().mmu();
    let moves = (0..4)
        .map(|i| mmu.read(pokered_symbols::wFieldMoves.address + i))
        .take_while(|&name| name != 0)
        .count();
    moves as u8 + 1
}

/// The swap the party menu does to itself: `SWITCH` marks a mon, and the next one chosen changes
/// places with it.
#[test]
fn a_swap_reorders_the_party_as_the_cartridge_does() {
    let mut gb = open_the_start_menu();
    cartridge_cursor_to(&mut gb, POKEMON_ROW);
    press_a_and_settle(&mut gb, 600);

    // The first slot opens the submenu, and `SWITCH` arms the swap.
    gb.hold_buttons(joypad(Joypad::A));
    let submenu = breakpoint(pokered_symbols::DisplayFieldMoveMonMenu);
    let (stop, _) = gb.run_until(&[submenu], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(submenu), "the first slot never opened the submenu");
    gb.hold_buttons(JoypadButtonState::default());
    cartridge_until_polling(&mut gb);
    let switch = switch_row(&gb);
    cartridge_cursor_to(&mut gb, switch);
    press_a_and_settle(&mut gb, 600);

    let armed = gb.core().mmu().read_pointer(&pokered_symbols::wMenuItemToSwap);
    assert_eq!(armed, 1, "the first mon is marked, counting from one");

    // The prompt goes up over a list that is already drawn, so the recreation needs one too. The
    // normal menu underneath is only there to draw it, and nothing compared can see it.
    let mut game = Game::new(World { party: the_party(&gb), ..World::default() },
        GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)));
    recreation_until_polling(&mut game, Decision::PartyMenu);
    game.push(Mode::PartyMenu(PartyMenu::swapping(armed - 1)));
    recreation_until_polling(&mut game, Decision::PartyMenu);

    let cartridge = |gb: &GameBoy| (0..12).map(|y| tile_row(gb, y)).collect::<Vec<_>>();
    let recreation = |game: &Game| (0..12).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>();
    assert_eq!(cartridge(&gb), recreation(&game), "the list under the swap prompt");

    // Down one and commit: both should end up holding the same party in the same order.
    for press in [Joypad::DOWN, Joypad::A] {
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::PartyMenu);
    }
    assert_eq!(cartridge(&gb), recreation(&game), "the list after the swap");
    assert_eq!(gb.core().mmu().read_pointer(&pokered_symbols::wMenuItemToSwap), 0, "and nothing is marked");
}
