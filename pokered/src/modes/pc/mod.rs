//! `ActivatePC` and `PCMainMenu`: a Pokémon Center's PC, turned on and its menu put up until the
//! player logs off.
//!
//! The menu is `DisplayPCMainMenu`: SOMEONE's PC (BILL's once `EVENT_MET_BILL` is set), the player's
//! PC, PROF.OAK's PC once `EVENT_GOT_POKEDEX` is set, `<PKMN>LEAGUE` once a team is in the Hall of
//! Fame, and LOG OFF. The box grows with the rows, but the league's row and the taller box follow
//! `wNumHoFTeams` alone, so a Hall of Fame without the Pokédex draws a box a row too tall.
//!
//! The menu sets `BIT_NO_MENU_BUTTON_SOUND`, so A and B leave it silently and each row plays its
//! own sound instead: `SFX_ENTER_PC` into a PC, `SFX_TURN_OFF_PC` on the way out. The player's PC
//! clears the bit before it starts; the others leave it set for their own menus. Every PC but the
//! player's goes back to the menu over a reloaded map.
//!
//! `BIT_USING_GENERIC_PC` is the `via_pc` constructors' argument rather than a flag: nothing but the
//! PCs reads it.

pub mod bills_pc;
pub mod league_pc;
pub mod oaks_pc;
pub mod players_pc;
#[cfg(test)]
mod tests;

use poke_core::symbols::pokered_events::{EVENT_GOT_POKEDEX, EVENT_MET_BILL};
use poke_core::symbols::{pokered_symbols as sym, DmgPointer};
use poke_core::text_script::far_text;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::place_string::{ch, ligature};
use crate::modes::text_box::TextBox;
use bills_pc::BillsPc;
use league_pc::LeaguePc;
use oaks_pc::OaksPc;
use players_pc::PlayerPc;

/// `wTopMenuItemX` and `Y` for both PC menus.
pub(crate) const TOP: (u8, u8) = (1, 2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcMenu {
    /// `wTileMapBackup2`.
    saved: Option<UiSurface>,
    /// `wMaxMenuItem` as the menu was drawn.
    max: u8,
    phase: Phase,
    /// PROF.OAK's or the league's PC while it is open, which are not modes of their own.
    #[serde(default)]
    open: Option<Opened>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Opened {
    Oaks(OaksPc),
    League(LeaguePc),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Child(After),
    /// `WaitForSoundToFinish`, then this.
    Sound(Next),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    TurnedOn,
    Menu,
    /// The text a row prints before opening its PC.
    Accessed(Row),
    /// A PC a row opened has closed.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Next {
    TurnedOn,
    Enter(Row),
    LogOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Row {
    Bills,
    Players,
    Oaks,
    League,
}

impl PcMenu {
    /// `ActivatePC`, from `TX_SCRIPT_POKECENTER_PC`.
    pub fn new() -> Self {
        Self { saved: None, max: 2, phase: Phase::Child(After::TurnedOn), open: None }
    }

    /// Which PC a row of a menu of `max + 1` rows opens, or `None` for LOG OFF.
    pub fn row(max: u8, row: u8) -> Option<Row> {
        match (max, row) {
            (_, 0) => Some(Row::Bills),
            (_, 1) => Some(Row::Players),
            (3 | 4, 2) => Some(Row::Oaks),
            (4, 3) => Some(Row::League),
            _ => None,
        }
    }

    fn text(&mut self, label: &str, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        print(label, ctx)
    }

    fn wait_for_sound(&mut self, next: Next, ctx: &mut Ctx) -> Transition {
        if !ctx.audio.sound_finished() {
            self.phase = Phase::Sound(next);
            return Transition::Stay;
        }
        match next {
            Next::TurnedOn => {
                // `LoadScreenTilesFromBuffer2`; its `Delay3` is loading.
                if let Some(saved) = &self.saved {
                    ctx.screen.ui = saved.clone();
                }
                self.main_menu(ctx)
            }
            Next::Enter(Row::Bills) => {
                let label = if ctx.world.events.is_set(EVENT_MET_BILL) { "_AccessedBillsPCText" } else { "_AccessedSomeonesPCText" };
                self.text(label, After::Accessed(Row::Bills), ctx)
            }
            Next::Enter(Row::Players) => self.text("_AccessedMyPCText", After::Accessed(Row::Players), ctx),
            Next::Enter(row) => self.open_pc(row, ctx),
            Next::LogOff => {
                ctx.menu.no_menu_button_sound = false;
                Transition::Pop(Outcome::Done)
            }
        }
    }

    /// `DisplayPCMainMenu` and `PCMainMenu`'s `HandleMenuInput`.
    fn main_menu(&mut self, ctx: &mut Ctx) -> Transition {
        self.saved = Some(ctx.screen.ui.clone());
        let league = ctx.world.hall_of_fame_teams != 0;
        let dex = ctx.world.events.is_set(EVENT_GOT_POKEDEX);
        let height = if league { 10 } else if dex { 8 } else { 6 };
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 0, 14, height);
        update_sprites(ctx);
        let ui = &mut ctx.screen.ui;
        let bills = if ctx.world.events.is_set(EVENT_MET_BILL) { sym::BillsPCText } else { sym::SomeonesPCText };
        place_rom_string(ui, 2, 2, bills);
        let name = &ctx.world.player_name;
        ui.place(2, 4, name);
        place_rom_string(ui, 2 + name.len(), 4, sym::PlayersPCText);
        self.max = match (dex, league) {
            (false, _) => {
                place_rom_string(ui, 2, 6, sym::LogOffPCText);
                2
            }
            (true, false) => {
                place_rom_string(ui, 2, 6, sym::OaksPCText);
                place_rom_string(ui, 2, 8, sym::LogOffPCText);
                3
            }
            (true, true) => {
                place_rom_string(ui, 2, 6, sym::OaksPCText);
                place_rom_string(ui, 2, 8, sym::PKMNLeaguePCText);
                place_rom_string(ui, 2, 10, sym::LogOffPCText);
                4
            }
        };
        ctx.menu.last_item = 0;
        ctx.menu.no_menu_button_sound = true;
        self.phase = Phase::Child(After::Menu);
        Transition::Push(Mode::CursorMenu(CursorMenu::new(0, self.max, TOP)))
    }

    fn play_then(&mut self, sound: SoundId, next: Next, ctx: &mut Ctx) -> Transition {
        ctx.audio.play_sound(sound);
        self.wait_for_sound(next, ctx)
    }

    /// The PC a row names, once its sound and its text are over.
    fn open_pc(&mut self, row: Row, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(After::Closed);
        match row {
            Row::Players => Transition::Push(Mode::PlayerPc(PlayerPc::via_pc(self.saved.clone().unwrap_or_default()))),
            Row::Bills => Transition::Push(Mode::BillsPc(BillsPc::via_pc(self.saved.clone().unwrap_or_default()))),
            Row::Oaks => {
                let (pc, transition) = OaksPc::start(ctx);
                self.open = Some(Opened::Oaks(pc));
                transition
            }
            Row::League => {
                let (pc, transition) = LeaguePc::start(ctx);
                self.open = Some(Opened::League(pc));
                transition
            }
        }
    }

    /// What the open PC does next; its `Pop` is this menu's `Closed`.
    fn opened(&mut self, transition: Transition, ctx: &mut Ctx) -> Transition {
        match transition {
            Transition::Pop(_) => {
                self.open = None;
                self.reload_main_menu(ctx)
            }
            transition => transition,
        }
    }

    /// `ReloadMainMenu`: `ReloadMapData` draws the map over the whole screen, with the LCD off.
    fn reload_main_menu(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
        ctx.screen.tiles.load_text_box_tiles();
        if let Some(tileset) = ctx.screen.map.tileset {
            ctx.screen.tiles.load_tileset(tileset);
        }
        update_sprites(ctx);
        self.main_menu(ctx)
    }
}

impl Default for PcMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for PcMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.saved = Some(ctx.screen.ui.clone());
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.audio.play_sound(sounds::SFX_TURN_ON_PC);
        self.text("_TurnedOnPC1Text", After::TurnedOn, ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match (&mut self.open, self.phase) {
            (Some(Opened::Oaks(pc)), _) => {
                let transition = pc.update(ctx);
                self.opened(transition, ctx)
            }
            (Some(Opened::League(pc)), _) => {
                let transition = pc.update(ctx);
                self.opened(transition, ctx)
            }
            (None, Phase::Child(_)) => Transition::Stay,
            (None, Phase::Sound(next)) => self.wait_for_sound(next, ctx),
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match &mut self.open {
            Some(Opened::Oaks(pc)) => {
                let transition = pc.resume(outcome, ctx);
                return self.opened(transition, ctx);
            }
            Some(Opened::League(pc)) => {
                let transition = pc.resume(outcome, ctx);
                return self.opened(transition, ctx);
            }
            None => {}
        }
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match after {
            After::TurnedOn => self.wait_for_sound(Next::TurnedOn, ctx),
            After::Menu => match outcome {
                Outcome::Chosen(row) => match Self::row(self.max, row) {
                    Some(Row::Players) => {
                        ctx.menu.no_menu_button_sound = false;
                        self.play_then(sounds::SFX_ENTER_PC, Next::Enter(Row::Players), ctx)
                    }
                    Some(row) => self.play_then(sounds::SFX_ENTER_PC, Next::Enter(row), ctx),
                    None => self.play_then(sounds::SFX_TURN_OFF_PC, Next::LogOff, ctx),
                },
                _ => self.play_then(sounds::SFX_TURN_OFF_PC, Next::LogOff, ctx),
            },
            After::Accessed(row) => self.open_pc(row, ctx),
            After::Closed => self.reload_main_menu(ctx),
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

/// `PrintText`, whose `UpdateSprites` follows the box.
pub(crate) fn print(label: &str, ctx: &mut Ctx) -> Transition {
    let script = far_text(label).expect("the PC's texts are in the cartridge");
    update_sprites(ctx);
    Transition::Push(Mode::TextBox(TextBox::script(script)))
}

/// `PrintText` of a text by its address, for one whose wait is in the `text_far` wrapper rather
/// than in the text itself.
pub(crate) fn print_at(at: DmgPointer, ctx: &mut Ctx) -> Transition {
    let script = poke_core::text_script::decode(at).expect("the PC's texts are in the cartridge");
    update_sprites(ctx);
    Transition::Push(Mode::TextBox(TextBox::script(script)))
}

/// `PrintText` with `hAutoBGTransferEnabled` off: `shown` stays up until the text is done.
pub(crate) fn print_off_screen(label: &str, shown: UiSurface, ctx: &mut Ctx) -> Transition {
    let script = far_text(label).expect("the PC's texts are in the cartridge");
    update_sprites(ctx);
    Transition::Push(Mode::TextBox(TextBox::script(script).off_screen(shown)))
}

/// `UpdateSprites`, which hides whatever sprite a box now covers.
pub(crate) fn update_sprites(ctx: &mut Ctx) {
    ctx.update_sprites = true;
}

/// `PlaceString` of a string in the cartridge, ligatures expanded.
fn place_rom_string(ui: &mut UiSurface, x: usize, y: usize, at: DmgPointer) {
    let mut column = x;
    for &byte in poke_core::rom_gfx::rom_slice(at).iter().take_while(|&&b| b != ch::TERMINATOR) {
        let tiles = ligature(byte).map_or_else(|| vec![byte], <[u8]>::to_vec);
        ui.place(column, y, &tiles);
        column += tiles.len();
    }
}
