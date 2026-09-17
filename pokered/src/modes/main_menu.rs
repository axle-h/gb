//! `MainMenu` from `.mainMenuLoop`: `CONTINUE`, `NEW GAME` and `OPTION` after the title screen, and
//! `DisplayContinueGameInfo`'s box of the player, badges, Pokédex owned and time.
//!
//! Whether a save exists is the caller's to say, and so is what follows: the menu answers
//! [`CONTINUE`] or [`NEW_GAME`], or `Cancelled` for B, which is the title screen again. `OPTION` is
//! answered inside, by the option screen and the menu over again.
//!
//! The waits are pacing, not loading, so all three are kept: the 20 frames before the menu is
//! drawn, the 20 after a choice, and the 30 the info box stands before it reads the pad. So is the
//! 10 on a blank screen after `CONTINUE` is confirmed; the `GBPalWhiteOutWithDelay3` before it is
//! not.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::option_menu::OptionMenu;
use crate::systems::pokedex::count_set_bits;
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::status_screen::{encode, place_lines};

pub const CONTINUE: u8 = 0;
pub const NEW_GAME: u8 = 1;
const OPTION: u8 = 2;

/// `.mainMenuLoop`'s and the choice's `DelayFrames 20`, `DisplayContinueGameInfo`'s 30 and
/// `.pressedA`'s 10.
const BEFORE_MENU: u8 = 20;
const AFTER_CHOICE: u8 = 20;
const INFO_STANDS: u8 = 30;
const AFTER_CONTINUE: u8 = 10;
/// The `:` of the text box tiles, which `PrintPlayTime` writes by its id.
const TIME_COLON: u8 = 0x6D;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A `DelayFrames` counting down, and what runs when it ends.
    Hold(u8, After),
    Menu,
    /// `.inputLoop`, which reads what is held rather than what is new.
    Info,
    /// The option screen is up.
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    DrawMenu,
    Choose,
    WaitOnInfo,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainMenu {
    /// `wSaveFileStatus` is 2.
    save_exists: bool,
    phase: Phase,
    input: MenuInput,
    answered: u32,
    /// `wOptionsInitialized`: the option screen was visited, so a new game keeps what was set there.
    #[serde(default)]
    options_initialized: bool,
}

impl MainMenu {
    pub fn new(save_exists: bool) -> Self {
        let input = MenuInput::new(0, 0, (1, 2), Joypad::A | Joypad::B | Joypad::START);
        Self { save_exists, phase: Phase::Hold(BEFORE_MENU, After::DrawMenu), input, answered: 0, options_initialized: false }
    }

    /// `CONTINUE` only when there is a save to continue.
    pub fn rows(&self) -> u8 {
        if self.save_exists { 3 } else { 2 }
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// Answers the info box has taken, so a driver can see its press land.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// `.mainMenuLoop` after its wait: the saved cursors zeroed, the screen cleared and the menu up.
    fn draw_menu(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.party_and_bills = 0;
        ctx.menu.bag_saved = 0;
        ctx.menu.battle_and_start = 0;
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        ctx.screen.sgb.run(&PaletteCommand::Default);
        ctx.screen.tiles.load_text_box_tiles();
        ctx.screen.tiles.load_font();
        let (height, text) = if self.save_exists {
            (6, "CONTINUE<NEXT>NEW GAME<NEXT>OPTION")
        } else {
            (4, "NEW GAME<NEXT>OPTION")
        };
        ctx.screen.ui.text_box_border(0, 0, 13, height);
        place_lines(&mut ctx.screen.ui, 2 * SCREEN_TILES_X + 2, &encode(text), false);
        self.input = MenuInput::new(0, self.rows() - 1, (1, 2), Joypad::A | Joypad::B | Joypad::START);
        ctx.menu.last_item = 0;
        self.phase = Phase::Menu;
        self.input.call(ctx);
        self.update(ctx)
    }

    /// What `wCurrentMenuItem` means once the missing `CONTINUE` row is counted back in.
    fn choice(&self) -> u8 {
        self.input.current + if self.save_exists { 0 } else { 1 }
    }

    fn chosen(&mut self, ctx: &mut Ctx) -> Transition {
        match self.choice() {
            CONTINUE => {
                save_screen_info(ctx, (4, 7));
                self.phase = Phase::Hold(INFO_STANDS, After::WaitOnInfo);
                Transition::Stay
            }
            OPTION => {
                self.phase = Phase::Options;
                Transition::Push(Mode::OptionMenu(OptionMenu::new()))
            }
            _ => {
                // `PrepareOakSpeech`'s `InitOptions`, which only this menu knows to skip.
                if !self.options_initialized {
                    init_options(ctx);
                }
                Transition::Pop(Outcome::Chosen(NEW_GAME))
            }
        }
    }

    fn info(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Info;
        ctx.pad.poll();
        if ctx.pad.held.contains(Joypad::A) {
            self.answered += 1;
            ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
            self.phase = Phase::Hold(AFTER_CONTINUE, After::Continue);
        } else if ctx.pad.held.contains(Joypad::B) {
            self.answered += 1;
            self.phase = Phase::Hold(BEFORE_MENU, After::DrawMenu);
        }
        Transition::Stay
    }
}

/// `DisplayContinueGameInfo`'s box, without its wait. `PrintSaveScreenText` is the same box and the
/// same four rows at the same offsets from its corner, eight rows higher.
pub fn save_screen_info(ctx: &mut Ctx, corner: (usize, usize)) {
    let (left, top) = corner;
    let at = |x: usize, y: usize| (top + y) * SCREEN_TILES_X + left + x;
    let ui = &mut ctx.screen.ui;
    ui.text_box_border(left, top, 14, 8);
    place_lines(ui, at(1, 2), &encode("PLAYER<NEXT>BADGES    <NEXT>#DEX    <NEXT>TIME"), false);
    place_lines(ui, at(8, 2), &ctx.world.player_name, false);
    let plain = |digits| NumberFormat { digits, leading_zeroes: false, left_align: false };
    print_number(ui, at(13, 4), count_set_bits(&[ctx.world.badges]) as u32, plain(2));
    print_number(ui, at(12, 6), count_set_bits(&ctx.world.pokedex.owned) as u32, plain(3));
    let time = ctx.world.play_time;
    let end = print_number(ui, at(9, 8), time.hours as u32, plain(3));
    ui.set(end % SCREEN_TILES_X, end / SCREEN_TILES_X, TIME_COLON);
    let minutes = NumberFormat { digits: 2, leading_zeroes: true, left_align: false };
    print_number(ui, end + 1, time.minutes as u32, minutes);
}

/// `InitOptions`: medium text, animations on, shift, and the letter delay's fast bit.
fn init_options(ctx: &mut Ctx) {
    ctx.world.options = crate::world::Options::default();
    ctx.world.one_frame_letter_delay = false;
}

impl ModeUpdate for MainMenu {
    /// `MainMenu`'s `InitOptions`, which a save's own options then replace.
    fn enter(&mut self, ctx: &mut Ctx) {
        if !self.save_exists {
            init_options(ctx);
        } else {
            ctx.world.one_frame_letter_delay = false;
        }
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Hold(frames, after) if frames > 1 => {
                self.phase = Phase::Hold(frames - 1, after);
                Transition::Stay
            }
            Phase::Hold(_, After::DrawMenu) => self.draw_menu(ctx),
            Phase::Hold(_, After::Choose) => self.chosen(ctx),
            Phase::Hold(_, After::WaitOnInfo) => self.info(ctx),
            Phase::Hold(_, After::Continue) => Transition::Pop(Outcome::Chosen(CONTINUE)),
            Phase::Menu => match self.input.update(ctx) {
                None => Transition::Stay,
                Some(keys) if keys.contains(Joypad::B) => Transition::Pop(Outcome::Cancelled),
                Some(_) => {
                    self.phase = Phase::Hold(AFTER_CHOICE, After::Choose);
                    Transition::Stay
                }
            },
            Phase::Info => self.info(ctx),
            Phase::Options => Transition::Stay,
        }
    }

    /// Back from the option screen, to `.mainMenuLoop` from the top.
    fn resume(&mut self, _outcome: Outcome, _ctx: &mut Ctx) -> Transition {
        self.options_initialized = true;
        self.phase = Phase::Hold(BEFORE_MENU, After::DrawMenu);
        Transition::Stay
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Menu if self.input.is_polling() => Status::Waiting(Decision::MainMenu),
            Phase::Info => Status::Waiting(Decision::ContinueGame),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::command::{Command, Reply};
    use crate::systems::play_time::PlayTime;
    use crate::world::World;
    use crate::rng::GameRng;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    fn game(save_exists: bool) -> Game {
        let mut world = World {
            player_name: encode("RED"),
            badges: 0b0000_0111,
            play_time: PlayTime { hours: 3, minutes: 7, ..PlayTime::default() },
            ..World::default()
        };
        world.pokedex.owned[0] = 0b0001_1111;
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::MainMenu(MainMenu::new(save_exists)));
        game
    }

    fn until(game: &mut Game, decision: Decision) -> u32 {
        for frames in 1..200 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(decision.clone()) {
                return frames;
            }
        }
        panic!("never waited for {decision:?}");
    }

    fn row(game: &Game, y: usize, from: usize, text: &str) {
        let bytes = encode(text);
        assert_eq!(game.ui().row(y)[from..from + bytes.len()], bytes[..], "row {y} from {from}");
    }

    #[test]
    fn with_a_save_there_are_three_rows_and_the_menu_waits_twenty_frames_to_appear() {
        let mut game = game(true);
        assert_eq!(until(&mut game, Decision::MainMenu), BEFORE_MENU as u32);
        row(&game, 2, 2, "CONTINUE");
        row(&game, 4, 2, "NEW GAME");
        row(&game, 6, 2, "OPTION");
        assert_eq!(game.ui().get(0, 7), 0x7D, "the box closes under OPTION");
        assert_eq!(game.ui().get(1, 2), CURSOR);
    }

    #[test]
    fn without_one_new_game_is_the_first_row_and_still_answers_as_new_game() {
        let mut game = game(false);
        until(&mut game, Decision::MainMenu);
        row(&game, 2, 2, "NEW GAME");
        row(&game, 4, 2, "OPTION");
        game.frame(Input::Buttons(Joypad::A));
        for _ in 0..AFTER_CHOICE {
            assert!(!game.modes().is_empty(), "the choice waits");
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
    }

    #[test]
    fn continue_shows_the_save_and_a_takes_it() {
        let mut game = game(true);
        until(&mut game, Decision::MainMenu);
        game.frame(Input::Buttons(Joypad::A));
        assert_eq!(until(&mut game, Decision::ContinueGame), (AFTER_CHOICE + INFO_STANDS) as u32);
        row(&game, 9, 5, "PLAYER");
        row(&game, 9, 12, "RED");
        row(&game, 11, 5, "BADGES");
        row(&game, 11, 18, "3");
        row(&game, 13, 5, "POKéDEX");
        row(&game, 13, 18, "5");
        row(&game, 15, 5, "TIME");
        row(&game, 15, 15, "3");
        assert_eq!(game.ui().get(16, 15), TIME_COLON);
        row(&game, 15, 17, "07");

        assert_eq!(game.frame(Input::Command(Command::Advance)).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..AFTER_CONTINUE + 1 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::Advance)]);
        assert!(game.modes().is_empty(), "CONTINUE is answered");
    }

    #[test]
    fn b_on_the_info_goes_back_to_the_menu_and_b_on_the_menu_to_the_title() {
        let mut game = game(true);
        until(&mut game, Decision::MainMenu);
        game.frame(Input::Buttons(Joypad::A));
        until(&mut game, Decision::ContinueGame);
        game.frame(Input::Buttons(Joypad::B));
        assert_eq!(until(&mut game, Decision::MainMenu), BEFORE_MENU as u32);
        game.frame(Input::Buttons(Joypad::B));
        assert!(game.modes().is_empty());
    }

    /// `MainMenu`'s `TryLoadSaveFile` is the host handing `power_on` a world, and its bad checksum
    /// is the host having no save to hand over.
    #[test]
    fn continue_takes_the_world_the_host_handed_over_into_the_overworld() {
        let saved = World {
            player_name: encode("ASH"),
            player_id: 0x1234,
            badges: 0b0000_0011,
            play_time: PlayTime { hours: 9, minutes: 41, ..PlayTime::default() },
            ..World::default()
        };
        let mut game = Game::power_on(Some(saved.clone()), GameRng::seeded(5), Pacing::Faithful);
        for _ in 0..40_000 {
            if matches!(game.modes(), [Mode::Overworld(_)]) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::TitleScreen | Decision::ContinueGame) => Input::Command(Command::Advance),
                Status::Waiting(Decision::MainMenu) => Input::Command(Command::ChooseOption(CONTINUE)),
                _ => Input::None,
            };
            game.frame(input);
        }
        assert!(matches!(game.modes(), [Mode::Overworld(_)]), "{:?}", game.status());
        let world = game.world();
        assert_eq!((&world.player_name, world.player_id, world.badges), (&saved.player_name, 0x1234, 0b0000_0011));
        assert!(world.play_time.counting, "SpecialEnterMap starts the clock");
    }

    #[test]
    fn option_opens_the_screen_and_comes_back_to_the_menu() {
        let mut game = game(true);
        until(&mut game, Decision::MainMenu);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(2))).reply, Some(Reply::Accepted));
        until(&mut game, Decision::Options);
        game.frame(Input::Buttons(Joypad::B));
        until(&mut game, Decision::MainMenu);
        assert!(matches!(game.modes(), [Mode::MainMenu(_)]));
    }
}
