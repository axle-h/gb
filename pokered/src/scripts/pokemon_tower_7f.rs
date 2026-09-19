//! `PokemonTower7F_Script`: the three Rockets holding Mr Fuji, each of whom walks out of the room
//! when beaten, and Mr Fuji taking the player home with him.

use poke_core::map::Map;
use poke_core::pointer::{DmgBank, DmgPointer};
use poke_core::rom_gfx::rom_slice;
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
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

/// A movement list's end.
const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);
/// `MR_FUJIS_HOUSE`'s first warp, and the map the house is entered from.
const MR_FUJIS_HOUSE_WARP: u8 = 1;

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

/// `PokemonTower7FRocketLeaveMovementScript`.
fn rocket_leave_movement(rt: &mut Script) {
    let slot = rt.sprite_index();
    if let Some(path) = leave_movement(slot, rt.x(), rt.y()) {
        rt.move_sprite(slot, path);
    }
}

/// `PokemonTower7FNPCCoordMovementTable` searched from the Rocket in `slot`'s own four rows for the
/// square the player stands on. The loop has no end, so a square none of his rows names runs on into
/// the next Rocket's and walks him out their way; one found nowhere before the bank ends leaves him
/// standing, where the cartridge would read on past the ROM.
pub(super) fn leave_movement(slot: u8, x: u8, y: u8) -> Option<&'static [u8]> {
    let table = sym::PokemonTower7FNPCCoordMovementTable + (slot.wrapping_sub(1) << 4) as u16;
    let row = rom_slice(table).chunks_exact(4).find(|row| row[0] == y && row[1] == x)?;
    let address = u16::from_le_bytes([row[2], row[3]]);
    let bank = match address {
        0..0x4000 => DmgBank::ROM { bank: 0 },
        0x4000..0x8000 => table.bank,
        _ => return None,
    };
    let bytes = rom_slice(DmgPointer { bank, address });
    let end = bytes.iter().position(|&b| b == END).map_or(bytes.len(), |i| i + 1);
    Some(&bytes[..end])
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
            rocket_leave_movement(rt);
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
