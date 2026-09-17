//! `CeladonDiner_Script`: the man at the counter, who has lost everything at the Game Corner but
//! the Coin Case.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_COIN_CASE;
use poke_core::symbols::pokered_local_labels::CeladonDinerGymGuideText as guide;
use poke_core::symbols::pokered_map_scripts::TEXT_CELADONDINER_GYM_GUIDE;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    CoinCaseOffered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CELADONDINER_GYM_GUIDE {
        return None;
    }
    Some(match rt.check_event(EVENT_GOT_COIN_CASE) {
        true => rt.print_text(text_at(guide::WinItBackText)).ret(),
        false => rt.print_text(text_at(guide::ImFlatOutBustedText)).then(Label::CoinCaseOffered),
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::CoinCaseOffered => {
            if !rt.give_item(ItemId::CoinCase, 1) {
                return rt.print_text(text_at(guide::CoinCaseNoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_COIN_CASE);
            rt.print_text(text_at(guide::ReceivedCoinCaseText)).ret()
        }
    }
}
