//! `PlaceString`: charmap bytes onto the screen a letter at a time, and every control character in
//! the dictionary `PlaceNextChar` checks. The text commands above it call this and get `bc` back.

use serde::{Deserialize, Serialize};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::modes::blink::ArrowBlink;
use crate::Pacing;

/// `<POKE>` and `<PKMN>` are one byte in the cartridge's text and several tiles on the screen.
pub const POKE_TILES: [u8; 4] = [0x8F, 0x8E, 0x8A, 0xBA];
pub const PKMN_TILES: [u8; 2] = [0xE1, 0xE2];

/// The tiles a byte draws as, for the one caller that is not pacing a string: `None` is a byte that
/// draws as the tile it names.
pub fn ligature(byte: u8) -> Option<&'static [u8]> {
    match byte {
        ch::POKE => Some(&POKE_TILES),
        ch::PKMN => Some(&PKMN_TILES),
        _ => None,
    }
}

pub mod ch {
    pub const PAGE: u8 = 0x49;
    pub const PKMN: u8 = 0x4A;
    pub const CONT_: u8 = 0x4B;
    pub const SCROLL: u8 = 0x4C;
    pub const NEXT: u8 = 0x4E;
    pub const LINE: u8 = 0x4F;
    pub const TERMINATOR: u8 = 0x50;
    pub const PARA: u8 = 0x51;
    pub const PLAYER: u8 = 0x52;
    pub const RIVAL: u8 = 0x53;
    pub const POKE: u8 = 0x54;
    pub const CONT: u8 = 0x55;
    pub const SIX_DOTS: u8 = 0x56;
    pub const DONE: u8 = 0x57;
    pub const PROMPT: u8 = 0x58;
    pub const PC: u8 = 0x5B;
    pub const TM: u8 = 0x5C;
    pub const TRAINER: u8 = 0x5D;
    pub const ROCKET: u8 = 0x5E;
    pub const DEXEND: u8 = 0x5F;
    pub const PERIOD: u8 = 0xE8;
    pub const DOWN_ARROW: u8 = 0xEE;
}

pub const fn coord(x: usize, y: usize) -> u16 {
    (y * SCREEN_TILES_X + x) as u16
}

pub const ARROW: u16 = coord(18, 16);
pub const FIRST_LINE: u16 = coord(1, 14);
pub const SECOND_LINE: u16 = coord(1, 16);

/// `WaitForTextScrollButtonPress`'s iterations a frame, in hundredths.
const BLINK_PER_FRAME: u32 = 4454;

/// `PrintLetterDelay`, for anything that places a tile and then waits as a letter does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LetterDelay {
    /// Waiting for `hFrameCounter`.
    Counting,
    /// A or B was held, which waits one frame instead.
    NextFrame,
}

impl LetterDelay {
    /// The call, up to its first check of the buttons. `None` when there is nothing to wait for.
    pub fn start(ctx: &mut Ctx) -> Option<Self> {
        if ctx.pacing == Pacing::Instant || ctx.world.no_text_delay {
            return None;
        }
        *ctx.frame_counter = if ctx.world.one_frame_letter_delay { 1 } else { ctx.world.options.text_speed as u8 };
        Some(Self::check_buttons(ctx))
    }

    /// One frame of the wait. True when it is over, and the caller carries on in this frame.
    pub fn update(&mut self, ctx: &mut Ctx) -> bool {
        match self {
            Self::NextFrame => true,
            Self::Counting => {
                *self = Self::check_buttons(ctx);
                *self == Self::Counting && *ctx.frame_counter == 0
            }
        }
    }

    fn check_buttons(ctx: &mut Ctx) -> Self {
        ctx.pad.poll();
        if ctx.pad.held.intersects(Joypad::A | Joypad::B) { Self::NextFrame } else { Self::Counting }
    }
}

/// How `PlaceString` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Printed {
    /// `@`: back to the command that called it, with `hl` as the new `bc`.
    Returned(u16),
    /// `<DONE>`, or `<PROMPT>` falling into it: the whole script is over.
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceString {
    /// Charmap bytes; `<PLAYER>` and the like are spliced in as they are reached.
    text: Vec<u8>,
    cursor: usize,
    /// `hl`: where the next letter goes, as an index into the tile grid.
    at: u16,
    /// `PlaceString`'s pushed `hl`, which `<NEXT>` steps from.
    line: u16,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// Nothing is waiting: carry on printing this frame.
    Running,
    Letter(LetterDelay),
    /// `ManualTextScroll` with the `▼` up. The `ProtectedDelay3` before it is loading and not
    /// modelled, so the pad is read in the frame the arrow is drawn.
    Prompting { pause: Pause, blink: ArrowBlink },
    /// `ScrollTextUpOneLine` has run `scrolled` times and is in its five frames.
    Scrolling { scrolled: u8, frames: u8 },
    /// `Paragraph` and `PageChar`'s 20 frames after clearing.
    Cleared(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Pause {
    Prompt,
    Paragraph,
    Page,
    Cont,
}

impl PlaceString {
    /// The call: `de` is the text and `hl` the tile it starts at, which is also the pushed line.
    pub fn call(text: Vec<u8>, at: u16) -> Self {
        Self { text, cursor: 0, at, line: at, phase: Phase::Running }
    }

    pub fn put(ui: &mut UiSurface, at: u16, tile: u8) {
        ui.set(at as usize % SCREEN_TILES_X, at as usize / SCREEN_TILES_X, tile);
    }

    /// One frame; `Some` when the cartridge's `PlaceString` would return. `answered` counts the
    /// prompts a driver has seen land, and it outlives any one string.
    pub fn update(&mut self, ctx: &mut Ctx, answered: &mut u32) -> Option<Printed> {
        match self.phase {
            Phase::Running => self.run(ctx, answered),
            Phase::Cleared(frames) if frames > 1 => {
                self.phase = Phase::Cleared(frames - 1);
                None
            }
            Phase::Cleared(_) => self.run(ctx, answered),
            Phase::Letter(mut delay) => {
                if delay.update(ctx) {
                    self.run(ctx, answered)
                } else {
                    self.phase = Phase::Letter(delay);
                    None
                }
            }
            Phase::Prompting { pause, mut blink } => {
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    return self.answer(ctx, pause, answered);
                }
                if let Some(shown) = blink.tick(BLINK_PER_FRAME) {
                    Self::put(&mut ctx.screen.ui, ARROW, if shown { ch::DOWN_ARROW } else { UiSurface::BLANK });
                }
                self.phase = Phase::Prompting { pause, blink };
                None
            }
            Phase::Scrolling { scrolled, frames } if frames > 1 => {
                self.phase = Phase::Scrolling { scrolled, frames: frames - 1 };
                None
            }
            Phase::Scrolling { scrolled: 1, .. } => {
                Self::scroll_up_one_line(&mut ctx.screen.ui);
                self.phase = Phase::Scrolling { scrolled: 2, frames: self.delay(ctx, 5) };
                self.after_delay(ctx, answered)
            }
            Phase::Scrolling { .. } => {
                self.at = SECOND_LINE;
                self.run(ctx, answered)
            }
        }
    }

    /// Whether a prompt is waiting on the player, which is the mode's `Waiting(Decision::Text)`.
    pub fn is_prompting(&self) -> bool {
        matches!(self.phase, Phase::Prompting { .. })
    }

    /// `PlaceNextChar` until something waits a frame.
    fn run(&mut self, ctx: &mut Ctx, answered: &mut u32) -> Option<Printed> {
        self.phase = Phase::Running;
        loop {
            let Some(&byte) = self.text.get(self.cursor) else { return Some(Printed::Returned(self.at)) };
            self.cursor += 1;
            match byte {
                ch::TERMINATOR => return Some(Printed::Returned(self.at)),
                ch::DONE => return Some(Printed::Ended),
                ch::NEXT => {
                    self.line += 2 * SCREEN_TILES_X as u16;
                    self.at = self.line;
                }
                ch::LINE => {
                    self.line = SECOND_LINE;
                    self.at = SECOND_LINE;
                }
                ch::PARA => return self.arrow(ctx, Pause::Paragraph, answered),
                ch::PAGE => return self.arrow(ctx, Pause::Page, answered),
                ch::CONT | ch::CONT_ => return self.arrow(ctx, Pause::Cont, answered),
                ch::PROMPT => return self.arrow(ctx, Pause::Prompt, answered),
                ch::SCROLL => return self.scroll(ctx, answered),
                ch::DEXEND => {
                    Self::put(&mut ctx.screen.ui, self.at, ch::PERIOD);
                    return Some(Printed::Ended);
                }
                ch::PLAYER => self.splice(&ctx.world.player_name.clone()),
                ch::RIVAL => self.splice(&ctx.world.rival_name.clone()),
                ch::POKE => self.splice(&POKE_TILES),
                ch::PKMN => self.splice(&PKMN_TILES),
                ch::PC => self.splice(&[0x8F, 0x82]),
                ch::TM => self.splice(&[0x93, 0x8C]),
                ch::TRAINER => self.splice(&[0x93, 0x91, 0x80, 0x88, 0x8D, 0x84, 0x91]),
                ch::ROCKET => self.splice(&[0x91, 0x8E, 0x82, 0x8A, 0x84, 0x93]),
                ch::SIX_DOTS => self.splice(&[0x75, 0x75]),
                0x00 | 0x59 | 0x5A => panic!("text byte ${byte:02X} is not supported outside battle yet"),
                letter => {
                    Self::put(&mut ctx.screen.ui, self.at, letter);
                    self.at += 1;
                    if let Some(delay) = LetterDelay::start(ctx) {
                        self.phase = Phase::Letter(delay);
                        return None;
                    }
                }
            }
        }
    }

    /// A command character's string, printed as if it were the text itself.
    fn splice(&mut self, bytes: &[u8]) {
        self.text.splice(self.cursor..self.cursor, bytes.iter().copied());
    }

    fn arrow(&mut self, ctx: &mut Ctx, pause: Pause, answered: &mut u32) -> Option<Printed> {
        Self::put(&mut ctx.screen.ui, ARROW, ch::DOWN_ARROW);
        self.phase = Phase::Prompting { pause, blink: ArrowBlink::default() };
        self.update(ctx, answered)
    }

    fn scroll(&mut self, ctx: &mut Ctx, answered: &mut u32) -> Option<Printed> {
        Self::scroll_up_one_line(&mut ctx.screen.ui);
        self.phase = Phase::Scrolling { scrolled: 1, frames: self.delay(ctx, 5) };
        self.after_delay(ctx, answered)
    }

    /// A delay that is presentation, which `Instant` pacing skips.
    fn delay(&self, ctx: &Ctx, frames: u8) -> u8 {
        if ctx.pacing == Pacing::Instant { 0 } else { frames }
    }

    /// A delay of zero frames is over at once.
    fn after_delay(&mut self, ctx: &mut Ctx, answered: &mut u32) -> Option<Printed> {
        match self.phase {
            Phase::Scrolling { frames: 0, .. } | Phase::Cleared(0) =>
                self.update(ctx, answered),
            _ => None,
        }
    }

    /// `ScrollTextUpOneLine`: rows 14 to 16 move up one, and row 16 is blanked.
    pub fn scroll_up_one_line(ui: &mut UiSurface) {
        for y in 13..16 {
            for x in 0..SCREEN_TILES_X {
                ui.set(x, y, ui.get(x, y + 1));
            }
        }
        ui.fill(1, 16, SCREEN_TILES_X - 2, 1, UiSurface::BLANK);
    }

    fn answer(&mut self, ctx: &mut Ctx, pause: Pause, answered: &mut u32) -> Option<Printed> {
        *answered += 1;
        // `ManualTextScroll` plays it after the press.
        ctx.audio.play_sound(crate::audio::data::sounds::SFX_PRESS_AB);
        match pause {
            // `PromptText` falls into `DoneText`, which ends the whole text.
            Pause::Prompt => {
                Self::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                Some(Printed::Ended)
            }
            Pause::Paragraph => {
                ctx.screen.ui.fill(1, 13, 18, 4, UiSurface::BLANK);
                self.at = FIRST_LINE;
                self.line = FIRST_LINE;
                self.phase = Phase::Cleared(self.delay(ctx, 20));
                self.after_delay(ctx, answered)
            }
            Pause::Page => {
                ctx.screen.ui.fill(1, 10, 18, 7, UiSurface::BLANK);
                self.at = coord(1, 11);
                self.line = self.at;
                self.phase = Phase::Cleared(self.delay(ctx, 20));
                self.after_delay(ctx, answered)
            }
            Pause::Cont => {
                Self::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                self.scroll(ctx, answered)
            }
        }
    }
}
