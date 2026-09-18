//! `PokemonMansion3F_Script`: the top floor's switch, the three holes in its south wall and the two
//! trainers.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::EVENT_MANSION_SWITCH_ON;
use poke_core::symbols::pokered_local_labels::PokemonMansion2FSwitchText as switch;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_POKEMONMANSION3F_DEFAULT, TEXT_POKEMONMANSION3F_SCIENTIST,
    TEXT_POKEMONMANSION3F_SUPER_NERD, TEXT_POKEMONMANSION3F_SWITCH};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::pokemon_mansion_1f::{switch_answered, switch_done, switch_text};
use super::{Code, Flow, Script};

/// `Mansion3CheckReplaceSwitchDoorBlocks`: each wall in blocks, the block it is while the switch is
/// off and the block it is once it is on.
const GATES: [(u8, u8, u8, u8); 2] = [(7, 2, 0x0E, 0x5F), (7, 5, 0x5F, 0x0E)];

/// `.holeCoords`, as (x, y).
const HOLES: [(u8, u8); 3] = [(16, 14), (17, 14), (19, 14)];
/// `wWhichDungeonWarp` of the one hole that drops two floors rather than three.
const TO_2F: u8 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonMansion3FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    /// `ld [wPokemonMansion3FCurScript], a` after the table's routine.
    StoreCurScript,
    SwitchAsk,
    SwitchAnswered,
    SwitchDone,
}

pub fn script(rt: &mut Script) -> Flow {
    check_replace_switch_door_blocks(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_mansion_3f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Mansion3TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_POKEMONMANSION3F_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `Mansion3CheckReplaceSwitchDoorBlocks`.
fn check_replace_switch_door_blocks(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let on = rt.check_event(EVENT_MANSION_SWITCH_ON);
    for (x, y, off, switched) in GATES {
        rt.replace_tile_block(x, y, if on { switched } else { off });
    }
}

/// `PokemonMansion3FDefaultScript`, whose `.isPlayerFallingDownHole` takes the fall before the
/// floor's trainers get a look: the rightmost of the three holes lands on the second floor and the
/// other two drop all the way to the ground floor.
fn default_script(rt: &mut Script) -> Flow {
    let Some(which) = rt.are_player_coords_in_array(&HOLES) else {
        return rt.trainer_script(SCRIPT_POKEMONMANSION3F_DEFAULT).ret();
    };
    let below = if which == TO_2F { Map::PokemonMansion2F } else { Map::PokemonMansion1F };
    rt.fall_down_hole(below, which);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_POKEMONMANSION3F_SUPER_NERD => sym::Mansion3TrainerHeader0,
        TEXT_POKEMONMANSION3F_SCIENTIST => sym::Mansion3TrainerHeader1,
        // This floor's switch prints the second floor's words, not its own.
        TEXT_POKEMONMANSION3F_SWITCH => return Some(switch_text(rt, switch::Text, Label::SwitchAsk)),
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().pokemon_mansion_3f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::SwitchAsk => rt.yes_no_choice().then(Label::SwitchAnswered),
        Label::SwitchAnswered => switch_answered(rt, switch::PressedText, switch::NotPressed, Label::SwitchDone),
        Label::SwitchDone => switch_done(rt),
    }
}
