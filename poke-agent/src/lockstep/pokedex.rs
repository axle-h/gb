//! The Pokédex the start menu's first row opens: the list of numbers, the menu on its lower right
//! and a data page, all three drawn from the fixture's own seen and owned flags.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::pokedex::PokedexMenu;
use pokered::party::Pokedex;
use pokered::rng::GameRng;
use pokered::systems::pokedex::{front_pic_tiles, is_set, species_of, FLAG_BYTES};
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, cartridge_cursor_to, cartridge_until_polling, joypad, open_the_start_menu,
            recreation_until_polling, tile_row, to_vblank};


/// `POKéDEX` is the first row, which the Celadon fixture has.
const POKEDEX_ROW: u8 = 0;
/// `vFrontPic`, and the 49 tiles the data page's picture fills it with.
const FRONT_PIC: u16 = 0x9000;
const PIC_TILES: usize = 49;
/// The `▼` a page break blinks. Its rate is measured rather than exact, so it is left out of the
/// comparison: the record's ±1 frame would otherwise show up as a screen that differs.
const BLINKING_ARROW: (usize, usize) = (18, 16);

fn the_pokedex(gb: &GameBoy) -> Pokedex {
    let mmu = gb.core().mmu();
    let read = |at| -> [u8; FLAG_BYTES] {
        mmu.read_pointer_vec(&at, FLAG_BYTES).try_into().expect("nineteen bytes")
    };
    Pokedex { owned: read(pokered_symbols::wPokedexOwned), seen: read(pokered_symbols::wPokedexSeen) }
}

/// START, `POKéDEX`, and both stopped at the top of `ShowPokedexMenu` so that the screen the dex
/// draws is the same work on both sides.
fn open_the_pokedex() -> (GameBoy, Game) {
    let mut gb = open_the_start_menu();
    cartridge_cursor_to(&mut gb, POKEDEX_ROW);
    gb.hold_buttons(joypad(Joypad::A));
    let dex = breakpoint(pokered_symbols::ShowPokedexMenu);
    let (stop, _) = gb.run_until(&[dex], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(dex), "POKéDEX never opened the dex");
    gb.hold_buttons(JoypadButtonState::default());
    to_vblank(&mut gb);

    let pokedex = the_pokedex(&gb);
    let mut game = Game::new(World { pokedex, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::Pokedex(PokedexMenu::new()));
    (gb, game)
}

/// The dex number under the cartridge's cursor: the scroll plus the row plus one.
fn cartridge_selection(gb: &GameBoy) -> u8 {
    let mmu = gb.core().mmu();
    mmu.read_pointer(&pokered_symbols::wListScrollOffset)
        + mmu.read_pointer(&pokered_symbols::wCurrentMenuItem) + 1
}

fn cartridge_screen(gb: &GameBoy) -> Vec<Vec<u8>> {
    (0..18).map(|y| tile_row(gb, y)).collect()
}

fn recreation_screen(game: &Game) -> Vec<Vec<u8>> {
    (0..18).map(|y| game.ui().row(y).to_vec()).collect()
}

/// The screen with the blinking `▼` blanked on both sides.
fn without_the_arrow(mut rows: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    rows[BLINKING_ARROW.1][BLINKING_ARROW.0] = 0x7F;
    rows
}

/// Runs the recreation on to whichever of the dex's screens waits next, and says whether that is
/// the list: a data page's own waits are the breaks in its description.
fn recreation_waiting_on_the_list(game: &mut Game) -> bool {
    for _ in 0..600 {
        match game.status() {
            Status::Waiting(Decision::Pokedex) => return true,
            Status::Waiting(_) => return false,
            _ => {
                game.frame(Input::None);
            }
        }
    }
    panic!("the recreation never polled again");
}

fn press(gb: &mut GameBoy, game: &mut Game, button: Joypad) {
    gb.hold_buttons(joypad(button));
    game.frame(Input::Buttons(button));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    game.frame(Input::None);
}

/// The list, at every poll through a scroll off each end and a page each way. The first poll is
/// compared without a frame budget: the cartridge loads the dex's tile patterns through VBlank on
/// its way in, which is work the recreation does not have to do.
#[test]
fn the_pokedex_list_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, mut game) = open_the_pokedex();
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Pokedex);
    assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "the list as drawn");
    assert!(gb.core().mmu().read_pointer(&pokered_symbols::wDexMaxSeenMon) >= 8,
        "the fixture has to have seen enough to scroll");

    use Joypad as J;
    let presses = [J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::DOWN,
                   J::UP, J::UP, J::RIGHT, J::RIGHT, J::LEFT];
    for (step, button) in presses.into_iter().enumerate() {
        press(&mut gb, &mut game, button);
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until_polling(&mut game, Decision::Pokedex);
        assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "polling after press {step}");
        assert!((0..=2).contains(&(cartridge as i64 - recreation as i64)),
            "after press {step} the cartridge took {cartridge} frames and the recreation {recreation}");
        assert_eq!(gb.core().mmu().read_pointer(&pokered_symbols::wListScrollOffset),
            recreation_scroll(&game), "the scroll after press {step}");
    }
}

/// `wListScrollOffset` as the recreation holds it, read back off the screen: the number above the
/// first row is the scroll plus one.
fn recreation_scroll(game: &Game) -> u8 {
    let digits = &game.ui().row(2)[1..4];
    let number: u32 = digits.iter().map(|&tile| (tile.wrapping_sub(0xF6)) as u32)
        .fold(0, |number, digit| number * 10 + digit);
    number as u8 - 1
}

/// The side menu, the data page and the way back to the list.
#[test]
fn the_side_menu_and_a_data_page_match_the_cartridge() {
    let (mut gb, mut game) = open_the_pokedex();
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Pokedex);

    // The lowest number the fixture has owned, so the page is a full one: an unowned mon's page
    // carries the picture and nothing else. Walking down to it compares the list on the way.
    let owned = the_pokedex(&gb).owned;
    let dex = (1..=151u8).find(|&dex| is_set(&owned, dex)).expect("the fixture owns something");
    for step in 0..60 {
        if cartridge_selection(&gb) == dex {
            break;
        }
        press(&mut gb, &mut game, Joypad::DOWN);
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::Pokedex);
        assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "walking down, step {step}");
    }
    assert_eq!(cartridge_selection(&gb), dex, "the list never reached {dex}");

    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::PokedexSideMenu);
    assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "the side menu as drawn");

    // `DATA` is the row the menu opens on, so A goes straight to the page. It stops at the page
    // break in the middle of the description, which is where both are waiting on a button.
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::PokedexData);
    assert_eq!(without_the_arrow(cartridge_screen(&gb)), without_the_arrow(recreation_screen(&game)),
        "the data page as drawn");

    // The picture is 49 tiles of `vFrontPic`, mirrored within each byte and laid right to left.
    let species = species_of(dex).expect("a seen dex number is a species");
    let ours: Vec<u8> = front_pic_tiles(species, true).concat();
    let theirs = gb.core().mmu().read_vram_slice(FRONT_PIC, PIC_TILES * 16).unwrap().to_vec();
    assert_eq!(ours, theirs, "{species}'s picture as the cartridge decompressed it");

    // A description breaks into pages and each break waits on its own button, so the way out is to
    // keep answering until the list is back. Every page of it is compared on the way.
    for page in 0..4 {
        press(&mut gb, &mut game, Joypad::A);
        cartridge_until_polling(&mut gb);
        if recreation_waiting_on_the_list(&mut game) {
            break;
        }
        assert_eq!(without_the_arrow(cartridge_screen(&gb)), without_the_arrow(recreation_screen(&game)),
            "page {page} of the description");
    }
    assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "the list, drawn again from scratch");
}
