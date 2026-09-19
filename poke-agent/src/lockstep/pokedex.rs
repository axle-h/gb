//! The Pokédex the start menu's first row opens: the list of numbers, the menu on its lower right,
//! a data page and the nest map, all drawn from the fixture's own seen and owned flags.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::pokedex::PokedexMenu;
use pokered::party::Pokedex;
use pokered::systems::overworld::Location;
use poke_core::map::Map;
use pokered::rng::GameRng;
use pokered::systems::pokedex::{front_pic_tiles, is_set, species_of, FLAG_BYTES};
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{assert_late, breakpoint, ARROW, cartridge_cursor_to, cartridge_until_polling, joypad, open_the_start_menu,
            recreation_until_polling, tile_row, to_vblank, CURSOR, LIST_REDRAWN};


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
    // The nest map marks where the player stands.
    let map = Map::from_repr(gb.core().mmu().read_pointer(&pokered_symbols::wCurMap)).expect("a map the fixture is on");
    let location = Location { map, ..Location::default() };
    let mut game = Game::new(World { pokedex, location, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
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
        // Left, Right and a step off either end of the window return from `HandleMenuInput`, and the
        // list is printed again; anything else only moves the cursor.
        let current = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        let redraws = matches!(button, J::LEFT | J::RIGHT) || (button == J::UP && current == 0)
            || (button == J::DOWN && current == gb.core().mmu().read_pointer(&pokered_symbols::wMaxMenuItem));
        press(&mut gb, &mut game, button);
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until_polling(&mut game, Decision::Pokedex);
        assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "polling after press {step}");
        assert_late(cartridge, recreation, if redraws { LIST_REDRAWN } else { CURSOR }, &format!("after press {step}"));
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
    // break in the middle of the description, which is where both are waiting on a button. The
    // cartridge's way there is mostly loading the picture, so what is timed is the cry, from the
    // frame the picture goes up: the height, weight and description wait for all of it, and the
    // page break's `ProtectedDelay3` is the only loading after it. The recreation puts the picture
    // up in the frame A lands, one before `press` lets go.
    press(&mut gb, &mut game, Joypad::A);
    assert_eq!(game.ui().get(7, 1), 0, "the recreation's picture is up");
    let cry = breakpoint(pokered_symbols::PlayCry);
    let (stop, _) = gb.run_until(&[cry], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(cry), "the data page never cried");
    let cartridge = cartridge_until_polling(&mut gb) - 1;
    let recreation = 1 + recreation_until_polling(&mut game, Decision::PokedexData);
    assert_late(cartridge, recreation, ARROW, "the cry");
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

/// Walks the list down to `dex`, comparing every poll on the way.
fn walk_down_to(gb: &mut GameBoy, game: &mut Game, dex: u8) {
    for step in 0..160 {
        if cartridge_selection(gb) == dex {
            return;
        }
        press(gb, game, Joypad::DOWN);
        cartridge_until_polling(gb);
        recreation_until_polling(game, Decision::Pokedex);
        assert_eq!(cartridge_screen(gb), recreation_screen(game), "walking down, step {step}");
    }
    panic!("the list never reached {dex}");
}

/// `AREA`: the town map with a nest icon on every place the mon is met wild and the player's own
/// marker, then B back to the list. The cartridge turns the LCD off to load the map, so this is
/// compared and not timed.
#[test]
fn the_nest_map_matches_the_cartridge() {
    let (mut gb, mut game) = open_the_pokedex();
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Pokedex);

    // Pidgey, Rattata or Spearow: the first the fixture has seen, each met in plenty of grass.
    let seen = the_pokedex(&gb).seen;
    let dex = [16u8, 19, 21].into_iter().find(|&dex| is_set(&seen, dex)).expect("the fixture has seen a route bird or rat");
    walk_down_to(&mut gb, &mut game, dex);
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::PokedexSideMenu);
    for _ in 0..2 {
        press(&mut gb, &mut game, Joypad::DOWN);
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::PokedexSideMenu);
    }
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::TownMap);
    assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "the nest map as drawn");

    // The nests from slot 0 up, and the player's marker in its own four slots. What the cartridge
    // leaves in the slots between is whatever was there before, and is never shown.
    let oam = super::battle::cartridge_oam(&gb);
    let ours: Vec<[u8; 4]> = game.screen().sprites.iter().map(|o| [o.y, o.x, o.tile, o.attributes]).collect();
    let nests = ours.iter().take_while(|object| object[2] == 4 && object[0] < 160).count();
    assert!(nests > 1, "{nests} nests");
    assert_eq!(oam[..nests], ours[..nests], "the nests");
    assert_eq!(oam[36..], ours[36..], "the player's marker");

    press(&mut gb, &mut game, Joypad::B);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Pokedex);
    assert_eq!(cartridge_screen(&gb), recreation_screen(&game), "the list, drawn again from scratch");
}
