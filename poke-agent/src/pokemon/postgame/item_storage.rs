//! Item PC storage, deposit and withdraw: face the PC and press A, pick `<PLAYER>'s PC`, the
//! operation, the item and the quantity, then B until the overworld returns.

use gb::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::menu::TextBoxId;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

/// Which way an item is moving between the bag and PC item storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcItemOp {
    Deposit,
    Withdraw,
}

impl PcItemOp {
    /// Row in the player's PC menu (`engine/menus/players_pc.asm`).
    fn menu_index(self) -> u8 {
        match self { Self::Withdraw => 0, Self::Deposit => 1 }
    }

    /// Held in the inventory the on-screen list shows.
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

/// Live state of an in-progress deposit/withdraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPcState {
    pub op: PcItemOp,
    pub item: ItemId,
    pub qty: u8,
    /// Coordinate of the PC hidden object, from `MetaTileMap::pc_locations`.
    pub pc: Point8,
    /// Held in the source inventory at the start, the baseline a partial move is measured against.
    pub start_qty: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    /// Set once the driver has left the overworld.
    pub entered_menu: bool,
}

impl ItemPcState {
    /// `qty` is clamped to `start_qty`, or the completion test waits for more than exists.
    pub fn new(op: PcItemOp, item: ItemId, qty: u8, pc: Point8, start_qty: u8) -> Self {
        Self { op, item, qty: qty.min(start_qty), pc, start_qty, press: true, entered_menu: false }
    }
}

pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: ItemPcState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let moved = s.start_qty.saturating_sub(s.op.source_quantity(api, s.item));

    // Done: the requested quantity has left the source inventory.
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

    // Back in the overworld with nothing moved: the attempt fizzled.
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    if game_mode == GameMode::Overworld {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.pc, Some(PlayerFacingDirection::Up)).as_deref() {
            Some([]) => {
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
        // Quantity selector: Up/Down adjust `wItemQuantity`, A confirms.
        let shown = api.mart_item_quantity();
        // Clamp the target to what the source inventory holds right now, not to `qty`.
        let want = s.qty.min(s.op.source_quantity(api, s.item)).max(1);
        if shown < want { JoypadButton::Up }
        else if shown > want { JoypadButton::Down }
        else { JoypadButton::A }
    } else if tbid == Some(TextBoxId::ListMenuBox) {
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
