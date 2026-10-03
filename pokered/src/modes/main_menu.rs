//! `MainMenu` from `.mainMenuLoop`: `CONTINUE`, `NEW GAME` and `OPTION` after the title screen.
//!
//! `CONTINUE` is offered while the host holds any save slot, and opens the slots to load one; the
//! host replaces the game with the slot, and B comes back to the menu. Otherwise the menu answers
//! [`NEW_GAME`], or `Cancelled` for B, which is the title screen again. `OPTION` is answered inside,
//! by the option screen and the menu over again.
//!
//! The waits are pacing, not loading, so both are kept: the 20 frames before the menu is drawn and
//! the 20 after a choice.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::option_menu::OptionMenu;
use crate::modes::slot_selector::SlotSelector;
use crate::systems::status_screen::{encode, place_lines};

pub const CONTINUE: u8 = 0;
pub const NEW_GAME: u8 = 1;
const OPTION: u8 = 2;

/// `.mainMenuLoop`'s and the choice's `DelayFrames 20`.
const BEFORE_MENU: u8 = 20;
const AFTER_CHOICE: u8 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A `DelayFrames` counting down, and what runs when it ends.
    Hold(u8, After),
    Menu,
    /// The option screen or the slots are up.
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    DrawMenu,
    Choose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainMenu {
    /// A slot is used, as the menu was last drawn.
    save_exists: bool,
    phase: Phase,
    input: MenuInput,
    /// `wOptionsInitialized`: the option screen was visited, so a new game keeps what was set there.
    #[serde(default)]
    options_initialized: bool,
}

impl MainMenu {
    pub fn new() -> Self {
        let input = MenuInput::new(0, 0, (1, 2), Joypad::A | Joypad::B | Joypad::START);
        Self { save_exists: false, phase: Phase::Hold(BEFORE_MENU, After::DrawMenu), input, options_initialized: false }
    }

    /// `CONTINUE` only when there is a save to continue.
    pub fn rows(&self) -> u8 {
        if self.save_exists { 3 } else { 2 }
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// `.mainMenuLoop` after its wait: the saved cursors zeroed, the screen cleared and the menu up,
    /// over the slots as the host now holds them.
    fn draw_menu(&mut self, ctx: &mut Ctx) -> Transition {
        self.save_exists = any_slot(ctx);
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
                self.phase = Phase::Options;
                Transition::Push(Mode::SlotSelector(SlotSelector::load_only()))
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
}

/// Whether the host holds any slot to continue.
fn any_slot(ctx: &Ctx) -> bool {
    ctx.slots.iter().any(Option::is_some)
}

/// `InitOptions`: medium text, animations on, shift, and the letter delay's fast bit.
fn init_options(ctx: &mut Ctx) {
    ctx.world.options = crate::world::Options::default();
    ctx.world.one_frame_letter_delay = false;
}

impl ModeUpdate for MainMenu {
    /// `MainMenu`'s `InitOptions`, which the cartridge's save then replaced: here a game that has
    /// been played keeps its own.
    fn enter(&mut self, ctx: &mut Ctx) {
        if !any_slot(ctx) {
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
            Phase::Menu => match self.input.update(ctx) {
                None => Transition::Stay,
                Some(keys) if keys.contains(Joypad::B) => Transition::Pop(Outcome::Cancelled),
                Some(_) => {
                    self.phase = Phase::Hold(AFTER_CHOICE, After::Choose);
                    Transition::Stay
                }
            },
            Phase::Options => Transition::Stay,
        }
    }

    /// Back from the option screen or the slots, to `.mainMenuLoop` from the top.
    fn resume(&mut self, _outcome: Outcome, _ctx: &mut Ctx) -> Transition {
        self.options_initialized |= self.choice() == OPTION;
        self.phase = Phase::Hold(BEFORE_MENU, After::DrawMenu);
        Transition::Stay
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Menu if self.input.is_polling() => Status::Waiting(Decision::MainMenu),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::command::{Command, Reply};
    use crate::save_slots::{SavedAt, Slot, SlotSummary, Thumbnail};
    use crate::systems::play_time::PlayTime;
    use crate::world::World;
    use crate::rng::GameRng;
    use crate::{Game, Input, Pacing};
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
        if save_exists {
            let summary = SlotSummary::of(game.world(), Thumbnail::of(&game.screen.frame()));
            game.set_slots(vec![None, Some(Slot { summary, saved_at: SavedAt { unix_seconds: 0, utc_offset_minutes: 0 } })]);
        }
        game.push(Mode::MainMenu(MainMenu::new()));
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
    fn with_a_slot_there_are_three_rows_and_the_menu_waits_twenty_frames_to_appear() {
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
    fn continue_opens_the_slots_to_load_and_b_comes_back_to_the_menu() {
        let mut game = game(true);
        until(&mut game, Decision::MainMenu);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(CONTINUE))).reply, Some(Reply::Accepted));
        until(&mut game, Decision::SlotSelector);
        let [Mode::MainMenu(_), Mode::SlotSelector(selector)] = game.modes() else { panic!("{:?}", game.modes()) };
        assert_eq!(selector.current(), 1, "on the one used slot");
        game.frame(Input::Buttons(Joypad::B));
        until(&mut game, Decision::MainMenu);
        row(&game, 2, 2, "CONTINUE");
        game.frame(Input::Buttons(Joypad::B));
        assert!(game.modes().is_empty());
    }

    /// The slots are read each time the menu is drawn, so one emptied under the selector leaves
    /// NEW GAME on top.
    #[test]
    fn a_menu_drawn_with_no_slots_left_has_no_continue() {
        let mut game = game(true);
        until(&mut game, Decision::MainMenu);
        game.frame(Input::Command(Command::ChooseOption(CONTINUE)));
        until(&mut game, Decision::SlotSelector);
        game.set_slots(Vec::new());
        game.frame(Input::Buttons(Joypad::B));
        until(&mut game, Decision::MainMenu);
        row(&game, 2, 2, "NEW GAME");
        row(&game, 4, 2, "OPTION");
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
