//! `ViridianMart_Script`: the clerk stops a player who has not yet taken Oak his parcel and hands it
//! over, and once it is delivered the map reads its second text pointer table.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GOT_OAKS_PARCEL, EVENT_OAK_GOT_PARCEL};
use poke_core::symbols::pokered_local_labels::ViridianMartDefaultScript;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{ViridianMart_TextPointers, ViridianMart_TextPointers2};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wViridianMartCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ViridianMartDefaultScript` after its `DisplayTextID`.
    DefaultScriptWalk,
    /// `ViridianMartOaksParcelScript` after its `Delay3`, and after its `DisplayTextID`.
    OaksParcelText,
    OaksParcelGive,
}

pub fn script(rt: &mut Script) -> Flow {
    // `ViridianMartCheckParcelDeliveredScript`.
    let table = if rt.check_event(EVENT_OAK_GOT_PARCEL) { ViridianMart_TextPointers2 } else { ViridianMart_TextPointers };
    rt.set_text_pointers(table);
    rt.enable_auto_text_box_drawing();
    match rt.maps().viridian_mart.cur_script {
        SCRIPT_VIRIDIANMART_DEFAULT => {
            rt.update_sprites();
            rt.display_text_id(TEXT_VIRIDIANMART_CLERK_YOU_CAME_FROM_PALLET_TOWN).then(Label::DefaultScriptWalk)
        }
        SCRIPT_VIRIDIANMART_OAKS_PARCEL => {
            if rt.simulated_joypad_states_index() != 0 {
                return Flow::Return;
            }
            rt.delay3().then(Label::OaksParcelText)
        }
        _ => Flow::Return,
    }
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScriptWalk => {
            rt.simulate_joypad_rle(ViridianMartDefaultScript::PlayerMovement);
            rt.maps().viridian_mart.cur_script = SCRIPT_VIRIDIANMART_OAKS_PARCEL;
            Flow::Return
        }
        Label::OaksParcelText => rt.display_text_id(TEXT_VIRIDIANMART_CLERK_PARCEL_QUEST).then(Label::OaksParcelGive),
        Label::OaksParcelGive => {
            rt.give_item(ItemId::OaksParcel, 1);
            rt.set_event(EVENT_GOT_OAKS_PARCEL);
            rt.maps().viridian_mart.cur_script = SCRIPT_VIRIDIANMART_NOOP;
            Flow::Return
        }
    }
}
