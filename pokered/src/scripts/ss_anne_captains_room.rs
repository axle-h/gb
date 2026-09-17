//! `SSAnneCaptainsRoom_Script`: the seasick captain, whose back is rubbed for HM01 Cut.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GOT_HM01, EVENT_RUBBED_CAPTAINS_BACK};
use poke_core::symbols::pokered_map_scripts::TEXT_SSANNECAPTAINSROOM_CAPTAIN;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `SSAnneCaptainsRoomRubCaptainsBackText`'s own `text_asm`: the jingle, and the wait for it.
    RubbedBack,
    Healed,
    MusicBack,
    FeelMuchBetter,
}

/// `SSAnneCaptainsRoomEventScript`: a captain who is still being sick does not look up.
pub fn script(rt: &mut Script) -> Flow {
    rt.set_no_npc_face_player(!rt.check_event(EVENT_RUBBED_CAPTAINS_BACK));
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_SSANNECAPTAINSROOM_CAPTAIN {
        return None;
    }
    if rt.check_event(EVENT_GOT_HM01) {
        return Some(rt.print_text(text_at(sym::SSAnneCaptainsRoomCaptainNotSickAnymoreText)).ret());
    }
    Some(rt.print_text(text_at(sym::SSAnneCaptainsRoomRubCaptainsBackText)).then(Label::RubbedBack))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::RubbedBack => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.play_music(sounds::MUSIC_PKMN_HEALED);
            rt.wait_for_sound_to_finish().then(Label::Healed)
        }
        Label::Healed => {
            rt.set_event(EVENT_RUBBED_CAPTAINS_BACK);
            rt.set_no_npc_face_player(false);
            rt.play_default_music().then(Label::MusicBack)
        }
        Label::MusicBack => {
            rt.print_text(text_at(sym::SSAnneCaptainsRoomCaptainIFeelMuchBetterText)).then(Label::FeelMuchBetter)
        }
        // A bag with no room leaves him facing away again, so the offer can be taken up later.
        Label::FeelMuchBetter => {
            if !rt.give_item(ItemId::Hm01Cut, 1) {
                rt.set_no_npc_face_player(true);
                return rt.print_text(text_at(sym::SSAnneCaptainsRoomCaptainHM01NoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_HM01);
            rt.print_text(text_at(sym::SSAnneCaptainsRoomCaptainReceivedHM01Text)).ret()
        }
    }
}
