//! `PokemonTower4F_Script`: three channelers, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonTower4FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonTower4FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_tower_4f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PokemonTower4TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_POKEMONTOWER4F_CHANNELER1 => sym::PokemonTower4TrainerHeader0,
        TEXT_POKEMONTOWER4F_CHANNELER2 => sym::PokemonTower4TrainerHeader1,
        TEXT_POKEMONTOWER4F_CHANNELER3 => sym::PokemonTower4TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_tower_4f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
