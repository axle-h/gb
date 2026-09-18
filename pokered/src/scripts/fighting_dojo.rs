//! `FightingDojo_Script`: the Karate Master, who fights whoever walks up to him, and the two Poké
//! Balls behind him, of which the winner may take exactly one.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_BEAT_FIGHTING_DOJO_TRAINER_3, EVENT_BEAT_KARATE_MASTER,
    EVENT_DEFEATED_FIGHTING_DOJO, EVENT_GOT_HITMONCHAN, EVENT_GOT_HITMONLEE};
use poke_core::symbols::pokered_local_labels::{FightingDojoHitmonchanPokeBallText as hitmonchan,
    FightingDojoHitmonleePokeBallText as hitmonlee, FightingDojoKarateMasterText as master};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_FIGHTINGDOJO_DEFAULT,
    SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE, TEXT_FIGHTINGDOJO_BLACKBELT1, TEXT_FIGHTINGDOJO_BLACKBELT2,
    TEXT_FIGHTINGDOJO_BLACKBELT3, TEXT_FIGHTINGDOJO_BLACKBELT4, TEXT_FIGHTINGDOJO_HITMONCHAN_POKE_BALL,
    TEXT_FIGHTINGDOJO_HITMONLEE_POKE_BALL, TEXT_FIGHTINGDOJO_KARATE_MASTER,
    TEXT_FIGHTINGDOJO_KARATE_MASTER_I_WILL_GIVE_YOU_A_POKEMON};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::FIGHTINGDOJO_KARATE_MASTER;
use poke_core::symbols::pokered_toggles::{TOGGLE_FIGHTING_DOJO_GIFT_1, TOGGLE_FIGHTING_DOJO_GIFT_2};
use poke_core::symbols::DmgPointer;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::sprites::SPRITE_FACING_LEFT;
use super::{text_at, Code, Flow, Script};

/// `PLAYER_DIR_RIGHT`.
const PLAYER_DIR_RIGHT: u8 = 1;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// The square west of the Karate Master, the only one he answers a step onto.
const CHALLENGE_AT: (u8, u8) = (4, 3);
/// The level either ball's mon comes at.
const GIFT_LEVEL: u8 = 30;

/// One of the two Poké Balls: the text id it answers to, the words it asks with, the mon in it, the
/// event it sets and the toggle that takes it off the floor.
struct Gift {
    text_id: u8,
    question: DmgPointer,
    species: PokemonSpecies,
    event: u16,
    toggle: u16,
}

const BALLS: [Gift; 2] = [
    Gift {
        text_id: TEXT_FIGHTINGDOJO_HITMONLEE_POKE_BALL,
        question: hitmonlee::Text,
        species: PokemonSpecies::Hitmonlee,
        event: EVENT_GOT_HITMONLEE,
        toggle: TOGGLE_FIGHTING_DOJO_GIFT_1,
    },
    Gift {
        text_id: TEXT_FIGHTINGDOJO_HITMONCHAN_POKE_BALL,
        question: hitmonchan::Text,
        species: PokemonSpecies::Hitmonchan,
        event: EVENT_GOT_HITMONCHAN,
        toggle: TOGGLE_FIGHTING_DOJO_GIFT_2,
    },
];

fn ball(text_id: u8) -> &'static Gift {
    BALLS.iter().find(|ball| ball.text_id == text_id).expect("a dojo Poké Ball")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wFightingDojoCurScript`.
    pub cur_script: u8,
    /// `wSavedCoordIndex`: set where the player walked up to the Karate Master rather than talking
    /// to him, which is the only way the two of them need turning to face each other afterwards.
    pub saved_coord_index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    /// `ld [wFightingDojoCurScript], a` after the table's routine.
    StoreCurScript,
    /// `FightingDojoDefaultScript` once `CheckFightingMapTrainers` has had its look.
    CheckedTrainers,
    MasterTurned,
    /// `FightingDojoKarateMasterPostBattleScript.already_facing`, and the words it prints.
    AlreadyFacing,
    GiftOffered,
    /// `FightingDojoKarateMasterText` after `.Text`: the battle it engages.
    MasterChallenged,
    /// Either ball's `.GetMon`, by the text id of the ball.
    BallDexShown(u8),
    BallAsked(u8),
    BallAnswered(u8),
    BallGiven(u8),
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().fighting_dojo.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::FightingDojoTrainerHeaders);
    let entry: Code = match index {
        SCRIPT_FIGHTINGDOJO_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE => return karate_master_post_battle(rt),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `FightingDojoResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().fighting_dojo.cur_script = SCRIPT_FIGHTINGDOJO_DEFAULT;
    rt.set_cur_map_script(SCRIPT_FIGHTINGDOJO_DEFAULT);
    Flow::Return
}

/// `FightingDojoDefaultScript`: the four blackbelts have their look first, and the Karate Master
/// speaks up only for a player who has walked all the way to the square beside him.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_DEFEATED_FIGHTING_DOJO) {
        return Flow::Return;
    }
    rt.trainer_script(SCRIPT_FIGHTINGDOJO_DEFAULT).then(Label::CheckedTrainers)
}

/// `FightingDojoKarateMasterPostBattleScript`: the emblem stays, and a fighting Pokémon is offered
/// in its place.
fn karate_master_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    if rt.maps().fighting_dojo.saved_coord_index == 0 {
        return already_facing(rt);
    }
    rt.set_player_moving_direction(PLAYER_DIR_RIGHT);
    rt.set_sprite_facing_direction_and_delay(FIGHTINGDOJO_KARATE_MASTER, SPRITE_FACING_LEFT)
        .then(Label::AlreadyFacing)
}

/// `.already_facing`: beating him counts as beating the dojo's four blackbelts too, so nobody left
/// standing stops the walk to the Poké Balls.
fn already_facing(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_CTRL_PAD);
    for event in EVENT_BEAT_KARATE_MASTER..=EVENT_BEAT_FIGHTING_DOJO_TRAINER_3 {
        rt.set_event(event);
    }
    rt.display_text_id(TEXT_FIGHTINGDOJO_KARATE_MASTER_I_WILL_GIVE_YOU_A_POKEMON).then(Label::GiftOffered)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_FIGHTINGDOJO_KARATE_MASTER => return Some(karate_master_text(rt)),
        TEXT_FIGHTINGDOJO_HITMONLEE_POKE_BALL | TEXT_FIGHTINGDOJO_HITMONCHAN_POKE_BALL => {
            return Some(poke_ball_text(rt, text_id));
        }
        TEXT_FIGHTINGDOJO_BLACKBELT1 => sym::FightingDojoTrainerHeader0,
        TEXT_FIGHTINGDOJO_BLACKBELT2 => sym::FightingDojoTrainerHeader1,
        TEXT_FIGHTINGDOJO_BLACKBELT3 => sym::FightingDojoTrainerHeader2,
        TEXT_FIGHTINGDOJO_BLACKBELT4 => sym::FightingDojoTrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `FightingDojoKarateMasterText`.
fn karate_master_text(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_DEFEATED_FIGHTING_DOJO) {
        return rt.print_text(text_at(master::StayAndTrainWithUsText)).ret();
    }
    if rt.check_event(EVENT_BEAT_KARATE_MASTER) {
        return rt.print_text(text_at(master::IWillGiveYouAPokemonText)).ret();
    }
    rt.print_text(text_at(master::Text)).then(Label::MasterChallenged)
}

/// `FightingDojoHitmonleePokeBallText` and `FightingDojoHitmonchanPokeBallText`, which differ only
/// in the mon in the ball: taking either shuts both, since the event is checked for both.
fn poke_ball_text(rt: &mut Script, text_id: u8) -> Flow {
    if rt.check_event(EVENT_GOT_HITMONLEE) || rt.check_event(EVENT_GOT_HITMONCHAN) {
        return rt.print_text(text_at(sym::FightingDojoBetterNotGetGreedyText)).ret();
    }
    // The dex page is shown before the offer, and it is what puts the mon in `wCurPartySpecies`.
    rt.display_pokedex(ball(text_id).species).then(Label::BallDexShown(text_id))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().fighting_dojo.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::CheckedTrainers => {
            if rt.trainer_header_flag_bit() != 0 || rt.check_event(EVENT_BEAT_KARATE_MASTER) {
                return Flow::Return;
            }
            rt.clear_joy_held();
            rt.maps().fighting_dojo.saved_coord_index = 0;
            if (rt.x(), rt.y()) != CHALLENGE_AT {
                return Flow::Return;
            }
            rt.maps().fighting_dojo.saved_coord_index = 1;
            rt.set_player_moving_direction(PLAYER_DIR_RIGHT);
            rt.set_sprite_facing_direction_and_delay(FIGHTINGDOJO_KARATE_MASTER, SPRITE_FACING_LEFT)
                .then(Label::MasterTurned)
        }
        Label::MasterTurned => rt.display_text_id(TEXT_FIGHTINGDOJO_KARATE_MASTER).ret(),
        Label::AlreadyFacing => already_facing(rt),
        Label::GiftOffered => reset_scripts(rt),
        Label::MasterChallenged => {
            rt.save_end_battle_text(master::DefeatedText);
            rt.engage_map_trainer(rt.sprite_index(), 0);
            rt.maps().fighting_dojo.cur_script = SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE;
            rt.set_cur_map_script(SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE);
            Flow::Return
        }
        Label::BallDexShown(text_id) => {
            rt.print_text(text_at(ball(text_id).question)).then(Label::BallAsked(text_id))
        }
        Label::BallAsked(text_id) => rt.yes_no_choice().then(Label::BallAnswered(text_id)),
        Label::BallAnswered(text_id) => match rt.chose_yes() {
            true => rt.give_pokemon(ball(text_id).species, GIFT_LEVEL).then(Label::BallGiven(text_id)),
            false => Flow::Return,
        },
        // A full party is no reason to leave the ball there: the mon goes to a box and it is gone.
        Label::BallGiven(text_id) => {
            if !rt.gave_pokemon() {
                return Flow::Return;
            }
            let ball = ball(text_id);
            rt.hide_object(ball.toggle);
            rt.set_event(ball.event);
            rt.set_event(EVENT_DEFEATED_FIGHTING_DOJO);
            Flow::Return
        }
    }
}
