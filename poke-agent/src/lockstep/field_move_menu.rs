//! The submenu a chosen party mon opens: START, `POKéMON`, then a mon. Its shape depends on what
//! that mon knows, so the fixture's own party decides what is compared.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::field_move_menu::FieldMoveMenu;
use pokered::rng::GameRng;
use pokered::systems::field_moves::FieldMoves;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, cartridge_until_polling, cartridge_cursor_to, joypad, open_the_start_menu,
            press_a_and_settle, recreation_until_polling, tile_row, to_vblank};

/// `POKéMON` is the second row once the player has the Pokédex.
const POKEMON_ROW: u8 = 1;

/// What `GetMonFieldMoves` left in RAM, which is what the box is built from.
///
/// The count is no use by now: `DisplayFieldMoveMonMenu` sizes the box from `wNumFieldMoves` and
/// then zeroes it before printing the names, so once the menu is up it reads 0. The list itself is
/// zero-terminated, which is what the printing loop walks, and so does this.
fn the_field_moves(gb: &GameBoy) -> FieldMoves {
    let mmu = gb.core().mmu();
    FieldMoves {
        names: (0..4)
            .map(|i| mmu.read(pokered_symbols::wFieldMoves.address + i))
            .take_while(|&name| name != 0)
            .collect(),
        leftmost: mmu.read_pointer(&pokered_symbols::wFieldMovesLeftmostXCoord),
    }
}

#[test]
fn the_field_move_submenu_shows_what_the_cartridge_shows_at_every_poll() {
    let mut gb = open_the_start_menu();
    cartridge_cursor_to(&mut gb, POKEMON_ROW);
    // Into the party menu, then the first slot opens the submenu.
    press_a_and_settle(&mut gb, 600);
    gb.hold_buttons(joypad(Joypad::A));
    let submenu = breakpoint(pokered_symbols::DisplayFieldMoveMonMenu);
    let (stop, _) = gb.run_until(&[submenu], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(submenu), "the first party slot never opened the submenu");
    gb.hold_buttons(JoypadButtonState::default());
    cartridge_until_polling(&mut gb);

    let moves = the_field_moves(&gb);
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::FieldMoveMenu(FieldMoveMenu::new(moves.clone())));
    recreation_until_polling(&mut game, Decision::FieldMoveMenu);

    // The box only: the party menu behind it is not what this mode draws.
    let top = if moves.names.is_empty() { 11 } else { 10 - 2 * moves.names.len() };
    let left = if moves.names.is_empty() { 11 } else { moves.leftmost as usize - 1 };
    let cartridge = |gb: &GameBoy| (top..18).map(|y| tile_row(gb, y as u16)[left..].to_vec()).collect::<Vec<_>>();
    let recreation = |game: &Game| (top..18).map(|y| game.ui().row(y)[left..].to_vec()).collect::<Vec<_>>();
    assert_eq!(cartridge(&gb), recreation(&game), "the box as drawn");

    for (step, press) in [Joypad::DOWN, Joypad::DOWN, Joypad::UP].into_iter().enumerate() {
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::FieldMoveMenu);
        assert_eq!(cartridge(&gb), recreation(&game), "after press {step}");
    }
}
