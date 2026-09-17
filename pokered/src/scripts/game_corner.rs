//! `GameCorner_Script`: the Rocket guarding the poster, the switch behind it, and the four people
//! who hand out coins. The machines themselves are hidden events, not this map's script.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_ERIKA, EVENT_FOUND_ROCKET_HIDEOUT, EVENT_GOT_10_COINS,
    EVENT_GOT_20_COINS, EVENT_GOT_20_COINS_2};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_GAMECORNER_DEFAULT, SCRIPT_GAMECORNER_ROCKET_BATTLE,
    SCRIPT_GAMECORNER_ROCKET_EXIT, TEXT_GAMECORNER_CLERK1, TEXT_GAMECORNER_CLERK2, TEXT_GAMECORNER_FISHING_GURU,
    TEXT_GAMECORNER_GENTLEMAN, TEXT_GAMECORNER_GYM_GUIDE, TEXT_GAMECORNER_POSTER, TEXT_GAMECORNER_ROCKET,
    TEXT_GAMECORNER_ROCKET_AFTER_BATTLE};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::GAMECORNER_ROCKET;
use poke_core::symbols::pokered_toggles::TOGGLE_GAME_CORNER_ROCKET;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP};
use super::{text_at, Flow, Script};

const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// ¥1000, the price of fifty coins.
const COIN_PRICE: [u8; 3] = [0x00, 0x10, 0x00];

/// `GameCornerMovement_Rocket_WalkAroundPlayer` and `..._WalkDirect`: he leaves east either way,
/// dropping a row round the player first unless the player is out of the row he walks along.
const WALK_AROUND_PLAYER: [u8; 9] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP,
    NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, END];
const WALK_DIRECT: [u8; 6] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT,
    NPC_MOVEMENT_RIGHT, END];

/// The block the poster hides, and the wall that stands in its place until the switch is found.
const STAIRS_BLOCK: u8 = 0x43;
const WALL_BLOCK: u8 = 0x2A;
const STAIRS_AT: (u8, u8) = (8, 2);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wGameCornerCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `GameCornerRocketText`, which ends in the battle he is beaten in.
    RocketEngaged,
    /// `GameCornerRocketBattleScript`, after the words he leaves on.
    RocketSpokeTo,
    /// `GameCornerClerk1Text`, which asks before it charges.
    Clerk1Asked,
    Clerk1Answered,
    FishingGuruAsked,
    Clerk2Asked,
    GentlemanAsked,
    /// `GameCornerPosterText`: the switch, then the wall sliding back.
    PosterSwitch,
    PosterSwitchDone,
    PosterOpening,
    PosterOpened,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.game_corner_select_lucky_slot_machine();
    set_rocket_hideout_door_tile(rt);
    rt.enable_auto_text_box_drawing();
    match rt.maps().game_corner.cur_script {
        SCRIPT_GAMECORNER_ROCKET_BATTLE => rocket_battle(rt),
        SCRIPT_GAMECORNER_ROCKET_EXIT => rocket_exit(rt),
        _ => Flow::Return,
    }
}

/// `GameCornerSetRocketHideoutDoorTile`: the map's own blocks have the staircase open, so the wall
/// in front of it is put back on every load until the switch has been found.
fn set_rocket_hideout_door_tile(rt: &mut Script) {
    if rt.check_and_reset_cur_map_loaded(1) && !rt.check_event(EVENT_FOUND_ROCKET_HIDEOUT) {
        rt.replace_tile_block(STAIRS_AT.0, STAIRS_AT.1, WALL_BLOCK);
    }
}

/// `GameCornerReenterMapAfterPlayerLoss`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().game_corner.cur_script = SCRIPT_GAMECORNER_DEFAULT;
    rt.set_cur_map_script(SCRIPT_GAMECORNER_DEFAULT);
    Flow::Return
}

/// `GameCornerRocketBattleScript`.
fn rocket_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.display_text_id(TEXT_GAMECORNER_ROCKET_AFTER_BATTLE).then(Label::RocketSpokeTo)
}

/// `GameCornerRocketExitScript`.
fn rocket_exit(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.hide_object(TOGGLE_GAME_CORNER_ROCKET);
    // The two map-loaded flags are set again so the next pass rolls a lucky machine and puts the
    // wall back, both of which the square he was standing on has just moved past.
    rt.set_cur_map_loaded(1);
    rt.set_cur_map_loaded(2);
    rt.maps().game_corner.cur_script = SCRIPT_GAMECORNER_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_GAMECORNER_CLERK1 => {
            rt.game_corner_draw_coin_box();
            rt.print_text(text_at(local::GameCornerClerk1Text::DoYouNeedSomeGameCoins)).then(Label::Clerk1Asked)
        }
        TEXT_GAMECORNER_FISHING_GURU => match rt.check_event(EVENT_GOT_10_COINS) {
            true => rt.print_text(text_at(local::GameCornerFishingGuruText::WinsComeAndGoText)).ret(),
            false => rt.print_text(text_at(local::GameCornerFishingGuruText::WantToPlayText))
                .then(Label::FishingGuruAsked),
        },
        TEXT_GAMECORNER_CLERK2 => match rt.check_event(EVENT_GOT_20_COINS_2) {
            true => rt.print_text(text_at(local::GameCornerClerk2Text::INeedMoreCoinsText)).ret(),
            false => rt.print_text(text_at(local::GameCornerClerk2Text::WantSomeCoinsText)).then(Label::Clerk2Asked),
        },
        TEXT_GAMECORNER_GENTLEMAN => match rt.check_event(EVENT_GOT_20_COINS) {
            true => rt.print_text(text_at(local::GameCornerGentlemanText::CloselyWatchTheReelsText)).ret(),
            false => rt.print_text(text_at(local::GameCornerGentlemanText::ThrowingMeOffText))
                .then(Label::GentlemanAsked),
        },
        TEXT_GAMECORNER_GYM_GUIDE => {
            let words = match rt.check_event(EVENT_BEAT_ERIKA) {
                true => sym::GameCornerGymGuideTheyOfferRarePokemonText,
                false => sym::GameCornerGymGuideChampInMakingText,
            };
            rt.print_text(text_at(words)).ret()
        }
        TEXT_GAMECORNER_ROCKET => {
            rt.print_text(text_at(local::GameCornerRocketText::ImGuardingThisPosterText)).then(Label::RocketEngaged)
        }
        TEXT_GAMECORNER_POSTER => {
            rt.set_do_not_wait_for_button_press(true);
            rt.print_text(text_at(local::GameCornerPosterText::SwitchBehindPosterText)).then(Label::PosterSwitch)
        }
        _ => return None,
    })
}

/// `GameCornerOopsForgotCoinCaseText`, which every coin handout falls back to.
fn forgot_coin_case(rt: &mut Script) -> Flow {
    rt.print_text(text_at(sym::GameCornerOopsForgotCoinCaseText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::RocketEngaged => {
            rt.save_end_battle_text(local::GameCornerRocketText::BattleEndText);
            rt.engage_map_trainer(rt.sprite_index(), 0);
            rt.clear_joy_held();
            rt.maps().game_corner.cur_script = SCRIPT_GAMECORNER_ROCKET_BATTLE;
            Flow::Return
        }
        // He walks out east, round the player when the player is standing in the row he walks along.
        Label::RocketSpokeTo => {
            rt.set_sprite_movement_bytes_to_ff(GAMECORNER_ROCKET);
            let direct = rt.y() == 6 || rt.x() == 8;
            let path: &[u8] = if direct { &WALK_DIRECT } else { &WALK_AROUND_PLAYER };
            rt.move_sprite(GAMECORNER_ROCKET, path);
            rt.maps().game_corner.cur_script = SCRIPT_GAMECORNER_ROCKET_EXIT;
            Flow::Return
        }
        Label::Clerk1Asked => rt.yes_no_choice().then(Label::Clerk1Answered),
        Label::Clerk1Answered => {
            use local::GameCornerClerk1Text as words;
            if !rt.chose_yes() {
                return rt.print_text(text_at(words::PleaseComePlaySometime)).ret();
            }
            if !rt.is_item_in_bag(ItemId::CoinCase) {
                return rt.print_text(text_at(words::DontHaveCoinCase)).ret();
            }
            if rt.has_9990_coins() {
                return rt.print_text(text_at(words::CoinCaseIsFull)).ret();
            }
            if !rt.has_enough_money(COIN_PRICE) {
                return rt.print_text(text_at(words::CantAffordTheCoins)).ret();
            }
            rt.subtract_money(COIN_PRICE);
            rt.add_coins(0x50);
            rt.game_corner_draw_coin_box();
            rt.print_text(text_at(words::ThanksHereAre50Coins)).ret()
        }
        Label::FishingGuruAsked => {
            use local::GameCornerFishingGuruText as words;
            if !rt.is_item_in_bag(ItemId::CoinCase) {
                return forgot_coin_case(rt);
            }
            if rt.has_9990_coins() {
                return rt.print_text(text_at(words::DontNeedMyCoinsText)).ret();
            }
            rt.add_coins(0x10);
            rt.set_event(EVENT_GOT_10_COINS);
            rt.set_do_not_wait_for_button_press(true);
            rt.print_text(text_at(words::Received10CoinsText)).ret()
        }
        Label::Clerk2Asked => {
            use local::GameCornerClerk2Text as words;
            if !rt.is_item_in_bag(ItemId::CoinCase) {
                return forgot_coin_case(rt);
            }
            if rt.has_9990_coins() {
                return rt.print_text(text_at(words::YouHaveLotsOfCoinsText)).ret();
            }
            rt.add_coins(0x20);
            rt.set_event(EVENT_GOT_20_COINS_2);
            rt.print_text(text_at(words::Received20CoinsText)).ret()
        }
        // He tests for equality rather than for enough room, so he pays out until the case holds
        // exactly 9990 coins and `AddBCD` saturates the rest.
        Label::GentlemanAsked => {
            use local::GameCornerGentlemanText as words;
            if !rt.is_item_in_bag(ItemId::CoinCase) {
                return forgot_coin_case(rt);
            }
            if rt.coins() == [0x99, 0x90] {
                return rt.print_text(text_at(words::YouGotYourOwnCoinsText)).ret();
            }
            rt.add_coins(0x20);
            rt.set_event(EVENT_GOT_20_COINS);
            rt.print_text(text_at(words::Received20CoinsText)).ret()
        }
        Label::PosterSwitch => {
            rt.play_sound(sounds::SFX_SWITCH);
            rt.wait_for_sound_to_finish().then(Label::PosterSwitchDone)
        }
        Label::PosterSwitchDone => rt.wait_for_sound_to_finish().then(Label::PosterOpening),
        Label::PosterOpening => {
            rt.play_sound(sounds::SFX_GO_INSIDE);
            rt.wait_for_sound_to_finish().then(Label::PosterOpened)
        }
        Label::PosterOpened => {
            rt.set_event(EVENT_FOUND_ROCKET_HIDEOUT);
            rt.replace_tile_block(STAIRS_AT.0, STAIRS_AT.1, STAIRS_BLOCK);
            Flow::Return
        }
    }
}
