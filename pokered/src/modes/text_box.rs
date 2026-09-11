//! `PrintText` for a message box: `TextBoxBorder`, then `PlaceString` a letter at a time with
//! `PrintLetterDelay`, and `ManualTextScroll` at every `▼`.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::blink::ArrowBlink;
use crate::Pacing;

mod ch {
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

const fn coord(x: usize, y: usize) -> u16 {
    (y * SCREEN_TILES_X + x) as u16
}

const ARROW: u16 = coord(18, 16);
const FIRST_LINE: u16 = coord(1, 14);
const SECOND_LINE: u16 = coord(1, 16);

/// `WaitForTextScrollButtonPress`'s iterations a frame, in hundredths.
const BLINK_PER_FRAME: u32 = 4454;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextBox {
    /// Charmap bytes; `<PLAYER>` and the like are spliced in as they are reached.
    text: Vec<u8>,
    cursor: usize,
    /// `hl`: where the next letter goes, as an index into the tile grid.
    at: u16,
    /// `PlaceString`'s pushed `hl`, which `<NEXT>` steps from.
    line: u16,
    phase: Phase,
    /// Prompts answered so far, so a driver can see its press land.
    answered: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// `PrintText`'s `Delay3` after drawing the box.
    Opening(u8),
    /// `PrintLetterDelay`, waiting for `hFrameCounter`.
    LetterDelay,
    /// `PrintLetterDelay` saw A or B held and waits one frame.
    LetterNextFrame,
    /// `ProtectedDelay3` with the `▼` up.
    ArrowDelay(u8, Pause),
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

impl TextBox {
    pub fn new(text: Vec<u8>) -> Self {
        Self { text, cursor: 0, at: FIRST_LINE, line: FIRST_LINE, phase: Phase::Opening(3), answered: 0 }
    }

    pub fn answered(&self) -> u32 {
        self.answered
    }

    fn put(ui: &mut UiSurface, at: u16, tile: u8) {
        ui.set(at as usize % SCREEN_TILES_X, at as usize / SCREEN_TILES_X, tile);
    }

    /// `PlaceNextChar` until something waits a frame; the whole text finishing pops the box.
    fn run(&mut self, ctx: &mut Ctx) -> Transition {
        loop {
            let Some(&byte) = self.text.get(self.cursor) else { return Transition::Pop(Outcome::Done) };
            self.cursor += 1;
            match byte {
                ch::TERMINATOR | ch::DONE => return Transition::Pop(Outcome::Done),
                ch::NEXT => {
                    self.line += 2 * SCREEN_TILES_X as u16;
                    self.at = self.line;
                }
                ch::LINE => {
                    self.line = SECOND_LINE;
                    self.at = SECOND_LINE;
                }
                ch::PARA => return self.arrow(ctx, Pause::Paragraph),
                ch::PAGE => return self.arrow(ctx, Pause::Page),
                ch::CONT | ch::CONT_ => return self.arrow(ctx, Pause::Cont),
                ch::PROMPT => return self.arrow(ctx, Pause::Prompt),
                ch::SCROLL => return self.scroll(ctx),
                ch::DEXEND => {
                    Self::put(&mut ctx.screen.ui, self.at, ch::PERIOD);
                    return Transition::Pop(Outcome::Done);
                }
                ch::PLAYER => self.splice(&ctx.world.player_name.clone()),
                ch::RIVAL => self.splice(&ctx.world.rival_name.clone()),
                ch::POKE => self.splice(&[0x8F, 0x8E, 0x8A, 0xBA]),
                ch::PKMN => self.splice(&[0xE1, 0xE2]),
                ch::PC => self.splice(&[0x8F, 0x82]),
                ch::TM => self.splice(&[0x93, 0x8C]),
                ch::TRAINER => self.splice(&[0x93, 0x91, 0x80, 0x88, 0x8D, 0x84, 0x91]),
                ch::ROCKET => self.splice(&[0x91, 0x8E, 0x82, 0x8A, 0x84, 0x93]),
                ch::SIX_DOTS => self.splice(&[0x75, 0x75]),
                0x00 | 0x59 | 0x5A => panic!("text byte ${byte:02X} is not supported outside battle yet"),
                letter => {
                    Self::put(&mut ctx.screen.ui, self.at, letter);
                    self.at += 1;
                    if self.letter_delay(ctx) {
                        return Transition::Stay;
                    }
                }
            }
        }
    }

    /// A command character's string, printed as if it were the text itself.
    fn splice(&mut self, bytes: &[u8]) {
        self.text.splice(self.cursor..self.cursor, bytes.iter().copied());
    }

    /// `PrintLetterDelay`, up to its first check of the buttons. True when it waits.
    fn letter_delay(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.pacing == Pacing::Instant || ctx.world.no_text_delay {
            return false;
        }
        *ctx.frame_counter = ctx.world.options.text_speed as u8;
        self.phase = self.check_letter_buttons(ctx);
        true
    }

    fn check_letter_buttons(&self, ctx: &mut Ctx) -> Phase {
        ctx.pad.poll();
        if ctx.pad.held.intersects(Joypad::A | Joypad::B) { Phase::LetterNextFrame } else { Phase::LetterDelay }
    }

    fn arrow(&mut self, ctx: &mut Ctx, pause: Pause) -> Transition {
        Self::put(&mut ctx.screen.ui, ARROW, ch::DOWN_ARROW);
        self.phase = Phase::ArrowDelay(self.delay(ctx, 3), pause);
        self.after_delay(ctx)
    }

    fn scroll(&mut self, ctx: &mut Ctx) -> Transition {
        self.scroll_up_one_line(&mut ctx.screen.ui);
        self.phase = Phase::Scrolling { scrolled: 1, frames: self.delay(ctx, 5) };
        self.after_delay(ctx)
    }

    /// A delay that is presentation, which `Instant` pacing skips.
    fn delay(&self, ctx: &Ctx, frames: u8) -> u8 {
        if ctx.pacing == Pacing::Instant { 0 } else { frames }
    }

    /// A delay of zero frames is over at once.
    fn after_delay(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::ArrowDelay(0, _) | Phase::Scrolling { frames: 0, .. } | Phase::Cleared(0) => self.update(ctx),
            _ => Transition::Stay,
        }
    }

    /// `ScrollTextUpOneLine`: rows 14 to 16 move up one, and row 16 is blanked.
    fn scroll_up_one_line(&self, ui: &mut UiSurface) {
        for y in 13..16 {
            for x in 0..SCREEN_TILES_X {
                ui.set(x, y, ui.get(x, y + 1));
            }
        }
        ui.fill(1, 16, SCREEN_TILES_X - 2, 1, UiSurface::BLANK);
    }

    fn answer(&mut self, ctx: &mut Ctx, pause: Pause) -> Transition {
        self.answered += 1;
        match pause {
            // `PromptText` falls into `DoneText`, which ends the whole text.
            Pause::Prompt => {
                Self::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                Transition::Pop(Outcome::Done)
            }
            Pause::Paragraph => {
                ctx.screen.ui.fill(1, 13, 18, 4, UiSurface::BLANK);
                self.at = FIRST_LINE;
                self.phase = Phase::Cleared(self.delay(ctx, 20));
                self.after_delay(ctx)
            }
            Pause::Page => {
                ctx.screen.ui.fill(1, 10, 18, 7, UiSurface::BLANK);
                self.at = coord(1, 11);
                self.line = self.at;
                self.phase = Phase::Cleared(self.delay(ctx, 20));
                self.after_delay(ctx)
            }
            Pause::Cont => {
                Self::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                self.scroll(ctx)
            }
        }
    }
}

impl ModeUpdate for TextBox {
    /// `DisplayTextBoxID` with `MESSAGE_BOX`.
    fn enter(&mut self, ctx: &mut Ctx) {
        ctx.screen.ui.text_box_border(0, 12, 18, 4);
        self.phase = Phase::Opening(self.delay(ctx, 3));
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Opening(frames) | Phase::Cleared(frames) if frames > 1 => {
                self.phase = match self.phase {
                    Phase::Opening(_) => Phase::Opening(frames - 1),
                    _ => Phase::Cleared(frames - 1),
                };
                Transition::Stay
            }
            Phase::Opening(_) | Phase::Cleared(_) | Phase::LetterNextFrame => self.run(ctx),
            Phase::LetterDelay => {
                self.phase = self.check_letter_buttons(ctx);
                if self.phase == Phase::LetterDelay && *ctx.frame_counter == 0 {
                    self.run(ctx)
                } else {
                    Transition::Stay
                }
            }
            Phase::ArrowDelay(frames, pause) if frames > 1 => {
                self.phase = Phase::ArrowDelay(frames - 1, pause);
                Transition::Stay
            }
            Phase::ArrowDelay(_, pause) => {
                self.phase = Phase::Prompting { pause, blink: ArrowBlink::default() };
                self.update(ctx)
            }
            Phase::Prompting { pause, mut blink } => {
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    return self.answer(ctx, pause);
                }
                if let Some(shown) = blink.tick(BLINK_PER_FRAME) {
                    Self::put(&mut ctx.screen.ui, ARROW, if shown { ch::DOWN_ARROW } else { UiSurface::BLANK });
                }
                self.phase = Phase::Prompting { pause, blink };
                Transition::Stay
            }
            Phase::Scrolling { scrolled, frames } if frames > 1 => {
                self.phase = Phase::Scrolling { scrolled, frames: frames - 1 };
                Transition::Stay
            }
            Phase::Scrolling { scrolled: 1, .. } => {
                self.scroll_up_one_line(&mut ctx.screen.ui);
                self.phase = Phase::Scrolling { scrolled: 2, frames: self.delay(ctx, 5) };
                self.after_delay(ctx)
            }
            Phase::Scrolling { .. } => {
                self.at = SECOND_LINE;
                self.run(ctx)
            }
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Prompting { .. } => Status::Waiting(Decision::Text),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::command::{Command, Refusal, Reply};
    use crate::input::Joypad;
    use crate::mode::{Mode, Status};
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn game(text: &str, pacing: Pacing) -> Game {
        let world = World { player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), pacing);
        game.push(Mode::TextBox(TextBox::new(encode(text).unwrap())));
        game
    }

    fn row(game: &Game, y: usize) -> String {
        game.ui().row(y)[1..19].iter().map(|&tile| match tile {
            0x80..=0x99 => (b'A' + tile - 0x80) as char,
            0xEE => 'v',
            UiSurface::BLANK => ' ',
            _ => '?',
        }).collect::<String>().trim_end().to_string()
    }

    /// Frames until `done` holds, pressing nothing.
    fn frames_until(game: &mut Game, done: impl Fn(&Game) -> bool) -> u64 {
        let start = game.frames();
        while !done(game) {
            assert!(game.frames() - start < 1000, "never happened");
            game.frame(Input::None);
        }
        game.frames() - start
    }

    fn text_box(game: &Game) -> Option<&TextBox> {
        match game.modes().last() {
            Some(Mode::TextBox(text)) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn the_box_opens_three_frames_before_the_first_letter_and_letters_come_every_three() {
        let mut game = game("AB@", Pacing::Faithful);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "A"), 3);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "AB"), 3);
        assert_eq!(frames_until(&mut game, |g| g.modes().is_empty()), 3);
    }

    #[test]
    fn holding_a_prints_a_letter_a_frame_after_the_one_it_interrupts() {
        let mut game = game("ABCD@", Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 14) == "A");
        game.frame(Input::Buttons(Joypad::A));
        assert_eq!(row(&game, 14), "A", "`PrintLetterDelay` sees A, then waits its `DelayFrame`");
        for expected in ["AB", "ABC", "ABCD"] {
            game.frame(Input::Buttons(Joypad::A));
            assert_eq!(row(&game, 14), expected);
        }
    }

    #[test]
    fn a_prompt_waits_three_frames_then_takes_a_fresh_press() {
        let mut game = game("A<PROMPT>", Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 16).ends_with('v'));
        assert_eq!(game.status(), Status::Busy, "ProtectedDelay3");
        assert_eq!(frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text)), 3);

        game.frame(Input::Buttons(Joypad::A));
        assert!(game.modes().is_empty(), "a prompt ends the text");
        assert_eq!(row(&game, 16), "", "and takes the arrow with it");
    }

    #[test]
    fn a_press_held_through_the_letters_does_not_answer_the_prompt() {
        let mut game = game("A<PROMPT>", Pacing::Faithful);
        while game.status() != Status::Waiting(Decision::Text) {
            game.frame(Input::Buttons(Joypad::A));
        }
        game.frame(Input::Buttons(Joypad::A));
        assert_eq!(game.status(), Status::Waiting(Decision::Text));
        game.frame(Input::None);
        game.frame(Input::Buttons(Joypad::A));
        assert!(game.modes().is_empty());
    }

    #[test]
    fn a_paragraph_clears_the_box_and_starts_again_twenty_frames_later() {
        let mut game = game("A<LINE>B<PARA>C@", Pacing::Faithful);
        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        game.frame(Input::Buttons(Joypad::B));
        assert_eq!((row(&game, 14).as_str(), row(&game, 16).as_str()), ("", ""));
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "C"), 20);
    }

    #[test]
    fn cont_scrolls_the_second_line_up_in_two_steps() {
        let mut game = game("A<LINE>B<CONT>C@", Pacing::Faithful);
        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        game.frame(Input::Buttons(Joypad::A));
        assert_eq!([row(&game, 13), row(&game, 14), row(&game, 15), row(&game, 16)], ["A", "", "B", ""]);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "B"), 5);
        assert_eq!(frames_until(&mut game, |g| row(g, 16) == "C"), 5);
    }

    #[test]
    fn the_player_s_name_is_printed_a_letter_at_a_time() {
        let mut game = game("<PLAYER>@", Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 14) == "R");
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "RED"), 6);
    }

    #[test]
    fn instant_pacing_prints_up_to_the_prompt_in_one_frame() {
        let mut game = game("AB<LINE>CD<PROMPT>", Pacing::Instant);
        game.frame(Input::None);
        assert_eq!((row(&game, 14).as_str(), game.status()), ("AB", Status::Waiting(Decision::Text)));
        assert_eq!(row(&game, 16), "CD               v");
    }

    #[test]
    fn advance_is_refused_until_the_box_waits_and_then_answers_it() {
        let mut game = game("A<PARA>B<PROMPT>", Pacing::Faithful);
        let refused = game.frame(Input::Command(Command::Advance)).reply;
        assert!(matches!(refused, Some(Reply::Refused(Refusal::Invalid(_)))), "{refused:?}");

        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        assert_eq!(game.frame(Input::Command(Command::Advance)).reply, Some(Reply::Accepted));
        assert_eq!(text_box(&game).unwrap().answered(), 1);
        let next = game.frame(Input::Command(Command::Advance));
        assert_eq!(next.reply, Some(Reply::Refused(Refusal::Busy)), "the first has not reported yet");
        assert_eq!(next.events, [Event::CommandDone(Command::Advance)]);

        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        game.frame(Input::Command(Command::Advance));
        assert!(game.modes().is_empty());
    }

    #[test]
    fn a_save_taken_mid_letter_resumes_on_the_same_frame() {
        let text = "AB<LINE>CD<PARA>EF<PROMPT>";
        let mut whole = game(text, Pacing::Faithful);
        let mut halves = game(text, Pacing::Faithful);
        for _ in 0..8 {
            whole.frame(Input::None);
            halves.frame(Input::None);
        }
        let mut restored = Game::load(&halves.save(), Pacing::Faithful).unwrap();
        for frame in 0..60 {
            let press = if frame == 30 { Input::Buttons(Joypad::A) } else { Input::None };
            let again = if frame == 30 { Input::Buttons(Joypad::A) } else { Input::None };
            whole.frame(press);
            restored.frame(again);
            assert_eq!(whole.ui(), restored.ui(), "frame {frame}");
        }
    }
}
