//! `PokemonTower5F_Script`: four channelers and the purified zone, the one square of the tower where
//! nothing attacks and the party is healed.

use poke_core::symbols::pokered_events::EVENT_IN_PURIFIED_ZONE;
use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONTOWER5F_CHANNELER2, TEXT_POKEMONTOWER5F_CHANNELER3,
    TEXT_POKEMONTOWER5F_CHANNELER4, TEXT_POKEMONTOWER5F_CHANNELER5, TEXT_POKEMONTOWER5F_PURIFIEDZONE};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{Flow, Script};

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `PokemonTower5FPurifiedZoneCoords`: the four squares of the shrine.
const PURIFIED_ZONE: [(u8, u8); 4] = [(10, 8), (11, 8), (10, 9), (11, 9)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonTower5FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonTower5FCurScript], a` after the table's routine.
    StoreCurScript,
    Faded,
    Waited,
    Healed,
    ZoneExplained,
    ZoneWelcomed,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_tower_5f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PokemonTower5TrainerHeaders);
    match index {
        0 => default_script(rt),
        _ => rt.trainer_script(index).then(Label::StoreCurScript),
    }
}

/// `PokemonTower5FDefaultScript`: the zone's welcome is given once per visit to it, and stepping off
/// arms it again.
fn default_script(rt: &mut Script) -> Flow {
    if rt.are_player_coords_in_array(&PURIFIED_ZONE).is_none() {
        rt.set_no_battles(false);
        rt.reset_event(EVENT_IN_PURIFIED_ZONE);
        return rt.trainer_script(0).then(Label::StoreCurScript);
    }
    if rt.check_and_set_event(EVENT_IN_PURIFIED_ZONE) {
        return Flow::Return;
    }
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_no_battles(true);
    rt.heal_party();
    rt.gb_fade_out_to_white().then(Label::Faded)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_POKEMONTOWER5F_CHANNELER2 => sym::PokemonTower5TrainerHeader0,
        TEXT_POKEMONTOWER5F_CHANNELER3 => sym::PokemonTower5TrainerHeader1,
        TEXT_POKEMONTOWER5F_CHANNELER4 => sym::PokemonTower5TrainerHeader2,
        TEXT_POKEMONTOWER5F_CHANNELER5 => sym::PokemonTower5TrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_tower_5f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::Faded => rt.delay3().then(Label::Waited),
        Label::Waited => rt.delay3().then(Label::Healed),
        Label::Healed => rt.gb_fade_in_from_white().then(Label::ZoneExplained),
        Label::ZoneExplained => rt.display_text_id(TEXT_POKEMONTOWER5F_PURIFIEDZONE).then(Label::ZoneWelcomed),
        Label::ZoneWelcomed => {
            rt.joy_ignore(Joypad::empty());
            Flow::Return
        }
    }
}
