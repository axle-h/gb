//! `CinnabarLabFossilRoom_Script`: the scientist who revives a fossil, and the one who trades for a
//! Seel.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GAVE_FOSSIL_TO_LAB, EVENT_LAB_HANDING_OVER_FOSSIL_MON,
    EVENT_LAB_STILL_REVIVING_FOSSIL};
use poke_core::symbols::pokered_local_labels::CinnabarLabFossilRoomScientist1Text as scientist;
use poke_core::symbols::pokered_map_scripts::{TEXT_CINNABARLABFOSSILROOM_SCIENTIST1,
    TEXT_CINNABARLABFOSSILROOM_SCIENTIST2};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `TRADE_FOR_SAILOR`.
const TRADE_FOR_SAILOR: u8 = 3;
/// `FossilsList`, in the order the menu offers them.
const FOSSILS: [ItemId; 3] = [ItemId::DomeFossil, ItemId::HelixFossil, ItemId::OldAmber];
/// `ld c, 30`: whatever the fossil was, it comes back at level 30.
const REVIVED_LEVEL: u8 = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Offered,
    BackToLife,
    MonGiven,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_CINNABARLABFOSSILROOM_SCIENTIST1 => scientist1(rt),
        TEXT_CINNABARLABFOSSILROOM_SCIENTIST2 => rt.do_in_game_trade_dialogue(TRADE_FOR_SAILOR).ret(),
        _ => return None,
    })
}

fn scientist1(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_GAVE_FOSSIL_TO_LAB) {
        return rt.print_text(text_at(scientist::Text)).then(Label::Offered);
    }
    if rt.check_event(EVENT_LAB_STILL_REVIVING_FOSSIL) {
        return rt.print_text(text_at(scientist::GoForAWalkText)).ret();
    }
    rt.load_fossil_item_and_mon_name();
    rt.print_text(text_at(scientist::FossilIsBackToLifeText)).then(Label::BackToLife)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        // `Lab4Script_GetFossilsInBag`.
        Label::Offered => {
            let fossils: Vec<ItemId> = FOSSILS.into_iter().filter(|&item| rt.is_item_in_bag(item)).collect();
            if fossils.is_empty() {
                return rt.print_text(text_at(scientist::NoFossilsText)).ret();
            }
            rt.give_fossil_to_cinnabar_lab(fossils).ret()
        }
        Label::BackToLife => {
            rt.set_event(EVENT_LAB_HANDING_OVER_FOSSIL_MON);
            let Some(mon) = rt.fossil_mon() else { return Flow::Return };
            rt.give_pokemon(mon, REVIVED_LEVEL).then(Label::MonGiven)
        }
        // `GivePokemon`'s carry: a mon sent to the box is handed over as surely as one in the party,
        // and only a full box leaves all three events set for him to hold it until there is room.
        Label::MonGiven => {
            if rt.gave_pokemon() {
                rt.reset_event(EVENT_GAVE_FOSSIL_TO_LAB);
                rt.reset_event(EVENT_LAB_STILL_REVIVING_FOSSIL);
                rt.reset_event(EVENT_LAB_HANDING_OVER_FOSSIL_MON);
            }
            Flow::Return
        }
    }
}
