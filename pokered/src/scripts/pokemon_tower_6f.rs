//! `PokemonTower6F_Script`: three channelers and the ghost blocking the stairs, which is a wild
//! Marowak the Silph Scope makes visible. Losing to it or running from it shoves the player back off
//! its square rather than ending the encounter.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_BEAT_GHOST_MAROWAK;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_POKEMONTOWER6F_DEFAULT, SCRIPT_POKEMONTOWER6F_MAROWAK_BATTLE,
    SCRIPT_POKEMONTOWER6F_PLAYER_MOVING, TEXT_POKEMONTOWER6F_BEGONE, TEXT_POKEMONTOWER6F_CHANNELER1,
    TEXT_POKEMONTOWER6F_CHANNELER2, TEXT_POKEMONTOWER6F_CHANNELER3, TEXT_POKEMONTOWER6F_MAROWAK_DEPARTED};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{Flow, Script};

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);

/// `PokemonTower6FMarowakCoords`: the square below the ghost.
const MAROWAK_COORDS: [(u8, u8); 1] = [(10, 16)];
/// `RESTLESS_SOUL` is Marowak's own species constant.
const RESTLESS_SOUL_LEVEL: u8 = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonTower6FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonTower6FCurScript], a` after the table's routine.
    StoreCurScript,
    Challenged,
    Departed,
    MovedBack,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_tower_6f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PokemonTower6TrainerHeaders);
    match index {
        SCRIPT_POKEMONTOWER6F_DEFAULT => default_script(rt),
        SCRIPT_POKEMONTOWER6F_PLAYER_MOVING => player_moving(rt),
        SCRIPT_POKEMONTOWER6F_MAROWAK_BATTLE => marowak_battle(rt),
        _ => rt.trainer_script(index).then(Label::StoreCurScript),
    }
}

/// `PokemonTower6FSetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().pokemon_tower_6f.cur_script = SCRIPT_POKEMONTOWER6F_DEFAULT;
    Flow::Return
}

/// `PokemonTower6FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_BEAT_GHOST_MAROWAK) || rt.are_player_coords_in_array(&MAROWAK_COORDS).is_none() {
        return rt.trainer_script(0).then(Label::StoreCurScript);
    }
    rt.clear_joy_held();
    rt.display_text_id(TEXT_POKEMONTOWER6F_BEGONE).then(Label::Challenged)
}

/// `PokemonTower6FMarowakBattleScript`.
fn marowak_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return set_default_script(rt);
    }
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    if rt.talked_to_trainer() {
        return Flow::Return;
    }
    rt.update_sprites();
    rt.joy_ignore(PAD_CTRL_PAD);
    if rt.battle_result() != 0 {
        rt.simulate_joypad_presses(vec![Joypad::RIGHT]);
        rt.maps().pokemon_tower_6f.cur_script = SCRIPT_POKEMONTOWER6F_PLAYER_MOVING;
        return Flow::Return;
    }
    rt.set_event(EVENT_BEAT_GHOST_MAROWAK);
    rt.display_text_id(TEXT_POKEMONTOWER6F_MAROWAK_DEPARTED).then(Label::Departed)
}

/// `PokemonTower6FPlayerMovingScript`.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::MovedBack)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_POKEMONTOWER6F_CHANNELER1 => sym::PokemonTower6TrainerHeader0,
        TEXT_POKEMONTOWER6F_CHANNELER2 => sym::PokemonTower6TrainerHeader1,
        TEXT_POKEMONTOWER6F_CHANNELER3 => sym::PokemonTower6TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_tower_6f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::Challenged => {
            rt.start_wild_battle(PokemonSpecies::Marowak, RESTLESS_SOUL_LEVEL);
            rt.maps().pokemon_tower_6f.cur_script = SCRIPT_POKEMONTOWER6F_MAROWAK_BATTLE;
            Flow::Return
        }
        Label::MovedBack => {
            rt.maps().pokemon_tower_6f.cur_script = SCRIPT_POKEMONTOWER6F_DEFAULT;
            Flow::Return
        }
        Label::Departed => {
            rt.joy_ignore(Joypad::empty());
            rt.maps().pokemon_tower_6f.cur_script = SCRIPT_POKEMONTOWER6F_DEFAULT;
            Flow::Return
        }
    }
}
