//! `CeruleanTrashedHouse_Script`: the fishing guru whose TM was stolen, who has given up on it once
//! the player holds one.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_local_labels::CeruleanTrashedHouseFishingGuruText as guru;
use poke_core::symbols::pokered_map_scripts::TEXT_CERULEANTRASHEDHOUSE_FISHING_GURU;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// The ROM bank `CeruleanTrashedHouseFishingGuruText` runs in, which `predef` hands back in `a`.
const BANK: u8 = 0x07;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CERULEANTRASHEDHOUSE_FISHING_GURU {
        return None;
    }
    // `and b` tests the quantity against the bank number `predef` restores into `a`, not against
    // itself, so a count that is a multiple of eight reads as none.
    let words = match rt.get_quantity_of_item_in_bag(ItemId::Tm28Dig) & BANK {
        0 => guru::TheyStoleATMText,
        _ => guru::WhatsLostIsLostText,
    };
    Some(rt.print_text(text_at(words)).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
