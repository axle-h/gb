//! `SlotSelector`: the save slots, behind the start menu's SAVE row. The cartridge has no such
//! screen, so it is built from the game's own boxes, font, cursor, texts and sounds.
//!
//! The upper box previews the slot under the cursor: its picture, the player, badges, Pokédex, play
//! time, where it was saved and the host's time of saving. The lower box lists the slots, the
//! player's numbered and the Hall of Fame's autosave marked `A` below them. A on a slot offers SAVE,
//! LOAD and DELETE as far as the slot allows, never SAVE on the autosave; B backs out.
//!
//! Writing over a used slot asks first, in the words the cartridge uses for its older file, and so
//! does deleting. A save then keeps the cartridge's pacing: "Now saving..." for 120 frames, "saved
//! the game!", `SFX_SAVE` and 30 frames more.
//!
//! The save is raised in the frame the selector and the start menu close, as the cartridge closes
//! the menu after saving, so a slot holds the game as it resumes, on the overworld. A load or a
//! delete is the host's to answer between frames; the list is drawn again from what it hands back.

use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::command::Decision;
use crate::gfx::mon_icons::clear_sprites;
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::menu_input::{MenuInput, UNFILLED_CURSOR};
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::save_slots::{newest, SavedAt, Slot, SlotAction, SlotSummary, Thumbnail, AUTOSAVE, SLOTS};
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::status_screen::{encode, place_lines};

/// The preview's box, `(x, y, width, height)` as `TextBoxBorder` takes them, and what is in it.
const PREVIEW: (usize, usize, usize, usize) = (0, 0, 18, 7);
const PICTURE_AT: (usize, usize) = (1, 1);
const STATS_X: usize = 8;
const LOCATION_AT: (usize, usize) = (1, 6);
const SAVED_TIME_AT: (usize, usize) = (1, 7);
/// The list's box, a slot a row, and its cursor's first row.
const LIST: (usize, usize, usize, usize) = (0, 9, 18, SLOTS);
const LIST_TOP: (u8, u8) = (1, 10);
/// The actions' box over the list's right-hand side, its bottom edge on the list's.
const ACTIONS: (usize, usize, usize, usize) = (11, 10, 7, 6);
const ACTIONS_TOP: (u8, u8) = (12, 11);
/// The dialogue box's yes/no corner, `YES_NO_MENU`'s usual `hlcoord 14, 7`.
const YES_NO_AT: (usize, usize) = (14, 7);
/// The dialogue box, and where "Now saving..." goes in it, as the save menu places it.
const TEXT_BOX: (usize, usize, usize, usize) = (0, 12, 18, 4);
const NOW_SAVING_AT: (usize, usize) = (1, 14);
/// The cartridge's `SaveMenu`'s `DelayFrames 120` for "Now saving...", and its 30 after `SFX_SAVE`.
const NOW_SAVING: u16 = 120;
const AFTER_SAVED: u16 = 30;
/// `TextBoxGraphics`' colon, which the save screens print the play time with.
const TIME_COLON: u8 = 0x6D;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SlotChoice {
    Save,
    Load,
    Delete,
}

impl SlotChoice {
    fn label(self) -> &'static str {
        match self {
            SlotChoice::Save => "SAVE",
            SlotChoice::Load => "LOAD",
            SlotChoice::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    List,
    /// A menu or a text is up, and this is what its answer feeds.
    Child(After),
    /// A `DelayFrames` counting down, and what runs when it ends.
    Hold(u16, After),
    /// `WaitForSoundToFinish` after `SFX_SAVE`.
    Sound,
    /// The host answers between frames, so the list is drawn again in the next one.
    Redraw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    /// The actions' menu is answered.
    Chosen,
    /// The overwrite question is printed; put its yes/no up.
    OverwriteAsked,
    Overwrite,
    /// The delete question is printed; put its no/yes up.
    DeleteAsked,
    Delete,
    /// "Now saving..." has stood.
    Saved,
    /// "saved the game!" is printed; play the sound.
    Sound,
    /// Raise the save and close.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSelector {
    /// Only LOAD and DELETE are offered, for a game not yet under way.
    load_only: bool,
    /// What a save shows: the screen before the start menu went up.
    thumbnail: Option<Thumbnail>,
    input: MenuInput,
    /// The actions offered for the slot under the cursor.
    choices: Vec<SlotChoice>,
    /// `hTileAnimations`, held off while the picture is in the tileset's tiles.
    tile_animations: u8,
    phase: Phase,
}

impl SlotSelector {
    /// Over a game in play, which `thumbnail` shows as it was before any menu; without one, the
    /// screen as the selector opens.
    pub fn new(thumbnail: Option<Thumbnail>) -> Self {
        Self::with(false, thumbnail)
    }

    /// For a game not yet under way: nothing to save.
    pub fn load_only() -> Self {
        Self::with(true, None)
    }

    fn with(load_only: bool, thumbnail: Option<Thumbnail>) -> Self {
        Self { load_only, thumbnail, input: Self::list_input(0), choices: Vec::new(), tile_animations: 0, phase: Phase::List }
    }

    fn list_input(current: u8) -> MenuInput {
        let watched = Joypad::UP | Joypad::DOWN | Joypad::A | Joypad::B;
        let mut input = MenuInput::new(current, SLOTS as u8 - 1, LIST_TOP, watched);
        input.single_spaced = true;
        input
    }

    /// The slot under the cursor.
    pub fn current(&self) -> u8 {
        self.input.current
    }

    fn slot<'a>(slots: &'a [Option<Slot>], index: u8) -> Option<&'a Slot> {
        slots.get(index as usize).and_then(Option::as_ref)
    }

    /// The whole screen and the cursor on it.
    fn draw(&mut self, ctx: &mut Ctx) {
        self.draw_screen(ctx);
        ctx.menu.last_item = self.input.current;
        self.input.call(ctx);
    }

    /// Both boxes, the list and the preview.
    fn draw_screen(&self, ctx: &mut Ctx) {
        let slots = ctx.slots;
        let ui = &mut ctx.screen.ui;
        ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        for (x, y, width, height) in [PREVIEW, LIST] {
            ui.text_box_border(x, y, width, height);
        }
        for index in 0..SLOTS as u8 {
            let y = LIST_TOP.1 as usize + index as usize;
            let label = if index == AUTOSAVE { "A".to_string() } else { (index + 1).to_string() };
            ui.set(2, y, encode(&label)[0]);
            match Self::slot(slots, index) {
                Some(slot) => {
                    place_lines(ui, at(4, y), &slot.summary.player_name, false);
                    play_time(ui, at(13, y), &slot.summary);
                }
                None => ui.place(4, y, &encode("-------")),
            }
        }
        let slot = Self::slot(slots, self.input.current).cloned();
        preview(ctx, slot.as_ref().map(|slot| (&slot.summary, Some(slot.saved_at))));
    }

    /// The list with its cursor left unfilled, as the cartridge marks a menu's place under another.
    fn list_under(&self, ctx: &mut Ctx) {
        self.draw_screen(ctx);
        put(&mut ctx.screen.ui, at(LIST_TOP.0 as usize, LIST_TOP.1 as usize + self.input.current as usize), UNFILLED_CURSOR);
    }

    /// What A may do with the slot under the cursor.
    fn choices_for(&self, ctx: &Ctx) -> Vec<SlotChoice> {
        let used = Self::slot(ctx.slots, self.input.current).is_some();
        let save = (!self.load_only && self.input.current != AUTOSAVE).then_some(SlotChoice::Save);
        let rest = used.then_some([SlotChoice::Load, SlotChoice::Delete]).into_iter().flatten();
        save.into_iter().chain(rest).collect()
    }

    fn list(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        if keys.contains(Joypad::B) {
            return Self::close(self.tile_animations, ctx, Outcome::Cancelled);
        }
        if keys.contains(Joypad::A) {
            self.choices = self.choices_for(ctx);
            if self.choices.is_empty() {
                self.input.call(ctx);
                return Transition::Stay;
            }
            ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
            let (x, y, width, height) = ACTIONS;
            ctx.screen.ui.text_box_border(x, y, width, height);
            for (row, choice) in self.choices.iter().enumerate() {
                ctx.screen.ui.place(ACTIONS_TOP.0 as usize + 1, ACTIONS_TOP.1 as usize + row * 2, &encode(choice.label()));
            }
            ctx.menu.last_item = 0;
            self.phase = Phase::Child(After::Chosen);
            return Transition::Push(Mode::CursorMenu(CursorMenu::new(0, self.choices.len() as u8 - 1, ACTIONS_TOP)));
        }
        // UP or DOWN, the cursor already moved or stopped at an end.
        let slot = Self::slot(ctx.slots, self.input.current).cloned();
        preview(ctx, slot.as_ref().map(|slot| (&slot.summary, Some(slot.saved_at))));
        self.input.call(ctx);
        self.list(ctx)
    }

    fn print(&mut self, text: TextBox, after: After) -> Transition {
        self.phase = Phase::Child(after);
        Transition::Push(Mode::TextBox(text))
    }

    fn ask(&mut self, id: TwoOptionMenuId, after: After) -> Transition {
        self.phase = Phase::Child(after);
        Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(id, YES_NO_AT, false)))
    }

    /// Back to the list, the slots as the host now holds them.
    fn redraw(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::List;
        self.draw(ctx);
        Transition::Stay
    }

    /// The game being saved in the preview, and "Now saving..." in the dialogue box.
    fn now_saving(&mut self, ctx: &mut Ctx) -> Transition {
        preview(ctx, Some((&self.summary(ctx), None)));
        let (x, y, width, height) = TEXT_BOX;
        ctx.screen.ui.text_box_border(x, y, width, height);
        ctx.screen.ui.place(NOW_SAVING_AT.0, NOW_SAVING_AT.1, &encode("Now saving..."));
        self.phase = Phase::Hold(NOW_SAVING, After::Saved);
        Transition::Stay
    }

    fn chosen(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Outcome::Chosen(row) = outcome else { return self.redraw(ctx) };
        self.list_under(ctx);
        let slot = self.input.current;
        match self.choices[row as usize] {
            SlotChoice::Save if Self::slot(ctx.slots, slot).is_some() => {
                let text = poke_core::text_script::far_text("_OlderFileWillBeErasedText").expect("the text is in the cartridge");
                self.print(TextBox::script(text), After::OverwriteAsked)
            }
            SlotChoice::Save => self.now_saving(ctx),
            SlotChoice::Load => {
                ctx.slot = Some(SlotAction::Load(slot));
                self.phase = Phase::Redraw;
                Transition::Stay
            }
            SlotChoice::Delete => {
                let text = match slot {
                    AUTOSAVE => encode("Delete the<LINE>autosave?"),
                    _ => encode(&format!("Delete the file<LINE>in slot {}?", slot + 1)),
                };
                self.print(TextBox::new(text), After::DeleteAsked)
            }
        }
    }

    fn summary(&self, ctx: &Ctx) -> SlotSummary {
        SlotSummary::of(ctx.world, self.thumbnail.clone().expect("a game in play has its picture"))
    }

    fn step(&mut self, after: After, ctx: &mut Ctx) -> Transition {
        match after {
            After::Saved => {
                let text = poke_core::text_script::far_text("_GameSavedText").expect("the text is in the cartridge");
                self.print(TextBox::script(text), After::Sound)
            }
            After::Close => {
                ctx.slot = Some(SlotAction::Save(self.input.current, self.summary(ctx)));
                ctx.pad.poll();
                Self::close(self.tile_animations, ctx, Outcome::Done)
            }
            _ => Transition::Stay,
        }
    }

    /// The map's tiles and their animation back, as the trainer card leaves, and the overworld's
    /// sprites put back by whatever screen is under the selector.
    fn close(tile_animations: u8, ctx: &mut Ctx, outcome: Outcome) -> Transition {
        if let Some(tileset) = ctx.screen.map.tileset {
            ctx.screen.tiles.load_tileset(tileset);
        }
        ctx.screen.tiles.animation.kind = tile_animations;
        ctx.update_sprites = true;
        Transition::Pop(outcome)
    }
}


/// The upper box: a slot's picture, its stats and, once the host has stamped it, when it was saved.
fn preview(ctx: &mut Ctx, shown: Option<(&SlotSummary, Option<SavedAt>)>) {
    let (x, y, width, height) = PREVIEW;
    ctx.screen.ui.fill(x + 1, y + 1, width, height, UiSurface::BLANK);
    let Some((summary, saved_at)) = shown else { return };
    ctx.screen.tiles.load(V_CHARS2, &summary.thumbnail.tiles().concat());
    let (across, down) = Thumbnail::TILES;
    for row in 0..down {
        for column in 0..across {
            ctx.screen.ui.set(PICTURE_AT.0 + column, PICTURE_AT.1 + row, (row * across + column) as u8);
        }
    }

    let ui = &mut ctx.screen.ui;
    place_lines(ui, at(STATS_X, 1), &summary.player_name, false);
    place_lines(ui, at(STATS_X, 2), &encode("BADGES<NEXT>#DEX<NEXT>TIME"), true);
    let plain = |digits| NumberFormat { digits, leading_zeroes: false, left_align: false };
    print_number(ui, at(17, 2), summary.badges.count_ones(), plain(2));
    print_number(ui, at(16, 3), summary.owned as u32, plain(3));
    play_time(ui, at(13, 4), summary);
    if let Some(location) = summary.location() {
        place_lines(ui, at(LOCATION_AT.0, LOCATION_AT.1), &location, false);
    }

    let Some(saved_at) = saved_at else { return };
    let time = saved_at.local();
    let padded = |digits| NumberFormat { digits, leading_zeroes: true, left_align: false };
    let slash = encode("/")[0];
    let mut end = print_number(ui, at(SAVED_TIME_AT.0, SAVED_TIME_AT.1), time.year.clamp(0, 9999) as u32, padded(4));
    for part in [time.month, time.day] {
        put(ui, end, slash);
        end = print_number(ui, end + 1, part as u32, padded(2));
    }
    let end = print_number(ui, end + 2, time.hour as u32, padded(2));
    put(ui, end, TIME_COLON);
    print_number(ui, end + 1, time.minute as u32, padded(2));
}

fn at(x: usize, y: usize) -> usize {
    y * SCREEN_TILES_X + x
}

fn put(ui: &mut UiSurface, at: usize, tile: u8) {
    ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
}

/// Hours to three places, the colon, and minutes to two: six tiles.
fn play_time(ui: &mut UiSurface, at: usize, summary: &SlotSummary) {
    let hours = NumberFormat { digits: 3, leading_zeroes: false, left_align: false };
    let end = print_number(ui, at, summary.play_time.hours as u32, hours);
    put(ui, end, TIME_COLON);
    let minutes = NumberFormat { digits: 2, leading_zeroes: true, left_align: false };
    print_number(ui, end + 1, summary.play_time.minutes as u32, minutes);
}

impl ModeUpdate for SlotSelector {
    fn enter(&mut self, ctx: &mut Ctx) {
        if !self.load_only && self.thumbnail.is_none() {
            self.thumbnail = Some(Thumbnail::of(&ctx.screen.frame()));
        }
        self.tile_animations = ctx.screen.tiles.animation.kind;
        ctx.screen.tiles.animation.kind = 0;
        // The picture's tiles are map tiles to `UpdateSprites`, which would leave a sprite on it.
        clear_sprites(&mut ctx.screen.sprites);
        ctx.screen.tiles.load_text_box_tiles();
        self.input = Self::list_input(newest(ctx.slots).unwrap_or(0));
        self.draw(ctx);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::List => self.list(ctx),
            Phase::Redraw => {
                self.redraw(ctx);
                self.list(ctx)
            }
            Phase::Hold(frames, after) if frames > 1 => {
                self.phase = Phase::Hold(frames - 1, after);
                Transition::Stay
            }
            Phase::Hold(_, after) => self.step(after, ctx),
            Phase::Child(_) => Transition::Stay,
            Phase::Sound if ctx.audio.sound_finished() => {
                self.phase = Phase::Hold(AFTER_SAVED, After::Close);
                Transition::Stay
            }
            Phase::Sound => Transition::Stay,
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match after {
            After::Chosen => self.chosen(outcome, ctx),
            After::OverwriteAsked => self.ask(TwoOptionMenuId::YesNo, After::Overwrite),
            After::Overwrite if outcome == Outcome::Chosen(0) => self.now_saving(ctx),
            After::DeleteAsked => self.ask(TwoOptionMenuId::NoYes, After::Delete),
            After::Delete if outcome == Outcome::Chosen(1) => {
                ctx.slot = Some(SlotAction::Delete(self.input.current));
                self.phase = Phase::Redraw;
                Transition::Stay
            }
            After::Overwrite | After::Delete => self.redraw(ctx),
            After::Sound => {
                ctx.audio.play_sound(sounds::SFX_SAVE);
                self.phase = Phase::Sound;
                Transition::Stay
            }
            After::Saved | After::Close => Transition::Stay,
        }
    }

    fn status(&self) -> Status {
        if self.phase == Phase::List && self.input.is_polling() { Status::Waiting(Decision::SlotSelector) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests;
