//! `PokemonMansion2F_Script`: the second floor's switch, its three walls and the Super Nerd.

use poke_core::symbols::pokered_events::EVENT_MANSION_SWITCH_ON;
use poke_core::symbols::pokered_local_labels::PokemonMansion2FSwitchText as switch;
use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONMANSION2F_SUPER_NERD, TEXT_POKEMONMANSION2F_SWITCH};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::pokemon_mansion_1f::{switch_answered, switch_done, switch_text};
use super::{Flow, Script};

/// `Mansion2CheckReplaceSwitchDoorBlocks`: each wall in blocks, the block it is while the switch is
/// off and the block it is once it is on. Only the first of the three is a wall the switch closes.
const GATES: [(u8, u8, u8, u8); 3] = [(4, 2, 0x0E, 0x5F), (9, 4, 0x54, 0x0E), (3, 11, 0x5F, 0x0E)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonMansion2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonMansion2FCurScript], a` after the table's routine.
    StoreCurScript,
    SwitchAsk,
    SwitchAnswered,
    SwitchDone,
}

pub fn script(rt: &mut Script) -> Flow {
    check_replace_switch_door_blocks(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_mansion_2f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Mansion2TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `Mansion2CheckReplaceSwitchDoorBlocks`.
fn check_replace_switch_door_blocks(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let on = rt.check_event(EVENT_MANSION_SWITCH_ON);
    for (x, y, off, switched) in GATES {
        rt.replace_tile_block(x, y, if on { switched } else { off });
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_POKEMONMANSION2F_SUPER_NERD => Some(rt.talk_to_trainer(sym::Mansion2TrainerHeader0).ret()),
        // The words every floor's switch prints are this floor's.
        TEXT_POKEMONMANSION2F_SWITCH => Some(switch_text(rt, switch::Text, Label::SwitchAsk)),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_mansion_2f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::SwitchAsk => rt.yes_no_choice().then(Label::SwitchAnswered),
        Label::SwitchAnswered => switch_answered(rt, switch::PressedText, switch::NotPressed, Label::SwitchDone),
        Label::SwitchDone => switch_done(rt),
    }
}
