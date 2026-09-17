//! `RocketHideoutB1F_Script`: five Rockets, and the door the last of them opens.

use poke_core::symbols::pokered_events::{EVENT_BEAT_ROCKET_HIDEOUT_1_TRAINER_4, EVENT_ENTERED_ROCKET_HIDEOUT};
use poke_core::symbols::pokered_map_scripts::{TEXT_ROCKETHIDEOUTB1F_ROCKET1, TEXT_ROCKETHIDEOUTB1F_ROCKET2,
    TEXT_ROCKETHIDEOUTB1F_ROCKET3, TEXT_ROCKETHIDEOUTB1F_ROCKET4, TEXT_ROCKETHIDEOUTB1F_ROCKET5};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use super::{Flow, Script};

/// The door the fifth Rocket guards, and the floor it becomes.
const DOOR_BLOCK: u8 = 0x54;
const FLOOR_BLOCK: u8 = 0x0E;
const DOOR_AT: (u8, u8) = (12, 8);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRocketHideoutB1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRocketHideoutB1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    door_callback(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rocket_hideout_b1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::RocketHideout1TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `RocketHideoutB1FDoorCallbackScript`. The branch that opens the door checks
/// `EVENT_ENTERED_ROCKET_HIDEOUT` where it means to set it, and nothing else in the game sets it,
/// so the door's sound plays on every load of the floor once the Rocket is beaten.
fn door_callback(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let block = match rt.check_event(EVENT_ENTERED_ROCKET_HIDEOUT) {
        true => FLOOR_BLOCK,
        false if rt.check_event(EVENT_BEAT_ROCKET_HIDEOUT_1_TRAINER_4) => {
            rt.play_sound(sounds::SFX_GO_INSIDE);
            FLOOR_BLOCK
        }
        false => DOOR_BLOCK,
    };
    rt.replace_tile_block(DOOR_AT.0, DOOR_AT.1, block);
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROCKETHIDEOUTB1F_ROCKET1 => sym::RocketHideout1TrainerHeader0,
        TEXT_ROCKETHIDEOUTB1F_ROCKET2 => sym::RocketHideout1TrainerHeader1,
        TEXT_ROCKETHIDEOUTB1F_ROCKET3 => sym::RocketHideout1TrainerHeader2,
        TEXT_ROCKETHIDEOUTB1F_ROCKET4 => sym::RocketHideout1TrainerHeader3,
        TEXT_ROCKETHIDEOUTB1F_ROCKET5 => sym::RocketHideout1TrainerHeader4,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rocket_hideout_b1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
