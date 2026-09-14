//! `DisplayTwoOptionMenu`: yes/no and its seven variants, drawn over whatever is on screen and
//! taken back down again afterwards.
//!
//! The caller passes the border's corner and the cursor's position separately; every caller in the
//! cartridge derives the second from the first, so this takes the corner alone.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::{MenuExit, MenuInput};

/// `TwoOptionMenuStrings`: width, height, whether a blank line comes first, and the two options.
const MENUS: [(usize, usize, bool, &str, &str); 8] = [
    (4, 3, false, "YES", "NO"),
    (6, 3, false, "NORTH", "WEST"),
    (6, 3, false, "SOUTH", "EAST"),
    (6, 3, false, "YES", "NO"),
    (6, 3, false, "NORTH", "EAST"),
    (7, 3, false, "TRADE", "CANCEL"),
    (7, 4, true, "HEAL", "CANCEL"),
    (4, 3, false, "NO", "YES"),
];

/// `wBuffer` is 30 bytes, so exactly this much of the screen comes back. A menu wider or taller
/// than it leaves its bottom and right edges behind, which the cartridge's own comment admits.
const SAVED_WIDTH: usize = 6;
const SAVED_HEIGHT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TwoOptionMenuId {
    YesNo,
    NorthWest,
    SouthEast,
    WideYesNo,
    NorthEast,
    /// The Cable Club's, which draws its own border out of a different set of tiles. Not recreated:
    /// nothing outside the link code asks for this menu, and the link code is a non-goal.
    TradeCancel,
    HealCancel,
    /// Ignores B, because it confirms deleting a save file.
    NoYes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoOptionMenu {
    id: TwoOptionMenuId,
    /// `hl`: the border's upper left corner.
    at: (usize, usize),
    input: MenuInput,
    saved: Vec<u8>,
    chosen: u8,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Input,
    /// The `DelayFrames 15` that holds the answer on screen before the box comes down.
    Leaving(u8),
}

impl TwoOptionMenu {
    /// `second_default` is `BIT_SECOND_MENU_OPTION_DEFAULT`, which the cartridge packs into the
    /// menu id and clears as it reads it.
    pub fn new(id: TwoOptionMenuId, at: (usize, usize), second_default: bool) -> Self {
        let (_, _, blank_line, _, _) = MENUS[id as usize];
        let top = ((at.0 + 1) as u8, (at.1 + 1 + blank_line as usize) as u8);
        let input = MenuInput::new(second_default as u8, 1, top, Joypad::A | Joypad::B);
        Self { id, at, input, saved: Vec::new(), chosen: 0, phase: Phase::Input }
    }

    /// `wTwoOptionMenuID` as the cartridge stores it: the id in the low bits with
    /// `BIT_SECOND_MENU_OPTION_DEFAULT` on top.
    pub fn from_byte(byte: u8) -> (TwoOptionMenuId, bool) {
        const IDS: [TwoOptionMenuId; 8] = [
            TwoOptionMenuId::YesNo, TwoOptionMenuId::NorthWest, TwoOptionMenuId::SouthEast,
            TwoOptionMenuId::WideYesNo, TwoOptionMenuId::NorthEast, TwoOptionMenuId::TradeCancel,
            TwoOptionMenuId::HealCancel, TwoOptionMenuId::NoYes,
        ];
        (IDS[(byte & 0x7F) as usize % 8], byte & 0x80 != 0)
    }

    /// The corner worked back from the cursor. Every caller passes the two together and only the
    /// cursor reaches RAM, so this is how an observer recovers the rest.
    pub fn at_cursor(id: TwoOptionMenuId, cursor: (usize, usize), second_default: bool) -> Self {
        let blank_line = MENUS[id as usize].2 as usize;
        Self::new(id, (cursor.0 - 1, cursor.1 - 1 - blank_line), second_default)
    }

    /// `hl`: the border's upper left corner.
    pub fn corner(&self) -> (usize, usize) {
        self.at
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// `TwoOptionMenu_SaveScreenTiles` and its twin.
    fn region(&self) -> (usize, usize) {
        ((self.at.0 + SAVED_WIDTH).min(SCREEN_TILES_X), (self.at.1 + SAVED_HEIGHT).min(SCREEN_TILES_Y))
    }

    fn save_tiles(&mut self, ui: &UiSurface) {
        let (right, bottom) = self.region();
        self.saved = (self.at.1..bottom).flat_map(|y| (self.at.0..right).map(move |x| (x, y)))
            .map(|(x, y)| ui.get(x, y))
            .collect();
    }

    fn restore_tiles(&self, ui: &mut UiSurface) {
        let (right, bottom) = self.region();
        let mut saved = self.saved.iter();
        for y in self.at.1..bottom {
            for x in self.at.0..right {
                if let Some(&tile) = saved.next() {
                    ui.set(x, y, tile);
                }
            }
        }
    }
}

impl ModeUpdate for TwoOptionMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        let (width, height, blank_line, first, second) = MENUS[self.id as usize];
        self.save_tiles(&ctx.screen.ui);
        ctx.screen.ui.text_box_border(self.at.0, self.at.1, width, height);
        let text = |s: &str| poke_core::charmap::encode(s).expect("a two option menu's words encode");
        let top = self.at.1 + 1 + blank_line as usize;
        ctx.screen.ui.place(self.at.0 + 2, top, &text(first));
        ctx.screen.ui.place(self.at.0 + 2, top + 2, &text(second));
        ctx.menu.last_item = 0;
        self.input.call(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Leaving(frames) if frames > 1 => {
                self.phase = Phase::Leaving(frames - 1);
                Transition::Stay
            }
            Phase::Leaving(_) => {
                self.restore_tiles(&mut ctx.screen.ui);
                Transition::Pop(Outcome::Chosen(self.chosen))
            }
            Phase::Input => {
                let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
                let backed_out = keys.contains(Joypad::B);
                if backed_out && self.id == TwoOptionMenuId::NoYes {
                    // The one menu B cannot answer: it asks again.
                    self.input.call(ctx);
                    return self.update(ctx);
                }
                // B is not a refusal here, it picks the second option, which is the safe one.
                self.chosen = if backed_out { 1 } else { self.input.current };
                self.input.current = self.chosen;
                ctx.menu.chosen_item = self.chosen;
                ctx.menu.exit_method =
                    if self.chosen == 0 { MenuExit::CHOSE_FIRST } else { MenuExit::CHOSE_SECOND };
                self.phase = Phase::Leaving(15);
                Transition::Stay
            }
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Input if self.input.is_polling() => Status::Waiting(Decision::TwoOption),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::command::{Command, Refusal, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;
    /// `InitYesNoTextBoxParameters`, the corner every plain yes/no is drawn at.
    const YES_NO_AT: (usize, usize) = (14, 7);

    fn game_at(id: TwoOptionMenuId, at: (usize, usize), second_default: bool) -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::TwoOptionMenu(TwoOptionMenu::new(id, at, second_default)));
        game
    }

    fn game() -> Game {
        game_at(TwoOptionMenuId::YesNo, YES_NO_AT, false)
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::TwoOption) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    /// Presses, then the frames the menu takes to come down.
    fn answer(game: &mut Game, button: Joypad) -> u8 {
        until_waiting(game);
        press(game, button);
        for _ in 0..25 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty(), "the menu closed");
        game.menu().chosen_item
    }

    fn cursor_row(game: &Game, x: usize) -> Option<usize> {
        (0..18).find(|&y| game.ui().get(x, y) == CURSOR)
    }

    #[test]
    fn yes_over_no_with_the_cursor_on_yes() {
        let mut game = game();
        until_waiting(&mut game);
        assert_eq!(game.ui().row(8)[16..19], encode("YES").unwrap()[..]);
        assert_eq!(game.ui().row(10)[16..18], encode("NO").unwrap()[..]);
        assert_eq!(cursor_row(&game, 15), Some(8), "the cursor is one column left of the words");
    }

    /// `BIT_SECOND_MENU_OPTION_DEFAULT`, which the cartridge packs into the menu id.
    #[test]
    fn the_second_option_can_be_the_one_already_chosen() {
        let mut game = game_at(TwoOptionMenuId::YesNo, YES_NO_AT, true);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game, 15), Some(10), "on NO");
        assert_eq!(TwoOptionMenu::from_byte(0x80), (TwoOptionMenuId::YesNo, true));
        assert_eq!(TwoOptionMenu::from_byte(6), (TwoOptionMenuId::HealCancel, false));
    }

    #[test]
    fn a_chooses_the_row_and_b_chooses_the_second() {
        assert_eq!(answer(&mut game(), Joypad::A), 0);
        assert_eq!(answer(&mut game(), Joypad::B), 1, "B is the second option, not a refusal");
        let mut game = game();
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        assert_eq!(answer(&mut game, Joypad::A), 1);
    }

    #[test]
    fn the_exit_method_says_which_of_the_two_it_was() {
        let mut chose_first = game();
        answer(&mut chose_first, Joypad::A);
        assert_eq!(chose_first.menu().exit_method, MenuExit::CHOSE_FIRST);
        let mut chose_second = game();
        answer(&mut chose_second, Joypad::B);
        assert_eq!(chose_second.menu().exit_method, MenuExit::CHOSE_SECOND);
    }

    /// The one menu B cannot answer, because it confirms deleting a save file.
    #[test]
    fn the_no_yes_menu_ignores_b() {
        let mut game = game_at(TwoOptionMenuId::NoYes, YES_NO_AT, false);
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        for _ in 0..20 {
            game.frame(Input::None);
        }
        assert!(!game.modes().is_empty(), "B asks again");
        assert_eq!(answer(&mut game, Joypad::A), 0, "and NO is the first row here");
    }

    #[test]
    fn a_blank_line_pushes_the_words_down_a_row() {
        let mut game = game_at(TwoOptionMenuId::HealCancel, (11, 6), false);
        until_waiting(&mut game);
        assert_eq!(game.ui().row(8)[13..17], encode("HEAL").unwrap()[..]);
        assert_eq!(game.ui().row(10)[13..19], encode("CANCEL").unwrap()[..]);
        assert_eq!(cursor_row(&game, 12), Some(8));
    }

    /// `wBuffer` is 30 bytes, so only 6 columns by 5 rows come back. A menu bigger than that leaves
    /// its own edges on the screen, which the cartridge's comment admits and nothing fixes.
    #[test]
    fn a_menu_wider_than_the_buffer_leaves_its_edges_behind() {
        let mut game = game_at(TwoOptionMenuId::HealCancel, (11, 6), false);
        until_waiting(&mut game);
        answer(&mut game, Joypad::A);
        assert_eq!(game.ui().get(12, 6), UiSurface::BLANK, "inside the saved 6 by 5, restored");
        assert_ne!(game.ui().get(20 - 1, 6), UiSurface::BLANK, "the right edge is past it and stays");
    }

    #[test]
    fn a_command_answers_it() {
        let mut game = game();
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(1))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..60 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::ChooseOption(1))]);
        assert_eq!(game.menu().chosen_item, 1);
    }

    #[test]
    fn a_third_option_is_refused() {
        let mut game = game();
        until_waiting(&mut game);
        let reply = game.frame(Input::Command(Command::ChooseOption(2))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }

    #[test]
    fn a_save_mid_menu_resumes_identically() {
        let mut whole = game();
        until_waiting(&mut whole);
        whole.frame(Input::Buttons(Joypad::DOWN));
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..40 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}
