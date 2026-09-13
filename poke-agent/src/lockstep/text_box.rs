use std::collections::BTreeMap;
use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::RAM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::text_box::TextBox;
use pokered::rng::GameRng;
use poke_core::charmap::encode;
use poke_core::text_script::{decode_slice, TextNumber};
use pokered::world::{TextVars, World};
use pokered::{Game, Input, Pacing};
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::symbols::pokered_symbols;
use super::{breakpoint, tile_row, to_vblank};

const TX_START: u8 = 0x00;
const PROMPT: u8 = 0x58;

/// Mashes A from power-on to Oak's first `PrintText`, and leaves the machine on it.
pub(super) fn boot_to_oaks_first_text() -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    let print_text = breakpoint(pokered_symbols::PrintText);
    for frame in 0..10_000 {
        gb.hold_buttons(JoypadButtonState { a: frame % 16 < 2, ..Default::default() });
        if gb.run_until(&[print_text], MachineCycles::PER_FRAME).0 == Stop::Breakpoint(print_text) {
            gb.hold_buttons(JoypadButtonState::default());
            return gb;
        }
    }
    panic!("never reached PrintText");
}

/// `_OakSpeechText1`'s one string, up to and including its `<PROMPT>`.
fn oaks_first_text() -> Vec<u8> {
    let bytes = rom_slice(pokered_symbols::_OakSpeechText1);
    assert_eq!(bytes[0], TX_START);
    let end = bytes.iter().position(|&b| b == PROMPT).expect("the text ends on a prompt");
    bytes[1..=end].to_vec()
}

fn text_box_rows(gb: &GameBoy) -> Vec<Vec<u8>> {
    (12..18).map(|y| tile_row(gb, y)).collect()
}

fn game_rows(game: &Game) -> Vec<Vec<u8>> {
    (12..18).map(|y| game.ui().row(y).to_vec()).collect()
}

/// Oak's first box, printed by both, answered at each prompt `wait` frames after it starts waiting.
fn compare_oaks_first_text(wait: u64) {
    let mut gb = boot_to_oaks_first_text();
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::TextBox(TextBox::new(oaks_first_text())));

    to_vblank(&mut gb);
    assert_eq!(text_box_rows(&gb), game_rows(&game), "the box as drawn");

    let mut waited = 0;
    for frame in 1..2_000 {
        let press = game.status() == Status::Waiting(Decision::Text) && {
            waited += 1;
            waited > wait
        };
        if press {
            waited = 0;
        }
        let buttons = if press { Joypad::A } else { Joypad::empty() };
        gb.hold_buttons(JoypadButtonState { a: press, ..Default::default() });
        to_vblank(&mut gb);
        game.frame(Input::Buttons(buttons));
        if game.modes().is_empty() {
            return;
        }
        assert_eq!(text_box_rows(&gb), game_rows(&game), "frame {frame}");
    }
    panic!("the text never finished");
}

#[test]
fn oaks_first_text_prints_on_the_same_frames_as_the_cartridge() {
    compare_oaks_first_text(0);
}

/// The blink wanders a frame on the cartridge, so it is held to one (see `pokered`'s `ArrowBlink`).
#[test]
fn a_prompt_left_waiting_blinks_within_a_frame_of_the_cartridge() {
    let mut gb = boot_to_oaks_first_text();
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::TextBox(TextBox::new(oaks_first_text())));
    to_vblank(&mut gb);
    while game.status() != Status::Waiting(Decision::Text) {
        to_vblank(&mut gb);
        game.frame(Input::None);
    }
    let toggles = |arrow: &mut dyn FnMut() -> u8| {
        let mut last = arrow();
        (0..800).filter(|_| {
            let now = arrow();
            std::mem::replace(&mut last, now) != now
        }).collect::<Vec<u32>>()
    };
    let cartridge = toggles(&mut || { to_vblank(&mut gb); tile_row(&gb, 16)[18] });
    let recreation = toggles(&mut || { game.frame(Input::None); game.ui().row(16)[18] });
    assert_eq!(cartridge.len(), recreation.len(), "{cartridge:?} against {recreation:?}");
    for (c, r) in cartridge.iter().zip(&recreation) {
        assert!(c.abs_diff(*r) <= 1, "{cartridge:?} against {recreation:?}");
    }
}

/// `TX_START`, `TX_NUM`, `TX_PROMPT_BUTTON` and `TX_END`, as the assembler would lay them out.
fn command_script() -> Vec<u8> {
    let mut bytes = vec![0x00];
    bytes.extend(encode("Lv").unwrap());
    bytes.push(0x50);
    bytes.push(0x09);
    bytes.extend(pokered_symbols::wCurEnemyLevel.address.to_le_bytes());
    bytes.push(0x13); // one byte, three digits
    bytes.push(0x00);
    bytes.extend(encode("!").unwrap());
    bytes.push(0x50);
    bytes.push(0x06);
    bytes.push(0x00);
    bytes.extend(encode("OK").unwrap());
    bytes.push(0x50);
    bytes.push(0x50);
    bytes
}

/// A script the cartridge never carried, put where it can read one. `TextCommandProcessor` runs
/// from RAM as readily as from ROM, so pointing `PrintText` at the box data is what lets a
/// comparison choose which commands the cartridge runs: these four are in no text it ships.
#[test]
fn a_script_of_commands_prints_on_the_same_frames_as_the_cartridge() {
    const LEVEL: u8 = 7;
    let mut gb = boot_to_oaks_first_text();
    let script = command_script();
    let at = pokered_symbols::wBoxMonNicks;
    for (i, &byte) in script.iter().enumerate() {
        gb.core_mut().mmu_mut().write(at.address + i as u16, byte);
    }
    gb.core_mut().mmu_mut().write(pokered_symbols::wCurEnemyLevel.address, LEVEL);
    gb.core_mut().registers_mut().set_hl(at.address);

    let text = TextVars {
        numbers: BTreeMap::from([(TextNumber::CurEnemyLevel, LEVEL as u32)]),
        ..TextVars::default()
    };
    let mut game = Game::new(World { text, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::TextBox(TextBox::script(decode_slice(&script, at).unwrap())));

    to_vblank(&mut gb);
    assert_eq!(text_box_rows(&gb), game_rows(&game), "the box as drawn");
    for frame in 1..2_000 {
        let press = game.status() == Status::Waiting(Decision::Text);
        gb.hold_buttons(JoypadButtonState { a: press, ..Default::default() });
        to_vblank(&mut gb);
        game.frame(Input::Buttons(if press { Joypad::A } else { Joypad::empty() }));
        if game.modes().is_empty() {
            assert!(frame > 10, "the script ended before it printed anything");
            return;
        }
        assert_eq!(text_box_rows(&gb), game_rows(&game), "frame {frame}");
    }
    panic!("the script never finished");
}
