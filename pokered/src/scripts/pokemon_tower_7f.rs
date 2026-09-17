//! `PokemonTower7F_Script`: the three Rockets holding Mr Fuji, each of whom walks out of the room
//! when beaten, and Mr Fuji taking the player home with him.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_RESCUED_MR_FUJI, EVENT_RESCUED_MR_FUJI_2};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_POKEMONTOWER7F_DEFAULT, SCRIPT_POKEMONTOWER7F_HIDE_NPC,
    SCRIPT_POKEMONTOWER7F_WARP_TO_MR_FUJI_HOUSE, TEXT_POKEMONTOWER7F_MR_FUJI, TEXT_POKEMONTOWER7F_ROCKET1,
    TEXT_POKEMONTOWER7F_ROCKET2, TEXT_POKEMONTOWER7F_ROCKET3};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_MR_FUJIS_HOUSE_MR_FUJI, TOGGLE_POKEMON_TOWER_7F_MR_FUJI,
    TOGGLE_SAFFRON_CITY_E, TOGGLE_SAFFRON_CITY_F};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT};
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);
/// `MR_FUJIS_HOUSE`'s first warp, and the map the house is entered from.
const MR_FUJIS_HOUSE_WARP: u8 = 1;

const EXIT_RIGHT_DOWN_LEFT: [u8; 8] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, END];
const EXIT_DOWN_RIGHT: [u8; 7] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const EXIT_DOWN: [u8; 6] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, END];
const EXIT_LEFT_DOWN: [u8; 8] = [NPC_MOVEMENT_LEFT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const EXIT_DOWN_LEFT: [u8; 7] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const EXIT_RIGHT_DOWN: [u8; 8] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];

/// `PokemonTower7FNPCCoordMovementTable`: four squares per Rocket, each the square the player has to
/// have been standing on to fight him, and the way out that does not walk through the player.
const EXITS: [[((u8, u8), &[u8]); 4]; 3] = [
    [((9, 12), &EXIT_RIGHT_DOWN_LEFT), ((10, 11), &EXIT_DOWN_RIGHT), ((11, 11), &EXIT_DOWN), ((12, 11), &EXIT_DOWN)],
    [((12, 10), &EXIT_LEFT_DOWN), ((11, 9), &EXIT_DOWN_LEFT), ((10, 9), &EXIT_DOWN), ((9, 9), &EXIT_DOWN)],
    [((9, 8), &EXIT_RIGHT_DOWN), ((10, 7), &EXIT_DOWN), ((11, 7), &EXIT_DOWN), ((12, 7), &EXIT_DOWN)],
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonTower7FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPokemonTower7FCurScript], a` after the table's routine.
    StoreCurScript,
    BattleEnded,
    RocketSpokeTo,
    Rescued,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pokemon_tower_7f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PokemonTower7TrainerHeaders);
    match index {
        SCRIPT_POKEMONTOWER7F_HIDE_NPC => hide_npc(rt),
        SCRIPT_POKEMONTOWER7F_WARP_TO_MR_FUJI_HOUSE => warp_to_mr_fujis_house(rt),
        2 => end_battle(rt),
        _ => rt.trainer_script(index).then(Label::StoreCurScript),
    }
}

/// `PokemonTower7FSetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().pokemon_tower_7f.cur_script = SCRIPT_POKEMONTOWER7F_DEFAULT;
    Flow::Return
}

/// `PokemonTower7FEndBattleScript`.
fn end_battle(rt: &mut Script) -> Flow {
    rt.set_seen_by_trainer(false);
    if rt.lost_battle() {
        return set_default_script(rt);
    }
    rt.trainer_script(2).then(Label::BattleEnded)
}

/// `PokemonTower7FHideNPCScript`.
fn hide_npc(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object_for_sprite(rt.sprite_index());
    rt.joy_ignore(Joypad::empty());
    rt.set_sprite_index(0);
    rt.maps().pokemon_tower_7f.cur_script = SCRIPT_POKEMONTOWER7F_DEFAULT;
    Flow::Return
}

/// `PokemonTower7FWarpToMrFujiHouseScript`: Mr Fuji takes the player home, which is a warp the
/// script asks for rather than one the player walked onto.
fn warp_to_mr_fujis_house(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    rt.hide_object(TOGGLE_POKEMON_TOWER_7F_MR_FUJI);
    rt.set_player_facing(SPRITE_FACING_UP);
    rt.set_last_map(Map::LavenderTown);
    rt.warp_from_cur_script(Map::MrFujisHouse, MR_FUJIS_HOUSE_WARP);
    rt.maps().pokemon_tower_7f.cur_script = SCRIPT_POKEMONTOWER7F_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_POKEMONTOWER7F_ROCKET1 => sym::PokemonTower7TrainerHeader0,
        TEXT_POKEMONTOWER7F_ROCKET2 => sym::PokemonTower7TrainerHeader1,
        TEXT_POKEMONTOWER7F_ROCKET3 => sym::PokemonTower7TrainerHeader2,
        TEXT_POKEMONTOWER7F_MR_FUJI => {
            return Some(rt.print_text(text_at(local::PokemonTower7FMrFujiText::RescueText)).then(Label::Rescued));
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pokemon_tower_7f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::BattleEnded => {
            rt.joy_ignore(PAD_CTRL_PAD);
            // `hSpriteIndex` and `hTextID` are one byte, so the Rocket's slot is his own text id.
            rt.display_text_id(rt.sprite_index()).then(Label::RocketSpokeTo)
        }
        Label::RocketSpokeTo => {
            let slot = rt.sprite_index();
            let here = (rt.x(), rt.y());
            let exits = EXITS[slot as usize - 1];
            if let Some(&(_, path)) = exits.iter().find(|&&(at, _)| at == here) {
                rt.move_sprite(slot, path);
            }
            rt.maps().pokemon_tower_7f.cur_script = SCRIPT_POKEMONTOWER7F_HIDE_NPC;
            Flow::Return
        }
        Label::Rescued => {
            rt.set_event(EVENT_RESCUED_MR_FUJI);
            rt.set_event(EVENT_RESCUED_MR_FUJI_2);
            rt.show_object(TOGGLE_MR_FUJIS_HOUSE_MR_FUJI);
            rt.hide_object(TOGGLE_SAFFRON_CITY_E);
            rt.show_object(TOGGLE_SAFFRON_CITY_F);
            rt.maps().pokemon_tower_7f.cur_script = SCRIPT_POKEMONTOWER7F_WARP_TO_MR_FUJI_HOUSE;
            Flow::Return
        }
    }
}
