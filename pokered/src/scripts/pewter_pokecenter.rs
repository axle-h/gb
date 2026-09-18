//! `PewterPokecenter_Script`: Jigglypuff, who stops the music, sings, and turns round on the spot for
//! as long as the song lasts.

use poke_core::symbols::pokered_local_labels::PewterPokecenterJigglypuffText;
use poke_core::symbols::pokered_map_scripts::TEXT_PEWTERPOKECENTER_JIGGLYPUFF;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_LEFT, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `wSprite03`: Jigglypuff's slot.
const JIGGLYPUFF: u8 = 3;
/// `.FacingDirections`: the picture index each way round, in the order it turns.
const FACING_DIRECTIONS: [u8; 4] =
    [0x30 | SPRITE_FACING_DOWN, 0x30 | SPRITE_FACING_LEFT, 0x30 | SPRITE_FACING_UP, 0x30 | SPRITE_FACING_RIGHT];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After `.Text`, and after the 32 frames of silence.
    StopMusic,
    Sing,
    /// `.spinMovementLoop` at the `n`th turn, counted from the way Jigglypuff was facing.
    Spin(u8),
    /// After the 48 frames once the song has ended.
    SongOver,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_PEWTERPOKECENTER_JIGGLYPUFF {
        return None;
    }
    rt.set_do_not_wait_for_button_press(true);
    Some(rt.print_text(text_at(PewterPokecenterJigglypuffText::Text)).then(Label::StopMusic))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StopMusic => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.delay_frames(32).then(Label::Sing)
        }
        Label::Sing => {
            // `.findMatchingFacingDirectionLoop` has no end: a picture not in the list would run it
            // on through WRAM, which a Jigglypuff on screen and spoken to never has.
            let image = rt.sprite(JIGGLYPUFF).image_index;
            let start = FACING_DIRECTIONS.iter().position(|&index| index == image).unwrap_or(0) as u8;
            rt.play_music(sounds::MUSIC_JIGGLYPUFF_SONG);
            spin(rt, start)
        }
        Label::Spin(n) => {
            if rt.channel_sound_id(0) | rt.channel_sound_id(1) != 0 {
                return spin(rt, n);
            }
            rt.delay_frames(48).then(Label::SongOver)
        }
        Label::SongOver => rt.play_default_music().ret(),
    }
}

/// One turn of `.spinMovementLoop`: the picture set, the list rotated, and 24 frames.
fn spin(rt: &mut Script, n: u8) -> Flow {
    rt.set_sprite_image_index(JIGGLYPUFF, FACING_DIRECTIONS[n as usize % FACING_DIRECTIONS.len()]);
    rt.delay_frames(24).then(Label::Spin(n.wrapping_add(1)))
}
