//! `RedsHouse1F_Script`: Mom, who sends the player off to Oak and later heals the party, and the TV,
//! which only shows its film from the front.

use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_REDSHOUSE1F_MOM, TEXT_REDSHOUSE1F_TV};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `RedsHouse1FMomHealScript` after `RedsHouse1FMomYouShouldRestText`, after `GBFadeOutToWhite`,
    /// after `.next`'s wait for the jingle, and after `GBFadeInFromWhite`.
    HealFadeOut,
    HealJingle,
    HealJingleOver,
    HealFadedIn,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_REDSHOUSE1F_MOM => {
            if !rt.globals().got_starter {
                return Some(rt.print_text(text_at(local::RedsHouse1FMomText::WakeUpText)).ret());
            }
            rt.print_text(text_at(sym::RedsHouse1FMomYouShouldRestText)).then(Label::HealFadeOut)
        }
        TEXT_REDSHOUSE1F_TV => {
            let words = match rt.player_facing() {
                SPRITE_FACING_UP => local::RedsHouse1FTVText::StandByMeMovieText,
                _ => local::RedsHouse1FTVText::WrongSideText,
            };
            rt.print_text(text_at(words)).ret()
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::HealFadeOut => rt.gb_fade_out_to_white().then(Label::HealJingle),
        // `ReloadMapData` is loading, done behind the white screen.
        Label::HealJingle => {
            rt.heal_party();
            rt.play_new_sound(sounds::MUSIC_PKMN_HEALED.id);
            rt.wait_while_channel_plays(0, sounds::MUSIC_PKMN_HEALED.id).then(Label::HealJingleOver)
        }
        Label::HealJingleOver => {
            let song = rt.map_music_sound_id();
            rt.play_new_sound(song);
            rt.gb_fade_in_from_white().then(Label::HealFadedIn)
        }
        Label::HealFadedIn => rt.print_text(text_at(sym::RedsHouse1FMomLookingGreatText)).ret(),
    }
}
