//! `DisplayOptionMenu`: text speed, battle animation and battle style, each a row of words with the
//! cursor standing in for the setting, and below them the ruleset, which the cartridge has no row
//! for. It keeps no selection of its own beyond the cursor's position, so the cursor columns *are*
//! the options, packed into `wOptions` and `World::ruleset` every frame.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::modes::menu_input::UNFILLED_CURSOR;
use crate::world::{BattleStyle, Options, Ruleset, TextSpeed};

/// `TextSpeedOptionData`: the cursor column for each speed, and the frames it delays a letter. The
/// last row is the terminator `IsInArray` stops on, and its column is the default a speed the table
/// does not name falls back to.
const TEXT_SPEED_OPTIONS: [(u8, TextSpeed); 3] =
    [(14, TextSpeed::Slow), (7, TextSpeed::Medium), (1, TextSpeed::Fast)];
const DEFAULT_TEXT_SPEED_X: u8 = 7;

/// The row each setting's cursor sits on, which is two below its label because `<NEXT>` is two
/// lines. Cancel has no label of its own. Rows are counted down the whole screen, which is four
/// taller than the Game Boy's, so the ruleset's box and Cancel are scrolled into view.
const TEXT_SPEED_Y: u8 = 3;
const BATTLE_ANIM_Y: u8 = 8;
const BATTLE_STYLE_Y: u8 = 13;
const RULESET_Y: u8 = 18;
const CANCEL_Y: u8 = 21;
const ROWS: [u8; 5] = [TEXT_SPEED_Y, BATTLE_ANIM_Y, BATTLE_STYLE_Y, RULESET_Y, CANCEL_Y];
const BOX_TOPS: [u8; 4] = [0, 5, 10, 15];
/// The rows above the ruleset's box, all a comparison with the cartridge's screen can hold it to.
pub const CARTRIDGE_ROWS: usize = BOX_TOPS[3] as usize;
const SCROLLED: u8 = CANCEL_Y + 1 - SCREEN_TILES_Y as u8;

const WORDS: [(u8, u8, &str); 9] = [
    (1, 1, "TEXT SPEED"),
    (1, TEXT_SPEED_Y, " FAST  MEDIUM SLOW"),
    (1, 6, "BATTLE ANIMATION"),
    (1, BATTLE_ANIM_Y, " ON       OFF"),
    (1, 11, "BATTLE STYLE"),
    (1, BATTLE_STYLE_Y, " SHIFT    SET"),
    (1, 16, "RULESET"),
    (1, RULESET_Y, " GEN 1    MODERN"),
    (2, CANCEL_Y, "CANCEL"),
];

/// The two columns a toggle moves between, `xor 1 ^ 10`.
const LEFT: u8 = 1;
const RIGHT: u8 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionMenu {
    /// `wTopMenuItemY` and `wTopMenuItemX`, which here are the cursor itself rather than a row index.
    cursor: (u8, u8),
    /// `wOptionsTextSpeedCursorX`, `wOptionsBattleAnimCursorX`, `wOptionsBattleStyleCursorX`.
    text_speed_x: u8,
    battle_anim_x: u8,
    battle_style_x: u8,
    #[serde(default = "modern_x")]
    ruleset_x: u8,
    /// The first row on the screen: 0, or `SCROLLED` while the ruleset or Cancel is chosen.
    #[serde(default)]
    top: u8,
}

fn modern_x() -> u8 {
    RIGHT
}

impl OptionMenu {
    pub fn new() -> Self {
        Self { cursor: (1, TEXT_SPEED_Y), text_speed_x: DEFAULT_TEXT_SPEED_X, battle_anim_x: LEFT, battle_style_x: LEFT,
               ruleset_x: RIGHT, top: 0 }
    }

    /// `SetCursorPositionsFromOptions`, including the `▷` it leaves on every row.
    fn cursors_from_options(&mut self, ctx: &mut Ctx) {
        let options = ctx.world.options;
        self.text_speed_x = TEXT_SPEED_OPTIONS.iter()
            .find(|&&(_, speed)| speed == options.text_speed)
            .map_or(DEFAULT_TEXT_SPEED_X, |&(x, _)| x);
        self.battle_anim_x = if options.battle_animation { LEFT } else { RIGHT };
        self.battle_style_x = if options.battle_style == BattleStyle::Shift { LEFT } else { RIGHT };
        self.ruleset_x = if ctx.world.ruleset.is_gen1() { LEFT } else { RIGHT };
        self.top = 0;
        self.draw(ctx);
        self.cursor = (self.text_speed_x, TEXT_SPEED_Y);
    }

    /// `SetOptionsFromCursorPositions`, run every pass of the loop rather than on the way out.
    fn options_from_cursors(&self, ctx: &mut Ctx) {
        let text_speed = TEXT_SPEED_OPTIONS.iter()
            .find(|&&(x, _)| x == self.text_speed_x)
            .map_or(TextSpeed::Medium, |&(_, speed)| speed);
        ctx.world.options = Options {
            text_speed,
            battle_animation: self.battle_anim_x == LEFT,
            battle_style: if self.battle_style_x == LEFT { BattleStyle::Shift } else { BattleStyle::Set },
        };
        ctx.world.ruleset = if self.ruleset_x == LEFT { Ruleset::Gen1 } else { Ruleset::Modern };
    }

    /// The rows from `top` down: the boxes, their words and a `▷` on each setting. At the top it
    /// draws over every tile, as the cartridge draws over the main menu without clearing it.
    fn draw(&self, ctx: &mut Ctx) {
        let ui = &mut ctx.screen.ui;
        let mut boxed = UiSurface::default();
        boxed.text_box_border(0, 0, 18, 3);
        for y in 0..SCREEN_TILES_Y {
            let row = y + self.top as usize;
            match BOX_TOPS.iter().map(|&top| top as usize).find(|top| (*top..top + 5).contains(&row)) {
                Some(top) => ui.place(0, y, boxed.row(row - top)),
                None => ui.fill(0, y, SCREEN_TILES_X, 1, UiSurface::BLANK),
            }
        }
        let text = |s: &str| poke_core::charmap::encode(s).expect("the option menu's words encode");
        for (x, row, words) in WORDS {
            if let Some(y) = self.on_screen(row) {
                ui.place(x as usize, y, &text(words));
            }
        }
        for row in ROWS {
            if let Some(y) = self.on_screen(row) {
                ui.set(self.column_of(row) as usize, y, UNFILLED_CURSOR);
            }
        }
    }

    fn on_screen(&self, row: u8) -> Option<usize> {
        row.checked_sub(self.top).map(usize::from).filter(|&y| y < SCREEN_TILES_Y)
    }

    fn at(&self) -> usize {
        (self.cursor.1 - self.top) as usize * SCREEN_TILES_X + self.cursor.0 as usize
    }

    /// `PlaceMenuCursor` with a menu item of 0, so only the tile under the cursor is touched.
    fn place_cursor(&self, ctx: &mut Ctx) {
        let at = self.at();
        ctx.menu.place_at(&mut ctx.screen.ui, at);
    }

    /// Down and up between the rows, carrying each row's remembered column, and scrolling only
    /// when the row is off the screen.
    fn move_to(&mut self, y: u8, ctx: &mut Ctx) {
        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        self.cursor = (self.column_of(y), y);
        let top = match self.on_screen(y) {
            Some(_) => self.top,
            None if y < self.top => 0,
            None => SCROLLED,
        };
        if top != self.top {
            self.top = top;
            self.draw(ctx);
        }
    }

    fn column_of(&self, y: u8) -> u8 {
        match y {
            TEXT_SPEED_Y => self.text_speed_x,
            BATTLE_ANIM_Y => self.battle_anim_x,
            BATTLE_STYLE_Y => self.battle_style_x,
            RULESET_Y => self.ruleset_x,
            _ => LEFT,
        }
    }

    /// A toggle blanks the cursor where it stood and redraws it in the other column.
    fn toggle(&mut self, ctx: &mut Ctx) {
        let column = match self.cursor.1 {
            BATTLE_ANIM_Y => &mut self.battle_anim_x,
            RULESET_Y => &mut self.ruleset_x,
            _ => &mut self.battle_style_x,
        };
        *column = if *column == LEFT { RIGHT } else { LEFT };
        self.cursor.0 = *column;
        ctx.menu.erase_cursor(&mut ctx.screen.ui);
    }

    /// Left and right along the three speeds, which stop at either end rather than wrapping.
    fn move_text_speed(&mut self, left: bool, ctx: &mut Ctx) {
        self.text_speed_x = match (self.text_speed_x, left) {
            (1, true) | (14, false) => self.text_speed_x,
            (7, true) => 1,
            (_, true) => 7,
            (7, false) => 14,
            (_, false) => 7,
        };
        self.cursor.0 = self.text_speed_x;
        ctx.menu.erase_cursor(&mut ctx.screen.ui);
    }
}

impl Default for OptionMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for OptionMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.cursors_from_options(ctx);
    }

    /// The `Delay3` after drawing is loading and not modelled, so `.loop` starts in this frame.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.place_cursor(ctx);
        self.options_from_cursors(ctx);

        let keys = ctx.pad.low_sensitivity(ctx.frame_counter);
        // SELECT alone does not wake the loop.
        if keys.difference(Joypad::SELECT).is_empty() {
            return Transition::Stay;
        }
        // `.exitMenu` plays the press sound.
        if keys.intersects(Joypad::B | Joypad::START) {
            ctx.audio.play_sound(crate::audio::data::sounds::SFX_PRESS_AB);
            return Transition::Pop(Outcome::Done);
        }
        if keys.contains(Joypad::A) {
            // A does nothing anywhere but Cancel.
            if self.cursor.1 != CANCEL_Y {
                return Transition::Stay;
            }
            ctx.audio.play_sound(crate::audio::data::sounds::SFX_PRESS_AB);
            return Transition::Pop(Outcome::Done);
        }
        if keys.intersects(Joypad::DOWN | Joypad::UP) {
            let at = ROWS.iter().position(|&row| row == self.cursor.1).unwrap_or(0);
            let next = if keys.contains(Joypad::DOWN) { at + 1 } else { at + ROWS.len() - 1 };
            self.move_to(ROWS[next % ROWS.len()], ctx);
        } else if matches!(self.cursor.1, BATTLE_ANIM_Y | BATTLE_STYLE_Y | RULESET_Y) {
            self.toggle(ctx);
        } else if self.cursor.1 == TEXT_SPEED_Y {
            self.move_text_speed(keys.contains(Joypad::LEFT), ctx);
        }
        // Back to `.loop`, which has no `DelayFrame`: the cursor lands in this frame, not the next.
        self.place_cursor(ctx);
        self.options_from_cursors(ctx);
        Transition::Stay
    }

    fn status(&self) -> Status {
        Status::Waiting(Decision::Options)
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    fn game(options: Options) -> Game {
        let world = World { options, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::OptionMenu(OptionMenu::new()));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::Options) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the option screen never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    /// A word where it is drawn. The cursor covers the tile before each one, so a whole row read
    /// back includes it.
    fn word_at(game: &Game, x: usize, y: usize, word: &str) {
        let expected = encode(word).unwrap();
        assert_eq!(game.ui().row(y)[x..x + expected.len()], expected[..], "{word} at ({x}, {y})");
    }

    fn cursor_at(game: &Game) -> Option<(usize, usize)> {
        (0..18).flat_map(|y| (0..20).map(move |x| (x, y)))
            .find(|&(x, y)| game.ui().get(x, y) == CURSOR)
    }

    /// Each label is two rows above its words, because `<NEXT>` moves two lines rather than one,
    /// and that is what puts the words on the rows the cursor coordinates name.
    #[test]
    fn four_boxes_of_words_and_a_cancel_below_them() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        word_at(&game, 1, 1, "TEXT SPEED");
        word_at(&game, 2, 3, "FAST");
        word_at(&game, 8, 3, "MEDIUM");
        word_at(&game, 15, 3, "SLOW");
        word_at(&game, 1, 6, "BATTLE ANIMATION");
        word_at(&game, 2, 8, "ON");
        word_at(&game, 11, 8, "OFF");
        word_at(&game, 1, 11, "BATTLE STYLE");
        word_at(&game, 2, 13, "SHIFT");
        word_at(&game, 11, 13, "SET");
        word_at(&game, 1, 16, "RULESET");
        for _ in 0..3 {
            press(&mut game, Joypad::DOWN);
        }
        word_at(&game, 1, 12, "RULESET");
        word_at(&game, 2, 14, "GEN 1");
        word_at(&game, 11, 14, "MODERN");
        word_at(&game, 2, 17, "CANCEL");
    }

    /// The screen has no state of its own: what it shows is the options, and what it writes back
    /// every frame is whatever its cursors stand on.
    #[test]
    fn every_setting_opens_on_itself_and_survives_untouched() {
        for text_speed in [TextSpeed::Fast, TextSpeed::Medium, TextSpeed::Slow] {
            for battle_animation in [true, false] {
                for battle_style in [BattleStyle::Shift, BattleStyle::Set] {
                    let options = Options { text_speed, battle_animation, battle_style };
                    let mut game = game(options);
                    until_waiting(&mut game);
                    assert_eq!(game.world().options, options, "{options:?}");
                    let speed_x = match text_speed {
                        TextSpeed::Fast => 1,
                        TextSpeed::Medium => 7,
                        TextSpeed::Slow => 14,
                    };
                    assert_eq!(cursor_at(&game), Some((speed_x, 3)), "{options:?}");
                    let anim_x = if battle_animation { 1 } else { 10 };
                    let style_x = if battle_style == BattleStyle::Shift { 1 } else { 10 };
                    assert_eq!(game.ui().get(anim_x, 8), UNFILLED_CURSOR, "{options:?}");
                    assert_eq!(game.ui().get(style_x, 13), UNFILLED_CURSOR, "{options:?}");
                }
            }
        }
    }

    #[test]
    fn a_direction_toggles_the_row_the_cursor_is_on() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        assert_eq!(cursor_at(&game), Some((1, 8)), "the battle animation row");
        press(&mut game, Joypad::RIGHT);
        assert_eq!(cursor_at(&game), Some((10, 8)));
        assert!(!game.world().options.battle_animation);
        press(&mut game, Joypad::DOWN);
        press(&mut game, Joypad::LEFT);
        assert_eq!(cursor_at(&game), Some((10, 13)));
        assert_eq!(game.world().options.battle_style, BattleStyle::Set);
    }

    #[test]
    fn text_speed_steps_between_the_three_and_stops_at_either_end() {
        let mut game = game(Options { text_speed: TextSpeed::Fast, ..Options::default() });
        until_waiting(&mut game);
        for expected in [TextSpeed::Medium, TextSpeed::Slow, TextSpeed::Slow] {
            press(&mut game, Joypad::RIGHT);
            assert_eq!(game.world().options.text_speed, expected);
        }
        for expected in [TextSpeed::Medium, TextSpeed::Fast, TextSpeed::Fast] {
            press(&mut game, Joypad::LEFT);
            assert_eq!(game.world().options.text_speed, expected);
        }
    }

    /// Moving away leaves a `▷` behind, which is how all four rows show their setting at once.
    #[test]
    fn a_row_the_cursor_leaves_keeps_a_marker() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        assert_eq!(game.ui().get(7, 3), UNFILLED_CURSOR, "medium's own column, where the cursor was");
        assert_eq!(cursor_at(&game), Some((1, 8)));
    }

    #[test]
    fn a_leaves_only_from_cancel() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        assert!(!game.modes().is_empty(), "A does nothing on the text speed row");
        for _ in 0..4 {
            press(&mut game, Joypad::DOWN);
        }
        assert_eq!(cursor_at(&game), Some((1, 17)), "cancel");
        press(&mut game, Joypad::A);
        assert!(game.modes().is_empty());
    }

    /// The screen scrolls four rows to show the ruleset and Cancel, and back as the cursor leaves
    /// them, up past the top included.
    #[test]
    fn the_ruleset_row_scrolls_into_view_and_toggles() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        assert_eq!(game.world().ruleset, Ruleset::Modern);
        for _ in 0..3 {
            press(&mut game, Joypad::DOWN);
        }
        assert_eq!(cursor_at(&game), Some((10, 14)), "on MODERN, scrolled");
        assert_eq!(game.ui().get(1, 9), UNFILLED_CURSOR, "the battle style row, moved up with the rest");
        press(&mut game, Joypad::LEFT);
        assert_eq!(cursor_at(&game), Some((1, 14)));
        assert_eq!(game.world().ruleset, Ruleset::Gen1);
        press(&mut game, Joypad::UP);
        assert_eq!(cursor_at(&game), Some((1, 9)), "battle style, still scrolled");
        press(&mut game, Joypad::UP);
        press(&mut game, Joypad::UP);
        assert_eq!(cursor_at(&game), Some((7, 3)), "text speed, at the top again");
        word_at(&game, 1, 1, "TEXT SPEED");
        press(&mut game, Joypad::UP);
        assert_eq!(cursor_at(&game), Some((1, 17)), "round to cancel");
        assert_eq!(game.ui().get(1, 14), UNFILLED_CURSOR, "GEN 1 kept its marker");
        assert_eq!(game.world().ruleset, Ruleset::Gen1);
    }

    #[test]
    fn a_gen_1_world_opens_on_gen_1() {
        let world = World { ruleset: Ruleset::Gen1, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::OptionMenu(OptionMenu::new()));
        until_waiting(&mut game);
        assert_eq!(game.world().ruleset, Ruleset::Gen1);
        for _ in 0..3 {
            press(&mut game, Joypad::DOWN);
        }
        assert_eq!(cursor_at(&game), Some((1, 14)));
    }

    #[test]
    fn b_leaves_from_anywhere() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        assert!(game.modes().is_empty());
    }

    #[test]
    fn select_alone_does_not_wake_the_loop() {
        let mut game = game(Options::default());
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        assert_eq!(cursor_at(&game), Some((7, 3)), "still on medium, where it opened");
        assert!(!game.modes().is_empty());
    }

    #[test]
    fn a_save_mid_screen_resumes_identically() {
        let mut whole = game(Options::default());
        until_waiting(&mut whole);
        for _ in 0..3 {
            press(&mut whole, Joypad::DOWN);
        }
        press(&mut whole, Joypad::LEFT);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..40 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
        assert_eq!(whole.world().options, restored.world().options);
        assert_eq!(restored.world().ruleset, Ruleset::Gen1);
    }
}
