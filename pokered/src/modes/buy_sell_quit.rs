//! `DoBuySellQuitMenu`: the mart's front door. `QUIT` answers the same way B does, so a caller has
//! one thing to check rather than two.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::text_boxes::TextBoxId;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::{MenuExit, MenuInput};

const QUIT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuySellQuitMenu {
    input: MenuInput,
}

impl BuySellQuitMenu {
    pub fn new() -> Self {
        Self { input: MenuInput::new(0, QUIT, (1, 1), Joypad::A | Joypad::B) }
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }
}

impl Default for BuySellQuitMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for BuySellQuitMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        TextBoxId::BuySellQuitTemplate.draw(&mut ctx.screen.ui);
        ctx.menu.last_item = 0;
        self.input.call(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        let chosen = self.input.current;
        ctx.menu.chosen_item = chosen;
        ctx.menu.exit_method = if keys.contains(Joypad::B) || chosen == QUIT {
            MenuExit::Cancelled
        } else {
            MenuExit::Chose
        };
        Transition::Pop(Outcome::Chosen(chosen))
    }

    fn status(&self) -> Status {
        if self.input.is_polling() { Status::Waiting(Decision::BuySellQuit) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn game() -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::BuySellQuitMenu(BuySellQuitMenu::new()));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::BuySellQuit) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the mart's menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn answer(presses: &[Joypad]) -> (u8, MenuExit) {
        let mut game = game();
        for &button in presses {
            until_waiting(&mut game);
            press(&mut game, button);
        }
        for _ in 0..10 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        (game.menu().chosen_item, game.menu().exit_method)
    }

    #[test]
    fn buy_is_a_choice_and_quit_is_not() {
        assert_eq!(answer(&[Joypad::A]), (0, MenuExit::Chose), "BUY");
        assert_eq!(answer(&[Joypad::DOWN, Joypad::A]), (1, MenuExit::Chose), "SELL");
        assert_eq!(answer(&[Joypad::DOWN, Joypad::DOWN, Joypad::A]), (2, MenuExit::Cancelled), "QUIT");
    }

    #[test]
    fn b_backs_out_wherever_the_cursor_is() {
        assert_eq!(answer(&[Joypad::DOWN, Joypad::B]), (1, MenuExit::Cancelled));
    }
}
