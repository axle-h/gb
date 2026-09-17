//! `HandleMenuInput` over rows the caller has already drawn, which is how the events put up their
//! own menus: the vending machine's drinks, the prize vendor's prizes, the fossils the lab takes,
//! Bill's list and the link cable help. A pops `Chosen(row)` and B `Cancelled`; which row means
//! cancel is the caller's to know.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorMenu {
    input: MenuInput,
}

impl CursorMenu {
    /// `wCurrentMenuItem`, `wMaxMenuItem` and `wTopMenuItemX`/`Y` as the caller writes them, with
    /// `wMenuWatchedKeys` at `PAD_A | PAD_B`. `wLastMenuItem` is the caller's too: not every caller
    /// resets it.
    pub fn new(current: u8, max: u8, top: (u8, u8)) -> Self {
        Self { input: MenuInput::new(current, max, top, Joypad::A | Joypad::B) }
    }

    /// More keys that return, as the Viridian blackboard watches left and right. A key other than A
    /// or B pops `Chosen` with [`CursorMenu::PRESSED_LEFT`] or [`CursorMenu::PRESSED_RIGHT`] over
    /// the row.
    pub fn watching(mut self, keys: Joypad) -> Self {
        self.input.watched |= keys;
        self
    }

    /// `BIT_DOUBLE_SPACED_MENU`, which steps the cursor a row at a time, as the PC's box list does.
    pub fn single_spaced(mut self) -> Self {
        self.input.single_spaced = true;
        self
    }

    pub const ROW: u8 = 0x0F;
    pub const PRESSED_LEFT: u8 = 0x80;
    pub const PRESSED_RIGHT: u8 = 0x40;

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    pub fn rows(&self) -> u8 {
        self.input.max + 1
    }
}

impl ModeUpdate for CursorMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.input.call(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.input.update(ctx) {
            None => Transition::Stay,
            Some(keys) if keys.contains(Joypad::B) => Transition::Pop(Outcome::Cancelled),
            Some(keys) if keys.contains(Joypad::A) => Transition::Pop(Outcome::Chosen(self.input.current)),
            Some(keys) => {
                let side = if keys.contains(Joypad::LEFT) { Self::PRESSED_LEFT } else { Self::PRESSED_RIGHT };
                Transition::Pop(Outcome::Chosen(self.input.current | side))
            }
        }
    }

    fn status(&self) -> Status {
        if self.input.is_polling() { Status::Waiting(Decision::CursorMenu) } else { Status::Busy }
    }
}
