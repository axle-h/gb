//! `BillsPC_`: WITHDRAW, DEPOSIT and RELEASE a mon, CHANGE BOX, and SEE YA!, over the party and the
//! current box.
//!
//! The menu comes back after every answer, drawn afresh over the screen from before the PC with
//! "What?" and the box number, its cursor on the row last chosen (`wParentMenuItem`). Withdrawing
//! wants a mon in the box and room in the party; depositing wants a mon left behind and room in the
//! box; releasing wants a mon in the box. A mon is chosen from `PCPOKEMONLISTMENU`, then WITHDRAW or
//! DEPOSIT, STATS or CANCEL; STATS shows the mon and asks again. A deposit goes to the end of the
//! box and a withdrawal to the end of the party, each crying as it moves. A release asks first, and
//! a NO goes back to the list where it was.
//!
//! CHANGE BOX asks whether the game may be saved, lists the twelve boxes with a ball beside each
//! that holds a mon, and makes the one chosen current. Changing one is `SaveGameData` and `SFX_SAVE`,
//! the same save the SAVE menu writes. `EmptyAllSRAMBoxes` on the first change is not modelled,
//! because no box but the current one can hold a mon before a change.
//!
//! `ExitListMenu` clears `BIT_NO_TEXT_DELAY`, which `BillsPC_` set, so once a list has closed every
//! text prints a letter at a time. "What?" and "Choose a <PKMN> BOX." print with the background
//! transfer off, so they spend their letter delays unseen and appear whole with the menu under them.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::text_script::TextBuffer;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::list_menu::ListMenu;
use crate::modes::status_screen::StatusScreen;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::party::{BoxMon, Named, MONS_PER_BOX, PARTY_LENGTH};
use crate::systems::add_mon::{deposit, withdraw};
use crate::systems::status_screen::place_lines;
use super::{print, print_off_screen, update_sprites, TOP};

/// `BillsPCMenuText`'s rows.
pub const WITHDRAW: u8 = 0;
pub const DEPOSIT: u8 = 1;
pub const RELEASE: u8 = 2;
pub const CHANGE_BOX: u8 = 3;
pub const SEE_YA: u8 = 4;
/// `DisplayDepositWithdrawMenu`'s rows.
pub const MOVE: u8 = 0;
pub const STATS: u8 = 1;
/// `NUM_BOXES`.
pub const NUM_BOXES: u8 = 12;
/// `hlcoord 14, 7`: where `YesNoChoice` asks.
const YES_NO_AT: (usize, usize) = (14, 7);
/// `wTopMenuItemX` and `Y` of `DisplayDepositWithdrawMenu`.
const DEPOSIT_WITHDRAW_TOP: (u8, u8) = (10, 12);
/// `wTopMenuItemX` and `Y` of `DisplayChangeBoxMenu`.
const BOX_LIST_TOP: (u8, u8) = (12, 1);
/// The ball `BillsPCMenu` loads at `vChars2` tile `$78`.
const BALL: u8 = 0x78;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillsPc {
    /// `BIT_USING_GENERIC_PC`.
    generic: bool,
    /// `wTileMapBackup2`, which every return to the menu puts back.
    saved: Option<UiSurface>,
    /// `wTileMapBackup`, from before the status screen.
    under_stats: Option<UiSurface>,
    /// `wParentMenuItem`.
    parent: u8,
    /// `wListScrollOffset` as `BillsPC_` pushed it.
    scroll: u8,
    /// `wWhichPokemon`.
    slot: u8,
    /// `wCurPartySpecies`.
    species: PokemonSpecies,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Child(After),
    /// `WaitForSoundToFinish`, then this.
    Sound(Next),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    SwitchedOn,
    /// "What?", then the box number and the menu.
    MenuText,
    Menu,
    /// A text after which the menu comes back.
    ToMenu,
    List,
    DepositWithdraw,
    Stats,
    OnceReleased,
    ConfirmRelease,
    WhenYouChangeBox,
    ConfirmChangeBox,
    /// "Choose a <PKMN> BOX.", then the list of boxes.
    ChooseABox,
    BoxList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Next {
    TurnedOff,
    /// `PlaySoundWaitForCurrent` with the mon's cry, then the move.
    BeforeCry,
    Moved,
    /// Released, and `WaitForSoundToFinish` before `PlayCry`.
    Released,
    ReleasedCry,
    /// `PlaySoundWaitForCurrent` with `SFX_SAVE`.
    BeforeSave,
    Saved,
}

impl BillsPc {
    fn with(generic: bool, saved: Option<UiSurface>) -> Self {
        Self {
            generic,
            saved,
            under_stats: None,
            parent: 0,
            scroll: 0,
            slot: 0,
            species: PokemonSpecies::Rhydon,
            phase: Phase::Child(After::SwitchedOn),
        }
    }

    /// `BillsPC_` from `TX_SCRIPT_BILLS_PC`, which saves the screen as it calls in.
    pub fn direct() -> Self {
        Self::with(false, None)
    }

    /// `BillsPC_` from the Pokémon Center's menu, whose screen from before its box is `saved`.
    pub fn via_pc(saved: UiSurface) -> Self {
        Self::with(true, Some(saved))
    }

    fn text(&mut self, label: &str, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        print(label, ctx)
    }

    /// `BillsPCMenu` up to its "What?".
    fn menu(&mut self, ctx: &mut Ctx) -> Transition {
        let shown = ctx.screen.ui.clone();
        // `CopyVideoData`'s frames are loading.
        ctx.screen.tiles.load(V_CHARS2 + BALL as usize, &rom_slice(sym::PokeballTileGraphics)[..TILE_BYTES]);
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 0, 12, 10);
        place_lines(ui, 2 * SCREEN_TILES_X + 2, rom_slice(sym::BillsPCMenuText), false);
        ctx.menu.last_item = 0;
        ctx.menu.party_and_bills = 0;
        ctx.menu.list_scroll = 0;
        self.phase = Phase::Child(After::MenuText);
        print_off_screen("_WhatText", shown, ctx)
    }

    /// `ExitBillsPC`, after its sound when there is one.
    fn exit(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.no_menu_button_sound = false;
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        ctx.menu.list_scroll = self.scroll;
        ctx.world.no_text_delay = false;
        Transition::Pop(Outcome::Done)
    }

    fn current_box<'a>(&self, ctx: &'a mut Ctx) -> &'a mut Vec<Named<BoxMon>> {
        let current = ctx.world.current_box as usize;
        if ctx.world.boxes.len() <= current {
            ctx.world.boxes.resize(current + 1, Vec::new());
        }
        &mut ctx.world.boxes[current]
    }

    /// `DisplayMonListMenu` over the party for a deposit and the box otherwise.
    fn mon_list(&mut self, ctx: &mut Ctx) -> Transition {
        let mons = if self.parent == DEPOSIT {
            ctx.world.party.iter().map(|named| (named.nick.clone(), named.mon.level)).collect()
        } else {
            self.current_box(ctx).iter().map(|named| (named.nick.clone(), named.mon.box_level)).collect()
        };
        self.phase = Phase::Child(After::List);
        update_sprites(ctx);
        Transition::Push(Mode::ListMenu(ListMenu::pokemon(mons, ctx.menu.party_and_bills, ctx.menu.list_scroll)))
    }

    /// `DisplayDepositWithdrawMenu` up to its `HandleMenuInput`.
    fn deposit_withdraw_menu(&mut self, ctx: &mut Ctx) -> Transition {
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(9, 10, 9, 6);
        let label = if self.parent == WITHDRAW { sym::WithdrawPCText } else { sym::DepositPCText };
        place_lines(ui, 12 * SCREEN_TILES_X + 11, rom_slice(label), false);
        place_lines(ui, 14 * SCREEN_TILES_X + 11, rom_slice(sym::StatsCancelPCText), false);
        ctx.menu.last_item = 0;
        ctx.menu.list_scroll = 0;
        ctx.menu.party_and_bills = 0;
        self.deposit_withdraw_input(MOVE)
    }

    fn deposit_withdraw_input(&mut self, current: u8) -> Transition {
        self.phase = Phase::Child(After::DepositWithdraw);
        Transition::Push(Mode::CursorMenu(CursorMenu::new(current, 2, DEPOSIT_WITHDRAW_TOP)))
    }

    fn yes_no(&mut self, after: After) -> Transition {
        self.phase = Phase::Child(after);
        Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
    }

    /// `BillsPCWithdraw`, `BillsPCDeposit` and `BillsPCRelease` up to their list.
    fn start(&mut self, ctx: &mut Ctx) -> Transition {
        let in_box = self.current_box(ctx).len();
        let refusal = match self.parent {
            DEPOSIT if ctx.world.party.len() <= 1 => Some("_CantDepositLastMonText"),
            DEPOSIT if in_box >= MONS_PER_BOX => Some("_BoxFullText"),
            WITHDRAW | RELEASE if in_box == 0 => Some("_NoMonText"),
            WITHDRAW if ctx.world.party.len() >= PARTY_LENGTH => Some("_CantTakeMonText"),
            _ => None,
        };
        match refusal {
            Some(label) => self.text(label, After::ToMenu, ctx),
            None => self.mon_list(ctx),
        }
    }

    /// The mon under the list's cursor: `wWhichPokemon`, `wCurPartySpecies` and its name in the
    /// buffers the texts read.
    fn chosen(&mut self, slot: u8, ctx: &mut Ctx) {
        self.slot = slot;
        let (species, nick) = if self.parent == DEPOSIT {
            let named = &ctx.world.party[slot as usize];
            (named.mon.mon.species, named.nick.clone())
        } else {
            let named = &self.current_box(ctx)[slot as usize];
            (named.mon.species, named.nick.clone())
        };
        self.species = species;
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick.clone());
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, nick);
    }

    /// `MoveMon` and `RemovePokemon`, the cry already begun.
    fn move_mon(&mut self, ctx: &mut Ctx) {
        let slot = self.slot as usize;
        if self.parent == DEPOSIT {
            let named = ctx.world.party.remove(slot);
            let boxed = Named { mon: deposit(named.mon), ot: named.ot, nick: named.nick };
            self.current_box(ctx).push(boxed);
        } else {
            let named = self.current_box(ctx).remove(slot);
            ctx.world.party.push(Named { mon: withdraw(named.mon), ot: named.ot, nick: named.nick });
        }
    }

    /// `DisplayChangeBoxMenu` from after its "Choose a <PKMN> BOX.", and `ChangeBox`'s
    /// `HandleMenuInput`.
    fn box_list(&mut self, ctx: &mut Ctx) -> Transition {
        let current = ctx.world.current_box;
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(11, 0, 7, 12);
        place_lines(ui, SCREEN_TILES_X + 13, rom_slice(sym::BoxNames), true);
        place_box_number(ui, 1, 2, current);
        place_lines(ui, 2 * SCREEN_TILES_X + 1, rom_slice(sym::BoxNoText), false);
        for i in 0..NUM_BOXES as usize {
            if ctx.world.boxes.get(i).is_some_and(|mons| !mons.is_empty()) {
                ui.set(18, 1 + i, BALL);
            }
        }
        update_sprites(ctx);
        ctx.menu.last_item = current;
        self.phase = Phase::Child(After::BoxList);
        Transition::Push(Mode::CursorMenu(CursorMenu::new(current, NUM_BOXES - 1, BOX_LIST_TOP).single_spaced()))
    }

    fn wait_for_sound(&mut self, next: Next, ctx: &mut Ctx) -> Transition {
        if !ctx.audio.sound_finished() {
            self.phase = Phase::Sound(next);
            return Transition::Stay;
        }
        match next {
            Next::TurnedOff => self.exit(ctx),
            Next::BeforeCry => {
                ctx.audio.play_cry(self.species as u8);
                self.move_mon(ctx);
                self.wait_for_sound(Next::Moved, ctx)
            }
            Next::Moved if self.parent == DEPOSIT => {
                let number = ctx.world.current_box + 1;
                let digits = poke_core::charmap::encode(&number.to_string()).expect("digits are in the charmap");
                ctx.world.text.strings.insert(TextBuffer::BoxNumString, digits);
                self.text("_MonWasStoredText", After::ToMenu, ctx)
            }
            Next::Moved => self.text("_MonIsTakenOutText", After::ToMenu, ctx),
            Next::Released => {
                ctx.audio.play_cry(self.species as u8);
                self.wait_for_sound(Next::ReleasedCry, ctx)
            }
            Next::ReleasedCry => self.text("_MonWasReleasedText", After::ToMenu, ctx),
            Next::BeforeSave => {
                ctx.audio.play_sound(sounds::SFX_SAVE);
                self.wait_for_sound(Next::Saved, ctx)
            }
            Next::Saved => self.menu(ctx),
        }
    }

    /// `ExitBillsPC`: the sound first when the PC was turned on directly.
    fn see_ya(&mut self, ctx: &mut Ctx) -> Transition {
        if self.generic {
            return self.exit(ctx);
        }
        ctx.screen.tiles.load_text_box_tiles();
        ctx.audio.play_sound(sounds::SFX_TURN_OFF_PC);
        self.wait_for_sound(Next::TurnedOff, ctx)
    }
}

/// The box number as `BillsPCMenu` and `DisplayChangeBoxMenu` write it, ending at `x + 8`.
fn place_box_number(ui: &mut UiSurface, x: usize, y: usize, current: u8) {
    const ZERO: u8 = 0xF6;
    let number = current + 1;
    if number >= 10 {
        ui.set(x + 7, y, ZERO + 1);
    }
    ui.set(x + 8, y, ZERO + number % 10);
}

impl ModeUpdate for BillsPc {
    fn enter(&mut self, ctx: &mut Ctx) {
        if self.saved.is_none() {
            self.saved = Some(ctx.screen.ui.clone());
        }
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.world.no_text_delay = true;
        self.parent = 0;
        ctx.screen.tiles.load_hp_bar_and_status_tiles();
        self.scroll = ctx.menu.list_scroll;
        if self.generic {
            return self.menu(ctx);
        }
        ctx.audio.play_sound(sounds::SFX_TURN_ON_PC);
        self.text("_SwitchOnText", After::SwitchedOn, ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Child(_) => Transition::Stay,
            Phase::Sound(next) => self.wait_for_sound(next, ctx),
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match (after, outcome) {
            (After::SwitchedOn | After::ToMenu, _) => self.menu(ctx),
            (After::MenuText, _) => {
                let ui = &mut ctx.screen.ui;
                ui.text_box_border(9, 14, 9, 2);
                place_box_number(ui, 10, 16, ctx.world.current_box);
                place_lines(ui, 16 * SCREEN_TILES_X + 10, rom_slice(sym::BoxNoPCText), false);
                // The `Delay3` before the menu is loading.
                self.phase = Phase::Child(After::Menu);
                Transition::Push(Mode::CursorMenu(CursorMenu::new(self.parent, SEE_YA, TOP)))
            }
            (After::Menu, Outcome::Chosen(row)) => {
                ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                self.parent = row;
                match row {
                    WITHDRAW | DEPOSIT | RELEASE => self.start(ctx),
                    CHANGE_BOX => self.text("_WhenYouChangeBoxText", After::WhenYouChangeBox, ctx),
                    _ => self.see_ya(ctx),
                }
            }
            (After::Menu, _) => self.see_ya(ctx),
            (After::List, Outcome::Chosen(slot)) => {
                ctx.menu.party_and_bills = ctx.menu.chosen_item;
                self.chosen(slot, ctx);
                if self.parent == RELEASE {
                    return self.text("_OnceReleasedText", After::OnceReleased, ctx);
                }
                self.deposit_withdraw_menu(ctx)
            }
            (After::List, _) => self.menu(ctx),
            (After::DepositWithdraw, Outcome::Chosen(MOVE)) => self.wait_for_sound(Next::BeforeCry, ctx),
            (After::DepositWithdraw, Outcome::Chosen(STATS)) => {
                self.under_stats = Some(ctx.screen.ui.clone());
                let screen = if self.parent == WITHDRAW {
                    StatusScreen::from_box(self.current_box(ctx)[self.slot as usize].clone())
                } else {
                    StatusScreen::new(ctx.world.party[self.slot as usize].clone())
                };
                self.phase = Phase::Child(After::Stats);
                Transition::Push(Mode::StatusScreen(screen))
            }
            (After::DepositWithdraw, _) => self.menu(ctx),
            (After::Stats, _) => {
                if let Some(saved) = self.under_stats.take() {
                    ctx.screen.ui = saved;
                }
                if let Some(tileset) = ctx.screen.map.tileset {
                    ctx.screen.tiles.load_tileset(tileset);
                }
                ctx.screen.sgb.run(&PaletteCommand::Default);
                self.deposit_withdraw_input(STATS)
            }
            (After::OnceReleased, _) => self.yes_no(After::ConfirmRelease),
            (After::ConfirmRelease, Outcome::Chosen(0)) => {
                self.current_box(ctx).remove(self.slot as usize);
                self.wait_for_sound(Next::Released, ctx)
            }
            (After::ConfirmRelease, _) => self.mon_list(ctx),
            (After::WhenYouChangeBox, _) => self.yes_no(After::ConfirmChangeBox),
            (After::ConfirmChangeBox, Outcome::Chosen(0)) => {
                let shown = ctx.screen.ui.clone();
                ctx.screen.ui.text_box_border(0, 0, 9, 2);
                self.phase = Phase::Child(After::ChooseABox);
                print_off_screen("_ChooseABoxText", shown, ctx)
            }
            (After::ConfirmChangeBox, _) => self.menu(ctx),
            (After::ChooseABox, _) => self.box_list(ctx),
            (After::BoxList, Outcome::Chosen(row)) => {
                ctx.world.current_box = row;
                ctx.save_game = true;
                self.wait_for_sound(Next::BeforeSave, ctx)
            }
            (After::BoxList, _) => self.menu(ctx),
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}
