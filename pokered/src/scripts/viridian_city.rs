//! `ViridianCity_Script`: the gym nobody can get into, the old man asleep across the road north, and
//! the catching lesson he gives once he has had his coffee.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI, EVENT_GOT_POKEDEX, EVENT_GOT_TM42,
    EVENT_VIRIDIAN_GYM_OPEN};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::VIRIDIANCITY_YOUNGSTER2;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::script::SpritePosition;
use crate::systems::overworld::sprites::SPRITE_FACING_DOWN;
use super::{text_at, Flow, Script};

/// `~(1 << BIT_EARTHBADGE)`: every badge but the Earth Badge, which is when Giovanni is back.
const ALL_BUT_EARTH: u8 = 0x7F;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wViridianCityCurScript`.
    pub cur_script: u8,
    /// The sprite the catching lesson's battle overwrites, saved across it.
    pub saved_sprite: Option<SpritePosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    GymLocked,
    OldManSleepyStopped,
    CatchTrainingDelay,
    CatchTrainingText,
    PlayerMovingDownDelay,
    /// `ViridianCityYoungster2Text`'s yes/no, and `ViridianCityOldManText`'s.
    Youngster2YesNo,
    Youngster2Answered,
    FisherGiveTm,
    OldManDelay,
    OldManYesNo,
    OldManAnswered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().viridian_city.cur_script {
        SCRIPT_VIRIDIANCITY_DEFAULT => match check_gym_open(rt) {
            Some(flow) => flow,
            None => check_got_pokedex(rt),
        },
        SCRIPT_VIRIDIANCITY_OLD_MAN_START_CATCH_TRAINING => start_catch_training(rt),
        SCRIPT_VIRIDIANCITY_OLD_MAN_END_CATCH_TRAINING => end_catch_training(rt),
        SCRIPT_VIRIDIANCITY_PLAYER_MOVING_DOWN => {
            if rt.simulated_joypad_states_index() != 0 {
                return Flow::Return;
            }
            rt.delay3().then(Label::PlayerMovingDownDelay)
        }
        _ => Flow::Return,
    }
}

/// `ViridianCityCheckGymOpenScript`: the gym's door is shut until every other badge is in, and a
/// player who walks up to it is told so and pushed back down.
fn check_gym_open(rt: &mut Script) -> Option<Flow> {
    if rt.check_event(EVENT_VIRIDIAN_GYM_OPEN) {
        return None;
    }
    if rt.badges() == ALL_BUT_EARTH {
        rt.set_event(EVENT_VIRIDIAN_GYM_OPEN);
        return None;
    }
    if (rt.x(), rt.y()) != (32, 8) {
        return None;
    }
    Some(rt.display_text_id(TEXT_VIRIDIANCITY_GYM_LOCKED).then(Label::GymLocked))
}

/// `ViridianCityCheckGotPokedexScript`: the sleeping man blocks the way north until the Pokédex is in.
fn check_got_pokedex(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_GOT_POKEDEX) || (rt.x(), rt.y()) != (19, 9) {
        return Flow::Return;
    }
    rt.display_text_id(TEXT_VIRIDIANCITY_OLD_MAN_SLEEPY).then(Label::OldManSleepyStopped)
}

/// `ViridianCityMovePlayerDownScript`.
fn move_player_down(rt: &mut Script) {
    rt.simulate_joypad_presses(vec![Joypad::DOWN]);
    rt.set_player_facing(SPRITE_FACING_DOWN);
    rt.joy_ignore(Joypad::empty());
    rt.maps().viridian_city.cur_script = SCRIPT_VIRIDIANCITY_PLAYER_MOVING_DOWN;
}

/// `ViridianCityOldManStartCatchTrainingScript`: a Weedle in an `OLD_MAN` battle, which Oak fights.
fn start_catch_training(rt: &mut Script) -> Flow {
    let at = rt.sprite_position(VIRIDIANCITY_YOUNGSTER2);
    rt.maps().viridian_city.saved_sprite = Some(at);
    rt.set_old_man_battle(true);
    rt.start_wild_battle(PokemonSpecies::Weedle, 5);
    rt.maps().viridian_city.cur_script = SCRIPT_VIRIDIANCITY_OLD_MAN_END_CATCH_TRAINING;
    Flow::Return
}

/// `ViridianCityOldManEndCatchTrainingScript`.
fn end_catch_training(rt: &mut Script) -> Flow {
    if let Some(at) = rt.maps().viridian_city.saved_sprite.take() {
        rt.set_sprite_position(VIRIDIANCITY_YOUNGSTER2, at);
    }
    rt.update_sprites();
    rt.delay3().then(Label::CatchTrainingDelay)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_VIRIDIANCITY_GAMBLER1 => {
            let returned = rt.badges() == ALL_BUT_EARTH || rt.check_event(EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI);
            let words = match returned {
                true => local::ViridianCityGambler1Text::GymLeaderReturnedText,
                false => local::ViridianCityGambler1Text::GymAlwaysClosedText,
            };
            rt.print_text(text_at(words)).ret()
        }
        TEXT_VIRIDIANCITY_YOUNGSTER2 => {
            rt.print_text(text_at(local::ViridianCityYoungster2Text::YouWantToKnowAboutText))
                .then(Label::Youngster2YesNo)
        }
        TEXT_VIRIDIANCITY_GIRL => {
            let words = match rt.check_event(EVENT_GOT_POKEDEX) {
                true => local::ViridianCityGirlText::WhenIGoShopText,
                false => local::ViridianCityGirlText::HasntHadHisCoffeeYetText,
            };
            rt.print_text(text_at(words)).ret()
        }
        TEXT_VIRIDIANCITY_OLD_MAN_SLEEPY => {
            rt.print_text(text_at(local::ViridianCityOldManSleepyText::PrivatePropertyText))
                .then(Label::OldManSleepyStopped)
        }
        TEXT_VIRIDIANCITY_FISHER => {
            if rt.check_event(EVENT_GOT_TM42) {
                return Some(rt.print_text(text_at(local::ViridianCityFisherText::TM42ExplanationText)).ret());
            }
            rt.print_text(text_at(local::ViridianCityFisherText::YouCanHaveThisText)).then(Label::FisherGiveTm)
        }
        TEXT_VIRIDIANCITY_OLD_MAN => {
            rt.print_text(text_at(local::ViridianCityOldManText::HadMyCoffeeNowText)).then(Label::OldManDelay)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        // The gym's text and the sleeping man's both end the same way, and the gym's leaves the
        // pass to run the Pokédex check the cartridge falls through to.
        Label::GymLocked => {
            rt.clear_joy_held();
            move_player_down(rt);
            check_got_pokedex(rt)
        }
        Label::OldManSleepyStopped => {
            rt.clear_joy_held();
            move_player_down(rt);
            Flow::Return
        }
        Label::PlayerMovingDownDelay => {
            rt.maps().viridian_city.cur_script = SCRIPT_VIRIDIANCITY_DEFAULT;
            Flow::Return
        }

        Label::CatchTrainingDelay => {
            rt.joy_ignore(Joypad::empty());
            rt.display_text_id(TEXT_VIRIDIANCITY_OLD_MAN_YOU_NEED_TO_WEAKEN_THE_TARGET).then(Label::CatchTrainingText)
        }
        Label::CatchTrainingText => {
            rt.set_old_man_battle(false);
            rt.joy_ignore(Joypad::empty());
            rt.maps().viridian_city.cur_script = SCRIPT_VIRIDIANCITY_DEFAULT;
            Flow::Return
        }
        Label::Youngster2YesNo => rt.yes_no_choice().then(Label::Youngster2Answered),
        Label::Youngster2Answered => {
            let words = match rt.chose_yes() {
                true => local::ViridianCityYoungster2Text::CaterpieAndWeedleDescriptionText,
                false => local::ViridianCityYoungster2Text::OkThenText,
            };
            rt.print_text(text_at(words)).ret()
        }
        Label::FisherGiveTm => {
            if !rt.give_item(ItemId::Tm42DreamEater, 1) {
                return rt.print_text(text_at(local::ViridianCityFisherText::TM42NoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_TM42);
            rt.print_text(text_at(local::ViridianCityFisherText::ReceivedTM42Text)).ret()
        }
        Label::OldManDelay => rt.delay_frames(2).then(Label::OldManYesNo),
        Label::OldManYesNo => rt.yes_no_choice().then(Label::OldManAnswered),
        // The old man asks whether the player is in a hurry, so yes is the answer that refuses him.
        Label::OldManAnswered => {
            if rt.chose_yes() {
                return rt.print_text(text_at(local::ViridianCityOldManText::TimeIsMoneyText)).ret();
            }
            rt.maps().viridian_city.cur_script = SCRIPT_VIRIDIANCITY_OLD_MAN_START_CATCH_TRAINING;
            rt.print_text(text_at(local::ViridianCityOldManText::KnowHowToCatchPokemonText)).ret()
        }
    }
}
