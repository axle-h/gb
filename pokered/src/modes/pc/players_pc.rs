//! `PlayerPC`: WITHDRAW ITEM, DEPOSIT ITEM, TOSS ITEM and LOG OFF over the items in the PC.
//!
//! Each of the first three lists its side, the bag or the PC, and loops back to the list after
//! every item until the list is left. A key item or an HM moves one at a time; anything else asks
//! how many first, and B there goes back to the list. A deposit or a withdrawal the other side has
//! no slot for says so and changes nothing. A toss goes through `TossItem_`, whose yes/no leaves
//! `wCurrentMenuItem` on its answer, so after a NO the list reopens a row down. Emptying a slot puts
//! the list back at its top, as `RemoveItemFromInventory_` does.
//!
//! The menu reopens on the row last chosen (`wParentMenuItem`). Reached from a Pokémon Center's
//! menu it is silent on the way in and out; turned on directly, as in the player's room, it plays
//! `SFX_TURN_ON_PC` and `SFX_TURN_OFF_PC` itself.

use poke_core::item::{self, is_key_item, ItemId};
use poke_core::text_script::TextBuffer;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::gfx::ui::UiSurface;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::list_menu::{remove_from_bag, ListMenu};
use crate::modes::quantity_menu::QuantityMenu;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::inventory::Inventory;
use super::{print, update_sprites, TOP};

/// `PlayersPCMenuEntries`: WITHDRAW ITEM, DEPOSIT ITEM, TOSS ITEM, LOG OFF.
const LOG_OFF: u8 = 3;
/// `hlcoord 14, 7`: where `TossItem_` asks its yes/no.
const YES_NO_AT: (usize, usize) = (14, 7);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerPc {
    /// `BIT_USING_GENERIC_PC`.
    generic: bool,
    /// `wTileMapBackup2`, which every return to the menu puts back.
    saved: Option<UiSurface>,
    /// `wParentMenuItem`.
    parent: u8,
    /// `wCurrentMenuItem` between one list and the next.
    current: u8,
    action: Action,
    item: ItemId,
    /// `wWhichPokemon`: the chosen item's slot.
    slot: u8,
    /// `wItemQuantity`.
    quantity: u8,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Withdraw,
    Deposit,
    Toss,
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
    /// "What do you want to do?", then the menu.
    MenuText,
    Menu,
    /// A text after which the menu comes back.
    ToMenu,
    /// "What do you want to ...?", then the list.
    ListText,
    List,
    /// "How many?", then the count.
    HowMany,
    Quantity,
    /// A text after which the list comes back.
    ToList,
    AskToss,
    ConfirmToss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Next {
    /// `SFX_TURN_OFF_PC` is over.
    TurnedOff,
    /// Whatever was sounding before `SFX_WITHDRAW_DEPOSIT`.
    BeforeMoved,
    Moved,
}

impl PlayerPc {
    fn with(generic: bool, saved: Option<UiSurface>) -> Self {
        Self {
            generic,
            saved,
            parent: 0,
            current: 0,
            action: Action::Withdraw,
            item: ItemId::Potion,
            slot: 0,
            quantity: 0,
            phase: Phase::Child(After::TurnedOn),
        }
    }

    /// `PlayerPC` from `TX_SCRIPT_PLAYERS_PC`, which saves the screen as it calls in.
    pub fn direct() -> Self {
        Self::with(false, None)
    }

    /// `PlayerPC` from the Pokémon Center's menu, whose screen from before its box is `saved`.
    pub fn via_pc(saved: UiSurface) -> Self {
        Self::with(true, Some(saved))
    }

    fn text(&mut self, label: &str, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        print(label, ctx)
    }

    /// `PlayerPCMenu`.
    fn menu(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.no_menu_button_sound = true;
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        ctx.screen.ui.text_box_border(0, 0, 14, 8);
        update_sprites(ctx);
        for (row, entry) in ["WITHDRAW ITEM", "DEPOSIT ITEM", "TOSS ITEM", "LOG OFF"].into_iter().enumerate() {
            let entry = poke_core::charmap::encode(entry).expect("the menu's rows are in the charmap");
            ctx.screen.ui.place(2, 2 + 2 * row, &entry);
        }
        ctx.menu.list_scroll = 0;
        self.text("_WhatDoYouWantText", After::MenuText, ctx)
    }

    /// `ExitPlayerPC`, after its sound when there is one.
    fn exit(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.no_menu_button_sound = false;
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        ctx.menu.list_scroll = 0;
        ctx.menu.bag_saved = 0;
        ctx.world.no_text_delay = false;
        Transition::Pop(Outcome::Done)
    }

    fn inventory<'a>(&self, ctx: &'a Ctx) -> &'a Inventory {
        match self.action {
            Action::Deposit => &ctx.world.bag,
            Action::Withdraw | Action::Toss => &ctx.world.pc_items,
        }
    }

    /// `PlayerPCWithdraw`, `PlayerPCDeposit` and `PlayerPCToss` up to their `.loop`.
    fn start(&mut self, action: Action, ctx: &mut Ctx) -> Transition {
        self.action = action;
        self.current = 0;
        ctx.menu.list_scroll = 0;
        if self.inventory(ctx).items.is_empty() {
            let label = if action == Action::Deposit { "_NothingToDepositText" } else { "_NothingStoredText" };
            return self.text(label, After::ToMenu, ctx);
        }
        self.list_loop(ctx)
    }

    /// `.loop`.
    fn list_loop(&mut self, ctx: &mut Ctx) -> Transition {
        let label = match self.action {
            Action::Withdraw => "_WhatToWithdrawText",
            Action::Deposit => "_WhatToDepositText",
            Action::Toss => "_WhatToTossText",
        };
        self.text(label, After::ListText, ctx)
    }

    fn list(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(After::List);
        let scroll = ctx.menu.list_scroll;
        let list = match self.action {
            Action::Deposit => ListMenu::bag(self.current, scroll),
            Action::Withdraw | Action::Toss => ListMenu::pc_items(self.current, scroll),
        };
        update_sprites(ctx);
        Transition::Push(Mode::ListMenu(list))
    }

    /// `DisplayListMenuID`'s answer: the item named into `wNameBuffer`, then how many unless it is
    /// a key item or an HM.
    fn chosen(&mut self, slot: u8, ctx: &mut Ctx) -> Transition {
        let held = self.inventory(ctx).items[slot as usize];
        (self.slot, self.item) = (slot, held.id);
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, item::name(held.id));
        self.quantity = 1;
        if is_key_item(held.id) || held.id.is_hm() {
            return self.move_item(ctx);
        }
        let label = match self.action {
            Action::Withdraw => "_WithdrawHowManyText",
            Action::Deposit => "_DepositHowManyText",
            Action::Toss => "_TossHowManyText",
        };
        self.text(label, After::HowMany, ctx)
    }

    /// `.next`: into the other side and out of this one, or `TossItem`.
    fn move_item(&mut self, ctx: &mut Ctx) -> Transition {
        match self.action {
            Action::Toss => self.toss(ctx),
            Action::Deposit => {
                if !ctx.world.pc_items.add(self.item, self.quantity) {
                    return self.text("_NoRoomToStoreText", After::ToList, ctx);
                }
                if remove_from_bag(ctx, self.slot as usize, self.quantity) {
                    self.current = 0;
                }
                self.wait_for_sound(Next::BeforeMoved, ctx)
            }
            Action::Withdraw => {
                if !ctx.world.bag.add(self.item, self.quantity) {
                    return self.text("_CantCarryMoreText", After::ToList, ctx);
                }
                self.remove_from_pc(ctx);
                self.wait_for_sound(Next::BeforeMoved, ctx)
            }
        }
    }

    /// `RemoveItemFromInventory` on `wNumBoxItems`, which resets the same menu bytes the bag's does.
    fn remove_from_pc(&mut self, ctx: &mut Ctx) {
        let before = ctx.world.pc_items.items.len();
        ctx.world.pc_items.remove(self.slot as usize, self.quantity);
        if ctx.world.pc_items.items.len() != before {
            ctx.menu.list_scroll = 0;
            ctx.menu.bag_saved = 0;
            self.current = 0;
        }
    }

    /// `TossItem_`, with the PC's items.
    fn toss(&mut self, ctx: &mut Ctx) -> Transition {
        if !Inventory::may_toss(self.item) {
            return self.text("_TooImportantToTossText", After::ToList, ctx);
        }
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, item::name(self.item));
        self.text("_IsItOKToTossItemText", After::AskToss, ctx)
    }

    fn wait_for_sound(&mut self, next: Next, ctx: &mut Ctx) -> Transition {
        if !ctx.audio.sound_finished() {
            self.phase = Phase::Sound(next);
            return Transition::Stay;
        }
        match next {
            Next::TurnedOff => self.exit(ctx),
            Next::BeforeMoved => {
                ctx.audio.play_sound(sounds::SFX_WITHDRAW_DEPOSIT);
                self.wait_for_sound(Next::Moved, ctx)
            }
            Next::Moved => {
                let label = if self.action == Action::Deposit { "_ItemWasStoredText" } else { "_WithdrewItemText" };
                self.text(label, After::ToList, ctx)
            }
        }
    }
}

impl ModeUpdate for PlayerPc {
    fn enter(&mut self, ctx: &mut Ctx) {
        if self.saved.is_none() {
            self.saved = Some(ctx.screen.ui.clone());
        }
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.world.no_text_delay = true;
        ctx.menu.bag_saved = 0;
        self.parent = 0;
        if self.generic {
            return self.menu(ctx);
        }
        ctx.audio.play_sound(sounds::SFX_TURN_ON_PC);
        self.text("_TurnedOnPC2Text", After::TurnedOn, ctx)
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
            (After::TurnedOn | After::ToMenu, _) => self.menu(ctx),
            (After::MenuText, _) => {
                self.phase = Phase::Child(After::Menu);
                Transition::Push(Mode::CursorMenu(CursorMenu::new(self.parent, LOG_OFF, TOP)))
            }
            (After::Menu, Outcome::Chosen(row)) => {
                ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                self.parent = row;
                match row {
                    0 => self.start(Action::Withdraw, ctx),
                    1 => self.start(Action::Deposit, ctx),
                    2 => self.start(Action::Toss, ctx),
                    _ => self.log_off(ctx),
                }
            }
            (After::Menu, _) => self.log_off(ctx),
            (After::ListText, _) => self.list(ctx),
            (After::List, Outcome::Chosen(slot)) => {
                self.current = ctx.menu.chosen_item;
                self.chosen(slot, ctx)
            }
            (After::List, _) => self.menu(ctx),
            (After::HowMany, _) => {
                self.phase = Phase::Child(After::Quantity);
                let held = self.inventory(ctx).items[self.slot as usize].quantity;
                Transition::Push(Mode::QuantityMenu(QuantityMenu::new(held, None)))
            }
            (After::Quantity, Outcome::Chosen(quantity)) => {
                self.quantity = quantity;
                self.move_item(ctx)
            }
            (After::Quantity, _) | (After::ToList, _) => self.list_loop(ctx),
            (After::AskToss, _) => {
                self.phase = Phase::Child(After::ConfirmToss);
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
            }
            (After::ConfirmToss, Outcome::Chosen(0)) => {
                self.current = 0;
                self.remove_from_pc(ctx);
                self.text("_ThrewAwayItemText", After::ToList, ctx)
            }
            (After::ConfirmToss, _) => {
                self.current = 1;
                self.list_loop(ctx)
            }
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

impl PlayerPc {
    /// `ExitPlayerPC`: the sound first when the PC was turned on directly.
    fn log_off(&mut self, ctx: &mut Ctx) -> Transition {
        if self.generic {
            return self.exit(ctx);
        }
        ctx.audio.play_sound(sounds::SFX_TURN_OFF_PC);
        self.wait_for_sound(Next::TurnedOff, ctx)
    }
}
