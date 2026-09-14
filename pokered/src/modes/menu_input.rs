//! `HandleMenuInput` and `PlaceMenuCursor`, which every cursor menu runs inside its own mode.

use serde::{Deserialize, Serialize};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::modes::blink::ArrowBlink;

const CURSOR: u8 = 0xED;
pub const UNFILLED_CURSOR: u8 = 0xEC;
const DOWN_ARROW: u8 = 0xEE;
/// The `▼` some menus put at `(18, 11)`, which `HandleMenuInput` blinks while it waits.
const ARROW: (usize, usize) = (18, 11);
/// `HandleMenuInput`'s iterations a frame, in hundredths.
const BLINK_PER_FRAME: u32 = 6661;

/// `wMenuExitMethod`: one byte read under two vocabularies. 1 is `CHOSE_MENU_ITEM` to a list and
/// `CHOSE_FIRST_ITEM` to a two-option menu; 2 is `CANCELLED_MENU` and `CHOSE_SECOND_ITEM`. Picking
/// the second option and backing out are therefore the same answer, which is why B picks it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MenuExit {
    #[default]
    Chose,
    Cancelled,
}

impl MenuExit {
    pub const CHOSE_FIRST: Self = Self::Chose;
    pub const CHOSE_SECOND: Self = Self::Cancelled;
}

/// The cursor globals that outlive the menu that set them. `cursor_at` is `wMenuCursorLocation`,
/// which is what `EraseMenuCursor` and `PlaceUnfilledArrowMenuCursor` act on: both mark wherever
/// the cursor was last placed, which may be a menu that is no longer on top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorMemory {
    pub last_item: u8,
    pub tile_behind: u8,
    pub cursor_at: usize,
    /// `wBattleAndStartSavedMenuItem`: where the start menu reopens.
    pub battle_and_start: u8,
    /// `wPartyAndBillsPCSavedMenuItem`, the same for the party menu and the PC.
    pub party_and_bills: u8,
    /// `wChosenMenuItem` and `wMenuExitMethod`, which a caller reads after its menu has gone.
    pub chosen_item: u8,
    pub exit_method: MenuExit,
    /// `wBagSavedMenuItem`: where the bag reopens.
    #[serde(default)]
    pub bag_saved: u8,
    /// `wListScrollOffset`, which outlives a list: the bag reopens scrolled where it was left, and
    /// the mart zeroes it and puts it back.
    #[serde(default)]
    pub list_scroll: u8,
}

impl CursorMemory {
    /// `PlaceMenuCursor`'s tail: save what the cursor covers, unless a cursor already covers it.
    pub fn place_at(&mut self, ui: &mut UiSurface, at: usize) {
        if tile_at(ui, at) != CURSOR {
            self.tile_behind = tile_at(ui, at);
        }
        put(ui, at, CURSOR);
        self.cursor_at = at;
    }

    /// `EraseMenuCursor`.
    pub fn erase_cursor(&self, ui: &mut UiSurface) {
        put(ui, self.cursor_at, UiSurface::BLANK);
    }

    /// `PlaceUnfilledArrowMenuCursor`: the `▷` that marks a parent menu's place, or a setting the
    /// cursor has left behind.
    pub fn unfilled_cursor(&self, ui: &mut UiSurface) {
        put(ui, self.cursor_at, UNFILLED_CURSOR);
    }
}

pub fn tile_at(ui: &UiSurface, at: usize) -> u8 {
    ui.get(at % SCREEN_TILES_X, at / SCREEN_TILES_X)
}

pub fn put(ui: &mut UiSurface, at: usize, tile: u8) {
    ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
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
    /// `BIT_DOUBLE_SPACED_MENU` set, which despite its name steps the cursor one row per item.
    #[serde(default)]
    pub single_spaced: bool,
    /// Whether the pad has been read since the call. A menu reports `Waiting` only once it has, so
    /// a driver's release is always seen before its next press.
    #[serde(default)]
    polled: bool,
    blink: Option<ArrowBlink>,
}

impl MenuInput {
    pub fn new(current: u8, max: u8, top: (u8, u8), watched: Joypad) -> Self {
        Self { current, max, top, watched, return_at_ends: false, wrapping: false, single_spaced: false, polled: false, blink: None }
    }

    pub fn is_polling(&self) -> bool {
        self.polled
    }

    /// The call, up to `.loop1`'s cursor. Its `Delay3` is loading and not modelled, so a caller
    /// whose loop has no `DelayFrame` of its own runs `update` in this same frame.
    pub fn call(&mut self, ctx: &mut Ctx) {
        self.blink = (ctx.screen.ui.get(ARROW.0, ARROW.1) == DOWN_ARROW).then(ArrowBlink::default);
        self.polled = false;
        self.place_cursor(&mut ctx.screen.ui, ctx.menu);
    }

    /// One frame of `HandleMenuInput`'s `.loop2`; `Some(hJoy5)` when it returns.
    pub fn update(&mut self, ctx: &mut Ctx) -> Option<Joypad> {
        self.polled = true;
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
        // Back to `.loop1`, whose `Delay3` is not modelled: the cursor moves in the frame the key lands.
        self.place_cursor(&mut ctx.screen.ui, ctx.menu);
        None
    }

    /// `PlaceMenuCursor`, for a double-spaced menu.
    pub fn place_cursor(&mut self, ui: &mut UiSurface, memory: &mut CursorMemory) {
        let top = self.top.1 as usize * SCREEN_TILES_X + self.top.0 as usize;
        let spacing = if self.single_spaced { 1 } else { 2 };
        let row = |item: u8| top + item as usize * spacing * SCREEN_TILES_X;
        let old = row(memory.last_item);
        if tile_at(ui, old) == CURSOR {
            put(ui, old, memory.tile_behind);
        }
        memory.place_at(ui, row(self.current));
        memory.last_item = self.current;
    }
}
