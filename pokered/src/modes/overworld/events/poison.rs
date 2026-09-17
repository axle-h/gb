//! `ApplyOutOfBattlePoisonDamage` on the step, and `SafariZoneGameOver`.

use poke_core::symbols::pokered_events::EVENT_SAFARI_GAME_OVER;
use poke_core::symbols::pokered_map_scripts::SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI;
use poke_core::symbols::{pokered_symbols as sym, DmgBank};
use poke_core::text_script::TextBuffer;
use crate::audio::data::{sounds, AudioBank, Sound, SoundId};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::systems::events::day_care::increment_day_care_mon_exp;
use crate::systems::events::poison::{any_party_alive, any_poisoned, poison_mon as poison_one};
use super::super::script::{Flow, Routine, Script, Then, TEXT_BLACKED_OUT, TEXT_MON_FAINTED, TEXT_SAFARI_GAME_OVER};
use super::super::Overworld;
use super::{print, Label};

/// `ChangeBGPalColor0_4Frames`.
const POISON_FLASH_FRAMES: u8 = 4;

/// The part of `ApplyOutOfBattlePoisonDamage` that takes no frames, the day care's step: whether the
/// rest of it has anything to show, which is a poisoned mon or a party with nothing standing on a
/// fourth step.
pub(in crate::modes::overworld) fn poison_step_takes_frames(ow: &mut Overworld, ctx: &mut Ctx) -> bool {
    if ow.scripted || ctx.world.party.is_empty() {
        return false;
    }
    if let Some(mon) = &mut ctx.world.day_care {
        mon.mon.exp = increment_day_care_mon_exp(mon.mon.exp);
    }
    ow.rt.step_counter & 3 == 0 && (any_poisoned(&ctx.world.party) || !any_party_alive(&ctx.world.party))
}

/// `.applyDamageLoop` from `slot`, stopping at each mon that faints for its text.
pub(super) fn poison_mon(s: &mut Script, slot: u8) -> Flow {
    for slot in slot as usize..s.ctx.world.party.len() {
        if poison_one(&mut s.ctx.world.party[slot].mon) == Some(true) {
            let nick = s.ctx.world.party[slot].nick.clone();
            s.ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
            s.joy_ignore(Joypad::empty());
            s.enable_auto_text_box_drawing();
            return Then::call(Routine::DisplayTextId(TEXT_MON_FAINTED)).then(Label::PoisonMon(slot as u8 + 1));
        }
    }
    if any_poisoned(&s.ctx.world.party) {
        s.ctx.screen.effects.bgp |= 0b10;
        return s.delay_frames(POISON_FLASH_FRAMES).then(Label::PoisonFlashed);
    }
    blackout_check(s)
}

pub(super) fn flashed(s: &mut Script) -> Flow {
    s.ctx.screen.effects.bgp &= 0b1111_1100;
    s.play_sound(sounds::SFX_POISONED);
    blackout_check(s)
}

/// `AnyPartyAlive`, and `TEXT_BLACKED_OUT` when nothing is.
fn blackout_check(s: &mut Script) -> Flow {
    if any_party_alive(&s.ctx.world.party) {
        return Flow::Return;
    }
    s.enable_auto_text_box_drawing();
    Then::call(Routine::DisplayTextId(TEXT_BLACKED_OUT)).then(Label::PoisonBlackedOut)
}

pub(super) fn blacked_out(s: &mut Script) -> Flow {
    s.ow.rt.battle_over_or_blackout = true;
    s.ow.rt.out_of_battle_blackout = true;
    Flow::Return
}

/// `SafariZoneGameOver`: the PA's chime, the text, and the warp set up for the loop to take.
pub(super) fn safari_zone_game_over(s: &mut Script) -> Flow {
    s.enable_auto_text_box_drawing();
    s.ctx.audio.fade_out(0);
    s.play_sound(SoundId::STOP_ALL_MUSIC);
    let DmgBank::ROM { bank } = sym::SFX_Safari_Zone_PA.bank else { unreachable!("a sound is in ROM") };
    let bank = AudioBank::from_rom_bank(bank).expect("an audio bank");
    s.play_music(Sound { bank, id: sounds::SFX_SAFARI_ZONE_PA });
    Then::call(Routine::DisplayTextId(TEXT_SAFARI_GAME_OVER)).then(Label::SafariGameOverDone)
}

/// The rest of `SafariZoneGameOver` after its text: the gate is warped to with its leaving script
/// already armed, so the worker takes the balls back on the first pass there.
pub(super) fn safari_game_over_done(s: &mut Script) -> Flow {
    s.maps().safari_zone_gate.cur_script = SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI;
    s.set_event(EVENT_SAFARI_GAME_OVER);
    Flow::Return
}

/// `SafariGameOverText`.
pub(super) fn safari_game_over_text(s: &mut Script) -> Flow {
    if s.ctx.world.safari_balls == 0 {
        return print(sym::GameOverText).ret();
    }
    print(sym::TimesUpText).then(Label::SafariGameOverTextDone)
}

pub(super) fn safari_game_over_text_done(_s: &mut Script) -> Flow {
    print(sym::GameOverText).ret()
}
