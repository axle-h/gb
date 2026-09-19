//! `TextCommandProcessor`: a text script's commands, run one after another over `PlaceString`.
//!
//! `PrintText` draws a message box and prints at (1, 14); the commands move that cursor, print a
//! buffer or a number at it, and wait for the player. A `<DONE>` or `<PROMPT>` inside a printed run
//! ends the whole script, which is why the printer reports how it returned.

use poke_core::species::PokemonSpecies;
use poke_core::text_script::{TextCommand, TextSound};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::command::Decision;
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::ui::UiSurface;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::blink::ArrowBlink;
use crate::modes::place_string::{ch, LetterDelay, PlaceString, Printed, ARROW, FIRST_LINE, SECOND_LINE};
use crate::systems::print_num::{bcd_writes, print_number, BcdFormat, BcdWrite, NumberFormat};

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
    /// `PrintText_NoCreatingTextBox`: printed over whatever is there, with no `MESSAGE_BOX` drawn.
    #[serde(default)]
    no_box: bool,
    /// Printing with `hAutoBGTransferEnabled` off. Until `enter` this is the screen to keep showing;
    /// from then on it is the surface the text is drawn on, swapped in for each step and left on
    /// the screen when the text ends.
    #[serde(default)]
    off_screen: Option<UiSurface>,
    /// What the script's `text_asm` does when it plays a sound and runs on into more text, which
    /// is then the rest of `commands`. Without one a `text_asm` ends the script.
    #[serde(default)]
    asm_sound: Option<SoundId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A command is printing, or the next one runs this frame.
    Running,
    /// `ManualTextScroll`, with the `▼` up for `TX_PROMPT_BUTTON` and not for `TX_WAIT_BUTTON`.
    Waiting { arrow: bool, blink: ArrowBlink },
    /// `TX_PAUSE`: thirty frames, unless A or B is already held.
    Pausing(u8),
    /// `TX_DOTS`: a `…` every ten frames, or at once while A or B is held.
    Dotting { left: u8, frames: u8 },
    /// `TX_BCD`: `PrintBCDNumber`'s writes, a letter delay after each digit.
    Digits { writes: Vec<BcdWrite>, next: usize, end: u16, delay: Option<LetterDelay> },
    /// `TX_SOUND`'s `WaitForSoundToFinish`, or `PlayCry`'s own.
    Sounding,
    /// `TX_SCROLL`: `scrolled` lines of two are up, and `frames` of the last one's five are left.
    Scrolling { scrolled: u8, frames: u8 },
    /// A `text_asm` that is `PlaySoundWaitForCurrent`, waiting for the current sound to end.
    AsmSound(SoundId),
}

impl TextBox {
    /// One string, printed as `PrintText` prints it: the common case, and what `TX_START` is.
    pub fn new(text: Vec<u8>) -> Self {
        Self::script(vec![TextCommand::Text(text)])
    }

    pub fn script(commands: Vec<TextCommand>) -> Self {
        Self { commands, index: 0, dest: FIRST_LINE, printer: None, phase: Phase::Running, answered: 0, no_box: false,
               off_screen: None, asm_sound: None }
    }

    /// A script whose `text_asm` is `PlaySoundWaitForCurrent` of `sound` and a jump to the text after
    /// it, printed on from where the text before it stopped.
    pub fn with_asm_sound(self, sound: SoundId) -> Self {
        Self { asm_sound: Some(sound), ..self }
    }

    /// `PrintText_NoCreatingTextBox`: `DisplayTextID`'s own, which draws its box itself unless
    /// `BIT_NO_AUTO_TEXT_BOX` says not to.
    pub fn without_box(commands: Vec<TextCommand>) -> Self {
        Self { no_box: true, ..Self::script(commands) }
    }

    /// Printed with the background transfer off: `shown` stays on the screen, the letters keep their
    /// delays, and the text appears whole in the frame it ends.
    pub fn off_screen(self, shown: UiSurface) -> Self {
        Self { off_screen: Some(shown), ..self }
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
                let (writes, end) = bcd_writes(self.dest as usize, &digits, format);
                self.phase = Phase::Digits { writes, next: 0, end: end as u16, delay: None };
                return Some(self.digits(ctx));
            }
            TextCommand::Low => self.dest = SECOND_LINE,
            TextCommand::Move(at) => self.dest = at.wrapping_sub(TILE_MAP),
            TextCommand::Box { at, width, height } => {
                let at = at.wrapping_sub(TILE_MAP) as usize;
                ctx.screen.ui.text_box_border(at % 20, at / 20, width as usize, height as usize);
            }
            // No `▼` and no wait for a press: two `ScrollTextUpOneLine`s, five frames each.
            TextCommand::Scroll => {
                PlaceString::put(&mut ctx.screen.ui, ARROW, UiSurface::BLANK);
                return Some(self.scroll(ctx, 0));
            }
            TextCommand::PromptButton | TextCommand::WaitButton => {
                let arrow = command == TextCommand::PromptButton;
                if arrow {
                    PlaceString::put(&mut ctx.screen.ui, ARROW, ch::DOWN_ARROW);
                }
                self.phase = Phase::Waiting { arrow, blink: ArrowBlink::default() };
                // `ManualTextScroll` reads the pad in the frame it is called.
                return Some(self.wait(ctx));
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
            TextCommand::Sound(sound) => {
                match sound {
                    TextSound::CryNidorina => ctx.audio.play_cry(PokemonSpecies::Nidorina as u8),
                    TextSound::CryPidgeot => ctx.audio.play_cry(PokemonSpecies::Pidgeot as u8),
                    TextSound::CryDewgong => ctx.audio.play_cry(PokemonSpecies::Dewgong as u8),
                    // `SFX_GET_ITEM_1` is `SFX_LEVEL_UP`'s id in the battle bank, so which plays is
                    // the bank's business.
                    TextSound::GetItem1 | TextSound::GetItem1Duplicate => ctx.audio.play_sound(sounds::SFX_GET_ITEM_1),
                    TextSound::GetItem2 => ctx.audio.play_sound(sounds::SFX_GET_ITEM_2),
                    TextSound::GetKeyItem => ctx.audio.play_sound(sounds::SFX_GET_KEY_ITEM),
                    TextSound::CaughtMon => ctx.audio.play_sound(sounds::SFX_CAUGHT_MON),
                    TextSound::DexPageAdded => ctx.audio.play_sound(sounds::SFX_DEX_PAGE_ADDED),
                    TextSound::PokedexRating => ctx.audio.play_sound(sounds::SFX_POKEDEX_RATING),
                }
                if ctx.pacing != crate::Pacing::Instant && !ctx.audio.sound_finished() {
                    self.phase = Phase::Sounding;
                    return Some(Transition::Stay);
                }
            }
            TextCommand::Asm(_) => match self.asm_sound {
                Some(sound) => {
                    self.phase = Phase::AsmSound(sound);
                    return Some(self.step(ctx));
                }
                // The 599 escapes are each their own chunk's.
                None => return Some(Transition::Pop(Outcome::Done)),
            },
        }
        None
    }

    /// `ScrollTextUpOneLine` after `scrolled` of them, and on to the second line after both.
    fn scroll(&mut self, ctx: &mut Ctx, scrolled: u8) -> Transition {
        if scrolled == 2 {
            self.dest = SECOND_LINE;
            self.phase = Phase::Running;
            return self.run(ctx);
        }
        PlaceString::scroll_up_one_line(&mut ctx.screen.ui);
        let frames = self.delay(ctx, 5);
        if frames == 0 {
            return self.scroll(ctx, scrolled + 1);
        }
        self.phase = Phase::Scrolling { scrolled: scrolled + 1, frames };
        Transition::Stay
    }

    /// `PrintBCDNumber`'s writes up to the next letter delay, then the next command.
    fn digits(&mut self, ctx: &mut Ctx) -> Transition {
        let Phase::Digits { writes, mut next, end, mut delay } = std::mem::replace(&mut self.phase, Phase::Running) else {
            unreachable!("only a number being printed has digits");
        };
        if let Some(waiting) = &mut delay {
            if !waiting.update(ctx) {
                self.phase = Phase::Digits { writes, next, end, delay };
                return Transition::Stay;
            }
        }
        while let Some(&BcdWrite { at, tile, delayed }) = writes.get(next) {
            PlaceString::put(&mut ctx.screen.ui, at as u16, tile);
            next += 1;
            if delayed && let Some(waiting) = LetterDelay::start(ctx) {
                self.phase = Phase::Digits { writes, next, end, delay: Some(waiting) };
                return Transition::Stay;
            }
        }
        self.dest = end;
        self.run(ctx)
    }

    /// `ManualTextScroll`: one pass of its loop, which answers or blinks.
    fn wait(&mut self, ctx: &mut Ctx) -> Transition {
        let Phase::Waiting { arrow, mut blink } = self.phase else {
            unreachable!("only a waiting box reads the pad for its prompt");
        };
        if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
            self.answered += 1;
            ctx.audio.play_sound(sounds::SFX_PRESS_AB);
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

    /// A frame of whatever the box is doing.
    fn step(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Running => self.run(ctx),
            Phase::Waiting { .. } => self.wait(ctx),
            Phase::Digits { .. } => self.digits(ctx),
            Phase::Sounding if ctx.audio.sound_finished() => {
                self.phase = Phase::Running;
                self.run(ctx)
            }
            Phase::Sounding => Transition::Stay,
            Phase::AsmSound(sound) if ctx.pacing == crate::Pacing::Instant || ctx.audio.sound_finished() => {
                ctx.audio.play_sound(sound);
                self.phase = Phase::Running;
                self.run(ctx)
            }
            Phase::AsmSound(_) => Transition::Stay,
            Phase::Scrolling { scrolled, frames } if frames > 1 => {
                self.phase = Phase::Scrolling { scrolled, frames: frames - 1 };
                Transition::Stay
            }
            Phase::Scrolling { scrolled, .. } => self.scroll(ctx, scrolled),
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

    /// `step` drawn on the off-screen surface when there is one, which is shown once the text ends.
    fn drawn(&mut self, ctx: &mut Ctx, step: impl FnOnce(&mut Self, &mut Ctx) -> Transition) -> Transition {
        let Some(mut surface) = self.off_screen.take() else { return step(self, ctx) };
        std::mem::swap(&mut ctx.screen.ui, &mut surface);
        let transition = step(self, ctx);
        if !matches!(transition, Transition::Pop(_)) {
            std::mem::swap(&mut ctx.screen.ui, &mut surface);
            self.off_screen = Some(surface);
        }
        transition
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
        if !self.no_box {
            TextBoxId::MessageBox.draw(&mut ctx.screen.ui);
        }
        if let Some(shown) = &mut self.off_screen {
            std::mem::swap(&mut ctx.screen.ui, shown);
        }
    }

    /// `PrintText`'s `Delay3` after drawing the box is loading and not modelled, so the first
    /// command runs in the frame the box goes up.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.drawn(ctx, Self::run)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.drawn(ctx, Self::step)
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
    fn the_first_letter_goes_up_with_the_box_and_letters_come_every_three() {
        let mut game = game("AB@", Pacing::Faithful);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "A"), 0);
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
    fn a_prompt_waits_from_the_frame_its_arrow_goes_up_and_takes_a_fresh_press() {
        let mut game = game("A<PROMPT>", Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 16).ends_with('v'));
        assert_eq!(game.status(), Status::Waiting(Decision::Text), "no ProtectedDelay3");

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

    /// Printed with the background transfer off: the letters take as long, the screen keeps what
    /// was on it, and the whole text lands in the frame the box closes.
    #[test]
    fn an_off_screen_text_is_only_seen_once_it_is_done() {
        let world = World { player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.screen_mut().ui.set(0, 0, 0x31);
        let shown = game.ui().clone();
        game.push(Mode::TextBox(TextBox::new(encode("AB@").unwrap()).off_screen(shown)));
        let frames = frames_until(&mut game, |g| {
            assert!(g.modes().is_empty() || row(g, 14).is_empty(), "the letters are on the screen");
            g.modes().is_empty()
        });
        assert_eq!(frames, 6, "the same frames as a text drawn on the screen");
        assert_eq!(row(&game, 14), "AB");
        assert_eq!(game.ui().get(0, 0), 0x31, "and the screen it was drawn off is still under it");
    }

    /// `TX_SCROLL`: the two lines go up one at a time, five frames apart, and the next command
    /// prints on the second line five frames after the second.
    #[test]
    fn scroll_moves_the_text_up_two_lines_over_ten_frames() {
        let commands = vec![
            TextCommand::Text(encode("A<LINE>B@").unwrap()),
            TextCommand::Scroll,
            TextCommand::Text(encode("C@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| row(g, 15) == "B");
        assert_eq!([row(&game, 13), row(&game, 14), row(&game, 15), row(&game, 16)], ["A", "", "B", ""]);
        assert_eq!(frames_until(&mut game, |g| row(g, 14) == "B"), 5);
        assert_eq!(row(&game, 16), "", "no `▼` and no wait for a press");
        assert_eq!(frames_until(&mut game, |g| row(g, 16) == "C"), 5);
    }

    #[test]
    fn instant_pacing_scrolls_at_once() {
        let commands = vec![
            TextCommand::Text(encode("A<LINE>B@").unwrap()),
            TextCommand::Scroll,
            TextCommand::Text(encode("C@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Instant);
        game.frame(Input::None);
        assert_eq!((row(&game, 14).as_str(), row(&game, 16).as_str()), ("B", "C"));
    }

    /// Frames a sound plays for before `WaitForSoundToFinish` returns, measured on an engine of
    /// its own.
    fn frames_playing(play: impl Fn(&mut crate::audio::engine::AudioEngine)) -> u64 {
        let mut engine = crate::audio::engine::AudioEngine::new(crate::audio::data::AudioBank::One);
        play(&mut engine);
        let mut frames = 0;
        while !engine.sound_finished() {
            engine.frame();
            frames += 1;
        }
        frames
    }

    /// `TX_SOUND` plays its sound and the text waits the whole of it before going on.
    #[test]
    fn a_sound_is_heard_out_before_the_text_goes_on() {
        let commands = vec![
            TextCommand::Text(encode("A@").unwrap()),
            TextCommand::Sound(TextSound::GetKeyItem),
            TextCommand::Text(encode("B@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| !g.audio().sound_finished());
        assert!((4..8).any(|channel| game.audio().channel_sound_id(channel) == sounds::SFX_GET_KEY_ITEM.0),
            "SFX_GET_KEY_ITEM is playing");
        assert_eq!(game.status(), Status::Busy, "a sound is not a decision");
        let waited = frames_until(&mut game, |g| {
            assert!(row(g, 14) == "A" || g.audio().sound_finished(), "B went up under the sound");
            row(g, 14) == "AB"
        });
        let length = frames_playing(|engine| engine.play_sound(sounds::SFX_GET_KEY_ITEM));
        assert!(waited >= length && waited <= length + 1, "waited {waited} frames for a {length}-frame sound");
    }

    /// The three cries go through `PlayCry`, whose own wait holds the text the same way.
    #[test]
    fn a_cry_is_heard_out_before_the_text_goes_on() {
        let commands = vec![
            TextCommand::Text(encode("A@").unwrap()),
            TextCommand::Sound(TextSound::CryDewgong),
            TextCommand::Text(encode("B@").unwrap()),
        ];
        let mut game = scripted(commands, TextVars::default(), Pacing::Faithful);
        frames_until(&mut game, |g| !g.audio().sound_finished());
        let cry = crate::audio::engine::AudioEngine::new(crate::audio::data::AudioBank::One)
            .get_cry_data(PokemonSpecies::Dewgong as u8);
        assert_eq!(game.audio().channel_sound_id(4), cry.0, "Dewgong's cry");
        let waited = frames_until(&mut game, |g| {
            assert!(row(g, 14) == "A" || g.audio().sound_finished(), "B went up under the cry");
            row(g, 14) == "AB"
        });
        let length = frames_playing(|engine| engine.play_cry(PokemonSpecies::Dewgong as u8));
        assert!(waited >= length && waited <= length + 1, "waited {waited} frames for a {length}-frame cry");
    }

    /// A `text_asm` that plays a sound runs on into the text after it, from where the text stopped.
    #[test]
    fn an_asm_sound_plays_and_the_text_runs_on_where_it_stopped() {
        let commands = vec![
            TextCommand::Text(encode("A@").unwrap()),
            TextCommand::Asm(pokered_symbols::OneTwoAndText),
            TextCommand::Text(encode("B@").unwrap()),
        ];
        let world = World { player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::TextBox(TextBox::script(commands).with_asm_sound(sounds::SFX_SWAP)));
        frames_until(&mut game, |g| !g.audio().sound_finished());
        assert!((4..8).any(|channel| game.audio().channel_sound_id(channel) == sounds::SFX_SWAP.0));
        frames_until(&mut game, |g| row(g, 14) == "AB");
    }

    #[test]
    fn a_text_without_its_box_prints_over_what_is_on_the_screen() {
        let world = World { player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.screen_mut().ui.set(0, 12, 0x31);
        game.push(Mode::TextBox(TextBox::without_box(vec![TextCommand::Text(encode("AB@").unwrap())])));
        frames_until(&mut game, |g| g.modes().is_empty());
        assert_eq!(row(&game, 14), "AB");
        assert_eq!(game.ui().get(0, 12), 0x31, "no border drawn over the corner");
    }

}
