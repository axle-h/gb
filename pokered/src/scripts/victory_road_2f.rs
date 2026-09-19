//! `VictoryRoad2F_Script`: two switches, each with a gate of its own, Moltres, and the five
//! trainers between them.

use poke_core::symbols::pokered_events::{EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH,
    EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VICTORYROAD2F_DEFAULT, TEXT_VICTORYROAD2F_COOLTRAINER_M,
    TEXT_VICTORYROAD2F_HIKER, TEXT_VICTORYROAD2F_MOLTRES, TEXT_VICTORYROAD2F_SUPER_NERD1,
    TEXT_VICTORYROAD2F_SUPER_NERD2, TEXT_VICTORYROAD2F_SUPER_NERD3};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use poke_core::species::PokemonSpecies;
use super::{text_at, Code, Flow, Routine, Script};

/// `.SwitchCoords`, as (x, y), and the block and place of the gate each one opens.
const SWITCHES: [(u8, u8); 2] = [(1, 16), (9, 16)];
const GATES: [(u8, (u8, u8)); 2] = [(0x15, (3, 4)), (0x1D, (11, 7))];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wVictoryRoad2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    /// `ld [wVictoryRoad2FCurScript], a` after the table's routine.
    StoreCurScript,
    /// `VictoryRoad2FMoltresBattleText`, whose `text_asm` plays Moltres's cry before the battle.
    MoltresBattleText,
    MoltresCry,
}

pub fn script(rt: &mut Script) -> Flow {
    // `VictoryRoad2FResetBoulderEventScript` falls through into the check: coming back up the steps
    // is what shuts the floor below's gate again, since its boulder is left where this floor's are.
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.reset_event(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH);
        open_gates(rt);
    }
    if rt.check_and_reset_cur_map_loaded(1) {
        open_gates(rt);
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().victory_road_2f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::VictoryRoad2TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_VICTORYROAD2F_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `VictoryRoad2FCheckBoulderEventScript`: each gate whose switch is held down is drawn open again.
fn open_gates(rt: &mut Script) {
    for (event, (block, at)) in [EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2]
        .into_iter().zip(GATES)
    {
        if rt.check_event(event) {
            rt.replace_tile_block(at.0, at.1, block);
        }
    }
}

/// `VictoryRoad2FDefaultScript`: the switch a boulder now stands on, if it is a new one, marks the
/// map loaded again so the script above opens its gate.
fn default_script(rt: &mut Script) -> Flow {
    let Some(switch) = rt.check_boulder_coords(&SWITCHES) else {
        return rt.trainer_script(SCRIPT_VICTORYROAD2F_DEFAULT).ret();
    };
    let event = match switch {
        0 => EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1,
        _ => EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2,
    };
    if rt.check_and_set_event(event) {
        return Flow::Return;
    }
    rt.set_cur_map_loaded(1);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_VICTORYROAD2F_HIKER => sym::VictoryRoad2TrainerHeader0,
        TEXT_VICTORYROAD2F_SUPER_NERD1 => sym::VictoryRoad2TrainerHeader1,
        TEXT_VICTORYROAD2F_COOLTRAINER_M => sym::VictoryRoad2TrainerHeader2,
        TEXT_VICTORYROAD2F_SUPER_NERD2 => sym::VictoryRoad2TrainerHeader3,
        TEXT_VICTORYROAD2F_SUPER_NERD3 => sym::VictoryRoad2TrainerHeader4,
        // Moltres' object carries a species and a level rather than a trainer, so its header's zero
        // opponent starts a wild battle where a trainer's would start a trainer one.
        TEXT_VICTORYROAD2F_MOLTRES => {
            let before = Some(Label::MoltresBattleText.into());
            return Some(rt.talk_to_trainer_asm(sym::MoltresTrainerHeader, before, None).ret());
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().victory_road_2f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::MoltresBattleText => rt.print_text(text_at(sym::VictoryRoad2FMoltresBattleText)).then(Label::MoltresCry),
        Label::MoltresCry => {
            rt.play_cry(PokemonSpecies::Moltres);
            rt.wait_for_sound_to_finish().then(Routine::TalkToTrainerNotYetFought)
        }
    }
}
