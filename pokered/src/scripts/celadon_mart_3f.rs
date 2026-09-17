//! `CeladonMart3F_Script`: the clerk on the toy floor, who hands over TM18.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_TM18;
use poke_core::symbols::pokered_local_labels::CeladonMart3FClerkText as clerk;
use poke_core::symbols::pokered_map_scripts::TEXT_CELADONMART3F_CLERK;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Tm18Offered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CELADONMART3F_CLERK {
        return None;
    }
    Some(match rt.check_event(EVENT_GOT_TM18) {
        true => rt.print_text(text_at(clerk::TM18ExplanationText)).ret(),
        false => rt.print_text(text_at(clerk::TM18PreReceiveText)).then(Label::Tm18Offered),
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Tm18Offered => {
            if !rt.give_item(ItemId::Tm18Counter, 1) {
                return rt.print_text(text_at(clerk::TM18NoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_TM18);
            rt.print_text(text_at(clerk::ReceivedTM18Text)).ret()
        }
    }
}
