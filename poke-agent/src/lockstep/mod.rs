//! The recreation and the emulator fed the same buttons and compared.

mod audio;
mod battle;
mod battle_animations;
mod events;
mod evolution;
mod field_move_menu;
mod item_menu;
mod learn_move;
mod list_menu;
mod main_menu;
mod movie;
mod naming_screen;
mod overworld;
mod party_menu;
mod pc;
mod pokedex;
mod pokemart;
mod save;
mod screen;
mod scripts;
mod start_menu;
mod status_screen;
mod synth;
mod text_box;
mod trainer_card;
mod two_option_menu;

use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::cycles::MachineCycles;
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::input::Joypad;
use pokered::command::Decision;
use pokered::mode::Status;
use pokered::{Game, Input};
use crate::pokemon::symbols::{DmgBank, DmgPointer};

pub(crate) fn breakpoint(pointer: DmgPointer) -> Breakpoint {
    match pointer.bank {
        DmgBank::ROM { bank } => Breakpoint::new(bank, pointer.address),
        _ => Breakpoint::new(0, pointer.address),
    }
}

/// Runs to the start of the next VBlank handler, when the frame before it has done its work.
pub(crate) fn to_vblank(gb: &mut GameBoy) {
    let vblank = breakpoint(crate::pokemon::symbols::pokered_symbols::VBlank);
    let (stop, _) = gb.run_until(&[vblank], MachineCycles::PER_FRAME * 2);
    assert_eq!(stop, Stop::Breakpoint(vblank), "no VBlank within two frames");
}

pub(crate) fn tile_row(gb: &GameBoy, y: u16) -> Vec<u8> {
    let tile_map = crate::pokemon::symbols::pokered_symbols::wTileMap.address;
    (0..20).map(|x| gb.core().mmu().read(tile_map + y * 20 + x)).collect()
}

/// Frames until the cartridge's menu loop polls, finishing that frame.
pub(crate) fn cartridge_until_polling(gb: &mut GameBoy) -> u32 {
    let poll = breakpoint(crate::pokemon::symbols::pokered_symbols::JoypadLowSensitivity);
    let vblank = breakpoint(crate::pokemon::symbols::pokered_symbols::VBlank);
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

/// `Delay3`: the cartridge waiting for what it has just drawn to reach the screen, which is loading
/// and which the recreation leaves out.
pub(crate) const DELAY3: u32 = 3;
/// `HandleMenuInput`'s, after it places the cursor.
pub(crate) const CURSOR: u32 = DELAY3;
/// `PrintText`'s, after it draws the box.
pub(crate) const BOX: u32 = DELAY3;
/// `ProtectedDelay3`, at a `▼`.
pub(crate) const ARROW: u32 = DELAY3;
/// `DisplayListMenuID`: the `DelayFrames 10` after its box, the `Delay3` after its entries, and the
/// cursor's.
pub(crate) const LIST: u32 = 10 + DELAY3 + CURSOR;
/// A list printed again, which scrolling, paging and swapping do.
pub(crate) const LIST_REDRAWN: u32 = DELAY3 + CURSOR;

/// A text a press opens. The press is still held for the first letter's `PrintLetterDelay`, which
/// the box's `Delay3` kept it clear of on the cartridge, so that letter waits a frame rather than
/// the text speed.
pub(crate) fn hurried_letter(game: &Game) -> u32 {
    game.world().options.text_speed as u32 - 1
}

/// The cartridge reaches a point both agree on late by the loading the recreation leaves out on the
/// way plus up to two lag frames, and never early. A recreation that is waiting already reads a
/// frame long, because `recreation_until_polling` always runs one.
pub(crate) fn assert_late(cartridge: u32, recreation: u32, loading: u32, what: &str) {
    let late = cartridge as i64 - recreation as i64;
    let allowed = (loading as i64 - 1).max(0)..=loading as i64 + 2;
    assert!(allowed.contains(&late),
        "{what}: the cartridge took {cartridge} frames and the recreation {recreation}, {late} late for {loading} of loading");
}

pub(crate) fn recreation_until_polling(game: &mut Game, decision: Decision) -> u32 {
    for frames in 1..600 {
        game.frame(Input::None);
        if game.status() == Status::Waiting(decision.clone()) {
            return frames;
        }
    }
    panic!("the recreation never polled");
}

/// The Celadon fixture with the start menu open and the buttons let go. START is held until the
/// menu actually opens: a single frame of it can fall between the overworld's own polls.
pub(crate) fn open_the_start_menu() -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/at-celadon.bin")).unwrap();
    gb.run(MachineCycles::PER_FRAME * 60);
    gb.hold_buttons(JoypadButtonState { start: true, ..Default::default() });
    let menu = breakpoint(crate::pokemon::symbols::pokered_symbols::DisplayStartMenu);
    let (stop, _) = gb.run_until(&[menu], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(menu), "START never opened the menu");
    gb.hold_buttons(JoypadButtonState::default());
    gb
}

/// Steps the cartridge's menu cursor to `row`, a press at a time, and leaves it polling there.
pub(crate) fn cartridge_cursor_to(gb: &mut GameBoy, row: u8) {
    use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
    for _ in 0..12 {
        cartridge_until_polling(gb);
        let current = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        if current == row {
            return;
        }
        let press = if current < row { Joypad::DOWN } else { Joypad::UP };
        gb.hold_buttons(joypad(press));
        to_vblank(gb);
        gb.hold_buttons(JoypadButtonState::default());
    }
    panic!("the cursor never reached row {row}");
}

/// Every button, not just the four a cursor needs: a press this drops is one the cartridge never
/// sees, and the recreation acting on it alone looks like a timing bug rather than a missing wire.
pub(crate) fn joypad(buttons: Joypad) -> JoypadButtonState {
    JoypadButtonState {
        a: buttons.contains(Joypad::A),
        b: buttons.contains(Joypad::B),
        select: buttons.contains(Joypad::SELECT),
        start: buttons.contains(Joypad::START),
        up: buttons.contains(Joypad::UP),
        down: buttons.contains(Joypad::DOWN),
        left: buttons.contains(Joypad::LEFT),
        right: buttons.contains(Joypad::RIGHT),
    }
}

/// Presses A and waits for whatever comes next to start polling, however long it takes. The party
/// menu loads its icons with the LCD off, so this cannot be counted in frames.
pub(crate) fn press_a_and_settle(gb: &mut GameBoy, budget: u64) {
    gb.hold_buttons(joypad(Joypad::A));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let poll = breakpoint(crate::pokemon::symbols::pokered_symbols::JoypadLowSensitivity);
    let (stop, _) = gb.run_until(&[poll], MachineCycles::PER_FRAME * budget);
    assert_eq!(stop, Stop::Breakpoint(poll), "nothing ever polled again");
    to_vblank(gb);
}
