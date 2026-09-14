//! `TextCommandProcessor`: a text script's commands, run one after another over `PlaceString`.
//!
//! `PrintText` draws a message box and prints at (1, 14); the commands move that cursor, print a
//! buffer or a number at it, and wait for the player. A `<DONE>` or `<PROMPT>` inside a printed run
//! ends the whole script, which is why the printer reports how it returned.

use poke_core::text_script::TextCommand;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::ui::UiSurface;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::blink::ArrowBlink;
use crate::modes::place_string::{ch, PlaceString, Printed, ARROW, FIRST_LINE, SECOND_LINE};
use crate::systems::print_num::{print_bcd, print_number, BcdFormat, NumberFormat};

/// `wTileMap`: `TX_MOVE` and `TX_BOX` name a tile by its address, and the engine works in indices.
const TILE_MAP: u16 = 0xC3A0;

/// `'…'`, which `TX_DOTS` writes one of at a time.
const DOTS: u8 = 0x75;

/// `ManualTextScroll`'s iterations a frame, in hundredths; it is `WaitForTextScrollButtonPress`.
const BLINK_PER_FRAME: u32 = 4454;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextBox {
    commands: Vec<TextCommand>,
    index: usize,
    /// `wTextDest`, which the cartridge keeps in `bc`: where the next command prints.
    dest: u16,
    printer: Option<PlaceString>,
    phase: Phase,
    /// Prompts answered so far, so a driver can see its press land.
    answered: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// `PrintText`'s `Delay3` after drawing the box.
    Opening(u8),
    /// A command is printing, or the next one runs this frame.
    Running,
    /// `ManualTextScroll`, with the `▼` up for `TX_PROMPT_BUTTON` and not for `TX_WAIT_BUTTON`.
    Waiting { arrow: bool, blink: ArrowBlink },
    /// `TX_PAUSE`: thirty frames, unless A or B is already held.
    Pausing(u8),
    /// `TX_DOTS`: a `…` every ten frames, or at once while A or B is held.
    Dotting { left: u8, frames: u8 },
}

impl TextBox {
    /// One string, printed as `PrintText` prints it: the common case, and what `TX_START` is.
    pub fn new(text: Vec<u8>) -> Self {
        Self::script(vec![TextCommand::Text(text)])
    }

    pub fn script(commands: Vec<TextCommand>) -> Self {
        Self { commands, index: 0, dest: FIRST_LINE, printer: None, phase: Phase::Opening(3), answered: 0 }
    }

    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// A delay that is presentation, which `Instant` pacing skips.
    fn delay(&self, ctx: &Ctx, frames: u8) -> u8 {
        if ctx.pacing == crate::Pacing::Instant { 0 } else { frames }
    }

    /// `NextTextCommand`: drive the live printer, then run commands until one waits.
    fn run(&mut self, ctx: &mut Ctx) -> Transition {
        loop {
            if let Some(printer) = &mut self.printer {
                match printer.update(ctx, &mut self.answered) {
                    None => {
                        self.phase = Phase::Running;
                        return Transition::Stay;
                    }
                    Some(Printed::Ended) => return Transition::Pop(Outcome::Done),
                    Some(Printed::Returned(at)) => {
                        self.dest = at;
                        self.printer = None;
                    }
                }
            }
            let Some(command) = self.commands.get(self.index).cloned() else {
                return Transition::Pop(Outcome::Done);
            };
            self.index += 1;
            if let Some(transition) = self.start(ctx, command) {
                return transition;
            }
        }
    }

    /// One command. `Some` when it waits a frame or ends the script; `None` runs the next at once.
    fn start(&mut self, ctx: &mut Ctx, command: TextCommand) -> Option<Transition> {
        match command {
            TextCommand::Text(bytes) => self.printer = Some(PlaceString::call(bytes, self.dest)),
            TextCommand::Buffer(source) => {
                let bytes = ctx.world.text.string(source);
                self.printer = Some(PlaceString::call(bytes, self.dest));
            }
            TextCommand::Number { source, digits, .. } => {
                let value = ctx.world.text.number(source);
                let format = NumberFormat { digits, left_align: true, leading_zeroes: false };
                self.dest = print_number(&mut ctx.screen.ui, self.dest as usize, value, format) as u16;
            }
            TextCommand::Bcd { source, skip_leading_zeroes, left_align, money_sign, .. } => {
                let digits = ctx.world.text.bcd(source);
                let format = BcdFormat { skip_leading_zeroes, left_align, money_sign };
                self.dest = print_bcd(&mut ctx.screen.ui, self.dest as usize, &digits, format) as u16;
            }
            TextCommand::Low => self.dest = SECOND_LINE,
            TextCommand::Move(at) => self.dest = at.wrapping_sub(TILE_MAP),
            TextCommand::Box { at, width, height } => {
                let at = at.wrapping_sub(TILE_MAP) as usize;
                ctx.screen.ui.text_box_border(at % 20, at / 20, width as usize, height as usize);
            }
            TextCommand::Scroll => {
                // No `▼` and no wait: the two scrolls and then on with the next command.
                PlaceString::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                PlaceString::scroll_up_one_line(&mut ctx.screen.ui);
                PlaceString::scroll_up_one_line(&mut ctx.screen.ui);
                self.dest = SECOND_LINE;
            }
            TextCommand::PromptButton | TextCommand::WaitButton => {
                let arrow = command == TextCommand::PromptButton;
                if arrow {
                    PlaceString::put(&mut ctx.screen.ui, ARROW, ch::DOWN_ARROW);
                }
                self.phase = Phase::Waiting { arrow, blink: ArrowBlink::default() };
                return Some(Transition::Stay);
            }
            TextCommand::Pause => {
                ctx.pad.poll();
                let frames = if ctx.pad.held.intersects(Joypad::A | Joypad::B) { 0 } else { self.delay(ctx, 30) };
                if frames > 0 {
                    self.phase = Phase::Pausing(frames);
                    return Some(Transition::Stay);
                }
            }
            TextCommand::Dots(dots) => {
                self.phase = Phase::Dotting { left: dots, frames: 0 };
                return Some(Transition::Stay);
            }
            // The audio engine's, and the 599 escapes are each their own chunk's.
            TextCommand::Sound(_) => {}
            TextCommand::Asm(_) => return Some(Transition::Pop(Outcome::Done)),
        }
        None
    }

    /// One `…` of `TX_DOTS`, which waits ten frames unless A or B is held.
    fn dot(&mut self, ctx: &mut Ctx, left: u8) -> Transition {
        PlaceString::put(&mut ctx.screen.ui, self.dest, DOTS);
        self.dest += 1;
        ctx.pad.poll();
        let held = ctx.pad.held.intersects(Joypad::A | Joypad::B);
        let frames = if held { 0 } else { self.delay(ctx, 10) };
        self.phase = match (left - 1, frames) {
            (0, 0) => return self.run(ctx),
            // The wait after the last one is `Pausing`: frames, and then the next command.
            (0, frames) => Phase::Pausing(frames),
            (left, frames) => Phase::Dotting { left, frames },
        };
        Transition::Stay
    }
}

impl ModeUpdate for TextBox {
    /// `DisplayTextBoxID` with `MESSAGE_BOX`.
    fn enter(&mut self, ctx: &mut Ctx) {
        TextBoxId::MessageBox.draw(&mut ctx.screen.ui);
        self.phase = Phase::Opening(self.delay(ctx, 3));
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Opening(frames) if frames > 1 => {
                self.phase = Phase::Opening(frames - 1);
                Transition::Stay
            }
            Phase::Opening(_) | Phase::Running => self.run(ctx),
            Phase::Waiting { arrow, mut blink } => {
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    self.answered += 1;
                    if arrow {
                        PlaceString::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                    }
                    self.phase = Phase::Running;
                    return self.run(ctx);
                }
                if let Some(shown) = blink.tick(BLINK_PER_FRAME)
                    && arrow
                {
                    PlaceString::put(&mut ctx.screen.ui, ARROW, if shown { ch::DOWN_ARROW } else { UiSurface::BLANK });
                }
                self.phase = Phase::Waiting { arrow, blink };
                Transition::Stay
            }
            Phase::Pausing(frames) if frames > 1 => {
                self.phase = Phase::Pausing(frames - 1);
                Transition::Stay
            }
            Phase::Pausing(_) => {
                self.phase = Phase::Running;
                self.run(ctx)
            }
            Phase::Dotting { left, frames } if frames > 1 => {
                self.phase = Phase::Dotting { left, frames: frames - 1 };
                Transition::Stay
            }
            Phase::Dotting { left, .. } => self.dot(ctx, left),
        }
    }

    fn status(&self) -> Status {
        let prompting = matches!(self.phase, Phase::Waiting { .. })
            || self.printer.as_ref().is_some_and(PlaceString::is_prompting);
        if prompting { Status::Waiting(Decision::Text) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use poke_core::charmap::encode;
    use poke_core::symbols::pokered_symbols;
    use poke_core::text_script::{decode, TextBuffer, TextNumber};
    use crate::world::TextVars;
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
            0xA0..=0xB9 => (b'a' + (tile - 0xA0)) as char,
            0xF6..=0xFF => (b'0' + (tile - 0xF6)) as char,
            0x75 => '.',
            0xE7 => '!',
            0xE8 => '.',
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

    fn scripted(commands: Vec<TextCommand>, text: TextVars, pacing: Pacing) -> Game {
        let world = World { player_name: encode("RED").unwrap(), text, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), pacing);
        game.push(Mode::TextBox(TextBox::script(commands)));
        game
    }

    /// A real text out of the cartridge, decoded and run: a number, then the words after it.
    #[test]
    fn a_cartridge_text_prints_its_number_and_then_its_words() {
        let text = TextVars {
            numbers: BTreeMap::from([(TextNumber::ExpAmountGained, 1337)]),
            ..TextVars::default()
        };
        let commands = decode(pokered_symbols::_ExpPointsText).unwrap();
        let mut game = scripted(commands, text, Pacing::Instant);
        game.frame(Input::None);
        assert_eq!(row(&game, 14), "1337 EXP. Points!");
        assert_eq!(game.status(), Status::Waiting(Decision::Text), "the text ends on a prompt");
    }

    /// `TX_RAM`: the string the caller left in a buffer, then the run carries on after it.
    #[test]
    fn a_buffer_prints_where_the_cursor_is_and_the_text_goes_on() {
        let text = TextVars {
            strings: BTreeMap::from([(TextBuffer::NameBuffer, encode("NIDORAN@").unwrap())]),
            ..TextVars::default()
        };
        let commands = decode(pokered_symbols::_GrewLevelText).unwrap();
        let mut game = scripted(commands, text, Pacing::Instant);
        game.frame(Input::None);
        assert_eq!(row(&game, 14), "NIDORAN grew");
        assert_eq!(row(&game, 16), "to level 0!", "no level was set, so the cartridge's scratch is zero");
    }

    /// `TX_LOW` prints on the second line without anything having to scroll there.
    #[test]
    fn low_moves_the_cursor_to_the_second_line() {
        let commands = vec![
            TextCommand::Text(encode("ONE@").unwrap()),
            TextCommand::Low,
            TextCommand::Text(encode("TWO@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Instant);
        game.frame(Input::None);
        assert_eq!((row(&game, 14).as_str(), row(&game, 16).as_str()), ("ONE", "TWO"));
    }

    /// `TX_PROMPT_BUTTON` waits with the `▼` up, and unlike `<PROMPT>` the script goes on after it.
    #[test]
    fn a_prompt_button_waits_and_then_the_script_continues() {
        let commands = vec![
            TextCommand::Text(encode("A@").unwrap()),
            TextCommand::PromptButton,
            TextCommand::Text(encode("B@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        assert!(row(&game, 16).ends_with('v'), "the arrow is up: {:?}", row(&game, 16));

        game.frame(Input::Buttons(Joypad::A));
        assert_eq!(text_box(&game).unwrap().answered(), 1);
        assert_eq!(row(&game, 16), "", "and the arrow goes again");
        frames_until(&mut game, |g| row(g, 14) == "AB");
    }

    /// `TX_WAIT_BUTTON` is the same wait with no arrow drawn.
    #[test]
    fn a_wait_button_waits_without_an_arrow() {
        let commands = vec![TextCommand::WaitButton, TextCommand::Text(encode("A@").unwrap())];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| g.status() == Status::Waiting(Decision::Text));
        for _ in 0..200 {
            game.frame(Input::None);
            assert_eq!(row(&game, 16), "", "no arrow is ever drawn");
        }
        game.frame(Input::Buttons(Joypad::A));
        frames_until(&mut game, |g| row(g, 14) == "A");
    }

    /// `TX_PAUSE` needs no press: thirty frames and it goes on by itself.
    #[test]
    fn a_pause_gives_up_waiting_after_thirty_frames() {
        let commands = vec![
            TextCommand::Text(encode("A@").unwrap()),
            TextCommand::Pause,
            TextCommand::Text(encode("B@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 14) == "A");
        assert_eq!(game.status(), Status::Busy, "a pause is not a decision");
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "AB"), 3 + 30, "A's letter delay, then the pause");
    }

    /// `TX_DOTS` writes one `…` every ten frames.
    #[test]
    fn dots_are_printed_one_every_ten_frames() {
        let commands = vec![TextCommand::Dots(3), TextCommand::Text(encode("!@").unwrap())];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 14) == ".");
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == ".."), 10);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "..."), 10);
        frames_until(&mut game, |g| row(g, 14) == "...!");
    }

    /// The whole processor is state, so a save mid-script resumes mid-script.
    #[test]
    fn a_save_taken_between_commands_resumes_there() {
        let commands = vec![
            TextCommand::Text(encode("AB@").unwrap()),
            TextCommand::Low,
            TextCommand::Text(encode("CD<PROMPT>").unwrap()),
        ];
        let whole = || scripted(commands.clone(), TextVars::default(), Pacing::Faithful);
        let (mut whole, mut halves) = (whole(), whole());
        for _ in 0..8 {
            whole.frame(Input::None);
            halves.frame(Input::None);
        }
        let mut restored = Game::load(&halves.save(), Pacing::Faithful).unwrap();
        for frame in 0..40 {
            whole.frame(Input::None);
            restored.frame(Input::None);
            assert_eq!(whole.ui(), restored.ui(), "frame {frame}");
        }
    }
}
