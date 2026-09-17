//! `MtMoonB2F_Script`: the four Rockets, the Super Nerd guarding the two fossils, and the one he
//! walks over to take once the player has chosen.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD, EVENT_GOT_DOME_FOSSIL,
    EVENT_GOT_HELIX_FOSSIL};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::{MtMoon3TrainerHeader0, MtMoon3TrainerHeader1, MtMoon3TrainerHeader2,
    MtMoon3TrainerHeader3, MtMoon3TrainerHeaders, MTMOONB2F_SUPER_NERD};
use poke_core::symbols::pokered_toggles::{TOGGLE_MT_MOON_B2F_FOSSIL_1, TOGGLE_MT_MOON_B2F_FOSSIL_2};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP};
use super::{text_at, Code, Flow, Script};

const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `MtMoonB2FFossilAreaCoords`: while the Super Nerd is still deciding, a wild battle in the bowl
/// he stands in would leave his walk half done, so none is rolled there.
const FOSSIL_AREA: [(u8, u8); 16] = [(11, 5), (12, 5), (13, 5), (14, 5), (11, 6), (12, 6), (13, 6), (14, 6),
    (11, 7), (12, 7), (13, 7), (14, 7), (11, 8), (12, 8), (13, 8), (14, 8)];
/// `MtMoonB2FPlayerNearDomeFossilCoords` and `...HelixFossilCoords`: which fossil the player is
/// standing beside is what decides the one step the Super Nerd takes to the other.
const NEAR_DOME: [(u8, u8); 3] = [(12, 7), (11, 6), (12, 5)];
const NEAR_HELIX: [(u8, u8); 3] = [(13, 7), (14, 6), (14, 5)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wMtMoonB2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    DefeatedSuperNerd,
    MoveSuperNerd,
    TakesOtherFossil,
    /// `ld [wMtMoonB2FCurScript], a` and the no-battles check after it.
    StoreCurScript,
    DefeatedDelay,
    TakesFossilText,
    SuperNerdBattleText,
    /// The fossil the player asked for, carried over its yes/no.
    FossilYesNo(bool),
    FossilAnswered(bool),
    FossilGiven(bool),
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().mt_moon_b2f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, MtMoon3TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD => Label::DefeatedSuperNerd.into(),
        SCRIPT_MTMOONB2F_MOVE_SUPER_NERD => Label::MoveSuperNerd.into(),
        SCRIPT_MTMOONB2F_SUPER_NERD_TAKES_OTHER_FOSSIL => Label::TakesOtherFossil.into(),
        SCRIPT_MTMOONB2F_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `MtMoonB2FDefaultScript`: the Super Nerd speaks up the moment the player steps in front of him.
fn default_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD) && (rt.x(), rt.y()) == (13, 8) {
        rt.clear_joy_held();
        return rt.display_text_id(TEXT_MTMOONB2F_SUPER_NERD).ret();
    }
    if got_a_fossil(rt) {
        return Flow::Return;
    }
    rt.trainer_script(SCRIPT_MTMOONB2F_DEFAULT).ret()
}

fn got_a_fossil(rt: &Script) -> bool {
    rt.check_event(EVENT_GOT_DOME_FOSSIL) || rt.check_event(EVENT_GOT_HELIX_FOSSIL)
}

/// `MtMoonB2FResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_MTMOONB2F_DEFAULT);
    Flow::Return
}

/// `ld [wMtMoonB2FCurScript], a` and `ld [wCurMapScript], a` together, as every routine here writes
/// them.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().mt_moon_b2f.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `MtMoonB2FMoveSuperNerdScript`: one step to whichever fossil the player left.
fn move_super_nerd(rt: &mut Script) -> Flow {
    rt.set_sprite_movement_bytes_to_ff(MTMOONB2F_SUPER_NERD);
    let step = if rt.are_player_coords_in_array(&NEAR_DOME).is_some() {
        NPC_MOVEMENT_RIGHT
    } else if rt.are_player_coords_in_array(&NEAR_HELIX).is_some() {
        NPC_MOVEMENT_UP
    } else {
        return rt.trainer_script(SCRIPT_MTMOONB2F_DEFAULT).ret();
    };
    rt.move_sprite(MTMOONB2F_SUPER_NERD, &[step, END]);
    set_script(rt, SCRIPT_MTMOONB2F_SUPER_NERD_TAKES_OTHER_FOSSIL);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_MTMOONB2F_SUPER_NERD => return Some(super_nerd_text(rt)),
        TEXT_MTMOONB2F_DOME_FOSSIL => return Some(fossil_text(rt, true)),
        TEXT_MTMOONB2F_HELIX_FOSSIL => return Some(fossil_text(rt, false)),
        TEXT_MTMOONB2F_ROCKET1 => MtMoon3TrainerHeader0,
        TEXT_MTMOONB2F_ROCKET2 => MtMoon3TrainerHeader1,
        TEXT_MTMOONB2F_ROCKET3 => MtMoon3TrainerHeader2,
        TEXT_MTMOONB2F_ROCKET4 => MtMoon3TrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `MtMoonB2FSuperNerdText`: he fights for both fossils, and only once he has lost does he offer one.
fn super_nerd_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD) {
        return rt.print_text(text_at(sym::MtMoonB2FSuperNerdTheyreBothMineText)).then(Label::SuperNerdBattleText);
    }
    let words = match got_a_fossil(rt) {
        true => sym::MtMoonB2FSuperNerdTheresAPokemonLabText,
        false => sym::MtMoonB2fSuperNerdEachTakeOneText,
    };
    rt.print_text(text_at(words)).ret()
}

/// `MtMoonB2FDomeFossilText` and `MtMoonB2FHelixFossilText`, which differ only in what they hand over.
fn fossil_text(rt: &mut Script, dome: bool) -> Flow {
    rt.set_do_not_wait_for_button_press(true);
    let words = match dome {
        true => local::MtMoonB2FDomeFossilText::YouWantText,
        false => local::MtMoonB2FHelixFossilText::YouWantText,
    };
    rt.print_text(text_at(words)).then(Label::FossilYesNo(dome))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::MoveSuperNerd => move_super_nerd(rt),
        Label::DefeatedSuperNerd => {
            if rt.lost_battle() {
                return reset_scripts(rt);
            }
            rt.update_sprites();
            rt.delay3().then(Label::DefeatedDelay)
        }
        Label::DefeatedDelay => {
            rt.set_event(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD);
            reset_scripts(rt)
        }
        Label::TakesOtherFossil => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_do_not_wait_for_button_press(true);
            rt.display_text_id(TEXT_MTMOONB2F_SUPER_NERD_THEN_THIS_IS_MINE).then(Label::TakesFossilText)
        }
        Label::TakesFossilText => {
            let toggle = match rt.check_event(EVENT_GOT_DOME_FOSSIL) {
                true => TOGGLE_MT_MOON_B2F_FOSSIL_2,
                false => TOGGLE_MT_MOON_B2F_FOSSIL_1,
            };
            rt.hide_object(toggle);
            reset_scripts(rt)
        }
        Label::StoreCurScript => {
            rt.maps().mt_moon_b2f.cur_script = rt.cur_map_script();
            if rt.check_event(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD) {
                let in_bowl = rt.are_player_coords_in_array(&FOSSIL_AREA).is_some();
                rt.set_no_battles(in_bowl);
            }
            Flow::Return
        }
        Label::SuperNerdBattleText => {
            rt.save_end_battle_text(sym::MtMoonB2FSuperNerdOkIllShareText);
            rt.engage_map_trainer(rt.sprite_index(), 0);
            set_script(rt, SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD);
            Flow::Return
        }
        Label::FossilYesNo(dome) => rt.yes_no_choice().then(Label::FossilAnswered(dome)),
        Label::FossilAnswered(dome) => {
            if !rt.chose_yes() {
                return Flow::Return;
            }
            let item = if dome { ItemId::DomeFossil } else { ItemId::HelixFossil };
            if !rt.give_item(item, 1) {
                return rt.print_text(text_at(local::MtMoonB2FYouHaveNoRoomText::Text)).ret();
            }
            rt.print_text(text_at(local::MtMoonB2FReceivedFossilText::Text)).then(Label::FossilGiven(dome))
        }
        Label::FossilGiven(dome) => {
            let (toggle, event) = match dome {
                true => (TOGGLE_MT_MOON_B2F_FOSSIL_1, EVENT_GOT_DOME_FOSSIL),
                false => (TOGGLE_MT_MOON_B2F_FOSSIL_2, EVENT_GOT_HELIX_FOSSIL),
            };
            rt.hide_object(toggle);
            rt.set_event(event);
            set_script(rt, SCRIPT_MTMOONB2F_MOVE_SUPER_NERD);
            Flow::Return
        }
    }
}
