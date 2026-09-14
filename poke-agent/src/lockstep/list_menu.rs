use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::list_menu::ListMenu;
use pokered::rng::GameRng;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{assert_late, breakpoint, cartridge_until_polling, joypad, recreation_until_polling, tile_row, to_vblank,
            CURSOR, LIST, LIST_REDRAWN};

const ITEM_ROW: u8 = 2;

fn press(gb: &mut GameBoy, buttons: JoypadButtonState, then_wait: u64) {
    gb.hold_buttons(buttons);
    gb.run(MachineCycles::PER_FRAME);
    gb.hold_buttons(JoypadButtonState::default());
    gb.run(MachineCycles::PER_FRAME * then_wait);
}

/// Opens START, then ITEM, from the Celadon fixture, and leaves the machine on `DisplayListMenuID`;
/// the breakpoint is where it will return to.
fn open_the_bag() -> (GameBoy, Breakpoint) {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/at-celadon.bin")).unwrap();
    gb.run(MachineCycles::PER_FRAME * 60);
    gb.hold_buttons(JoypadButtonState { start: true, ..Default::default() });
    gb.run(MachineCycles::PER_FRAME * 4);
    gb.hold_buttons(JoypadButtonState::default());
    gb.run(MachineCycles::PER_FRAME * 40);
    for _ in 0..10 {
        let current = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        match current.cmp(&ITEM_ROW) {
            std::cmp::Ordering::Less => press(&mut gb, JoypadButtonState { down: true, ..Default::default() }, 15),
            std::cmp::Ordering::Greater => press(&mut gb, JoypadButtonState { up: true, ..Default::default() }, 15),
            std::cmp::Ordering::Equal => break,
        }
    }
    assert_eq!(gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem), ITEM_ROW);
    gb.hold_buttons(JoypadButtonState { a: true, ..Default::default() });
    let list = breakpoint(pokered_symbols::DisplayListMenuID);
    let (stop, _) = gb.run_until(&[list], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(list), "ITEM never opened the list");
    gb.hold_buttons(JoypadButtonState::default());
    let returns = Breakpoint::new(gb.core().mmu().rom_bank() as u8, gb.return_address());
    (gb, returns)
}

/// The list `wListPointer` names, and the cursor and scroll the caller left.
fn the_list(gb: &GameBoy) -> ListMenu {
    let mmu = gb.core().mmu();
    let list = mmu.read_pointer_u16_le(&pokered_symbols::wListPointer);
    let count = mmu.read(list) as u16;
    let entries = (0..count)
        .map(|i| (ItemId::from_repr(mmu.read(list + 1 + 2 * i)).unwrap(), mmu.read(list + 2 + 2 * i)))
        .collect();
    ListMenu::items(entries,
        mmu.read_pointer(&pokered_symbols::wCurrentMenuItem),
        mmu.read_pointer(&pokered_symbols::wListScrollOffset))
}

/// `LIST_MENU_BOX`, `(4, 2)` to `(19, 12)`.
fn cartridge_box(gb: &GameBoy) -> Vec<Vec<u8>> {
    (2..=12).map(|y| tile_row(gb, y)[4..].to_vec()).collect()
}

fn recreation_box(game: &Game) -> Vec<Vec<u8>> {
    (2..=12).map(|y| game.ui().row(y)[4..].to_vec()).collect()
}

/// Whether a press returns from `HandleMenuInput` and so prints the list again: SELECT, or a step
/// off either end of the three cursor rows. Anything else moves the cursor inside it.
fn redraws(gb: &GameBoy, press: Joypad) -> bool {
    let mmu = gb.core().mmu();
    let current = mmu.read_pointer(&pokered_symbols::wCurrentMenuItem);
    let max = mmu.read_pointer(&pokered_symbols::wMaxMenuItem);
    press == Joypad::SELECT || (press == Joypad::UP && current == 0) || (press == Joypad::DOWN && current == max)
}

/// The same box wherever both poll, the cartridge late by the loading the recreation leaves out:
/// opening the list, printing it again, or placing the cursor. Printing the entries can also
/// outlast a frame, and lag frames are not modelled.
fn compare(presses: &[Joypad]) {
    let (mut gb, returns) = open_the_bag();
    let menu = the_list(&gb);
    assert!(menu.len() > 4, "the fixture's bag has to scroll");
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::ListMenu(menu));
    to_vblank(&mut gb);

    let (last, presses) = presses.split_last().unwrap();
    let mut loading = LIST;
    for (step, &press) in presses.iter().chain([last]).enumerate() {
        let (cartridge, recreation) = (cartridge_until_polling(&mut gb), recreation_until_polling(&mut game, Decision::List));
        assert_eq!(cartridge_box(&gb), recreation_box(&game), "polling before press {step}");
        assert_late(cartridge, recreation, loading, &format!("before press {step}"));
        loading = if redraws(&gb, press) { LIST_REDRAWN } else { CURSOR };
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        if step == presses.len() {
            break;
        }
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
    }
    // The caller draws over the list in the frame it returns, so the last look is at the return.
    let (stop, _) = gb.run_until(&[returns], MachineCycles::PER_FRAME * 2);
    assert_eq!(stop, Stop::Breakpoint(returns), "the last press did not close the list");
    assert!(game.modes().is_empty(), "the last press closes the recreation's list too");
    assert_eq!(cartridge_box(&gb), recreation_box(&game), "as the list returns");
}

#[test]
fn scrolling_the_bag_shows_what_the_cartridge_shows_at_every_poll() {
    use Joypad as J;
    compare(&[J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::DOWN, J::UP, J::UP, J::UP, J::UP, J::A]);
}

/// SELECT arms an entry and a second SELECT commits it, both of which cost twenty frames the
/// cartridge spends before it redraws.
#[test]
fn swapping_two_entries_shows_what_the_cartridge_shows_at_every_poll() {
    use Joypad as J;
    compare(&[J::SELECT, J::DOWN, J::SELECT, J::B]);
}

#[test]
fn cancelling_the_bag_shows_what_the_cartridge_shows_at_every_poll() {
    compare(&[Joypad::DOWN, Joypad::B]);
}

/// Held to a frame, as the text box's `▼` is, on `HandleMenuInput`'s own loop rate.
#[test]
fn the_list_s_down_arrow_blinks_within_a_frame_of_the_cartridge() {
    let (mut gb, _) = open_the_bag();
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::ListMenu(the_list(&gb)));
    to_vblank(&mut gb);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::List);
    let toggles = |arrow: &mut dyn FnMut() -> u8| {
        let mut last = arrow();
        (0..400).filter(|_| {
            let now = arrow();
            std::mem::replace(&mut last, now) != now
        }).collect::<Vec<u32>>()
    };
    let cartridge = toggles(&mut || { to_vblank(&mut gb); tile_row(&gb, 11)[18] });
    let recreation = toggles(&mut || { game.frame(Input::None); game.ui().row(11)[18] });
    assert_eq!(cartridge.len(), recreation.len(), "{cartridge:?} against {recreation:?}");
    for (c, r) in cartridge.iter().zip(&recreation) {
        assert!(c.abs_diff(*r) <= 1, "{cartridge:?} against {recreation:?}");
    }
}
