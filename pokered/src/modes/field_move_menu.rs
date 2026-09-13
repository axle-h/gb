//! `DisplayFieldMoveMonMenu`: the submenu a chosen party mon opens, listing what it can do outside
//! a battle above `STATS`, `SWITCH` and `CANCEL`.
//!
//! The box grows upwards, two rows for every field move plus one blank, and its bottom stays on the
//! bottom of the screen. Its width comes from the leftmost name any of the moves wants, so a single
//! `SOFTBOILED` widens the box for every row above it.

use poke_core::move_name::PokemonMoveName;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::UiSurface;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::systems::field_moves::{name_of, move_of, FieldMoves};

/// `PokemonMenuEntries`, always the last three rows.
const ENTRIES: [&str; 3] = ["STATS", "SWITCH", "CANCEL"];
/// Where the cursor and the entries go when there is nothing to list.
const EMPTY_CURSOR_X: u8 = 12;

/// What a chosen row means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldMoveChoice {
    Move(PokemonMoveName),
    Stats,
    Switch,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMoveMenu {
    moves: FieldMoves,
    input: MenuInput,
    /// `wTileMapBackup`, which the cartridge's caller saves before the box is drawn and puts back
    /// the moment the menu returns.
    saved: Option<UiSurface>,
}

impl FieldMoveMenu {
    pub fn new(moves: FieldMoves) -> Self {
        let rows = moves.len() as u8;
        let (x, y) = if rows == 0 {
            (EMPTY_CURSOR_X, 12)
        } else {
            (moves.leftmost, 12 - 2 * rows)
        };
        let input = MenuInput::new(0, rows + 2, (x, y), Joypad::A | Joypad::B);
        Self { moves, input, saved: None }
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    pub fn rows(&self) -> u8 {
        self.moves.len() as u8 + 3
    }

    /// The row's meaning: the field moves first, then the three that are always there.
    pub fn choice(&self, row: u8) -> FieldMoveChoice {
        match self.moves.names.get(row as usize) {
            Some(&name) => move_of(name).map_or(FieldMoveChoice::Cancel, FieldMoveChoice::Move),
            None => match row as usize - self.moves.len() {
                0 => FieldMoveChoice::Stats,
                1 => FieldMoveChoice::Switch,
                _ => FieldMoveChoice::Cancel,
            },
        }
    }

    fn draw(&self, ui: &mut UiSurface) {
        let text = |s: &str| poke_core::charmap::encode(s).expect("the menu's words encode");
        let rows = self.moves.len() as u8;
        let entries_x = if rows == 0 {
            ui.text_box_border(11, 11, 7, 5);
            EMPTY_CURSOR_X as usize + 1
        } else {
            let left = self.moves.leftmost as usize;
            // Two rows a move and one blank, with the bottom left where it was.
            ui.text_box_border(left - 1, 10 - 2 * rows as usize, 19 - left, 6 + 2 * rows as usize);
            for (row, &name) in self.moves.names.iter().enumerate() {
                ui.place(left + 1, 12 - 2 * rows as usize + 2 * row, &text(name_of(name)));
            }
            left + 1
        };
        for (row, entry) in ENTRIES.iter().enumerate() {
            ui.place(entries_x, 12 + 2 * row, &text(entry));
        }
    }
}

impl ModeUpdate for FieldMoveMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.saved = Some(ctx.screen.ui.clone());
        self.draw(&mut ctx.screen.ui);
        ctx.menu.last_item = 0;
        self.input.call(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        // The caller puts the screen back before it looks at what was pressed.
        if let Some(saved) = self.saved.take() {
            ctx.screen.ui = saved;
        }
        if keys.contains(Joypad::B) {
            return Transition::Pop(Outcome::Cancelled);
        }
        ctx.menu.chosen_item = self.input.current;
        Transition::Pop(Outcome::Chosen(self.input.current))
    }

    fn status(&self) -> Status {
        if self.input.is_polling() { Status::Waiting(Decision::FieldMoveMenu) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::command::{Command, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::systems::field_moves::field_moves;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    fn menu(moves: [PokemonMoveName; 4]) -> Game {
        let found = field_moves(moves.map(|one| one as u8));
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::FieldMoveMenu(FieldMoveMenu::new(found)));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::FieldMoveMenu) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the submenu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn cursor(game: &Game) -> Option<(usize, usize)> {
        (0..18).flat_map(|y| (0..20).map(move |x| (x, y))).find(|&(x, y)| game.ui().get(x, y) == CURSOR)
    }

    const NONE: [PokemonMoveName; 4] = [PokemonMoveName::Tackle; 4];

    #[test]
    fn a_mon_with_nothing_to_offer_gets_the_three_rows_and_a_fixed_box() {
        let mut game = menu(NONE);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(11, 11), 0x79, "the box corner");
        assert_eq!(game.ui().row(12)[13..18], encode("STATS").unwrap()[..]);
        assert_eq!(game.ui().row(14)[13..19], encode("SWITCH").unwrap()[..]);
        assert_eq!(game.ui().row(16)[13..19], encode("CANCEL").unwrap()[..]);
        assert_eq!(cursor(&game), Some((12, 12)));
    }

    /// Every field move lifts the top of the box two rows and leaves the bottom where it was.
    #[test]
    fn one_field_move_raises_the_box_and_sits_above_stats() {
        let mut game = menu([PokemonMoveName::Cut, PokemonMoveName::Tackle, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(11, 8), 0x79, "the box grew upwards");
        assert_eq!(game.ui().get(11, 17), 0x7D, "and its bottom stayed put");
        assert_eq!(game.ui().row(10)[13..16], encode("CUT").unwrap()[..]);
        assert_eq!(game.ui().row(12)[13..18], encode("STATS").unwrap()[..]);
        assert_eq!(cursor(&game), Some((12, 10)), "on the field move");
    }

    /// The widest name decides the box for all of them.
    #[test]
    fn a_wider_name_moves_the_whole_box_left() {
        let mut game = menu([PokemonMoveName::Cut, PokemonMoveName::Strength, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(9, 6), 0x79, "two moves, and STRENGTH's column");
        assert_eq!(game.ui().row(8)[11..14], encode("CUT").unwrap()[..]);
        assert_eq!(game.ui().row(10)[11..19], encode("STRENGTH").unwrap()[..]);
        assert_eq!(game.ui().row(12)[11..16], encode("STATS").unwrap()[..]);
        assert_eq!(cursor(&game), Some((10, 8)));
    }

    #[test]
    fn a_row_says_what_it_is() {
        let game = menu([PokemonMoveName::Cut, PokemonMoveName::Tackle, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        let menu = match game.modes().last() {
            Some(Mode::FieldMoveMenu(menu)) => menu,
            _ => panic!("no submenu"),
        };
        assert_eq!(menu.choice(0), FieldMoveChoice::Move(PokemonMoveName::Cut));
        assert_eq!(menu.choice(1), FieldMoveChoice::Stats);
        assert_eq!(menu.choice(2), FieldMoveChoice::Switch);
        assert_eq!(menu.choice(3), FieldMoveChoice::Cancel);
    }

    #[test]
    fn the_screen_underneath_comes_back_either_way() {
        let mut game = menu(NONE);
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        assert!(game.modes().is_empty());
        assert_eq!(game.ui().get(11, 11), UiSurface::BLANK, "the box is gone");
    }

    #[test]
    fn a_command_chooses_a_row() {
        let mut game = menu(NONE);
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(1))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..60 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::ChooseOption(1))]);
        assert_eq!(game.menu().chosen_item, 1, "SWITCH");
    }
}
