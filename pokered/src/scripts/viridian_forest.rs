//! `ViridianForest_Script`: three Bug Catchers, and the item balls `PickUpItemText` takes care of.

use poke_core::symbols::pokered_map_scripts::{TEXT_VIRIDIANFOREST_YOUNGSTER2, TEXT_VIRIDIANFOREST_YOUNGSTER3,
    TEXT_VIRIDIANFOREST_YOUNGSTER4};
use poke_core::symbols::pokered_symbols::{ViridianForestTrainerHeader0, ViridianForestTrainerHeader1,
    ViridianForestTrainerHeader2, ViridianForestTrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wViridianForestCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wViridianForestCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().viridian_forest.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, ViridianForestTrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_VIRIDIANFOREST_YOUNGSTER2 => ViridianForestTrainerHeader0,
        TEXT_VIRIDIANFOREST_YOUNGSTER3 => ViridianForestTrainerHeader1,
        TEXT_VIRIDIANFOREST_YOUNGSTER4 => ViridianForestTrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().viridian_forest.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
