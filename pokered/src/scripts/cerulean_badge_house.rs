//! `CeruleanBadgeHouse_Script`: the man who explains whichever badge is picked off a list of all
//! eight, until the list is backed out of.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_map_scripts::TEXT_CERULEANBADGEHOUSE_MIDDLE_AGED_MAN;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

/// `.BadgeItemList`.
const BADGE_ITEM_LIST: [ItemId; 8] = [
    ItemId::BoulderBadge, ItemId::CascadeBadge, ItemId::ThunderBadge, ItemId::RainbowBadge,
    ItemId::SoulBadge, ItemId::MarshBadge, ItemId::VolcanoBadge, ItemId::EarthBadge,
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `.loop`, carrying the `wCurrentMenuItem` and `wListScrollOffset` the list reopens at.
    Loop(u8, u8),
    WhichBadgeAsked(u8, u8),
    BadgeChosen,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_CERULEANBADGEHOUSE_MIDDLE_AGED_MAN)
        .then(|| rt.print_text(text_named("CeruleanBadgeHouseMiddleAgedManText.Text")).then(Label::Loop(0, 0)))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Loop(current, scroll) => rt.print_text(text_named("CeruleanBadgeHouseMiddleAgedManText.WhichBadgeText")).then(Label::WhichBadgeAsked(current, scroll)),
        Label::WhichBadgeAsked(current, scroll) => {
            rt.display_special_list_menu(BADGE_ITEM_LIST.to_vec(), current, scroll).then(Label::BadgeChosen)
        }
        Label::BadgeChosen => {
            let (current, scroll) = rt.list_menu_position();
            let Some(badge) = rt.chosen_row() else {
                rt.set_list_scroll_offset(0);
                return rt.print_text(text_named("CeruleanBadgeHouseMiddleAgedManText.VisitAnyTimeText")).ret();
            };
            let words = [
                "CeruleanBadgeHouseBoulderBadgeText", "CeruleanBadgeHouseCascadeBadgeText",
                "CeruleanBadgeHouseThunderBadgeText", "CeruleanBadgeHouseRainbowBadgeText",
                "CeruleanBadgeHouseSoulBadgeText", "CeruleanBadgeHouseMarshBadgeText",
                "CeruleanBadgeHouseVolcanoBadgeText", "CeruleanBadgeHouseEarthBadgeText",
            ][badge as usize];
            rt.print_text(text_named(words)).then(Label::Loop(current, scroll))
        }
    }
}
