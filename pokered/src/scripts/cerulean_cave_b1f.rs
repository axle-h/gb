//! `CeruleanCaveB1F_Script`: Mewtwo, and two item balls `PickUpItemText` takes care of.

use poke_core::symbols::pokered_map_scripts::TEXT_CERULEANCAVEB1F_MEWTWO;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCeruleanCaveB1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wCeruleanCaveB1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().cerulean_cave_b1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::CeruleanCaveB1FTrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CERULEANCAVEB1F_MEWTWO {
        return None;
    }
    // Mewtwo's object carries a species and a level rather than a trainer, so its header's zero
    // opponent starts a wild battle where a trainer's would start a trainer one.
    Some(rt.talk_to_trainer(sym::MewtwoTrainerHeader).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().cerulean_cave_b1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
