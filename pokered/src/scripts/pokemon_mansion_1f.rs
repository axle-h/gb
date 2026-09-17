//! `PokemonMansion1F_Script`: the switch on the ground floor, which opens one wall and shuts three.

use poke_core::symbols::DmgPointer;
use poke_core::symbols::pokered_events::EVENT_MANSION_SWITCH_ON;
use poke_core::symbols::pokered_local_labels::PokemonMansion1FSwitchText as switch;
use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONMANSION1F_SCIENTIST, TEXT_POKEMONMANSION1F_SWITCH};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use super::{text_at, Code, Flow, Script};

/// `HORIZONTAL_GATE_BLOCK` and the plain floor that replaces it.
const HORIZONTAL_GATE: u8 = 0x2D;
const EMPTY_FLOOR: u8 = 0x0E;

/// `Mansion1CheckReplaceSwitchDoorBlocks`, in blocks. The first wall is the one the switch opens
/// when it is off; the other three it opens when it is on.
const GATES: [(u8, u8); 4] = [(12, 6), (8, 3), (10, 8), (13, 13)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonMansion1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonMansion1FCurScript], a` after the table's routine.
    StoreCurScript,
    SwitchAsk,
    SwitchAnswered,
    SwitchDone,
}

pub fn script(rt: &mut Script) -> Flow {
    check_replace_switch_door_blocks(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_mansion_1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Mansion1TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `Mansion1CheckReplaceSwitchDoorBlocks`.
fn check_replace_switch_door_blocks(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let on = rt.check_event(EVENT_MANSION_SWITCH_ON);
    for (index, &(x, y)) in GATES.iter().enumerate() {
        let shut = on == (index == 0);
        rt.replace_tile_block(x, y, if shut { HORIZONTAL_GATE } else { EMPTY_FLOOR });
    }
}

/// `PokemonMansion1FSwitchText`, which every floor's switch is: the same routine over its own three
/// texts, and one event that every floor's walls read.
pub fn switch_text(rt: &mut Script, ask: DmgPointer, answered: impl Into<Code>) -> Flow {
    rt.print_text(text_at(ask)).then(answered)
}

/// The half after the yes/no. A press sets `BIT_CUR_MAP_LOADED_1`, so the next pass redraws the
/// walls of whichever floor the player is on.
pub fn switch_answered(rt: &mut Script, pressed: DmgPointer, not_pressed: DmgPointer, done: impl Into<Code>) -> Flow {
    if !rt.chose_yes() {
        return rt.print_text(text_at(not_pressed)).ret();
    }
    rt.set_do_not_wait_for_button_press(true);
    rt.set_cur_map_loaded(1);
    rt.print_text(text_at(pressed)).then(done)
}

/// The switch toggles its one event rather than setting it, which is what makes it a switch.
pub fn switch_done(rt: &mut Script) -> Flow {
    rt.play_sound(sounds::SFX_GO_INSIDE);
    if rt.check_and_set_event(EVENT_MANSION_SWITCH_ON) {
        rt.reset_event(EVENT_MANSION_SWITCH_ON);
    }
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_POKEMONMANSION1F_SCIENTIST => Some(rt.talk_to_trainer(sym::Mansion1TrainerHeader0).ret()),
        TEXT_POKEMONMANSION1F_SWITCH => Some(switch_text(rt, switch::Text, Label::SwitchAsk)),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_mansion_1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::SwitchAsk => rt.yes_no_choice().then(Label::SwitchAnswered),
        Label::SwitchAnswered => switch_answered(rt, switch::PressedText, switch::NotPressedText, Label::SwitchDone),
        Label::SwitchDone => switch_done(rt),
    }
}
