//! The yes/no the save menu asks, which is the shortest route from a fixture to a two-option menu:
//! START, SAVE, and the prompt is up before anything is written.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::two_option_menu::TwoOptionMenu;
use pokered::rng::GameRng;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, cartridge_cursor_to, cartridge_until_polling, joypad, open_the_start_menu,
            recreation_until_polling, tile_row, to_vblank};

/// `SAVE` is the fifth row once the player has the Pokédex, which the Celadon fixture does.
const SAVE_ROW: u8 = 4;

/// Opens START, then SAVE, and stops on `DisplayTwoOptionMenu` with `wTwoOptionMenuID` still
/// readable: the menu clears it before it takes any input.
fn open_the_save_prompt() -> (GameBoy, u8) {
    let mut gb = open_the_start_menu();
    cartridge_cursor_to(&mut gb, SAVE_ROW);
    gb.hold_buttons(joypad(Joypad::A));
    let menu = breakpoint(pokered_symbols::DisplayTwoOptionMenu);
    let (stop, _) = gb.run_until(&[menu], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(menu), "SAVE never asked");
    gb.hold_buttons(JoypadButtonState::default());
    let id = gb.core().mmu().read_pointer(&pokered_symbols::wTwoOptionMenuID);
    (gb, id)
}

/// The menu's own rectangle. What is under it differs, because the recreation is not drawing the
/// save screen behind it.
fn box_rows(rows: &dyn Fn(usize) -> Vec<u8>, at: (usize, usize)) -> Vec<Vec<u8>> {
    (at.1..at.1 + 5).map(|y| rows(y)[at.0..at.0 + 6].to_vec()).collect()
}

#[test]
fn the_save_prompt_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, id) = open_the_save_prompt();
    let (menu_id, second_default) = TwoOptionMenu::from_byte(id);

    // The cursor is the only place the border's corner is recorded, so it is read back from there.
    cartridge_until_polling(&mut gb);
    let mmu = gb.core().mmu();
    let cursor = (
        mmu.read_pointer(&pokered_symbols::wTopMenuItemX) as usize,
        mmu.read_pointer(&pokered_symbols::wTopMenuItemY) as usize,
    );
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    let menu = TwoOptionMenu::at_cursor(menu_id, cursor, second_default);
    let at = menu.corner();
    game.push(Mode::TwoOptionMenu(menu));
    recreation_until_polling(&mut game, Decision::TwoOption);

    let cartridge = |gb: &GameBoy| box_rows(&|y| tile_row(gb, y as u16), at);
    let recreation = |game: &Game| box_rows(&|y| game.ui().row(y).to_vec(), at);
    assert_eq!(cartridge(&gb), recreation(&game), "the box as drawn");

    for (step, press) in [Joypad::DOWN, Joypad::UP].into_iter().enumerate() {
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::TwoOption);
        assert_eq!(cartridge(&gb), recreation(&game), "after press {step}");
    }

    // B answers NO, so the save is declined and nothing is written.
    gb.hold_buttons(joypad(Joypad::B));
    game.frame(Input::Buttons(Joypad::B));
    to_vblank(&mut gb);
    gb.hold_buttons(JoypadButtonState::default());
    for _ in 0..30 {
        game.frame(Input::None);
    }
    assert!(game.modes().is_empty(), "the recreation's menu came down");
    assert_eq!(game.menu().chosen_item, 1, "NO");
}
