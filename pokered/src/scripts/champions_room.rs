//! `ChampionsRoom_Script`: the rival waiting at the top of the League, and Oak walking the player
//! out towards the Hall of Fame.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_BEAT_CHAMPION_RIVAL;
use poke_core::symbols::pokered_local_labels::ChampionsRoomRivalText;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_CHAMPIONSROOM_CLEANUP_SCRIPT, SCRIPT_CHAMPIONSROOM_DEFAULT,
    SCRIPT_CHAMPIONSROOM_OAK_ARRIVES, SCRIPT_CHAMPIONSROOM_OAK_COME_WITH_ME,
    SCRIPT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER, SCRIPT_CHAMPIONSROOM_OAK_DISAPPOINTED_WITH_RIVAL,
    SCRIPT_CHAMPIONSROOM_OAK_EXITS, SCRIPT_CHAMPIONSROOM_PLAYER_ENTERS, SCRIPT_CHAMPIONSROOM_PLAYER_FOLLOWS_OAK,
    SCRIPT_CHAMPIONSROOM_RIVAL_DEFEATED, SCRIPT_CHAMPIONSROOM_RIVAL_READY_TO_BATTLE, TEXT_CHAMPIONSROOM_OAK,
    TEXT_CHAMPIONSROOM_OAK_COME_WITH_ME, TEXT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER,
    TEXT_CHAMPIONSROOM_OAK_DISAPPOINTED_WITH_RIVAL, TEXT_CHAMPIONSROOM_RIVAL};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::{CHAMPIONSROOM_OAK, CHAMPIONSROOM_RIVAL};
use poke_core::symbols::pokered_toggles::TOGGLE_CHAMPIONS_ROOM_OAK;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::NPC_MOVEMENT_UP;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_LEFT, SPRITE_FACING_RIGHT};
use super::{text_at, Code, Flow, Script};

/// `OPP_RIVAL3`.
const OPP_RIVAL3: u8 = OPP_ID_OFFSET + 43;
/// `PLAYER_DIR_LEFT`.
const PLAYER_DIR_LEFT: u8 = 2;
const END: u8 = 0xFF;
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// `RivalEntrance_RLEMovement` and `WalkToHallOfFame_RLEMovement`.
const RIVAL_ENTRANCE: [(Joypad, usize); 3] = [(Joypad::UP, 1), (Joypad::RIGHT, 1), (Joypad::UP, 3)];
const WALK_TO_HALL_OF_FAME: [(Joypad, usize); 2] = [(Joypad::UP, 4), (Joypad::LEFT, 1)];
/// `OakEntranceAfterVictoryMovement` and `OakExitChampionsRoomMovement`.
const OAK_ENTRANCE: [u8; 6] = [NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP,
    NPC_MOVEMENT_UP, END];
const OAK_EXIT: [u8; 3] = [NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, END];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wChampionsRoomCurScript`, which Agatha's room arms before the player ever arrives.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    PlayerEnters,
    RivalReadyToBattle,
    RivalDefeated,
    OakArrives,
    OakCongratulatesPlayer,
    OakDisappointedWithRival,
    OakComeWithMe,
    OakExits,
    PlayerFollowsOak,
    Cleanup,
    /// `ChampionsRoomRivalReadyToBattleScript` after each of its two waits.
    RivalSpoke,
    BattleArmed,
    /// Each `ChampionsRoom_DisplayTextID_AllowABSelectStart` and each turn it comes after.
    RivalDefeatedSpoke,
    OakArrivesMusic,
    OakArrivesSpoke,
    RivalTurned,
    OakTurnedToPlayer,
    CongratulationsSpoke,
    OakTurnedToRival,
    DisappointmentSpoke,
    OakTurnedAway,
    ComeWithMeSpoke,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    // `CallFunctionInTable`: this room has no trainer headers, so nothing re-reads `wCurMapScript`.
    let entry: Code = match rt.maps().champions_room.cur_script {
        SCRIPT_CHAMPIONSROOM_PLAYER_ENTERS => Label::PlayerEnters.into(),
        SCRIPT_CHAMPIONSROOM_RIVAL_READY_TO_BATTLE => Label::RivalReadyToBattle.into(),
        SCRIPT_CHAMPIONSROOM_RIVAL_DEFEATED => Label::RivalDefeated.into(),
        SCRIPT_CHAMPIONSROOM_OAK_ARRIVES => Label::OakArrives.into(),
        SCRIPT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER => Label::OakCongratulatesPlayer.into(),
        SCRIPT_CHAMPIONSROOM_OAK_DISAPPOINTED_WITH_RIVAL => Label::OakDisappointedWithRival.into(),
        SCRIPT_CHAMPIONSROOM_OAK_COME_WITH_ME => Label::OakComeWithMe.into(),
        SCRIPT_CHAMPIONSROOM_OAK_EXITS => Label::OakExits.into(),
        SCRIPT_CHAMPIONSROOM_PLAYER_FOLLOWS_OAK => Label::PlayerFollowsOak.into(),
        SCRIPT_CHAMPIONSROOM_CLEANUP_SCRIPT => Label::Cleanup.into(),
        // `ChampionsRoomDefaultScript`.
        _ => return Flow::Return,
    };
    Flow::Jump(entry)
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().champions_room.cur_script = index;
}

fn simulate(rt: &mut Script, rle: &[(Joypad, usize)]) {
    let presses = rle.iter().flat_map(|&(pad, count)| vec![pad; count]).collect();
    rt.simulate_joypad_presses(presses);
}

/// `ChampionsRoom_DisplayTextID_AllowABSelectStart`: the buttons are the player's while the text is
/// up, so the cutscene can be read at their own pace, and taken back the moment it closes.
fn display_text_id(rt: &mut Script, text_id: u8, then: Label) -> Flow {
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.display_text_id(text_id).then(then)
}

/// `ChampionsRoomPlayerEntersScript`: the rival walks the player into the room and shuts the way out
/// behind them.
fn player_enters(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    simulate(rt, &RIVAL_ENTRANCE);
    set_script(rt, SCRIPT_CHAMPIONSROOM_RIVAL_READY_TO_BATTLE);
    Flow::Return
}

/// `ChampionsRoomRivalReadyToBattleScript`.
fn rival_ready_to_battle(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::RivalSpoke)
}

/// `ChampionsRoomRivalDefeatedScript`: a battle lost puts the room back where it was, so the rival
/// is stood waiting to be spoken to again.
fn rival_defeated(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        rt.joy_ignore(Joypad::empty());
        set_script(rt, SCRIPT_CHAMPIONSROOM_DEFAULT);
        return Flow::Return;
    }
    rt.update_sprites();
    rt.set_event(EVENT_BEAT_CHAMPION_RIVAL);
    display_text_id(rt, TEXT_CHAMPIONSROOM_RIVAL, Label::RivalDefeatedSpoke)
}

/// `ChampionsRoomOakArrivesScript`.
fn oak_arrives(rt: &mut Script) -> Flow {
    rt.music_cities1_alternate_tempo().then(Label::OakArrivesMusic)
}

/// `ChampionsRoomOakCongratulatesPlayerScript`: the player and the rival are turned to face Oak
/// where he has stopped.
fn oak_congratulates_player(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.set_player_moving_direction(PLAYER_DIR_LEFT);
    rt.set_sprite_facing_direction_and_delay(CHAMPIONSROOM_RIVAL, SPRITE_FACING_LEFT).then(Label::RivalTurned)
}

/// `ChampionsRoomOakExitsScript`.
fn oak_exits(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(TOGGLE_CHAMPIONS_ROOM_OAK);
    set_script(rt, SCRIPT_CHAMPIONSROOM_PLAYER_FOLLOWS_OAK);
    Flow::Return
}

/// `ChampionsRoomPlayerFollowsOakScript`: the walk out of the door, which warps into the Hall of
/// Fame.
fn player_follows_oak(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    simulate(rt, &WALK_TO_HALL_OF_FAME);
    set_script(rt, SCRIPT_CHAMPIONSROOM_CLEANUP_SCRIPT);
    Flow::Return
}

/// `ChampionsRoomCleanupScript`.
fn cleanup(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_CHAMPIONSROOM_DEFAULT);
    Flow::Return
}

/// The rival's party, by the starter he took: `STARTER2` is Squirtle and `STARTER3` Bulbasaur.
fn trainer_no(rt: &mut Script) -> u8 {
    match PokemonSpecies::from_repr(rt.globals().rival_starter) {
        Some(PokemonSpecies::Squirtle) => 1,
        Some(PokemonSpecies::Bulbasaur) => 2,
        _ => 3,
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_CHAMPIONSROOM_RIVAL => {
            let words = match rt.check_event(EVENT_BEAT_CHAMPION_RIVAL) {
                true => sym::ChampionsRoomRivalAfterBattleText,
                false => ChampionsRoomRivalText::IntroText,
            };
            Some(rt.print_text(text_at(words)).ret())
        }
        TEXT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER => {
            let starter = PokemonSpecies::from_repr(rt.maps().oaks_lab.player_starter)?;
            rt.get_mon_name(starter);
            let words = poke_core::symbols::pokered_local_labels::ChampionsRoomOakCongratulatesPlayerText::Text;
            Some(rt.print_text(text_at(words)).ret())
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::PlayerEnters => player_enters(rt),
        Label::RivalReadyToBattle => rival_ready_to_battle(rt),
        Label::RivalDefeated => rival_defeated(rt),
        Label::OakArrives => oak_arrives(rt),
        Label::OakArrivesMusic => display_text_id(rt, TEXT_CHAMPIONSROOM_OAK, Label::OakArrivesSpoke),
        Label::OakCongratulatesPlayer => oak_congratulates_player(rt),
        Label::OakDisappointedWithRival => {
            rt.set_sprite_facing_direction_and_delay(CHAMPIONSROOM_OAK, SPRITE_FACING_RIGHT)
                .then(Label::OakTurnedToRival)
        }
        Label::OakComeWithMe => {
            rt.set_sprite_facing_direction_and_delay(CHAMPIONSROOM_OAK, SPRITE_FACING_DOWN)
                .then(Label::OakTurnedAway)
        }
        Label::OakExits => oak_exits(rt),
        Label::PlayerFollowsOak => player_follows_oak(rt),
        Label::Cleanup => cleanup(rt),
        // Animations are turned on for this battle whatever the player set, since it is the one the
        // credits follow.
        Label::RivalSpoke => {
            rt.joy_ignore(Joypad::empty());
            rt.set_battle_animation(true);
            rt.display_text_id(TEXT_CHAMPIONSROOM_RIVAL).then(Label::BattleArmed)
        }
        Label::BattleArmed => {
            let set = trainer_no(rt);
            rt.start_trainer_battle(OPP_RIVAL3, set, sym::RivalDefeatedText);
            rt.clear_joy_held();
            set_script(rt, SCRIPT_CHAMPIONSROOM_RIVAL_DEFEATED);
            rt.delay3().ret()
        }
        Label::RivalDefeatedSpoke => {
            take_pad(rt);
            rt.set_sprite_movement_bytes_to_ff(CHAMPIONSROOM_RIVAL);
            set_script(rt, SCRIPT_CHAMPIONSROOM_OAK_ARRIVES);
            Flow::Return
        }
        // Oak is shown only once he has been given somewhere to walk, so he is never stood still in
        // the doorway.
        Label::OakArrivesSpoke => {
            take_pad(rt);
            rt.set_sprite_movement_bytes_to_ff(CHAMPIONSROOM_OAK);
            rt.move_sprite(CHAMPIONSROOM_OAK, &OAK_ENTRANCE);
            rt.show_object(TOGGLE_CHAMPIONS_ROOM_OAK);
            set_script(rt, SCRIPT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER);
            Flow::Return
        }
        Label::RivalTurned => {
            rt.set_sprite_facing_direction_and_delay(CHAMPIONSROOM_OAK, SPRITE_FACING_DOWN)
                .then(Label::OakTurnedToPlayer)
        }
        Label::OakTurnedToPlayer => {
            display_text_id(rt, TEXT_CHAMPIONSROOM_OAK_CONGRATULATES_PLAYER, Label::CongratulationsSpoke)
        }
        Label::CongratulationsSpoke => {
            take_pad(rt);
            set_script(rt, SCRIPT_CHAMPIONSROOM_OAK_DISAPPOINTED_WITH_RIVAL);
            Flow::Return
        }
        Label::OakTurnedToRival => {
            display_text_id(rt, TEXT_CHAMPIONSROOM_OAK_DISAPPOINTED_WITH_RIVAL, Label::DisappointmentSpoke)
        }
        Label::DisappointmentSpoke => {
            take_pad(rt);
            set_script(rt, SCRIPT_CHAMPIONSROOM_OAK_COME_WITH_ME);
            Flow::Return
        }
        Label::OakTurnedAway => display_text_id(rt, TEXT_CHAMPIONSROOM_OAK_COME_WITH_ME, Label::ComeWithMeSpoke),
        Label::ComeWithMeSpoke => {
            take_pad(rt);
            rt.move_sprite(CHAMPIONSROOM_OAK, &OAK_EXIT);
            set_script(rt, SCRIPT_CHAMPIONSROOM_OAK_EXITS);
            Flow::Return
        }
    }
}

/// The second half of `ChampionsRoom_DisplayTextID_AllowABSelectStart`: the pad is the script's
/// again the moment the text closes.
fn take_pad(rt: &mut Script) {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
}
