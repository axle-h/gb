//! `DisplayStartMenu` and `DrawStartMenu`: the menu START opens, and the six screens it reaches.
//!
//! The cursor may sit one row below the last entry: `wMaxMenuItem` is one past `EXIT`, and it is
//! the start menu rather than `HandleMenuInput` that wraps, which is why a press of Up or Down is
//! watched and answered here.

use poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::item_menu::ItemMenu;
use crate::modes::option_menu::OptionMenu;
use crate::modes::pokemon_menu::PokemonMenu;

/// The entries in the order `.displayMenuItem` dispatches them, which is the order they are drawn
/// when the player has the Pokédex. Without it `POKéDEX` is missing and every row moves up one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum StartMenuEntry {
    Pokedex,
    Pokemon,
    Item,
    TrainerInfo,
    SaveReset,
    Option,
    Exit,
}

const FIRST_ROW: usize = 2;
const LABEL_X: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartMenu {
    entries: Vec<StartMenuEntry>,
    input: MenuInput,
    /// `wTileMapBackup2`. The cartridge saves it here and has each sub-menu restore it; restoring
    /// it as the sub-menu returns puts the same screen back a frame earlier.
    saved: Option<UiSurface>,
}

impl StartMenu {
    pub fn new() -> Self {
        Self { entries: Vec::new(), input: MenuInput::new(0, 0, (0, 0), Joypad::empty()), saved: None }
    }

    /// The index under the cursor, counting the blank row past `EXIT` as one past the last.
    pub fn selected(&self) -> u8 {
        self.input.current
    }

    pub fn index_of(&self, entry: StartMenuEntry) -> Option<u8> {
        self.entries.iter().position(|&e| e == entry).map(|index| index as u8)
    }

    /// `RedisplayStartMenu`: the box, the entries and a fresh `HandleMenuInput`.
    fn redisplay(&mut self, ctx: &mut Ctx) {
        let has_pokedex = ctx.world.events.is_set(EVENT_GOT_POKEDEX as u16);
        self.entries = [StartMenuEntry::Pokedex, StartMenuEntry::Pokemon, StartMenuEntry::Item,
                        StartMenuEntry::TrainerInfo, StartMenuEntry::SaveReset, StartMenuEntry::Option,
                        StartMenuEntry::Exit]
            .into_iter()
            .filter(|&entry| has_pokedex || entry != StartMenuEntry::Pokedex)
            .collect();

        let height = if has_pokedex { 14 } else { 12 };
        ctx.screen.ui.text_box_border(10, 0, 8, height);
        for (row, &entry) in self.entries.iter().enumerate() {
            let label = match entry {
                StartMenuEntry::TrainerInfo => ctx.world.player_name.clone(),
                other => text(other),
            };
            ctx.screen.ui.place(LABEL_X, FIRST_ROW + row * 2, &label);
        }

        // `wMaxMenuItem` is one past `EXIT`, so the cursor can reach a blank row.
        let watched = Joypad::DOWN | Joypad::UP | Joypad::START | Joypad::B | Joypad::A;
        self.input = MenuInput::new(ctx.menu.battle_and_start, self.entries.len() as u8, (11, 2), watched);
        ctx.menu.last_item = ctx.menu.battle_and_start;
        self.input.call(ctx);
    }

    /// `CloseStartMenu`. Its wait for A to be let go ends inside the frame it starts, because an
    /// edge is against the last poll and the loop polls again at once.
    fn close(&self, ctx: &mut Ctx) -> Transition {
        ctx.pad.poll();
        ctx.screen.tiles.load_text_box_tiles();
        Transition::Pop(Outcome::Done)
    }
}

impl Default for StartMenu {
    fn default() -> Self {
        Self::new()
    }
}

fn text(entry: StartMenuEntry) -> Vec<u8> {
    let word = match entry {
        StartMenuEntry::Pokedex => "POKéDEX",
        StartMenuEntry::Pokemon => "POKéMON",
        StartMenuEntry::Item => "ITEM",
        // `RESET` is the linked game's label, and a link state is not recreated.
        StartMenuEntry::SaveReset => "SAVE",
        StartMenuEntry::Option => "OPTION",
        StartMenuEntry::Exit => "EXIT",
        StartMenuEntry::TrainerInfo => unreachable!("the player's name is the label"),
    };
    poke_core::charmap::encode(word).expect("the start menu's entries encode")
}

impl ModeUpdate for StartMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.redisplay(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };

        if keys.contains(Joypad::UP) {
            // Only a press made at the top of the menu with the cursor already drawn there wraps.
            if self.input.current == 0 && ctx.menu.last_item == 0 {
                self.input.current = self.entries.len() as u8 - 1;
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
            }
            self.input.call(ctx);
            return self.update(ctx);
        }
        if keys.contains(Joypad::DOWN) {
            if self.input.current == self.entries.len() as u8 {
                self.input.current = 0;
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
            }
            self.input.call(ctx);
            return self.update(ctx);
        }

        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        ctx.menu.battle_and_start = self.input.current;
        if keys.intersects(Joypad::B | Joypad::START) {
            return self.close(ctx);
        }
        self.saved = Some(ctx.screen.ui.clone());
        match self.entries.get(self.input.current as usize) {
            Some(StartMenuEntry::Option) => {
                ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
                Transition::Push(Mode::OptionMenu(OptionMenu::new()))
            }
            // The other five screens are their own chunks; the choice is answered rather than run.
            Some(StartMenuEntry::Pokemon) => Transition::Push(Mode::PokemonMenu(PokemonMenu::new())),
            Some(StartMenuEntry::Item) => Transition::Push(Mode::ItemMenu(ItemMenu::new())),
            Some(&entry) if entry != StartMenuEntry::Exit => Transition::Pop(Outcome::Chosen(entry as u8)),
            _ => self.close(ctx),
        }
    }

    fn resume(&mut self, _outcome: Outcome, ctx: &mut Ctx) -> Transition {
        if let Some(saved) = self.saved.take() {
            ctx.screen.ui = saved;
        }
        ctx.screen.tiles.load_text_box_tiles();
        self.redisplay(ctx);
        self.update(ctx)
    }

    fn status(&self) -> Status {
        if self.input.is_polling() { Status::Waiting(Decision::StartMenu) } else { Status::Busy }
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
    const BOTTOM_LEFT: u8 = 0x7D;

    fn game(has_pokedex: bool) -> Game {
        let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
        if has_pokedex {
            world.events.set(EVENT_GOT_POKEDEX as u16);
        }
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::StartMenu(StartMenu::new()));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::StartMenu) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the start menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn label(game: &Game, y: usize) -> Vec<u8> {
        let mut bytes: Vec<u8> = game.ui().row(y)[LABEL_X..19].to_vec();
        while bytes.last() == Some(&UiSurface::BLANK) {
            bytes.pop();
        }
        bytes
    }

    fn cursor_row(game: &Game) -> Option<usize> {
        (0..18).find(|&y| game.ui().get(11, y) == CURSOR)
    }

    #[test]
    fn seven_rows_with_the_pokedex_in_a_taller_box() {
        let mut game = game(true);
        until_waiting(&mut game);
        for (row, word) in [(2, "POKéDEX"), (4, "POKéMON"), (6, "ITEM"), (8, "RED"),
                            (10, "SAVE"), (12, "OPTION"), (14, "EXIT")] {
            assert_eq!(label(&game, row), encode(word).unwrap(), "row {row}");
        }
        assert_eq!(game.ui().get(10, 15), BOTTOM_LEFT, "the box ends below EXIT");
        assert_eq!(cursor_row(&game), Some(2));
    }

    #[test]
    fn six_rows_without_it_and_every_row_moves_up_one() {
        let mut game = game(false);
        until_waiting(&mut game);
        for (row, word) in [(2, "POKéMON"), (4, "ITEM"), (6, "RED"), (8, "SAVE"),
                            (10, "OPTION"), (12, "EXIT")] {
            assert_eq!(label(&game, row), encode(word).unwrap(), "row {row}");
        }
        assert_eq!(game.ui().get(10, 13), BOTTOM_LEFT, "a box two rows shorter");
    }

    /// `wMaxMenuItem` is one past `EXIT`, and the start menu rather than `HandleMenuInput` catches
    /// the overshoot, which is why Up and Down are watched keys here.
    #[test]
    fn the_cursor_wraps_at_both_ends() {
        let mut game = game(true);
        until_waiting(&mut game);
        press(&mut game, Joypad::UP);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(14), "up from the top lands on EXIT");
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(2), "and down from EXIT comes back to the top");
    }

    #[test]
    fn a_step_that_does_not_overshoot_just_moves() {
        let mut game = game(true);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(4));
        press(&mut game, Joypad::UP);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(2), "back to the top without wrapping past it");
    }

    #[test]
    fn option_opens_the_screen_and_leaving_it_redraws_the_menu() {
        let mut game = game(true);
        until_waiting(&mut game);
        for _ in 0..5 {
            press(&mut game, Joypad::DOWN);
            until_waiting(&mut game);
        }
        assert_eq!(cursor_row(&game), Some(12), "OPTION");
        press(&mut game, Joypad::A);
        for _ in 0..20 {
            game.frame(Input::None);
        }
        assert!(matches!(game.modes().last(), Some(Mode::OptionMenu(_))), "{:?}", game.modes().len());
        press(&mut game, Joypad::B);
        until_waiting(&mut game);
        assert!(matches!(game.modes().last(), Some(Mode::StartMenu(_))));
        assert_eq!(label(&game, 12), encode("OPTION").unwrap(), "the menu is back");
        assert_eq!(cursor_row(&game), Some(12), "on the row it left from");
    }

    #[test]
    fn b_closes_the_menu_and_it_reopens_where_it_closed() {
        let mut game = game(true);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        assert!(game.modes().is_empty());
        game.push(Mode::StartMenu(StartMenu::new()));
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(4), "wBattleAndStartSavedMenuItem outlives the menu");
    }

    #[test]
    fn a_command_walks_to_the_row_it_names() {
        let mut game = game(true);
        until_waiting(&mut game);
        let command = Command::ChooseStartMenuEntry(StartMenuEntry::TrainerInfo);
        assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..60 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(command)]);
        assert!(game.modes().is_empty(), "TRAINER INFO is another chunk's, so the choice is answered");
    }

    #[test]
    fn a_row_the_menu_does_not_have_is_refused() {
        let mut game = game(false);
        until_waiting(&mut game);
        let reply = game.frame(Input::Command(Command::ChooseStartMenuEntry(StartMenuEntry::Pokedex))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }

    #[test]
    fn a_command_closes_it_too() {
        let mut game = game(true);
        until_waiting(&mut game);
        game.frame(Input::Command(Command::CloseStartMenu));
        for _ in 0..20 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
    }

    #[test]
    fn a_save_mid_menu_resumes_identically() {
        let mut whole = game(true);
        until_waiting(&mut whole);
        press(&mut whole, Joypad::DOWN);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..60 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}
