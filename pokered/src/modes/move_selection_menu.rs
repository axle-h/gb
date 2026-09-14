//! `MoveSelectionMenu` with `wMoveMenuType` 2, as the PP items ask it: a mon's four moves one to a
//! row in a box at (4, 7), a dash for each empty slot.
//!
//! The cursor's items are counted from the box's top border, so the first move is item 1 and the
//! menu's own maximum is one past the last move. Stepping onto either of those two extra rows is
//! what `SelectMenuItem_CursorUp` and `_CursorDown` catch: they rub the cursor out and wrap it to the
//! other end, so it never shows there. The moves are counted up to the first empty slot.

use poke_core::move_name::PokemonMoveName;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;

const DASH: u8 = 0xE3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSelectionMenu {
    moves: [Option<PokemonMoveName>; 4],
    input: MenuInput,
}

impl MoveSelectionMenu {
    pub fn relearn(moves: [Option<PokemonMoveName>; 4]) -> Self {
        let count = moves.iter().take_while(|mv| mv.is_some()).count() as u8;
        let mut input = MenuInput::new(1, count + 1, (5, 7), Joypad::UP | Joypad::DOWN | Joypad::A | Joypad::B);
        input.single_spaced = true;
        Self { moves, input }
    }

    /// `wNumMovesMinusOne` plus one.
    pub fn rows(&self) -> u8 {
        self.input.max - 1
    }

    /// The move under the cursor, from 0.
    pub fn selected(&self) -> u8 {
        self.input.current.saturating_sub(1)
    }
}

impl ModeUpdate for MoveSelectionMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(4, 7, 14, 4);
        // `FormatMovesString` stops naming at the first empty slot and dashes out the rest.
        let named = self.rows() as usize;
        for (row, mv) in self.moves.iter().enumerate() {
            match mv {
                Some(mv) if row < named => ui.place(6, 8 + row, &mv.name()),
                _ => ui.set(6, 8 + row, DASH),
            }
        }
        // `wLastMenuItem` is `wPlayerMoveListIndex + 1`, and the caller zeroed the index.
        ctx.menu.last_item = 1;
        self.input.call(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        if keys.contains(Joypad::UP) {
            if self.input.current == 0 {
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
                self.input.current = self.rows();
            }
            self.input.call(ctx);
            return self.update(ctx);
        }
        if keys.contains(Joypad::DOWN) {
            if self.input.current == self.input.max {
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
                self.input.current = 1;
            }
            self.input.call(ctx);
            return self.update(ctx);
        }
        let chosen = self.input.current.wrapping_sub(1);
        if keys.contains(Joypad::B) {
            return Transition::Pop(Outcome::Cancelled);
        }
        Transition::Pop(Outcome::Chosen(chosen))
    }

    fn status(&self) -> Status {
        if self.input.is_polling() { Status::Waiting(Decision::MoveMenu) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::move_name::PokemonMoveName::*;
    use crate::command::{Command, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    fn game() -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::MoveSelectionMenu(MoveSelectionMenu::relearn([Some(Tackle), Some(Growl), None, None])));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..20 {
            if game.status() == Status::Waiting(Decision::MoveMenu) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        until_waiting(game);
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn cursor_rows(game: &Game) -> Vec<usize> {
        (0..18).filter(|&y| game.ui().get(5, y) == CURSOR).collect()
    }

    #[test]
    fn the_moves_one_to_a_row_and_dashes_after() {
        let mut game = game();
        until_waiting(&mut game);
        assert_eq!(game.ui().row(8)[6..12], encode("TACKLE").unwrap()[..]);
        assert_eq!(game.ui().row(9)[6..11], encode("GROWL").unwrap()[..]);
        assert_eq!((game.ui().get(6, 10), game.ui().get(6, 11)), (DASH, DASH));
        assert_eq!(cursor_rows(&game), [8]);
    }

    #[test]
    fn the_cursor_wraps_without_showing_on_the_border_or_a_dash() {
        let mut game = game();
        press(&mut game, Joypad::UP);
        until_waiting(&mut game);
        assert_eq!(cursor_rows(&game), [9], "up from the first move is the last");
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(cursor_rows(&game), [8], "and down from the last is the first");
    }

    #[test]
    fn a_names_the_move_and_b_backs_out() {
        let mut game = game();
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(1))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..10 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::ChooseOption(1))]);
        assert!(game.modes().is_empty());

        let mut backed_out = self::game();
        until_waiting(&mut backed_out);
        backed_out.frame(Input::Command(Command::CancelOption));
        for _ in 0..3 {
            backed_out.frame(Input::None);
        }
        assert!(backed_out.modes().is_empty());
    }

    #[test]
    fn a_save_mid_menu_resumes_identically() {
        let mut whole = game();
        press(&mut whole, Joypad::DOWN);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..20 {
            let input = || if frame == 5 { Input::Buttons(Joypad::DOWN) } else { Input::None };
            whole.frame(input());
            restored.frame(input());
            assert_eq!(whole.ui(), restored.ui(), "frame {frame}");
        }
    }
}
