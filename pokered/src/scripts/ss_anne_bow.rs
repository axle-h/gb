//! `SSAnneBow_Script`: two Sailors on the deck.

use poke_core::symbols::pokered_map_scripts::{TEXT_SSANNEBOW_SAILOR2, TEXT_SSANNEBOW_SAILOR3};
use poke_core::symbols::pokered_symbols::{SSAnne5TrainerHeader0, SSAnne5TrainerHeader1, SSAnne5TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSSAnneBowCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSSAnneBowCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().ss_anne_bow.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, SSAnne5TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SSANNEBOW_SAILOR2 => SSAnne5TrainerHeader0,
        TEXT_SSANNEBOW_SAILOR3 => SSAnne5TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().ss_anne_bow.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
