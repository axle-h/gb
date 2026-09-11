use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::list_menu::ListMenu;
use pokered::rng::GameRng;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, tile_row, to_vblank};

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

fn joypad(buttons: Joypad) -> JoypadButtonState {
    JoypadButtonState {
        a: buttons.contains(Joypad::A),
        b: buttons.contains(Joypad::B),
        up: buttons.contains(Joypad::UP),
        down: buttons.contains(Joypad::DOWN),
        ..Default::default()
    }
}

/// Frames until the cartridge's `HandleMenuInput` polls, finishing that frame.
fn cartridge_until_polling(gb: &mut GameBoy) -> u32 {
    let poll = breakpoint(pokered_symbols::JoypadLowSensitivity);
    let vblank = breakpoint(pokered_symbols::VBlank);
    for frames in 0..600 {
        loop {
            let (stop, _) = gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 2);
            if stop == Stop::Breakpoint(poll) {
                to_vblank(gb);
                return frames + 1;
            }
            if stop == Stop::Breakpoint(vblank) {
                break;
            }
            panic!("{stop:?}");
        }
    }
    panic!("the cartridge never polled");
}

fn recreation_until_polling(game: &mut Game) -> u32 {
    for frames in 1..600 {
        game.frame(Input::None);
        if game.status() == Status::Waiting(Decision::List) {
            return frames;
        }
    }
    panic!("the recreation never polled");
}

/// The same box wherever both poll. The cartridge may get there late, never early: printing the
/// entries can outlast a frame, and lag frames are not modelled.
fn compare(presses: &[Joypad]) {
    let (mut gb, returns) = open_the_bag();
    let menu = the_list(&gb);
    assert!(menu.len() > 4, "the fixture's bag has to scroll");
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::ListMenu(menu));
    to_vblank(&mut gb);
    assert_eq!(cartridge_box(&gb), recreation_box(&game), "the box as drawn");

    let (last, presses) = presses.split_last().unwrap();
    for (step, &press) in presses.iter().chain([last]).enumerate() {
        let (cartridge, recreation) = (cartridge_until_polling(&mut gb), recreation_until_polling(&mut game));
        assert_eq!(cartridge_box(&gb), recreation_box(&game), "polling before press {step}");
        assert!((0..=2).contains(&(cartridge as i64 - recreation as i64)),
            "before press {step} the cartridge took {cartridge} frames and the recreation {recreation}");
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
    recreation_until_polling(&mut game);
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
