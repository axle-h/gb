//! **Phase 0, tasks 0.5 / 0.6** — item PC storage: deposit and withdraw.
//!
//! Not a workstream, but the thing every workstream is waiting on. The save arrives at the postgame
//! with a **20/20 bag**, so until items can be banked the agent physically cannot pick up HM02, a
//! fishing rod, or the Itemfinder. See `docs/postgame-coverage-plan.md` §2.
//!
//! # The menu chain
//!
//! Both operations share one state machine — deposit and withdraw are the same screens with two
//! indices swapped — driven by the same press/release "mashing" as [`AgentState::TossingItem`]:
//! press for one agent tick, release the next, so each input is a fresh rising edge.
//!
//! ```text
//! overworld            walk below the PC, face UP, press A     (pc.asm ActivatePC)
//!   → "<PLAYER> turned on the PC."                             mash A
//!   → PC parent menu                                           cursor → 1, A
//!         BILL's PC / <PLAYER>'s PC / PROF.OAK's PC / <PKMN>LEAGUE / LOG OFF
//!   → "Accessed my PC. Accessed Item Storage System."          mash A
//!   → player's PC menu                                         cursor → 0 or 1, A
//!         WITHDRAW ITEM / DEPOSIT ITEM / TOSS ITEM / LOG OFF
//!   → "What do you want to deposit?" + item list               cursor → the item's row, A
//!   → "How many?"                                              cursor → qty, A
//!   → "<ITEM> was stored via PC."                              mash A → back to the item list
//!   → B until the overworld returns
//! ```
//!
//! # Two indices that are safe, and two that are not
//!
//! `DisplayPCMainMenu` (`engine/pokemon/bills_pc.asm`) builds the parent menu **conditionally**:
//! `PROF.OAK's PC` appears only with `EVENT_GOT_POKEDEX`, `<PKMN>LEAGUE` only with
//! `wNumHoFTeams != 0`. So the menu is 3, 4 or 5 entries depending on progress and `LOG OFF`'s index
//! moves. But the first two entries are unconditional, so **`BILL's PC` is always 0 and the player's
//! item PC is always 1** — which is what this driver relies on. Nothing here indexes from the end.
//!
//! The two menus are also indistinguishable by geometry: `DisplayPCMainMenu` and `PlayerPCMenu` both
//! set `wTopMenuItemY = 2`, `wTopMenuItemX = 1`. They are told apart by their **text**, per the
//! standing rule that menus are detected by what is on screen rather than where it is.

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::menu::TextBoxId;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

/// Which way an item is moving between the bag and PC item storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcItemOp {
    /// Bag → PC storage (`wNumBagItems` → `wNumBoxItems`). Frees a bag slot.
    Deposit,
    /// PC storage → bag.
    Withdraw,
}

impl PcItemOp {
    /// Index of this operation in the player's PC menu: `WITHDRAW ITEM` 0, `DEPOSIT ITEM` 1,
    /// `TOSS ITEM` 2, `LOG OFF` 3 (`engine/menus/players_pc.asm:243`).
    fn menu_index(self) -> u8 {
        match self { Self::Withdraw => 0, Self::Deposit => 1 }
    }

    /// Where the item comes *from* — the inventory the on-screen list is showing.
    fn source_quantity(self, api: &PokemonApi<'_>, item: ItemId) -> u8 {
        match self {
            Self::Deposit => api.bag_item_quantity(item),
            Self::Withdraw => api.pc_box_item_quantity(item),
        }
    }

    fn source_position(self, api: &PokemonApi<'_>, item: ItemId) -> Option<u8> {
        match self {
            Self::Deposit => api.bag_item_position(item),
            Self::Withdraw => api.pc_box_item_position(item),
        }
    }
}

/// Live state of an in-progress deposit/withdraw. Carried in [`AgentState::UsingItemPc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPcState {
    pub op: PcItemOp,
    pub item: ItemId,
    pub qty: u8,
    /// Coordinate of the PC hidden object, from `MetaTileMap::pc_locations`.
    pub pc: Point8,
    /// Quantity held in the source inventory when the operation began — the baseline completion is
    /// measured against, so a partial move (`qty` < stack) is detected as precisely as a whole one.
    pub start_qty: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    /// Set once the PC menu has been opened, i.e. we have left the overworld at least once.
    pub entered_menu: bool,
}

impl ItemPcState {
    /// ⚠️ `qty` is **clamped to `start_qty`**, and that is not tidiness — it is the difference between
    /// working and hanging. `DisplayChooseQuantityMenu` wraps at `wMaxItemQuantity`, which the item
    /// list sets to the size of the stack (`home/list_menu.asm:.incrementQuantity`), so a target above
    /// what is actually held is **unrepresentable**: the driver presses Up for ever watching the
    /// counter cycle 1…n…1 past a number it can never see. Clamping turns "deposit more than I have"
    /// into "deposit all of it", which is what every caller means, and lets a caller pass a
    /// deliberately large `qty` for "the whole stack" without knowing the count. Found by G7, whose
    /// nine Great Balls had quietly become eight — see the plan's §11.
    pub fn new(op: PcItemOp, item: ItemId, qty: u8, pc: Point8, start_qty: u8) -> Self {
        Self { op, item, qty: qty.min(start_qty), pc, start_qty, press: true, entered_menu: false }
    }
}

/// One agent tick of the deposit/withdraw driver. Called from `agent.rs` via a single delegating
/// match arm.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: ItemPcState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let moved = s.start_qty.saturating_sub(s.op.source_quantity(api, s.item));

    // ── Done: the requested quantity has left the source inventory ──────────────────────────────
    if s.entered_menu && moved >= s.qty {
        if game_mode != GameMode::Overworld {
            // Gen 1 drops back to the item list after each transfer, so back out with B.
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::B); }
            agent.set_state(AgentState::UsingItemPc(ItemPcState { press: !s.press, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox {
            message: format!("{:?} {} x{} via the PC", s.op, s.item, s.qty),
        });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Back in the overworld with nothing moved — the attempt fizzled (e.g. "You have nothing to
    //    deposit."). Drop to Idle so the policy re-issues it and the chain restarts cleanly. ───────
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Still outside: walk to the tile below the PC and face up, then press A ───────────────────
    if game_mode == GameMode::Overworld {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.pc, Some(PlayerFacingDirection::Up)).as_deref() {
            Some([]) => {
                // In position and facing the PC — mash A to turn it on.
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::UsingItemPc(ItemPcState { press: !s.press, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::UsingItemPc(ItemPcState { press: true, ..s }));
            }
            _ => {
                agent.event(AgentEvent::TextBox { message: format!("Can't reach the PC at {}", s.pc) });
                api.release_all_buttons();
                agent.set_state(AgentState::Idle);
            }
        }
        return Ok(());
    }

    // ── Inside the menus ────────────────────────────────────────────────────────────────────────
    let s = ItemPcState { entered_menu: true, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingItemPc(ItemPcState { press: true, ..s }));
        return Ok(());
    }

    let (_, _, current, scroll) = api.menu_geometry();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let text = api.on_screen_text(false).unwrap_or_default();
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    let button = if text.contains("How many") {
        // Quantity selector: Up/Down adjust `wItemQuantity`, A confirms. Reading the live value
        // rather than counting presses keeps this correct if an input is dropped.
        let shown = api.mart_item_quantity();
        // ⚠️ Clamp the target to what the source inventory holds **right now**, not to `qty`. The
        // selector wraps at `wMaxItemQuantity`, which the list menu sets to the live stack size, so a
        // target above it is unrepresentable and the driver would press Up for ever watching the
        // counter cycle past a number it can never reach. The stack can shrink after the step began —
        // G7 watched nine Great Balls become eight between `ItemPcState::new` and this menu — and
        // completion is measured against `start_qty`, so moving the smaller amount still finishes.
        let want = s.qty.min(s.op.source_quantity(api, s.item)).max(1);
        if shown < want { JoypadButton::Up }
        else if shown > want { JoypadButton::Down }
        else { JoypadButton::A }
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        // The item list — the bag when depositing, PC storage when withdrawing.
        match s.op.source_position(api, s.item) {
            Some(target) => nav(current + scroll, target),
            None => JoypadButton::B,
        }
    } else if text.contains("DEPOSIT ITEM") {
        nav(current, s.op.menu_index()) // player's PC menu
    } else if text.contains("LOG OFF") {
        nav(current, 1) // PC parent menu → <PLAYER>'s PC (always index 1)
    } else {
        JoypadButton::A // transitional text: "turned on the PC", "Accessed my PC", "was stored…"
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingItemPc(ItemPcState { press: false, ..s }));
    Ok(())
}
