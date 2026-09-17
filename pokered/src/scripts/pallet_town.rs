//! `PalletTown_Script`: Oak stopping the player at the edge of the grass and leading them to his lab.

use poke_core::symbols::pokered_events::{EVENT_DAISY_WALKING, EVENT_ENTERED_BLUES_HOUSE, EVENT_FOLLOWED_OAK_INTO_LAB,
    EVENT_GOT_POKEBALLS_FROM_OAK, EVENT_GOT_TOWN_MAP, EVENT_OAK_APPEARED_IN_PALLET, EVENT_PALLET_AFTER_GETTING_POKEBALLS,
    EVENT_PALLET_AFTER_GETTING_POKEBALLS_2};
use poke_core::symbols::pokered_local_labels::PalletTownOakText;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::PALLETTOWN_OAK;
use poke_core::symbols::pokered_toggles::{TOGGLE_DAISY_SITTING, TOGGLE_DAISY_WALKING, TOGGLE_PALLET_TOWN_OAK};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::PALLET_MOVEMENT_SCRIPT;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_DOWN`.
const PLAYER_DIR_DOWN: u8 = 4;
/// `EXCLAMATION_BUBBLE`.
const EXCLAMATION_BUBBLE: u8 = 0;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPalletTownCurScript`.
    pub cur_script: u8,
    /// `wOakWalkedToPlayer`.
    pub oak_walked_to_player: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `PalletTownOakHeyWaitScript` after its `DisplayTextID`.
    OakHeyWaitShowOak,
    /// `PalletTownOakWalksToPlayerScript` after `SetSpriteFacingDirectionAndDelay`, and after its
    /// `Delay3`.
    OakWalksToPlayerDelay,
    OakWalksToPlayerFindPath,
    /// `PalletTownOakNotSafeComeWithMeScript` after its `DisplayTextID`.
    OakNotSafeFollow,
    /// `PalletTownOakText.HeyWaitDontGoOutText`'s `text_asm`, then after its `DelayFrames 10`, then
    /// after the bubble.
    HeyWaitDontGoOutAsm,
    HeyWaitBubble,
    HeyWaitFaceDown,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_GOT_POKEBALLS_FROM_OAK) {
        rt.set_event(EVENT_PALLET_AFTER_GETTING_POKEBALLS);
    }
    rt.enable_auto_text_box_drawing();
    match rt.maps().pallet_town.cur_script {
        SCRIPT_PALLETTOWN_DEFAULT => default_script(rt),
        SCRIPT_PALLETTOWN_OAK_HEY_WAIT => {
            rt.maps().pallet_town.oak_walked_to_player = false;
            rt.display_text_id(TEXT_PALLETTOWN_OAK).then(Label::OakHeyWaitShowOak)
        }
        SCRIPT_PALLETTOWN_OAK_WALKS_TO_PLAYER => {
            rt.set_sprite_facing_direction_and_delay(PALLETTOWN_OAK, SPRITE_FACING_UP).then(Label::OakWalksToPlayerDelay)
        }
        SCRIPT_PALLETTOWN_OAK_NOT_SAFE_COME_WITH_ME => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.set_player_facing(SPRITE_FACING_DOWN);
            rt.maps().pallet_town.oak_walked_to_player = true;
            rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
            rt.display_text_id(TEXT_PALLETTOWN_OAK).then(Label::OakNotSafeFollow)
        }
        SCRIPT_PALLETTOWN_PLAYER_FOLLOWS_OAK => {
            if !rt.npc_movement_script_running() {
                rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_DAISY;
            }
            Flow::Return
        }
        SCRIPT_PALLETTOWN_DAISY => daisy_script(rt),
        _ => Flow::Return,
    }
}

/// `PalletTownDefaultScript`: the player at the town's north edge before Oak has been followed.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_FOLLOWED_OAK_INTO_LAB) || rt.y() != 1 {
        return Flow::Return;
    }
    rt.clear_joy_held();
    rt.set_player_moving_direction(PLAYER_DIR_DOWN);
    rt.play_sound(SoundId::STOP_ALL_MUSIC);
    rt.play_music(sounds::MUSIC_MEET_PROF_OAK);
    rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
    rt.set_event(EVENT_OAK_APPEARED_IN_PALLET);
    rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_OAK_HEY_WAIT;
    Flow::Return
}

/// `PalletTownDaisyScript`.
fn daisy_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_DAISY_WALKING) && rt.check_event(EVENT_GOT_TOWN_MAP) && rt.check_event(EVENT_ENTERED_BLUES_HOUSE) {
        rt.set_event(EVENT_DAISY_WALKING);
        rt.hide_object(TOGGLE_DAISY_SITTING);
        rt.show_object(TOGGLE_DAISY_WALKING);
        return Flow::Return;
    }
    if rt.check_event(EVENT_GOT_POKEBALLS_FROM_OAK) {
        rt.set_event(EVENT_PALLET_AFTER_GETTING_POKEBALLS_2);
    }
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_PALLETTOWN_OAK => {
            if rt.maps().pallet_town.oak_walked_to_player {
                rt.print_text(text_at(PalletTownOakText::ItsUnsafeText)).ret()
            } else {
                rt.set_do_not_wait_for_button_press(true);
                rt.print_text(text_at(PalletTownOakText::HeyWaitDontGoOutText)).then(Label::HeyWaitDontGoOutAsm)
            }
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::OakHeyWaitShowOak => {
            rt.joy_ignore(Joypad::all());
            rt.show_object(TOGGLE_PALLET_TOWN_OAK);
            rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_OAK_WALKS_TO_PLAYER;
            Flow::Return
        }
        Label::OakWalksToPlayerDelay => rt.delay3().then(Label::OakWalksToPlayerFindPath),
        Label::OakWalksToPlayerFindPath => {
            rt.set_y(1);
            let path = rt.find_path_to_player(PALLETTOWN_OAK, true, -1);
            rt.move_sprite(PALLETTOWN_OAK, &path);
            rt.joy_ignore(Joypad::all());
            rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_OAK_NOT_SAFE_COME_WITH_ME;
            Flow::Return
        }
        Label::OakNotSafeFollow => {
            rt.joy_ignore(Joypad::all());
            rt.start_npc_movement_script(PALLET_MOVEMENT_SCRIPT, PALLETTOWN_OAK);
            rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_PLAYER_FOLLOWS_OAK;
            Flow::Return
        }
        Label::HeyWaitDontGoOutAsm => rt.delay_frames(10).then(Label::HeyWaitBubble),
        Label::HeyWaitBubble => rt.emotion_bubble(0, EXCLAMATION_BUBBLE).then(Label::HeyWaitFaceDown),
        Label::HeyWaitFaceDown => {
            rt.set_player_moving_direction(PLAYER_DIR_DOWN);
            Flow::Return
        }
    }
}
