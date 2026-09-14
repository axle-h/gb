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
use super::{breakpoint, cartridge_until_polling, recreation_until_polling, tile_row, to_vblank, DELAY3};

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

/// Every screen one side shows, with the frame it first shows on, and the frames it pressed A on.
#[derive(Default)]
struct Timeline {
    screens: Vec<(u32, Vec<Vec<u8>>)>,
    answers: Vec<u32>,
}

impl Timeline {
    fn saw(&mut self, frame: u32, rows: Vec<Vec<u8>>) {
        if self.screens.last().is_none_or(|(_, last)| *last != rows) {
            self.screens.push((frame, rows));
        }
    }
}

/// The recreation printing on its own, answering each prompt `wait` frames after it starts waiting.
fn recreation_timeline(game: &mut Game, wait: u32) -> Timeline {
    let mut timeline = Timeline::default();
    timeline.saw(0, game_rows(game));
    let mut waited = 0;
    for frame in 1..2_000 {
        let press = game.status() == Status::Waiting(Decision::Text) && {
            waited += 1;
            waited > wait
        };
        if press {
            waited = 0;
            timeline.answers.push(frame);
        }
        game.frame(Input::Buttons(if press { Joypad::A } else { Joypad::empty() }));
        timeline.saw(frame, game_rows(game));
        if game.modes().is_empty() {
            return timeline;
        }
    }
    panic!("the text never finished");
}

/// The cartridge on its own to frame `until`, answering at most `answers` prompts, each `wait` frames
/// after it first reads the pad for one. Frame 0 is the one `PrintText` is stopped in.
fn cartridge_timeline(gb: &mut GameBoy, wait: u32, answers: usize, until: u32) -> Timeline {
    let (poll, vblank) = (breakpoint(pokered_symbols::JoypadLowSensitivity), breakpoint(pokered_symbols::VBlank));
    to_vblank(gb);
    let mut timeline = Timeline::default();
    timeline.saw(0, text_box_rows(gb));
    let (mut waiting, mut waited) = (false, 0);
    for frame in 1..=until {
        let press = waiting && timeline.answers.len() < answers && {
            waited += 1;
            waited > wait
        };
        if press {
            waited = 0;
            timeline.answers.push(frame);
        }
        gb.hold_buttons(JoypadButtonState { a: press, ..Default::default() });
        waiting = false;
        loop {
            match gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 2).0 {
                // A poll that took the press has answered; one that did not is still waiting.
                Stop::Breakpoint(hit) if hit == poll => waiting = !press,
                Stop::Breakpoint(_) => break,
                stop => panic!("{stop:?}"),
            }
        }
        timeline.saw(frame, text_box_rows(gb));
    }
    timeline
}

/// Both print the same screens in the same order, the cartridge late by exactly the loading the
/// recreation leaves out: `PrintText`'s `Delay3` from the start, and `prompt` more for every prompt
/// answered, which is `ProtectedDelay3` at a `▼` and nothing at `TX_PROMPT_BUTTON`. The empty box the
/// cartridge shows through that first `Delay3` is the one screen the recreation never draws.
fn compare_timelines(mut gb: GameBoy, mut game: Game, wait: u32, prompt: u32) -> usize {
    let recreation = recreation_timeline(&mut game, wait);
    let offset = |answered: usize| DELAY3 + prompt * answered as u32;
    let (last, _) = *recreation.screens.last().expect("the recreation drew something");
    let until = last + offset(recreation.answers.len());
    let cartridge = cartridge_timeline(&mut gb, wait, recreation.answers.len(), until);

    let (loading, screens) = cartridge.screens.split_first().expect("the cartridge drew something");
    assert_eq!(loading.0, 0, "the box goes up in the frame PrintText is called");
    assert!(loading.1[1..5].iter().all(|row| row[1..19].iter().all(|&tile| tile == 0x7F)),
        "and nothing is printed in it through its Delay3");
    for (i, ((theirs, their_rows), (ours, our_rows))) in screens.iter().zip(&recreation.screens).enumerate() {
        let answered = recreation.answers.iter().filter(|&&answer| answer <= *ours).count();
        assert_eq!(their_rows, our_rows, "screen {i}, at cartridge frame {theirs} and recreation frame {ours}");
        assert_eq!(*theirs, ours + offset(answered), "screen {i} after {answered} prompts");
    }
    assert_eq!(screens.len(), recreation.screens.len(), "how many screens");
    assert_eq!(cartridge.answers.len(), recreation.answers.len(), "how many prompts");
    println!("{} screens and {} prompts, the cartridge {} frames late by the end",
        screens.len(), recreation.answers.len(), offset(recreation.answers.len()));
    screens.len()
}

#[test]
fn oaks_first_text_prints_on_the_same_frames_as_the_cartridge() {
    let gb = boot_to_oaks_first_text();
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::TextBox(TextBox::new(oaks_first_text())));
    compare_timelines(gb, game, 0, DELAY3);
}

/// The blink wanders a frame on the cartridge, so it is held to one (see `pokered`'s `ArrowBlink`).
#[test]
fn a_prompt_left_waiting_blinks_within_a_frame_of_the_cartridge() {
    let mut gb = boot_to_oaks_first_text();
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::TextBox(TextBox::new(oaks_first_text())));
    to_vblank(&mut gb);
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Text);
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
    let screens = compare_timelines(gb, game, 0, 0);
    assert!(screens > 5, "the script ended before it printed anything");
}
