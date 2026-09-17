//! `ShowPokedexMenu` and the three routines under it: `HandlePokedexListMenu`, the list of numbers
//! down the left with the seen and owned counts beside it; `HandlePokedexSideMenu`, the
//! DATA/CRY/AREA/QUIT menu on the lower right; and `ShowPokedexDataInternal`, the data page.
//!
//! One mode rather than three, because the cartridge's three routines are one call stack over one
//! set of menu globals: the side menu saves the list's cursor and scroll and puts them back, and
//! the data page is a call inside that. What the mode holds is exactly what the cartridge pushes.
//!
//! Exact: the seen and owned counts (`CountSetBits`), the highest seen number and the window it
//! bounds, all four ways the list scrolls, and the dex entry's height, weight and description as
//! the entry stores them. Every `ClearScreen`, `Delay3` and `GBPalWhiteOutWithDelay3` here is
//! loading and not modelled, so each screen takes input in the frame it is drawn.
//!
//! The list is scrolled with a *dex number*; the data page needs the cartridge's *index*, which is
//! what `PokedexToIndex` is for and what every table on that page is keyed by.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::place_string::{coord, PlaceString};
use crate::systems::pokedex::{count_set_bits, is_set, max_seen_mon, pokedex_to_index, species_of,
                              front_pic_tiles, DexEntry, NUM_POKEMON};
use crate::systems::print_num::{print_number, NumberFormat};

/// Rows of the list on screen at once, and the cursor's `wMaxMenuItem` for a full one.
const ROWS: u8 = 7;
/// `$72`, the ball beside a mon the player has owned.
const BALL: u8 = 0x72;
/// The vertical line down column 14, which `DrawPokedexVerticalLine` alternates with `$70`.
const VERTICAL_LINE: u8 = 0x71;
/// `.dashedLine`, printed in place of a name the player has not seen.
const DASHED_LINE: &str = "----------";
/// `PokedexDataDividerLine`, the row of tiles under the data page's height and weight.
const DIVIDER: [u8; SCREEN_TILES_X] = [0x68, 0x69, 0x6B, 0x69, 0x6B, 0x69, 0x6B, 0x69, 0x6B, 0x6B,
                                       0x6B, 0x6B, 0x69, 0x6B, 0x69, 0x6B, 0x69, 0x6B, 0x69, 0x6A];

/// The four rows of the side menu, in `PokedexMenuItemsText`'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideMenuEntry {
    Data,
    Cry,
    Area,
    Quit,
}

/// `HandlePokedexSideMenu`'s `b`: what the side menu did, which decides what the dex redraws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SideExit {
    /// 0: the data page or the area map was shown, so the tile patterns are loaded again.
    Shown,
    /// 1: `QUIT`.
    Quit,
    /// 2: the mon has not been seen, or B was pressed.
    Back,
}

/// The list's own menu globals, which `HandlePokedexSideMenu` pushes on entry and pops on the way
/// out: `wCurrentMenuItem`, `wLastMenuItem` and `wListScrollOffset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ListCursor {
    current: u8,
    last_item: u8,
    scroll: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokedexMenu {
    /// `wListScrollOffset`: the dex number above the first row, so row `n` is `scroll + n + 1`.
    scroll: u8,
    /// `wDexMaxSeenMon`, which is `$cd3d` again — the byte `wFieldMoves` and `wSwappedMenuItem`
    /// answer to. They are separate fields here because the cartridge can never have two live.
    max_seen: u8,
    input: MenuInput,
    phase: Phase,
    /// Set while the side menu or the data page is up.
    list: Option<ListCursor>,
    /// `wPokedexNum` as the side menu sets it: the dex number the page is about.
    dex: u8,
    printer: Option<PlaceString>,
    answered: u32,
    /// How many times the side menu has answered. A driver needs it because `CRY` answers without
    /// leaving the menu, so nothing else about the dex changes when that row is chosen.
    answers: u32,
    /// `ShowPokedexData` called from outside the dex, as a new catch's page is: no list behind it,
    /// and the page closes the mode.
    #[serde(default)]
    page_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    List,
    SideMenu,
    /// The description printing, which is instant: see `print_description`.
    DataPrinting,
    /// `.waitForButtonPress`.
    DataWaiting,
}

impl PokedexMenu {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            max_seen: 0,
            input: Self::list_input(0, ROWS - 1),
            phase: Phase::List,
            list: None,
            dex: 1,
            printer: None,
            answered: 0,
            answers: 0,
            page_only: false,
        }
    }

    /// `ShowPokedexData` for `dex`, from outside the dex.
    pub fn data_page(dex: u8) -> Self {
        Self { dex, page_only: true, phase: Phase::DataPrinting, ..Self::new() }
    }

    /// How many times the side menu has answered, which is how a driver sees its press land.
    pub fn answered(&self) -> u32 {
        self.answers
    }

    /// `.doPokemonListMenu`'s menu block. Up and Down are not watched, so `HandleMenuInput` moves
    /// the cursor itself and only comes back when it would leave the window, which is
    /// `wMenuWatchMovingOutOfBounds` and is what scrolls the list.
    fn list_input(current: u8, max: u8) -> MenuInput {
        let watched = Joypad::LEFT | Joypad::RIGHT | Joypad::B | Joypad::A;
        let mut input = MenuInput::new(current, max, (0, 3), watched);
        input.return_at_ends = true;
        input
    }

    /// The dex number under the cursor.
    pub fn selected_dex(&self) -> u8 {
        self.scroll + self.input.current + 1
    }

    /// `wDexMaxSeenMon`: the highest number the list will show.
    pub fn max_seen(&self) -> u8 {
        self.max_seen
    }

    /// The side menu's row under the cursor.
    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// What a side-menu row means.
    pub fn entry(row: u8) -> SideMenuEntry {
        match row {
            0 => SideMenuEntry::Data,
            1 => SideMenuEntry::Cry,
            2 => SideMenuEntry::Area,
            _ => SideMenuEntry::Quit,
        }
    }

    /// The next button that takes a driver towards `dex` being the entry under the cursor. Stepping
    /// off either end of the window scrolls it, so Up and Down alone reach anything.
    pub fn press_toward(&self, dex: u8) -> Joypad {
        match dex.cmp(&self.selected_dex()) {
            std::cmp::Ordering::Less => Joypad::UP,
            std::cmp::Ordering::Greater => Joypad::DOWN,
            std::cmp::Ordering::Equal => Joypad::A,
        }
    }

    /// `LoadPokedexTilePatterns`: the HP bar tiles, the dex's own frame and the ball.
    fn set_up_graphics(ctx: &mut Ctx) {
        ctx.screen.tiles.load_hp_bar_and_status_tiles();
        let start = pokered_symbols::PokedexTileGraphics;
        let len = (pokered_symbols::PokedexTileGraphicsEnd.address - start.address) as usize;
        ctx.screen.tiles.load(V_CHARS2 + 0x60, &rom_slice(start)[..len]);
        ctx.screen.tiles.load(V_CHARS2 + BALL as usize, &rom_slice(pokered_symbols::PokeballTileGraphics)[..TILE_BYTES]);
    }

    /// `HandlePokedexListMenu` from the top: the parts that do not change, then the window.
    fn open_list(&mut self, ctx: &mut Ctx) -> Transition {
        let text = |s: &str| poke_core::charmap::encode(s).expect("the dex's words encode");
        let ui = &mut ctx.screen.ui;
        ui.place(15, 8, &[text("─")[0]; 5]);
        ui.set(14, 0, VERTICAL_LINE);
        for top in [1, 9] {
            // The line alternates between its own tile and the box tile beside it.
            for row in 0..9 {
                ui.set(14, top + row, VERTICAL_LINE ^ (row as u8 & 1));
            }
        }
        let (seen, owned) = (ctx.world.pokedex.seen, ctx.world.pokedex.owned);
        let count = NumberFormat { digits: 3, ..NumberFormat::default() };
        print_number(ui, coord(16, 3) as usize, count_set_bits(&seen) as u32, count);
        print_number(ui, coord(16, 6) as usize, count_set_bits(&owned) as u32, count);
        ui.place(16, 2, &text("SEEN"));
        ui.place(16, 5, &text("OWN"));
        ui.place(1, 1, &text("CONTENTS"));
        for (row, word) in ["DATA", "CRY", "AREA", "QUIT"].into_iter().enumerate() {
            ui.place(16, 10 + 2 * row, &text(word));
        }
        self.max_seen = max_seen_mon(&seen);
        self.redraw_list(ctx)
    }

    /// `.loop`: the window of seven, and the cursor taking input again in the same frame.
    fn redraw_list(&mut self, ctx: &mut Ctx) -> Transition {
        // `ClearScreenArea` covers the names only: the numbers and the ball are rewritten in place.
        ctx.screen.ui.fill(4, 2, 10, 14, UiSurface::BLANK);
        let rows = if self.max_seen >= ROWS {
            ROWS
        } else {
            // One row per mon seen, and `wMaxMenuItem` one less. A dex with nothing seen shows
            // nothing; the cartridge walks off the front of its own flags there, which is layout.
            self.input.max = self.max_seen.wrapping_sub(1);
            self.max_seen
        };
        for row in 0..rows {
            self.print_entry(ctx, row);
        }
        self.input.call(ctx);
        self.phase = Phase::List;
        self.list_input_update(ctx)
    }

    fn list_input_update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        if keys.contains(Joypad::B) {
            return self.close(ctx);
        }
        if keys.contains(Joypad::A) {
            return self.open_side_menu(ctx);
        }
        self.scroll_by(keys);
        self.redraw_list(ctx)
    }

    fn side_menu_update(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        self.answers += 1;
        if keys.contains(Joypad::B) {
            // The column the side menu's cursor stood in is wiped before it leaves.
            ctx.screen.ui.fill(15, 10, 1, 7, UiSurface::BLANK);
            return self.exit_side_menu(SideExit::Back, ctx);
        }
        match Self::entry(self.input.current) {
            SideMenuEntry::Data => {
                ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
                self.draw_data(ctx);
                self.show_picture(ctx)
            }
            // The cry is the audio engine's, and it is the one row that does not leave.
            SideMenuEntry::Cry => {
                self.input.call(ctx);
                self.side_menu_update(ctx)
            }
            // `LoadTownMap_Nest` is the town map's chunk; the row still leaves as it does.
            SideMenuEntry::Area => self.exit_side_menu(SideExit::Shown, ctx),
            SideMenuEntry::Quit => self.exit_side_menu(SideExit::Quit, ctx),
        }
    }

    /// `.printPokemonLoop`: the number on the row above, then the ball and the name.
    fn print_entry(&self, ctx: &mut Ctx, row: u8) {
        let dex = self.scroll + row + 1;
        let (y, seen, owned) = (3 + 2 * row as usize, ctx.world.pokedex.seen, ctx.world.pokedex.owned);
        let format = NumberFormat { digits: 3, leading_zeroes: true, left_align: false };
        print_number(&mut ctx.screen.ui, coord(1, y - 1) as usize, dex as u32, format);
        let seen = dex <= NUM_POKEMON && is_set(&seen, dex);
        ctx.screen.ui.set(3, y, if dex <= NUM_POKEMON && is_set(&owned, dex) { BALL } else { UiSurface::BLANK });
        let name = match seen.then(|| species_of(dex)).flatten() {
            Some(species) => species.name(),
            None => poke_core::charmap::encode(DASHED_LINE).expect("the dashed line encodes"),
        };
        ctx.screen.ui.place(4, y, &name);
    }

    /// The four scrolls, in the order `HandlePokedexListMenu` tests them. Up and Down arrive only
    /// when the cursor tried to leave the window; Left and Right are watched keys.
    fn scroll_by(&mut self, keys: Joypad) {
        let last = self.max_seen.saturating_sub(ROWS);
        if keys.contains(Joypad::UP) {
            self.scroll = self.scroll.saturating_sub(1);
        } else if keys.contains(Joypad::DOWN) {
            if self.max_seen >= ROWS && self.scroll != last {
                self.scroll += 1;
            }
        } else if keys.contains(Joypad::RIGHT) {
            if self.max_seen >= ROWS {
                // A page down, held at the last window rather than wrapping.
                self.scroll = (self.scroll + ROWS).min(last);
            }
        } else if keys.contains(Joypad::LEFT) {
            self.scroll = self.scroll.saturating_sub(ROWS);
        }
    }

    /// `.goToSideMenu` and `HandlePokedexSideMenu` up to its own `HandleMenuInput`. A mon that has
    /// not been seen never opens the menu at all: it leaves again with `b = 2`.
    fn open_side_menu(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        self.list = Some(ListCursor { current: self.input.current, last_item: ctx.menu.last_item, scroll: self.scroll });
        self.dex = self.selected_dex();
        if self.dex > NUM_POKEMON || !is_set(&ctx.world.pokedex.seen, self.dex) {
            return self.exit_side_menu(SideExit::Back, ctx);
        }
        // The watched keys are the `3` still in `a` from `wMaxMenuItem`, which happens to be A | B;
        // the cartridge's own `ld a, PAD_A | PAD_B` is commented out beside it.
        self.input = MenuInput::new(0, 3, (15, 10), Joypad::A | Joypad::B);
        ctx.menu.last_item = 0;
        self.input.call(ctx);
        self.phase = Phase::SideMenu;
        self.side_menu_update(ctx)
    }

    /// `.exitSideMenu`: the list's cursor comes back, and the column the cursor stood in is wiped.
    fn exit_side_menu(&mut self, exit: SideExit, ctx: &mut Ctx) -> Transition {
        if let Some(list) = self.list.take() {
            self.scroll = list.scroll;
            self.input = Self::list_input(list.current, if self.max_seen >= ROWS { ROWS - 1 } else { self.input.max });
            ctx.menu.last_item = list.last_item;
        }
        ctx.screen.ui.fill(0, 3, 1, 13, UiSurface::BLANK);
        match exit {
            SideExit::Quit => self.close(ctx),
            SideExit::Shown => {
                Self::set_up_graphics(ctx);
                self.open_list(ctx)
            }
            SideExit::Back => self.open_list(ctx),
        }
    }

    /// `.exitPokedex`.
    fn close(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.pad.repeat_held = false;
        ctx.menu.last_item = 0;
        Transition::Pop(Outcome::Done)
    }

    /// `ShowPokedexDataInternal` up to its picture: the frame, the words and the number.
    fn draw_data(&mut self, ctx: &mut Ctx) {
        let text = |s: &str| poke_core::charmap::encode(s).expect("the data page's words encode");
        let index = pokedex_to_index(self.dex);
        let entry = DexEntry::of(index);
        let ui = &mut ctx.screen.ui;
        ui.fill(0, 0, SCREEN_TILES_X, 1, 0x64);
        ui.fill(0, 17, SCREEN_TILES_X, 1, 0x6F);
        ui.fill(0, 1, 1, 16, 0x66);
        ui.fill(19, 1, 1, 16, 0x67);
        for (x, y, corner) in [(0, 0, 0x63), (19, 0, 0x65), (0, 17, 0x6C), (19, 17, 0x6E)] {
            ui.set(x, y, corner);
        }
        ui.place(0, 9, &DIVIDER);
        // `HeightWeightText`, whose `?` are printed over.
        ui.place(9, 6, &text("HT  ?′??″"));
        ui.place(9, 8, &text("WT   ???lb"));
        ui.place(9, 2, &species_of(self.dex).expect("a seen mon has a species").name());
        ui.place(9, 4, &entry.species);
        ui.set(2, 8, text("№")[0]);
        ui.set(3, 8, text("<DOT>")[0]);
        let format = NumberFormat { digits: 3, leading_zeroes: true, left_align: false };
        print_number(ui, coord(4, 8) as usize, self.dex as u32, format);
    }

    /// The picture, the cry, and — for a mon the player has owned — the height, the weight and the
    /// description. An unowned page is the picture and the number alone.
    fn show_picture(&mut self, ctx: &mut Ctx) -> Transition {
        let species = species_of(self.dex).expect("a seen mon has a species");
        // `LoadFlippedFrontSpriteByMonIndex`: the tiles are mirrored within each byte, and the
        // columns are laid right to left, which is the other half of the flip.
        let tiles: Vec<u8> = front_pic_tiles(species, true).concat();
        ctx.screen.tiles.load(V_CHARS2, &tiles);
        for column in 0..7u8 {
            for row in 0..7u8 {
                ctx.screen.ui.set(7 - column as usize, 1 + row as usize, column * 7 + row);
            }
        }
        // `PlayCry` is the audio engine's.
        if !is_set(&ctx.world.pokedex.owned, self.dex) {
            self.phase = Phase::DataWaiting;
            return self.wait_for_button(ctx);
        }
        let entry = DexEntry::of(pokedex_to_index(self.dex));
        let ui = &mut ctx.screen.ui;
        let plain = NumberFormat { digits: 2, ..NumberFormat::default() };
        let zeroes = NumberFormat { digits: 2, leading_zeroes: true, left_align: false };
        let feet = print_number(ui, coord(12, 6) as usize, entry.feet as u32, plain);
        ui.set(feet % SCREEN_TILES_X, feet / SCREEN_TILES_X, poke_core::charmap::encode("′").unwrap()[0]);
        let inches = print_number(ui, coord(15, 6) as usize, entry.inches as u32, zeroes);
        ui.set(inches % SCREEN_TILES_X, inches / SCREEN_TILES_X, poke_core::charmap::encode("″").unwrap()[0]);
        Self::print_weight(ui, entry.weight);
        self.print_description(ctx, entry)
    }

    /// The weight, in tenths of a pound, with the decimal point pushed in afterwards: the last
    /// digit is moved one tile right and the point takes its place.
    fn print_weight(ui: &mut UiSurface, weight: u16) {
        let format = NumberFormat { digits: 5, ..NumberFormat::default() };
        print_number(ui, coord(11, 8) as usize, weight as u32, format);
        if weight < 10 {
            ui.set(14, 8, poke_core::charmap::encode("0").unwrap()[0]);
        }
        ui.set(16, 8, ui.get(15, 8));
        ui.set(15, 8, poke_core::charmap::encode("<DOT>").unwrap()[0]);
    }

    /// The description, printed at (1, 11). `TextCommandProcessor` is asked to clear
    /// `BIT_TEXT_DELAY` here, so the whole entry lands in one frame rather than a letter at a time;
    /// that flag is not modelled, and `no_text_delay` is the same thing to a player.
    fn print_description(&mut self, ctx: &mut Ctx, entry: DexEntry) -> Transition {
        ctx.world.no_text_delay = true;
        self.printer = Some(PlaceString::call(entry.description, coord(1, 11)));
        self.phase = Phase::DataPrinting;
        self.update(ctx)
    }

    /// `.waitForButtonPress`, and what `ShowPokedexDataInternal` does once it has one: the screen
    /// is cleared and the text box tiles go back over the dex's own.
    fn wait_for_button(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
            ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
            ctx.screen.tiles.load_text_box_tiles();
            if self.page_only {
                ctx.menu.last_item = 0;
                return Transition::Pop(Outcome::Done);
            }
            return self.exit_side_menu(SideExit::Shown, ctx);
        }
        Transition::Stay
    }
}

impl Default for PokedexMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for PokedexMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        if self.page_only {
            ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
            return;
        }
        // `GBPalWhiteOut`, `ClearScreen` and `UpdateSprites`; the palettes and the sprites are
        // other chunks'. `hJoy7` is what gives the list its held-key repeat.
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        ctx.pad.repeat_held = true;
        ctx.menu.last_item = 0;
        self.scroll = 0;
        self.input = Self::list_input(0, ROWS - 1);
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        Self::set_up_graphics(ctx);
        if self.page_only {
            self.draw_data(ctx);
            return self.show_picture(ctx);
        }
        self.open_list(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::List => self.list_input_update(ctx),
            Phase::SideMenu => self.side_menu_update(ctx),
            Phase::DataPrinting => {
                let mut printer = self.printer.take().expect("the description is printing");
                match printer.update(ctx, &mut self.answered) {
                    None => {
                        self.printer = Some(printer);
                        return Transition::Stay;
                    }
                    Some(_) => {
                        ctx.world.no_text_delay = false;
                        self.phase = Phase::DataWaiting;
                    }
                }
                // `.waitForButtonPress` polls the moment the description is done. Waiting a frame
                // instead would leave the pad's last read holding the button that turned the page,
                // so the next press would not be an edge and would go unseen.
                self.wait_for_button(ctx)
            }
            Phase::DataWaiting => self.wait_for_button(ctx),
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::List if self.input.is_polling() => Status::Waiting(Decision::Pokedex),
            Phase::SideMenu if self.input.is_polling() => Status::Waiting(Decision::PokedexSideMenu),
            Phase::DataWaiting => Status::Waiting(Decision::PokedexData),
            Phase::DataPrinting if self.printer.as_ref().is_some_and(PlaceString::is_prompting) =>
                Status::Waiting(Decision::PokedexData),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Refusal, Reply};
    use crate::mode::Mode;
    use crate::party::Pokedex;
    use crate::rng::GameRng;
    use crate::systems::pokedex::FLAG_BYTES;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    /// A dex with everything up to `seen` seen and everything up to `owned` owned.
    fn dex(seen: u8, owned: u8) -> Pokedex {
        let bits = |upto: u8| {
            let mut flags = [0u8; FLAG_BYTES];
            for dex in 1..=upto {
                flags[(dex as usize - 1) / 8] |= 1 << ((dex - 1) % 8);
            }
            flags
        };
        Pokedex { seen: bits(seen), owned: bits(owned) }
    }

    fn game(pokedex: Pokedex) -> Game {
        let world = World { pokedex, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Pokedex(PokedexMenu::new()));
        game
    }

    fn until_waiting(game: &mut Game, decision: Decision) {
        for _ in 0..400 {
            if game.status() == Status::Waiting(decision.clone()) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the dex never waited for {decision:?}");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn cursor_row(game: &Game) -> Option<usize> {
        (0..18).find(|&y| game.ui().get(0, y) == CURSOR)
    }

    fn row_text(game: &Game, y: usize, columns: std::ops::Range<usize>) -> Vec<u8> {
        game.ui().row(y)[columns].to_vec()
    }

    #[test]
    fn seven_numbered_rows_with_names_for_what_has_been_seen() {
        let mut game = game(dex(20, 3));
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 2, 1..4), encode("001").unwrap(), "the number sits above the name");
        assert_eq!(row_text(&game, 3, 4..13), PokemonSpecies::Bulbasaur.name());
        assert_eq!(game.ui().get(3, 3), BALL, "owned");
        assert_eq!(game.ui().get(3, 9), UiSurface::BLANK, "seen but not owned");
        assert_eq!(row_text(&game, 14, 1..4), encode("007").unwrap());
        assert_eq!(cursor_row(&game), Some(3));
    }

    /// A number the player has not seen is a dashed line rather than a name.
    #[test]
    fn an_unseen_number_shows_a_dashed_line() {
        let mut game = game(dex(151, 0));
        // Squirtle is dex 7 and Charmander 4; clear one in the middle of the window.
        game.frame(Input::None);
        let mut world = World { pokedex: dex(151, 0), ..World::default() };
        world.pokedex.seen[0] &= !(1 << 2);
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Pokedex(PokedexMenu::new()));
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 7, 4..14), encode(DASHED_LINE).unwrap(), "dex 3 is not seen");
        assert_eq!(row_text(&game, 5, 4..11), PokemonSpecies::Ivysaur.name(), "dex 2 still is");
    }

    /// The window is as long as the dex is: fewer than seven seen makes a shorter menu.
    #[test]
    fn a_short_dex_shows_only_what_it_has() {
        let mut game = game(dex(3, 3));
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 7, 4..12), PokemonSpecies::Venusaur.name(), "three rows");
        assert_eq!(row_text(&game, 9, 4..14), [UiSurface::BLANK; 10], "and no fourth");
        press(&mut game, Joypad::DOWN);
        press(&mut game, Joypad::DOWN);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(cursor_row(&game), Some(7), "and the cursor stops on the last");
    }

    #[test]
    fn down_at_the_bottom_scrolls_and_up_at_the_top_comes_back() {
        let mut game = game(dex(151, 151));
        until_waiting(&mut game, Decision::Pokedex);
        for _ in 0..7 {
            press(&mut game, Joypad::DOWN);
            until_waiting(&mut game, Decision::Pokedex);
        }
        assert_eq!(row_text(&game, 2, 1..4), encode("002").unwrap(), "one row further down the list");
        assert_eq!(cursor_row(&game), Some(15), "with the cursor still on the last row");
        for _ in 0..7 {
            press(&mut game, Joypad::UP);
            until_waiting(&mut game, Decision::Pokedex);
        }
        assert_eq!(row_text(&game, 2, 1..4), encode("001").unwrap());
    }

    #[test]
    fn right_and_left_move_a_page_at_a_time_and_stop_at_the_ends() {
        let mut game = game(dex(151, 151));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::RIGHT);
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 2, 1..4), encode("008").unwrap());
        for _ in 0..30 {
            press(&mut game, Joypad::RIGHT);
            until_waiting(&mut game, Decision::Pokedex);
        }
        assert_eq!(row_text(&game, 2, 1..4), encode("145").unwrap(), "the last window, not past it");
        press(&mut game, Joypad::LEFT);
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 2, 1..4), encode("138").unwrap());
        for _ in 0..30 {
            press(&mut game, Joypad::LEFT);
            until_waiting(&mut game, Decision::Pokedex);
        }
        assert_eq!(row_text(&game, 2, 1..4), encode("001").unwrap(), "and stops at the top");
    }

    #[test]
    fn a_seen_mon_opens_the_side_menu_and_b_closes_it() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        assert_eq!(row_text(&game, 10, 16..20), encode("DATA").unwrap());
        assert_eq!(game.ui().get(15, 10), CURSOR, "on DATA");
        press(&mut game, Joypad::B);
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(game.ui().get(15, 10), UiSurface::BLANK, "the side menu's cursor is wiped");
    }

    /// An unseen number never opens the menu: the list simply comes back.
    #[test]
    fn an_unseen_mon_does_not_open_the_side_menu() {
        let mut world = World { pokedex: dex(20, 20), ..World::default() };
        world.pokedex.seen[0] &= !1;
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Pokedex(PokedexMenu::new()));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::Pokedex);
        assert!(matches!(game.modes().last(), Some(Mode::Pokedex(_))), "and the dex stays open");
        assert_eq!(game.ui().get(15, 10), UiSurface::BLANK, "nothing was drawn");
    }

    #[test]
    fn quit_closes_the_whole_dex() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        for _ in 0..3 {
            press(&mut game, Joypad::DOWN);
            until_waiting(&mut game, Decision::PokedexSideMenu);
        }
        press(&mut game, Joypad::A);
        for _ in 0..10 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
    }

    #[test]
    fn b_on_the_list_closes_the_dex() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::B);
        for _ in 0..10 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
    }

    #[test]
    fn the_data_page_shows_a_height_a_weight_and_a_description() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexData);
        assert_eq!(row_text(&game, 2, 9..18), PokemonSpecies::Bulbasaur.name(), "the name");
        assert_eq!(row_text(&game, 4, 9..13), encode("SEED").unwrap(), "the species line");
        assert_eq!(row_text(&game, 8, 2..7), encode("№<DOT>001").unwrap());
        assert_eq!(row_text(&game, 6, 13..18), encode("2′04″").unwrap(), "2 feet 4 inches");
        // The entry says `dw 150`: tenths of a pound, with the point shuffled in afterwards.
        assert_eq!(row_text(&game, 8, 11..17), encode("  15<DOT>0").unwrap(), "15.0 lb");
        assert_ne!(game.ui().get(1, 11), UiSurface::BLANK, "and the description under the line");
        assert_eq!(game.ui().get(7, 1), 0, "the picture's first tile is its top right");
    }

    /// A mon that has been seen but never owned shows the picture and nothing else.
    #[test]
    fn an_unowned_page_has_no_height_weight_or_description() {
        let mut game = game(dex(20, 0));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexData);
        assert_eq!(row_text(&game, 6, 13..18), encode("?′??″").unwrap(), "the template's own marks");
        assert_eq!(game.ui().get(1, 11), UiSurface::BLANK, "and no description");
    }

    /// Leaving the data page puts the list back where it was.
    #[test]
    fn the_list_comes_back_where_it_was_left() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexData);
        // A description breaks into pages, and each break waits on its own button, so the way out
        // is to keep answering until the list is back rather than to count presses.
        for _ in 0..4 {
            if game.status() == Status::Waiting(Decision::Pokedex) {
                break;
            }
            press(&mut game, Joypad::A);
            for _ in 0..100 {
                if matches!(game.status(), Status::Waiting(_)) {
                    break;
                }
                game.frame(Input::None);
            }
        }
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(cursor_row(&game), Some(5), "the second row again");
        assert_eq!(row_text(&game, 3, 4..13), PokemonSpecies::Bulbasaur.name(), "and the same window");
    }

    /// A data page is left in two presses: one answers the break in the middle of the description
    /// and one answers the wait at the end.
    #[test]
    fn a_data_page_is_read_in_two_presses() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexData);
        assert_eq!(game.ui().get(18, 16), 0xEE, "the ▼ of the page break");
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexData);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::Pokedex);
    }

    #[test]
    fn the_counts_are_what_the_two_arrays_hold() {
        let mut game = game(dex(20, 3));
        until_waiting(&mut game, Decision::Pokedex);
        assert_eq!(row_text(&game, 3, 16..19), encode(" 20").unwrap());
        assert_eq!(row_text(&game, 6, 16..19), encode("  3").unwrap());
    }

    #[test]
    fn a_command_walks_the_list_to_the_number_it_names() {
        let mut game = game(dex(151, 151));
        until_waiting(&mut game, Decision::Pokedex);
        let command = Command::ChooseDexEntry(25);
        assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..600 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(command)]);
        assert!(matches!(game.status(), Status::Waiting(Decision::PokedexSideMenu)), "{:?}", game.status());
    }

    #[test]
    fn a_number_past_what_has_been_seen_is_refused() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        let reply = game.frame(Input::Command(Command::ChooseDexEntry(120))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }

    #[test]
    fn a_command_answers_the_side_menu_and_closes_the_page() {
        let mut game = game(dex(20, 20));
        until_waiting(&mut game, Decision::Pokedex);
        press(&mut game, Joypad::A);
        until_waiting(&mut game, Decision::PokedexSideMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        for _ in 0..400 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::PokedexData) {
                break;
            }
        }
        assert_eq!(game.status(), Status::Waiting(Decision::PokedexData));
        game.frame(Input::Command(Command::CloseDex));
        for _ in 0..400 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Pokedex) {
                break;
            }
        }
        assert_eq!(game.status(), Status::Waiting(Decision::Pokedex));
    }

    #[test]
    fn a_save_mid_dex_resumes_identically() {
        let mut whole = game(dex(30, 30));
        until_waiting(&mut whole, Decision::Pokedex);
        whole.frame(Input::Buttons(Joypad::DOWN));
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..60 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}
