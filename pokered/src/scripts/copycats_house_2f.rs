//! `CopycatsHouse2F_Script`: the Copycat, who swaps TM31 for a Poké Doll, and the PC she keeps her
//! secrets on.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_TM31;
use poke_core::symbols::pokered_local_labels::{CopycatsHouse2FCopycatText as copycat, CopycatsHouse2FPCText as pc};
use poke_core::symbols::pokered_map_scripts::{TEXT_COPYCATSHOUSE2F_COPYCAT, TEXT_COPYCATSHOUSE2F_PC};
use serde::{Deserialize, Serialize};
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `CopycatsHouse2FCopycatText` after each of its three prints.
    Mimicked,
    DollTaken,
    ReceivedTm31,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_COPYCATSHOUSE2F_COPYCAT => {
            if rt.check_event(EVENT_GOT_TM31) {
                return Some(rt.print_text(text_at(copycat::TM31Explanation2Text)).ret());
            }
            // She mimics the player's own greeting, and whether she has anything to add is not
            // asked until it is over, so the button press at the end of it is dropped.
            rt.set_do_not_wait_for_button_press(true);
            rt.print_text(text_at(copycat::DoYouLikePokemonText)).then(Label::Mimicked)
        }
        // Her PC faces the wall: a player standing beside it cannot read what is on the screen.
        TEXT_COPYCATSHOUSE2F_PC => {
            let words = match rt.player_facing() == SPRITE_FACING_UP {
                true => pc::MySecretsText,
                false => pc::CantSeeText,
            };
            rt.print_text(text_at(words)).ret()
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Mimicked => {
            if !rt.is_item_in_bag(ItemId::PokeDoll) {
                return Flow::Return;
            }
            rt.print_text(text_at(copycat::TM31PreReceiveText)).then(Label::DollTaken)
        }
        Label::DollTaken => {
            if !rt.give_item(ItemId::Tm31Mimic, 1) {
                return rt.print_text(text_at(copycat::TM31NoRoomText)).ret();
            }
            // `.ReceivedTM31Text` runs on into `.TM31Explanation1Text`, so what she says about
            // Mimic is part of the same print.
            rt.print_text(text_at(copycat::ReceivedTM31Text)).then(Label::ReceivedTm31)
        }
        Label::ReceivedTm31 => {
            rt.remove_item(ItemId::PokeDoll, 1);
            rt.set_event(EVENT_GOT_TM31);
            Flow::Return
        }
    }
}
