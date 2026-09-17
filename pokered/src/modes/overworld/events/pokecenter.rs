//! `DisplayPokemonCenterDialogue_` and `AnimateHealingMachine`: the nurse's welcome, the question
//! asked only the first time, HEAL/CANCEL, the balls going onto the machine one a jingle, the flashing
//! and the bow.
//!
//! Exact: every `DelayFrames`, the thirty frames a ball, the eight flashes ten frames apart, the waits
//! for the fade and for the jingle, the nurse's `Delay3` turn and her twenty-frame bow. Left out as
//! loading: the machine's tiles copied into VRAM.

use poke_core::map_objects::map_song;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::pokered_symbols as sym;
use crate::audio::data::{sounds, AudioBank, SoundId};
use crate::gfx::layers::Object;
use crate::gfx::tiles::V_CHARS0;
use crate::mode::{Mode, Outcome};
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::events::heal_party::{heal_party, set_last_blackout_map};
use super::super::script::{Block, Flow, Script, Then};
use super::{print, Label};

/// The nurse's `wSprite01StateData1ImageIndex` facing the machine, and bowing.
const NURSE_TURNED: u8 = 0x18;
const NURSE_BOWING: u8 = 0x14;
/// `PokeCenterOAMData`'s first object is `wShadowOAMSprite33`.
const MACHINE_OBJECT: usize = 33;
const BALL_FRAMES: u8 = 30;
const FLASHES: u8 = 8;
const FLASH_FRAMES: u8 = 10;
const FLASH_XOR: u8 = 0x28;
const AFTER_JINGLE_FRAMES: u8 = 32;

pub(super) fn pokemon_center(s: &mut Script) -> Flow {
    s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
    print(sym::PokemonCenterWelcomeText).then(Label::NurseWelcomed)
}

/// `BIT_USED_POKECENTER` read and set together: the question is asked once in a game.
pub(super) fn welcomed(s: &mut Script) -> Flow {
    if std::mem::replace(&mut s.ctx.world.used_pokecenter, true) {
        return Flow::Jump(Label::NurseAsk.into());
    }
    print(sym::ShallWeHealYourPokemonText).then(Label::NurseAsk)
}

/// `YesNoChoicePokeCenter`.
pub(super) fn ask(s: &mut Script) -> Flow {
    s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
    let menu = TwoOptionMenu::new(TwoOptionMenuId::HealCancel, (11, 6), false);
    Then::block(Block::Mode(Box::new(Mode::TwoOptionMenu(menu)))).then(Label::NurseAnswered)
}

pub(super) fn answered(s: &mut Script) -> Flow {
    if let Some(saved) = s.ow.rt.saved_screen.take() {
        s.ctx.screen.ui = saved;
    }
    if s.ow.rt.outcome != Some(Outcome::Chosen(0)) {
        return Flow::Jump(Label::NurseFarewell.into());
    }
    let location = &mut s.ctx.world.location;
    location.last_blackout_map = set_last_blackout_map(location.map, location.last_map, location.last_blackout_map);
    print(sym::NeedYourPokemonText).then(Label::NurseTurns)
}

pub(super) fn turns(s: &mut Script) -> Flow {
    s.ow.sprites[1].image_index = NURSE_TURNED;
    s.delay3().then(Label::HealingMachineMusicStopped)
}

/// `HealParty`, then `AnimateHealingMachine` up to its wait for the music to fade.
pub(super) fn music_stopped(s: &mut Script) -> Flow {
    heal_party(&mut s.ctx.world.party);
    let tiles = rom_slice(sym::PokeCenterFlashingMonitorAndHealBall);
    // Three tiles from a file of two, as the cartridge copies: nothing draws the third.
    s.ctx.screen.tiles.load(V_CHARS0 + 0x7C, &tiles[..3 * TILE_BYTES]);
    s.ow.rt.sprites_frozen = true;
    s.ow.rt.events.saved_obp1 = s.ctx.screen.effects.obp1;
    s.ctx.screen.effects.obp1 = 0xE0;
    copy_healing_machine_oam(s, 0);
    s.ctx.audio.stop_music(4);
    Then::block(Block::MusicStopped).then(Label::HealingMachineBall(0))
}

/// `CopyHealingMachineOAM`: object `n` of `PokeCenterOAMData`, the monitor first.
fn copy_healing_machine_oam(s: &mut Script, n: usize) {
    let data = rom_slice(sym::PokeCenterOAMData + 4 * n as u16);
    let objects = &mut s.ctx.screen.sprites;
    objects.resize(40, Object { y: 160, ..Object::default() });
    objects[MACHINE_OBJECT + n] = Object { y: data[0], x: data[1], tile: data[2], attributes: data[3] };
}

/// `.partyLoop`: a ball and its sound for each mon, thirty frames apart.
pub(super) fn ball(s: &mut Script, n: u8) -> Flow {
    if (n as usize) < s.ctx.world.party.len().max(1) {
        copy_healing_machine_oam(s, 1 + n as usize);
        s.play_sound(sounds::SFX_HEALING_MACHINE);
        return s.delay_frames(BALL_FRAMES).then(Label::HealingMachineBall(n + 1));
    }
    let bank = s.ctx.audio.bank();
    s.ow.rt.events.saved_audio_bank = Some(bank);
    if bank == AudioBank::Three {
        s.ctx.audio.play_new_sound(SoundId::STOP_ALL_MUSIC);
        s.ctx.audio.set_bank(sounds::MUSIC_PKMN_HEALED.bank);
    }
    s.ctx.audio.play_new_sound(sounds::MUSIC_PKMN_HEALED.id);
    Flow::Jump(Label::HealingMachineFlash(0).into())
}

/// `FlashSprite8Times`, then the wait while the jingle plays.
pub(super) fn flash(s: &mut Script, n: u8) -> Flow {
    if n < FLASHES {
        s.ctx.screen.effects.obp1 ^= FLASH_XOR;
        return s.delay_frames(FLASH_FRAMES).then(Label::HealingMachineFlash(n + 1));
    }
    Then::block(Block::ChannelPlaying { channel: 0, id: sounds::MUSIC_PKMN_HEALED.id.0 }).then(Label::HealingMachineJingleOver)
}

pub(super) fn jingle_over(s: &mut Script) -> Flow {
    s.delay_frames(AFTER_JINGLE_FRAMES).then(Label::HealingMachineDone)
}

/// The end of `AnimateHealingMachine`, and the map's song started again from the top.
pub(super) fn machine_done(s: &mut Script) -> Flow {
    s.ctx.screen.effects.obp1 = s.ow.rt.events.saved_obp1;
    s.ow.rt.sprites_frozen = false;
    s.update_sprites();
    let audio = &mut *s.ctx.audio;
    audio.fade_out(0);
    if let Some(bank) = s.ow.rt.events.saved_audio_bank.take() {
        audio.set_bank(bank);
    }
    let (song, _) = map_song(s.ctx.world.location.map);
    audio.set_last_music_sound_id(SoundId(song));
    audio.play_new_sound(SoundId(song));
    print(sym::PokemonFightingFitText).then(Label::NurseBows)
}

pub(super) fn bows(s: &mut Script) -> Flow {
    s.ow.sprites[1].image_index = NURSE_BOWING;
    s.delay_frames(NURSE_BOWING).then(Label::NurseFarewell)
}

pub(super) fn farewell(_s: &mut Script) -> Flow {
    print(sym::PokemonCenterFarewellText).then(Label::NurseDone)
}
