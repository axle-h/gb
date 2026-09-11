//! `HandleMenuInput` and `PlaceMenuCursor`, which every cursor menu runs inside its own mode.

use serde::{Deserialize, Serialize};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::modes::blink::ArrowBlink;

const CURSOR: u8 = 0xED;
const DOWN_ARROW: u8 = 0xEE;
/// The `▼` some menus put at `(18, 11)`, which `HandleMenuInput` blinks while it waits.
const ARROW: (usize, usize) = (18, 11);
/// `HandleMenuInput`'s iterations a frame, in hundredths.
const BLINK_PER_FRAME: u32 = 6661;

/// `wLastMenuItem` and `wTileBehindCursor`, which outlive the menu that set them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorMemory {
    pub last_item: u8,
    pub tile_behind: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuInput {
    /// `wCurrentMenuItem`.
    pub current: u8,
    /// `wMaxMenuItem`.
    pub max: u8,
    /// `wTopMenuItemX` and `Y`.
    pub top: (u8, u8),
    /// `wMenuWatchedKeys`.
    pub watched: Joypad,
    /// `wMenuWatchMovingOutOfBounds`: return rather than stop at either end.
    pub return_at_ends: bool,
    /// `wMenuWrappingEnabled`.
    pub wrapping: bool,
    /// `wMenuCursorLocation`.
    pub cursor_at: usize,
    /// `Delay3` after placing the cursor, counting down; polling once it is zero.
    delay: u8,
    blink: Option<ArrowBlink>,
}

impl MenuInput {
    pub fn new(current: u8, max: u8, top: (u8, u8), watched: Joypad) -> Self {
        Self { current, max, top, watched, return_at_ends: false, wrapping: false, cursor_at: 0, delay: 0, blink: None }
    }

    pub fn is_polling(&self) -> bool {
        self.delay == 0
    }

    /// The call. The blink starts afresh on each one, and only if the `▼` is already up.
    pub fn call(&mut self, ctx: &mut Ctx) {
        self.blink = (ctx.screen.ui.get(ARROW.0, ARROW.1) == DOWN_ARROW).then(ArrowBlink::default);
        self.restart(ctx);
    }

    /// `.loop1`: place the cursor, then `Delay3`.
    fn restart(&mut self, ctx: &mut Ctx) {
        self.place_cursor(&mut ctx.screen.ui, ctx.menu);
        self.delay = 3;
    }

    /// One frame of `HandleMenuInput`; `Some(hJoy5)` when it returns.
    pub fn update(&mut self, ctx: &mut Ctx) -> Option<Joypad> {
        if self.delay > 0 {
            self.delay -= 1;
            if self.delay > 0 {
                return None;
            }
        }
        let keys = ctx.pad.low_sensitivity(ctx.frame_counter);
        if keys.is_empty() {
            if let Some(shown) = self.blink.as_mut().and_then(|blink| blink.tick(BLINK_PER_FRAME)) {
                ctx.screen.ui.set(ARROW.0, ARROW.1, if shown { DOWN_ARROW } else { UiSurface::BLANK });
            }
            return None;
        }
        let mut stopped_at_an_end = false;
        if keys.contains(Joypad::UP) {
            if self.current > 0 {
                self.current -= 1;
            } else if self.wrapping {
                self.current = self.max;
            } else {
                stopped_at_an_end = true;
            }
        } else if keys.contains(Joypad::DOWN) {
            if self.current < self.max {
                self.current += 1;
            } else if self.wrapping {
                self.current = 0;
            } else {
                stopped_at_an_end = true;
            }
        }
        if keys.intersects(self.watched) || (stopped_at_an_end && self.return_at_ends) {
            self.wrapping = false;
            return Some(keys);
        }
        self.restart(ctx);
        None
    }

    /// `PlaceMenuCursor`, for a double-spaced menu.
    pub fn place_cursor(&mut self, ui: &mut UiSurface, memory: &mut CursorMemory) {
        let top = self.top.1 as usize * SCREEN_TILES_X + self.top.0 as usize;
        let row = |item: u8| top + item as usize * 2 * SCREEN_TILES_X;
        let tile = |ui: &UiSurface, at: usize| ui.get(at % SCREEN_TILES_X, at / SCREEN_TILES_X);
        let old = row(memory.last_item);
        if tile(ui, old) == CURSOR {
            ui.set(old % SCREEN_TILES_X, old / SCREEN_TILES_X, memory.tile_behind);
        }
        let new = row(self.current);
        if tile(ui, new) != CURSOR {
            memory.tile_behind = tile(ui, new);
        }
        ui.set(new % SCREEN_TILES_X, new / SCREEN_TILES_X, CURSOR);
        self.cursor_at = new;
        memory.last_item = self.current;
    }
}
