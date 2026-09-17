//! `CinnabarGym_Script`: the six quiz gates, the Super Nerd behind each wrong answer, Blaine, the
//! Volcano Badge and TM38.

use poke_core::item::ItemId;
use poke_core::symbols::DmgPointer;
use poke_core::symbols::pokered_events::{EVENT_2A7, EVENT_BEAT_BLAINE, EVENT_BEAT_CINNABAR_GYM_TRAINER_0,
    EVENT_BEAT_CINNABAR_GYM_TRAINER_6, EVENT_CINNABAR_GYM_GATE0_UNLOCKED, EVENT_GOT_TM38};
use poke_core::symbols::pokered_local_labels::{CinnabarGymBlaineText as blaine, CinnabarGymGymGuideText as guide,
    CinnabarGymSuperNerd1, CinnabarGymSuperNerd2, CinnabarGymSuperNerd3, CinnabarGymSuperNerd4,
    CinnabarGymSuperNerd5, CinnabarGymSuperNerd6, CinnabarGymSuperNerd7};
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{CINNABARGYM_BLAINE, CINNABARGYM_SUPER_NERD3};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_LEFT, NPC_MOVEMENT_UP};
use super::{text_at, Flow, Script};

/// `BIT_VOLCANOBADGE`.
const BIT_VOLCANOBADGE: u8 = 6;
/// `wGymLeaderNo` for Blaine.
const BLAINE: u8 = 7;
const PLAYER_DIR_RIGHT: u8 = 1;
const PLAYER_DIR_DOWN: u8 = 4;
const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `MovementNpcToLeftAndUp` and `MovementNpcToLeft`.
const TO_LEFT_AND_UP: [u8; 3] = [NPC_MOVEMENT_LEFT, NPC_MOVEMENT_UP, END];
const TO_LEFT: [u8; 2] = [NPC_MOVEMENT_LEFT, END];

/// A Super Nerd's three texts, indexed by his gate. His text id, his sprite slot and his
/// `EVENT_BEAT_CINNABAR_GYM_TRAINER_*` all run together two apart from each other.
const NERDS: [(DmgPointer, DmgPointer, DmgPointer); 7] = [
    (CinnabarGymSuperNerd1::BattleText, CinnabarGymSuperNerd1::EndBattleText, CinnabarGymSuperNerd1::AfterBattleText),
    (CinnabarGymSuperNerd2::BattleText, CinnabarGymSuperNerd2::EndBattleText, CinnabarGymSuperNerd2::AfterBattleText),
    (CinnabarGymSuperNerd3::BattleText, CinnabarGymSuperNerd3::EndBattleText, CinnabarGymSuperNerd3::AfterBattleText),
    (CinnabarGymSuperNerd4::BattleText, CinnabarGymSuperNerd4::EndBattleText, CinnabarGymSuperNerd4::AfterBattleText),
    (CinnabarGymSuperNerd5::BattleText, CinnabarGymSuperNerd5::EndBattleText, CinnabarGymSuperNerd5::AfterBattleText),
    (CinnabarGymSuperNerd6::BattleText, CinnabarGymSuperNerd6::EndBattleText, CinnabarGymSuperNerd6::AfterBattleText),
    (CinnabarGymSuperNerd7::BattleText, CinnabarGymSuperNerd7::EndBattleText, CinnabarGymSuperNerd7::AfterBattleText),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCinnabarGymCurScript`.
    pub cur_script: u8,
    /// `wTrainerHeaderFlagBit`, which this map keeps a text id in: the gate two below it is the one
    /// the battle opens.
    pub trainer_header_flag_bit: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    OpenGateWait,
    OpenGateSound,
    ReceiveTm38,
    BadgeInfo,
    ReceivedTm38,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` Blaine's text does once the TM38 script has returned.
    TextDone,
    BlainePreBattle,
    NerdChallenged,
}

pub fn script(rt: &mut Script) -> Flow {
    set_map_and_tiles(rt);
    rt.enable_auto_text_box_drawing();
    match rt.maps().cinnabar_gym.cur_script {
        SCRIPT_CINNABARGYM_GET_OPPONENT_TEXT => get_opponent_text(rt),
        SCRIPT_CINNABARGYM_OPEN_GATE => open_gate(rt),
        SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE => blaine_post_battle(rt),
        _ => default_script(rt),
    }
}

/// `CinnabarGymSetMapAndTiles`.
fn set_map_and_tiles(rt: &mut Script) {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("CINNABAR ISLAND", "BLAINE");
    }
    if rt.check_and_reset_cur_map_loaded(1) {
        rt.update_cinnabar_gym_gate_tile_blocks();
    }
    rt.reset_event(EVENT_2A7);
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().cinnabar_gym.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `CinnabarGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_CINNABARGYM_DEFAULT);
    rt.set_opponent_after_wrong_answer(0);
    Flow::Return
}

/// `CinnabarGymDefaultScript`: a wrong answer sends the gate's own trainer out to fight.
fn default_script(rt: &mut Script) -> Flow {
    let opponent = rt.opponent_after_wrong_answer();
    if opponent == 0 {
        return Flow::Return;
    }
    rt.set_sprite_index(opponent);
    // Super Nerd 3 alone stands below his gate rather than to the right of it.
    let (direction, path) = match opponent == CINNABARGYM_SUPER_NERD3 {
        true => (PLAYER_DIR_DOWN, &TO_LEFT_AND_UP[..]),
        false => (PLAYER_DIR_RIGHT, &TO_LEFT[..]),
    };
    rt.set_player_moving_direction(direction);
    rt.move_sprite(opponent, path);
    set_script(rt, SCRIPT_CINNABARGYM_GET_OPPONENT_TEXT);
    Flow::Return
}

/// `CinnabarGymGetOpponentTextScript`: his sprite slot is also his text id.
fn get_opponent_text(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    let opponent = rt.opponent_after_wrong_answer();
    rt.maps().cinnabar_gym.trainer_header_flag_bit = opponent;
    rt.display_text_id(opponent).ret()
}

/// The gate a text id opens, and the trainer behind it.
fn gate(rt: &mut Script) -> u16 {
    rt.maps().cinnabar_gym.trainer_header_flag_bit.wrapping_sub(2) as u16
}

/// `CinnabarGymOpenGateScript`: a gate already won stays open in silence.
fn open_gate(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    let gate = gate(rt);
    if rt.check_event(EVENT_BEAT_CINNABAR_GYM_TRAINER_0 + gate) {
        return open_gate_done(rt);
    }
    rt.wait_for_sound_to_finish().then(Label::OpenGateSound)
}

fn open_gate_done(rt: &mut Script) -> Flow {
    let gate = gate(rt);
    rt.set_event(EVENT_BEAT_CINNABAR_GYM_TRAINER_0 + gate);
    rt.set_event(EVENT_CINNABAR_GYM_GATE0_UNLOCKED + gate);
    rt.update_cinnabar_gym_gate_tile_blocks();
    reset_scripts(rt)
}

/// `CinnabarGymBlainePostBattleScript`.
fn blaine_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm38(rt)
}

/// `CinnabarGymReceiveTM38`.
fn receive_tm38(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_CINNABARGYM_BLAINE_VOLCANO_BADGE_INFO).then(Label::BadgeInfo)
}

/// `CinnabarGymStartBattleScript`: whoever the player is facing, by the sprite slot the map or the
/// overworld put in `hSpriteIndex`.
fn start_battle_script(rt: &mut Script, gym_leader_no: u8) -> Flow {
    let slot = rt.sprite_index();
    rt.engage_map_trainer(slot, gym_leader_no);
    let next = match slot == CINNABARGYM_BLAINE {
        true => SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE,
        false => SCRIPT_CINNABARGYM_OPEN_GATE,
    };
    set_script(rt, next);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_CINNABARGYM_BLAINE => blaine_text(rt),
        TEXT_CINNABARGYM_GYM_GUIDE => {
            let said = match rt.check_event(EVENT_BEAT_BLAINE) {
                true => guide::BeatBlaineText,
                false => guide::ChampInMakingText,
            };
            rt.print_text(text_at(said)).ret()
        }
        TEXT_CINNABARGYM_SUPER_NERD1..=TEXT_CINNABARGYM_SUPER_NERD7 => super_nerd(rt, text_id),
        _ => return None,
    })
}

/// `CinnabarGymBlaineText`: he hands the badge over himself if a full bag stopped the TM.
fn blaine_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_BLAINE) {
        return rt.print_text(text_at(blaine::PreBattleText)).then(Label::BlainePreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM38) {
        return Flow::Call(Label::ReceiveTm38.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(blaine::PostBattleAdviceText)).ret()
}

/// `CinnabarGymSetTrainerHeader` and the Super Nerd's own text either side of it.
fn super_nerd(rt: &mut Script, text_id: u8) -> Flow {
    rt.maps().cinnabar_gym.trainer_header_flag_bit = text_id;
    let gate = gate(rt);
    let (battle, _, after) = NERDS[gate as usize];
    if rt.check_event(EVENT_BEAT_CINNABAR_GYM_TRAINER_0 + gate) {
        return rt.print_text(text_at(after)).ret();
    }
    rt.print_text(text_at(battle)).then(Label::NerdChallenged)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::OpenGateWait => open_gate_done(rt),
        Label::OpenGateSound => {
            rt.play_sound(crate::audio::data::sounds::SFX_GO_INSIDE);
            rt.wait_for_sound_to_finish().then(Label::OpenGateWait)
        }
        Label::ReceiveTm38 => receive_tm38(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_BLAINE);
            let text_id = match rt.give_item(ItemId::Tm38FireBlast, 1) {
                true => TEXT_CINNABARGYM_BLAINE_RECEIVED_TM38,
                false => TEXT_CINNABARGYM_BLAINE_TM38_NO_ROOM,
            };
            rt.display_text_id(text_id).then(Label::ReceivedTm38)
        }
        Label::ReceivedTm38 => {
            if rt.is_item_in_bag(ItemId::Tm38FireBlast) {
                rt.set_event(EVENT_GOT_TM38);
            }
            resume(rt, Label::GymVictory)
        }
        // `.gymVictory`: every gate is marked won, and the next pass opens all six.
        Label::GymVictory => {
            rt.set_badge(BIT_VOLCANOBADGE);
            for event in EVENT_BEAT_CINNABAR_GYM_TRAINER_0..=EVENT_BEAT_CINNABAR_GYM_TRAINER_6 {
                rt.set_event(event);
            }
            rt.set_cur_map_loaded(1);
            reset_scripts(rt)
        }
        Label::BlainePreBattle => {
            rt.save_end_battle_text(blaine::ReceivedVolcanoBadgeText);
            start_battle_script(rt, BLAINE)
        }
        Label::NerdChallenged => {
            let (_, end, _) = NERDS[gate(rt) as usize];
            rt.save_end_battle_text(end);
            start_battle_script(rt, 0)
        }
    }
}
