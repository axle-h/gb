//! `PokemonMansionB1F_Script`: the basement's switch, and the Secret Key behind the wall it opens.

use poke_core::symbols::pokered_events::EVENT_MANSION_SWITCH_ON;
use poke_core::symbols::pokered_local_labels::PokemonMansion2FSwitchText as switch;
use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONMANSIONB1F_BURGLAR, TEXT_POKEMONMANSIONB1F_SCIENTIST,
    TEXT_POKEMONMANSIONB1F_SWITCH};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::pokemon_mansion_1f::{switch_answered, switch_done, switch_text};
use super::{Flow, Script};

/// `MansionB1FCheckReplaceSwitchDoorBlocks`: each wall in blocks, the block it is while the switch
/// is off and the block it is once it is on. Two of the four are walls the switch closes.
const GATES: [(u8, u8, u8, u8); 4] = [(13, 8, 0x0E, 0x2D), (6, 11, 0x0E, 0x5F), (4, 3, 0x5F, 0x0E), (8, 8, 0x54, 0x0E)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonMansionB1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonMansionB1FCurScript], a` after the table's routine.
    StoreCurScript,
    SwitchAsk,
    SwitchAnswered,
    SwitchDone,
}

pub fn script(rt: &mut Script) -> Flow {
    check_replace_switch_door_blocks(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_mansion_b1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Mansion4TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `MansionB1FCheckReplaceSwitchDoorBlocks`.
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
    let header = match text_id {
        TEXT_POKEMONMANSIONB1F_BURGLAR => sym::Mansion4TrainerHeader0,
        TEXT_POKEMONMANSIONB1F_SCIENTIST => sym::Mansion4TrainerHeader1,
        // The basement's switch prints the second floor's words, not its own.
        TEXT_POKEMONMANSIONB1F_SWITCH => return Some(switch_text(rt, switch::Text, Label::SwitchAsk)),
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_mansion_b1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::SwitchAsk => rt.yes_no_choice().then(Label::SwitchAnswered),
        Label::SwitchAnswered => switch_answered(rt, switch::PressedText, switch::NotPressed, Label::SwitchDone),
        Label::SwitchDone => switch_done(rt),
    }
}
