//! The music and sound effect interpreter: `audio/engine_(1|2|3).asm`, `home/audio.asm`,
//! `home/fade_audio.asm` and `audio/low_health_alarm.asm`.
//!
//! Eight software channels play on the Game Boy's four hardware ones: 1 to 4 are music and 5 to 8
//! the sound effect that pre-empts it, one for one. A channel is a pointer into a byte stream in
//! the cartridge. One tick — one VBlank, so one [`AudioEngine::frame`] — advances every channel by
//! a frame: commands that take no time (tempo, octave, duty cycle, a call, a loop) are executed
//! until a note or a rest is reached, and that note's length, in 8.8 fixed point frames, is how
//! long the channel then waits.
//!
//! Exact, all of it: note length and the delay counter's fractional part, the frequency table and
//! its octave shifts, vibrato, the pitch slide including its borrow bug, the duty cycle rotation,
//! the sound effect priority rules, the fade-out's volume steps and the cry modifiers. The frame
//! counts are exact too, because the engine ticks once a frame and nothing in it waits on anything
//! else.
//!
//! What comes out is a [`Write`] per register the cartridge would have written, in order, so a
//! backend renders the same sound without the engine knowing what a backend is, and no oscillator
//! state is held here to end up in a save.
//!
//! The cartridge holds three copies of this, one per audio ROM bank. `AUDIO_2` is the one that
//! differs, in three places, all about battle sound effects and the low health alarm; [`AudioBank`]
//! is the three-way flag and each difference is commented where it is.

use serde::{Deserialize, Serialize};
use super::data::{
    AudioBank, Cry, Sound, SoundHeader, SoundId, BATTLE_SFX_END, BATTLE_SFX_START, CRY_SFX_END,
    CRY_SFX_START, NOISE_INSTRUMENTS_END,
};
use super::Write;

const NUM_CHANNELS: usize = 8;
const CHAN3: usize = 2;
const CHAN4: usize = 3;
const CHAN5: usize = 4;
const CHAN7: usize = 6;
const CHAN8: usize = 7;

// wChannelFlags1
const BIT_PERFECT_PITCH: u8 = 0;
const BIT_SOUND_CALL: u8 = 1;
const BIT_NOISE_OR_SFX: u8 = 2;
const BIT_VIBRATO_DIRECTION: u8 = 3;
const BIT_PITCH_SLIDE_ON: u8 = 4;
const BIT_PITCH_SLIDE_DECREASING: u8 = 5;
const BIT_ROTATE_DUTY_CYCLE: u8 = 6;
// wChannelFlags2, which has only the one
const BIT_EXECUTE_MUSIC: u8 = 0;
const BIT_MUTE_AUDIO: u8 = 7;
const BIT_LOW_HEALTH_ALARM: u8 = 7;
const LOW_HEALTH_TIMER_MASK: u8 = 0b0111_1111;
const DISABLE_LOW_HEALTH_ALARM: u8 = 0xFF;

// The command bytes, from `macros/scripts/audio.asm`.
const PITCH_SWEEP_CMD: u8 = 0x10;
const SFX_NOTE_CMD: u8 = 0x20;
const DRUM_NOTE_CMD: u8 = 0xB0;
const REST_CMD: u8 = 0xC0;
const NOTE_TYPE_CMD: u8 = 0xD0;
const OCTAVE_CMD: u8 = 0xE0;
const TOGGLE_PERFECT_PITCH_CMD: u8 = 0xE8;
const VIBRATO_CMD: u8 = 0xEA;
const PITCH_SLIDE_CMD: u8 = 0xEB;
const DUTY_CYCLE_CMD: u8 = 0xEC;
const TEMPO_CMD: u8 = 0xED;
const STEREO_PANNING_CMD: u8 = 0xEE;
const UNKNOWNMUSIC0XEF_CMD: u8 = 0xEF;
const VOLUME_CMD: u8 = 0xF0;
const EXECUTE_MUSIC_CMD: u8 = 0xF8;
const DUTY_CYCLE_PATTERN_CMD: u8 = 0xFC;
const SOUND_CALL_CMD: u8 = 0xFD;
const SOUND_LOOP_CMD: u8 = 0xFE;
const SOUND_RET_CMD: u8 = 0xFF;

// The sound registers, by the names `hardware.inc` gives them.
const R_AUD1SWEEP: u16 = 0xFF10;
const R_AUD1ENV: u16 = 0xFF12;
const R_AUD1HIGH: u16 = 0xFF14;
const R_AUD2ENV: u16 = 0xFF17;
const R_AUD2HIGH: u16 = 0xFF19;
const R_AUD3ENA: u16 = 0xFF1A;
const R_AUD3LEVEL: u16 = 0xFF1C;
const R_AUD4ENV: u16 = 0xFF21;
const R_AUD4GO: u16 = 0xFF23;
const R_AUDVOL: u16 = 0xFF24;
const R_AUDTERM: u16 = 0xFF25;
const R_AUDENA: u16 = 0xFF26;
const AUD3WAVERAM: u16 = 0xFF30;
const AUD3WAVE_SIZE: u16 = 16;
const AUD1SWEEP_DOWN: u8 = 0x08;
const AUD1HIGH_LENGTH_ON: u8 = 0x40;
const AUD3ENA_ON: u8 = 0x80;
const AUDENA_ON: u8 = 0x80;

/// `REG_DUTY_SOUND_LEN`, `REG_VOLUME_ENVELOPE`, `REG_FREQUENCY_LO`: the offset of a register from
/// its hardware channel's base.
const REG_DUTY_SOUND_LEN: u16 = 1;
const REG_VOLUME_ENVELOPE: u16 = 2;
const REG_FREQUENCY_LO: u16 = 3;

/// `AudioN_HWChannelBaseAddresses`, which the eight software channels index four apart, so channel
/// 5 shares hardware channel 1 with channel 1.
const HW_CHANNEL_BASE: [u16; 4] = [0x10, 0x15, 0x1A, 0x1F];
/// `AudioN_HWChannelEnableMasks`; the disable masks are their complements.
const HW_CHANNEL_ENABLE_MASK: [u8; 4] = [0b0001_0001, 0b0010_0010, 0b0100_0100, 0b1000_1000];

/// The sound registers as the engine last left them. Three are read back — `rAUDTERM` to turn one
/// channel's output on or off, `rAUDVOL` to fade, and a channel's duty/length register to rotate
/// its duty cycle — so what was written has to be remembered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Registers {
    bytes: Vec<u8>,
}

impl Default for Registers {
    fn default() -> Self {
        Self { bytes: vec![0; 0x30] }
    }
}

/// What a register reads back as whatever was written to it: the bits the hardware does not keep
/// read as ones, `$FF10` to `$FF26`. This matters in one place and matters a lot there — the sound
/// length is write-only, so `ApplyDutyCyclePattern`, which reads the duty and length register to
/// put the next duty cycle in it, writes the length back as `$3F` every frame it rotates.
const READ_MASK: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10 to NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // NR21 to NR24, with the gap the cartridge never writes
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30 to NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // NR41 to NR44, with its gap
    0x00, 0x00, 0x70, // NR50, NR51, NR52
];

impl Registers {
    fn get(&self, address: u16) -> u8 {
        let value = self.bytes[address as usize - 0xFF10];
        match READ_MASK.get(address as usize - 0xFF10) {
            Some(mask) => value | mask,
            None => value,
        }
    }

    fn set(&mut self, address: u16, value: u8) {
        self.bytes[address as usize - 0xFF10] = value;
    }
}

/// One software channel: the `wChannel*` arrays, one element of each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ChannelState {
    command_pointer: u16,
    return_address: u16,
    sound_id: u8,
    flags1: u8,
    flags2: u8,
    duty_cycle: u8,
    duty_cycle_pattern: u8,
    vibrato_delay_counter: u8,
    vibrato_extent: u8,
    vibrato_rate: u8,
    frequency_low: u8,
    vibrato_delay_counter_reload_value: u8,
    pitch_slide_length_modifier: u8,
    pitch_slide_frequency_steps: u8,
    pitch_slide_frequency_steps_fractional_part: u8,
    pitch_slide_current_frequency_fractional_part: u8,
    pitch_slide_current_frequency_high_byte: u8,
    pitch_slide_current_frequency_low_byte: u8,
    pitch_slide_target_frequency_high_byte: u8,
    pitch_slide_target_frequency_low_byte: u8,
    note_delay_counter: u8,
    loop_counter: u8,
    note_speed: u8,
    note_delay_counter_fractional_part: u8,
    octave: u8,
    volume: u8,
}

impl ChannelState {
    fn test(&self, flag: u8) -> bool {
        self.flags1 & 1 << flag != 0
    }

    fn set(&mut self, flag: u8) {
        self.flags1 |= 1 << flag;
    }

    fn res(&mut self, flag: u8) {
        self.flags1 &= !(1 << flag);
    }

    fn executing_music(&self) -> bool {
        self.flags2 & 1 << BIT_EXECUTE_MUSIC != 0
    }
}

/// Whether the command just executed took no time, so the next one is read at once, or was a note
/// and the channel's work for this frame is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Next,
    Done,
}

/// How `AudioN_note_length` left: either it returned to `AudioN_sfx_note` with the sound length in
/// `a`, or it fell through into `AudioN_note_pitch` carrying the command byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteLength {
    Returned(u8),
    Pitch(u8),
}

/// The eight software channels, the globals around them, and the registers they write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioEngine {
    /// `wAudioROMBank`: which of the three copies of the engine is running, and so which bank every
    /// header and command stream is read from.
    bank: AudioBank,
    /// `wAudioSavedROMBank`, which a fade-out puts back when it finishes.
    saved_bank: AudioBank,
    channels: [ChannelState; NUM_CHANNELS],
    sound_id: u8,
    new_sound_id: u8,
    last_music_sound_id: u8,
    /// `wMusicTempo` and `wSfxTempo`, stored big endian as the `tempo` command writes them.
    music_tempo: [u8; 2],
    sfx_tempo: [u8; 2],
    music_wave_instrument: u8,
    sfx_wave_instrument: u8,
    stereo_panning: u8,
    saved_volume: u8,
    disable_channel_output_when_sfx_ends: u8,
    mute_audio_and_pause_music: u8,
    unused_music_byte: u8,
    /// `wFrequencyModifier` and `wTempoModifier`: the cry's detune and stretch.
    frequency_modifier: u8,
    tempo_modifier: u8,
    low_health_alarm: u8,
    audio_fade_out_control: u8,
    audio_fade_out_counter: u8,
    audio_fade_out_counter_reload_value: u8,
    /// `wStatusFlags2`'s `BIT_NO_AUDIO_FADE_OUT`, which the chunk that owns that byte sets. While
    /// it is set nothing restores the master volume, which is how a screen fade keeps the sound
    /// down.
    pub no_audio_fade_out: bool,
    registers: Registers,
    #[serde(skip)]
    out: Vec<Write>,
}

impl Default for AudioBank {
    fn default() -> Self {
        AudioBank::One
    }
}

impl AudioEngine {
    pub fn new(bank: AudioBank) -> Self {
        Self { bank, saved_bank: bank, ..Default::default() }
    }

    pub fn bank(&self) -> AudioBank {
        self.bank
    }

    /// One VBlank's worth: the fade, then, on `AUDIO_2` only, the low health alarm, then every
    /// channel. The order is `VBlank`'s own.
    pub fn frame(&mut self) -> Vec<Write> {
        self.fade_out_audio();
        if self.bank == AudioBank::Two {
            self.music_do_low_health_alarm();
        }
        self.update_music();
        self.take_writes()
    }

    /// Everything written since the last take, which is what a sound started between two frames
    /// leaves behind for the next one to carry.
    pub fn take_writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.out)
    }

    /// `PlayMusic`: the song and the bank it lives in, with any fade abandoned.
    pub fn play_music(&mut self, sound: Sound) {
        self.new_sound_id = sound.id.0;
        self.audio_fade_out_control = 0;
        self.bank = sound.bank;
        self.saved_bank = sound.bank;
        self.play_sound(sound.id);
    }

    /// `PlaySound`: what every caller in the game reaches for. A sound effect plays at once; music
    /// waits for a fade-out that is already running to finish, and is what plays when it does.
    pub fn play_sound(&mut self, id: SoundId) {
        if self.new_sound_id != 0 {
            for c in CHAN5..NUM_CHANNELS {
                self.channels[c].sound_id = 0;
            }
        }
        if self.audio_fade_out_control != 0 {
            if self.new_sound_id == 0 {
                return;
            }
            self.new_sound_id = 0;
            if self.last_music_sound_id != 0xFF {
                // Hand the id to the fade, which plays it once the volume has reached zero.
                self.last_music_sound_id = id.0;
                self.audio_fade_out_counter_reload_value = self.audio_fade_out_control;
                self.audio_fade_out_counter = self.audio_fade_out_control;
                self.audio_fade_out_control = id.0;
                return;
            }
            // The music was stopped, so there is nothing to fade: start the new sound now.
            self.audio_fade_out_control = 0;
        }
        self.new_sound_id = 0;
        self.engine_play_sound(id);
    }

    /// `PlayCry`: `GetCryData` picks the base cry and sets the two modifiers, then it is an
    /// ordinary sound. `index` is the cartridge's internal species index, one-based.
    pub fn play_cry(&mut self, index: u8) {
        let cry = Cry::of_species_index(index);
        self.frequency_modifier = cry.frequency_modifier;
        self.tempo_modifier = cry.tempo_modifier;
        self.play_sound(cry.sound);
    }

    /// `WaitForSoundToFinish`'s condition: channels 5, 6 and 8 are quiet, channel 7 not being
    /// looked at because a cry parks it on a `sound_ret` that never clears.
    pub fn sound_finished(&self) -> bool {
        if self.low_health_alarm & 1 << BIT_LOW_HEALTH_ALARM != 0 {
            return true;
        }
        [CHAN5, CHAN5 + 1, CHAN8].iter().all(|&c| self.channels[c].sound_id == 0)
    }

    /// `wLowHealthAlarm`. Setting it arms the alarm on the next tick of `AUDIO_2`; clearing it asks
    /// for the silencing tone, which is what `DISABLE_LOW_HEALTH_ALARM` means.
    pub fn set_low_health_alarm(&mut self, on: bool) {
        self.low_health_alarm = if on { 1 << BIT_LOW_HEALTH_ALARM } else { DISABLE_LOW_HEALTH_ALARM };
    }

    /// `wAudioFadeOutControl`: how many frames each step of the fade lasts. The sound it is given
    /// plays when the fade reaches silence.
    pub fn fade_out(&mut self, frames: u8) {
        self.audio_fade_out_control = frames;
    }

    /// `AudioN_OverwriteChannelPointer`: `Music_RivalAlternateStart` and the Poké Flute start a
    /// song and then move a channel to different data. Nothing else in the game edits a pointer.
    pub fn overwrite_channel_pointer(&mut self, channel: usize, address: u16) {
        self.channels[channel].command_pointer = address;
    }

    /// `.playChannel`: a channel back to nothing but its three counters. The fractional part of the
    /// note delay, the octave and the volume are not among the arrays it clears, so they carry over
    /// from whatever played last.
    fn reset_channel(&mut self, c: usize) {
        let kept = self.channels[c];
        self.channels[c] = ChannelState {
            loop_counter: 1,
            note_delay_counter: 1,
            note_speed: 1,
            note_delay_counter_fractional_part: kept.note_delay_counter_fractional_part,
            octave: kept.octave,
            volume: kept.volume,
            ..ChannelState::default()
        };
    }

    fn write_register(&mut self, address: u16, value: u8) {
        self.registers.set(address, value);
        if let Some(write) = Write::decode(address, value) {
            self.out.push(write);
        }
    }

    /// `AudioN_GetRegisterPointer`: register `reg` of the hardware channel software channel `c`
    /// plays on.
    fn register_pointer(c: usize, reg: u16) -> u16 {
        0xFF00 + HW_CHANNEL_BASE[c % 4] + reg
    }

    fn write_channel_register(&mut self, c: usize, reg: u16, value: u8) {
        self.write_register(Self::register_pointer(c, reg), value);
    }

    /// `AudioN_UpdateMusic`.
    fn update_music(&mut self) {
        for c in 0..NUM_CHANNELS {
            if self.channels[c].sound_id == 0 {
                continue;
            }
            if c < CHAN5 && self.mute_audio_and_pause_music != 0 {
                if self.mute_audio_and_pause_music & 1 << BIT_MUTE_AUDIO != 0 {
                    continue;
                }
                self.mute_audio_and_pause_music |= 1 << BIT_MUTE_AUDIO;
                self.write_register(R_AUDTERM, 0);
                self.write_register(R_AUD3ENA, 0);
                self.write_register(R_AUD3ENA, AUD3ENA_ON);
                continue;
            }
            self.apply_music_affects(c);
        }
    }

    /// `AudioN_ApplyMusicAffects`: count the note down, and while it lasts apply whatever the
    /// channel has switched on.
    fn apply_music_affects(&mut self, c: usize) {
        if self.channels[c].note_delay_counter == 1 {
            return self.play_next_note(c);
        }
        self.channels[c].note_delay_counter = self.channels[c].note_delay_counter.wrapping_sub(1);
        // A music channel whose sound effect twin is playing has nothing to do: the sound effect
        // owns the hardware channel until it ends.
        if c < CHAN5 && self.channels[c + CHAN5].sound_id != 0 {
            return;
        }
        // AUDIO_2 only: while the alarm is on it owns channel 5 outright.
        if self.bank == AudioBank::Two
            && c == CHAN5
            && self.low_health_alarm & 1 << BIT_LOW_HEALTH_ALARM != 0
        {
            return;
        }
        if self.channels[c].test(BIT_ROTATE_DUTY_CYCLE) {
            self.apply_duty_cycle_pattern(c);
        }
        if !self.channels[c].executing_music() && self.channels[c].test(BIT_NOISE_OR_SFX) {
            return;
        }
        if self.channels[c].test(BIT_PITCH_SLIDE_ON) {
            return self.apply_pitch_slide(c);
        }
        if self.channels[c].vibrato_delay_counter != 0 {
            self.channels[c].vibrato_delay_counter -= 1;
            return;
        }
        self.apply_vibrato(c);
    }

    /// The tail of `AudioN_ApplyMusicAffects`. The extent byte holds the half above the note in its
    /// high nybble and the half below in its low one, and the direction bit is set and reset only
    /// here, so the pitch alternates either side for ever.
    fn apply_vibrato(&mut self, c: usize) {
        let extent = self.channels[c].vibrato_extent;
        if extent == 0 {
            return;
        }
        let rate = self.channels[c].vibrato_rate;
        if rate & 0xF != 0 {
            self.channels[c].vibrato_rate = rate - 1;
            return;
        }
        // Reload the counter from its own high nybble: `swap` then `or` puts the reload value in
        // both halves.
        self.channels[c].vibrato_rate = rate | rate.rotate_left(4);
        let e = self.channels[c].frequency_low;
        let d = if self.channels[c].test(BIT_VIBRATO_DIRECTION) {
            self.channels[c].res(BIT_VIBRATO_DIRECTION);
            e.checked_sub(extent & 0xF).unwrap_or(0)
        } else {
            self.channels[c].set(BIT_VIBRATO_DIRECTION);
            e.checked_add(extent >> 4).unwrap_or(0xFF)
        };
        self.write_channel_register(c, REG_FREQUENCY_LO, d);
    }

    /// `AudioN_PlayNextNote`.
    fn play_next_note(&mut self, c: usize) {
        self.channels[c].vibrato_delay_counter = self.channels[c].vibrato_delay_counter_reload_value;
        self.channels[c].res(BIT_PITCH_SLIDE_ON);
        self.channels[c].res(BIT_PITCH_SLIDE_DECREASING);
        self.sound_ret(c);
    }

    /// `AudioN_sound_ret` and the chain of handlers that falls out of it: every command that takes
    /// no time is executed and the next byte fetched, until a note or a rest ends the frame.
    fn sound_ret(&mut self, c: usize) {
        loop {
            let d = self.get_next_music_byte(c);
            if self.command(c, d) == Flow::Done {
                return;
            }
        }
    }

    /// `AudioN_GetNextMusicByte`.
    fn get_next_music_byte(&mut self, c: usize) -> u8 {
        let at = self.channels[c].command_pointer;
        self.channels[c].command_pointer = at.wrapping_add(1);
        self.bank.byte(at)
    }

    /// One command. The tests are the source's, in the source's order, which is why `$E9` — a byte
    /// no macro emits — comes out as an octave command: it falls past every full-byte test and is
    /// caught by `octave`'s `and $f0`.
    fn command(&mut self, c: usize, d: u8) -> Flow {
        match d {
            SOUND_RET_CMD => return self.sound_ret_cmd(c),
            SOUND_CALL_CMD => return self.sound_call(c),
            SOUND_LOOP_CMD => return self.sound_loop(c),
            _ => {}
        }
        if d & 0xF0 == NOTE_TYPE_CMD {
            return self.note_type(c, d);
        }
        match d {
            TOGGLE_PERFECT_PITCH_CMD => {
                self.channels[c].flags1 ^= 1 << BIT_PERFECT_PITCH;
                return Flow::Next;
            }
            VIBRATO_CMD => return self.vibrato(c),
            PITCH_SLIDE_CMD => return self.pitch_slide(c),
            DUTY_CYCLE_CMD => {
                let duty = self.get_next_music_byte(c);
                self.channels[c].duty_cycle = duty.rotate_right(2) & 0xC0;
                return Flow::Next;
            }
            TEMPO_CMD => return self.tempo(c),
            STEREO_PANNING_CMD => {
                self.stereo_panning = self.get_next_music_byte(c);
                return Flow::Next;
            }
            UNKNOWNMUSIC0XEF_CMD => return self.unknownmusic0xef(c),
            DUTY_CYCLE_PATTERN_CMD => return self.duty_cycle_pattern(c),
            VOLUME_CMD => {
                let volume = self.get_next_music_byte(c);
                self.write_register(R_AUDVOL, volume);
                return Flow::Next;
            }
            EXECUTE_MUSIC_CMD => {
                self.channels[c].flags2 |= 1 << BIT_EXECUTE_MUSIC;
                return Flow::Next;
            }
            _ => {}
        }
        if d & 0xF0 == OCTAVE_CMD {
            self.channels[c].octave = d & 0xF;
            return Flow::Next;
        }
        if d & 0xF0 == SFX_NOTE_CMD && c >= CHAN4 && !self.channels[c].executing_music() {
            return self.sfx_note(c, d);
        }
        if c >= CHAN5 && d == PITCH_SWEEP_CMD && !self.channels[c].executing_music() {
            let sweep = self.get_next_music_byte(c);
            self.write_register(R_AUD1SWEEP, sweep);
            return Flow::Next;
        }
        self.note(c, d)
    }

    /// `AudioN_sound_ret`'s own command, `$FF`: return from a `sound_call`, or end the channel.
    fn sound_ret_cmd(&mut self, c: usize) -> Flow {
        if self.channels[c].test(BIT_SOUND_CALL) {
            self.channels[c].res(BIT_SOUND_CALL);
            self.channels[c].command_pointer = self.channels[c].return_address;
            return Flow::Next;
        }
        let mut disable_output = true;
        if c >= CHAN4 {
            self.channels[c].res(BIT_NOISE_OR_SFX);
            self.channels[c].flags2 &= !(1 << BIT_EXECUTE_MUSIC);
            // The `jr nz` below reads the flags `cp CHAN7` set, which the wave restart does not
            // touch, so only channel 7 ever reaches `wDisableChannelOutputWhenSfxEnds` at all.
            if c == CHAN7 {
                self.write_register(R_AUD3ENA, 0);
                self.write_register(R_AUD3ENA, AUD3ENA_ON);
                disable_output = self.disable_channel_output_when_sfx_ends != 0;
                if disable_output {
                    self.disable_channel_output_when_sfx_ends = 0;
                }
            } else {
                disable_output = false;
            }
        }
        if disable_output {
            let term = self.registers.get(R_AUDTERM) & !HW_CHANNEL_ENABLE_MASK[c % 4];
            self.write_register(R_AUDTERM, term);
        }
        if self.is_cry() {
            // A cry's other two channels rewind onto this same `sound_ret` instead of ending, so
            // they stay claimed until channel 5's cry is over.
            if c != CHAN5 && self.go_back_one_command_if_cry(c) {
                return Flow::Done;
            }
            self.write_register(R_AUDVOL, self.saved_volume);
            self.saved_volume = 0;
        }
        self.channels[c].sound_id = 0;
        Flow::Done
    }

    /// `AudioN_sound_call`.
    fn sound_call(&mut self, c: usize) -> Flow {
        let low = self.get_next_music_byte(c);
        let high = self.get_next_music_byte(c);
        self.channels[c].return_address = self.channels[c].command_pointer;
        self.channels[c].command_pointer = u16::from_le_bytes([low, high]);
        self.channels[c].set(BIT_SOUND_CALL);
        Flow::Next
    }

    /// `AudioN_sound_loop`. A count of zero loops for ever; otherwise the counter is compared
    /// before it is raised, so `sound_loop n` plays the run `n` times.
    fn sound_loop(&mut self, c: usize) -> Flow {
        let count = self.get_next_music_byte(c);
        if count != 0 {
            if self.channels[c].loop_counter == count {
                self.channels[c].loop_counter = 1;
                self.get_next_music_byte(c);
                self.get_next_music_byte(c);
                return Flow::Next;
            }
            self.channels[c].loop_counter = self.channels[c].loop_counter.wrapping_add(1);
        }
        let low = self.get_next_music_byte(c);
        let high = self.get_next_music_byte(c);
        self.channels[c].command_pointer = u16::from_le_bytes([low, high]);
        Flow::Next
    }

    /// `AudioN_note_type`, `$Dx`: the speed every note length is multiplied by, and on every
    /// channel but the noise one a second byte of volume and fade.
    fn note_type(&mut self, c: usize, d: u8) -> Flow {
        self.channels[c].note_speed = d & 0xF;
        if c == CHAN4 {
            return Flow::Next;
        }
        let mut param = self.get_next_music_byte(c);
        if c == CHAN3 || c == CHAN7 {
            let instrument = param & 0xF;
            if c == CHAN3 {
                self.music_wave_instrument = instrument;
            } else {
                self.sfx_wave_instrument = instrument;
            }
            // Channel 3 has no envelope: the two low bits of the volume nybble are `rAUD3LEVEL`.
            param = (param & 0x30) << 1;
        }
        self.channels[c].volume = param;
        Flow::Next
    }

    /// `AudioN_vibrato`.
    fn vibrato(&mut self, c: usize) -> Flow {
        let delay = self.get_next_music_byte(c);
        self.channels[c].vibrato_delay_counter = delay;
        self.channels[c].vibrato_delay_counter_reload_value = delay;
        let param = self.get_next_music_byte(c);
        // The extent splits in two: `(n / 2) + (n % 2)` above the note and `n / 2` below it.
        let n = param >> 4;
        self.channels[c].vibrato_extent = (n / 2 + n % 2) << 4 | n / 2;
        // Both nybbles of the rate are the reload value, because the low one is the counter and it
        // starts full.
        let rate = param & 0xF;
        self.channels[c].vibrato_rate = rate << 4 | rate;
        Flow::Next
    }

    /// `AudioN_pitch_slide`. Its third byte is the note the slide runs over, which is why the
    /// handler ends inside `note_length` rather than fetching another command.
    fn pitch_slide(&mut self, c: usize) -> Flow {
        self.channels[c].pitch_slide_length_modifier = self.get_next_music_byte(c);
        let param = self.get_next_music_byte(c);
        let (d, e) = self.calculate_frequency(param & 0xF, param >> 4);
        self.channels[c].pitch_slide_target_frequency_high_byte = d;
        self.channels[c].pitch_slide_target_frequency_low_byte = e;
        self.channels[c].set(BIT_PITCH_SLIDE_ON);
        let note = self.get_next_music_byte(c);
        match self.note_length(c, note) {
            NoteLength::Returned(_) => Flow::Done,
            NoteLength::Pitch(command) => self.note_pitch(c, command),
        }
    }

    /// `AudioN_tempo`, which also clears every fractional part so the new tempo starts clean.
    fn tempo(&mut self, c: usize) -> Flow {
        let high = self.get_next_music_byte(c);
        let low = self.get_next_music_byte(c);
        let (tempo, channels) = if c < CHAN5 {
            (&mut self.music_tempo, 0..CHAN5)
        } else {
            (&mut self.sfx_tempo, CHAN5..NUM_CHANNELS)
        };
        *tempo = [high, low];
        for channel in channels {
            self.channels[channel].note_delay_counter_fractional_part = 0;
        }
        Flow::Next
    }

    /// `AudioN_unknownmusic0xef`, which no music uses: it plays a sound and then hides the noise
    /// channel's id so the sound's end disables the channel's output.
    fn unknownmusic0xef(&mut self, c: usize) -> Flow {
        let id = self.get_next_music_byte(c);
        self.engine_play_sound(SoundId(id));
        if self.disable_channel_output_when_sfx_ends == 0 {
            self.disable_channel_output_when_sfx_ends = self.channels[CHAN8].sound_id;
            self.channels[CHAN8].sound_id = 0;
        }
        Flow::Next
    }

    /// `AudioN_duty_cycle_pattern`: four duty cycles, two bits each, rotated one a frame from here
    /// on.
    fn duty_cycle_pattern(&mut self, c: usize) -> Flow {
        let pattern = self.get_next_music_byte(c);
        self.channels[c].duty_cycle_pattern = pattern;
        self.channels[c].duty_cycle = pattern & 0b1100_0000;
        self.channels[c].set(BIT_ROTATE_DUTY_CYCLE);
        Flow::Next
    }

    /// `AudioN_ApplyDutyCyclePattern`: rotate the pattern two bits and put the top of it into the
    /// duty bits of the register, leaving the sound length alone.
    fn apply_duty_cycle_pattern(&mut self, c: usize) {
        let pattern = self.channels[c].duty_cycle_pattern.rotate_left(2);
        self.channels[c].duty_cycle_pattern = pattern;
        let register = Self::register_pointer(c, REG_DUTY_SOUND_LEN);
        let value = self.registers.get(register) & 0x3F | pattern & 0xC0;
        self.write_register(register, value);
    }

    /// `AudioN_sfx_note`, `$2x`: a whole note written out — length, volume and fade, then the
    /// frequency, which the noise channel states in one byte rather than two.
    fn sfx_note(&mut self, c: usize, d: u8) -> Flow {
        let NoteLength::Returned(length) = self.note_length(c, d) else {
            unreachable!("a sound effect channel is never executing music here");
        };
        let duty = self.channels[c].duty_cycle;
        self.write_channel_register(c, REG_DUTY_SOUND_LEN, duty | length);
        let envelope = self.get_next_music_byte(c);
        self.write_channel_register(c, REG_VOLUME_ENVELOPE, envelope);
        let e = self.get_next_music_byte(c);
        let d = if c == CHAN8 { 0 } else { self.get_next_music_byte(c) };
        self.apply_duty_cycle_and_sound_length(c);
        self.enable_channel_output(c);
        self.apply_wave_pattern_and_frequency(c, d, e);
        Flow::Done
    }

    /// `AudioN_note`. On the music noise channel a command below `$B0` is a drum: the instrument is
    /// played as a sound effect of its own and the command byte goes on to be its note length.
    fn note(&mut self, c: usize, d: u8) -> Flow {
        let mut d = d;
        if c == CHAN4 && d & 0xF0 <= DRUM_NOTE_CMD {
            let (instrument, length) = if d & 0xF0 == DRUM_NOTE_CMD {
                (self.get_next_music_byte(c), d & 0xF)
            } else {
                // One byte rather than two, which nothing in the game uses: the high nybble is the
                // instrument, so instrument 2 is unreachable because `$2x` is a `sfx_note`.
                (d >> 4, d & 0xF)
            };
            if self.disable_channel_output_when_sfx_ends == 0 {
                self.engine_play_sound(SoundId(instrument));
            }
            d = length;
        }
        match self.note_length(c, d) {
            NoteLength::Returned(_) => Flow::Done,
            NoteLength::Pitch(command) => self.note_pitch(c, command),
        }
    }

    /// `AudioN_note_length`: the note's delay in frames, 8.8 fixed point, as
    /// `length * note speed * tempo` carried on from the fractional part left over.
    fn note_length(&mut self, c: usize, d: u8) -> NoteLength {
        let length = (d & 0xF) + 1;
        let speed = self.channels[c].note_speed;
        // Only the low byte of `length * speed` survives, which is the multiplier the tempo is
        // applied to.
        let scaled = multiply_add(0, speed, length as u16) as u8;
        let tempo = if c < CHAN5 {
            u16::from_be_bytes(self.music_tempo)
        } else if c == CHAN8 {
            0x0100
        } else {
            self.set_sfx_tempo();
            u16::from_be_bytes(self.sfx_tempo)
        };
        let delay = multiply_add(self.channels[c].note_delay_counter_fractional_part, scaled, tempo);
        let [counter, fractional] = delay.to_be_bytes();
        self.channels[c].note_delay_counter_fractional_part = fractional;
        self.channels[c].note_delay_counter = counter;
        if self.channels[c].executing_music() || !self.channels[c].test(BIT_NOISE_OR_SFX) {
            NoteLength::Pitch(d)
        } else {
            NoteLength::Returned(counter)
        }
    }

    /// `AudioN_note_pitch`: a rest silences the channel, a note works out its frequency and starts
    /// it. Either way a music channel whose sound effect twin is playing does nothing at all.
    fn note_pitch(&mut self, c: usize, command: u8) -> Flow {
        if command & 0xF0 == REST_CMD {
            if c < CHAN5 && self.channels[c + CHAN5].sound_id != 0 {
                return Flow::Done;
            }
            if c == CHAN3 || c == CHAN7 {
                let term = self.registers.get(R_AUDTERM) & !HW_CHANNEL_ENABLE_MASK[c % 4];
                self.write_register(R_AUDTERM, term);
            } else {
                // Fade the sound in from nothing and restart it, which is silence with a trigger.
                self.write_channel_register(c, REG_VOLUME_ENVELOPE, 0x08);
                self.write_channel_register(c, REG_FREQUENCY_LO + 1, 0x80);
            }
            return Flow::Done;
        }
        let octave = self.channels[c].octave;
        let (mut d, mut e) = self.calculate_frequency(command >> 4, octave);
        if self.channels[c].test(BIT_PITCH_SLIDE_ON) {
            // A note that arms a pitch slide does not sound at its own pitch for its first frame:
            // the divide below runs in the frequency's own registers and this is what it leaves.
            // The slide itself still starts from the right frequency, which was stored first.
            (d, e) = self.init_pitch_slide_vars(c, d, e);
        }
        if c < CHAN5 && self.channels[c + CHAN5].sound_id != 0 {
            return Flow::Done;
        }
        let volume = self.channels[c].volume;
        self.write_channel_register(c, REG_VOLUME_ENVELOPE, volume);
        self.apply_duty_cycle_and_sound_length(c);
        self.enable_channel_output(c);
        if self.channels[c].test(BIT_PERFECT_PITCH) {
            // `inc e` never sets carry, so the high byte is never carried into. No note the game
            // actually plays needs it to be.
            e = e.wrapping_add(1);
        }
        self.channels[c].frequency_low = e;
        self.apply_wave_pattern_and_frequency(c, d, e);
        Flow::Done
    }

    /// `AudioN_EnableChannelOutput`: turn this channel's two `rAUDTERM` bits on, and where the
    /// channel is the sound effect noise one or a music channel with no sound effect over it, let
    /// `stereo_panning` decide which of the two.
    fn enable_channel_output(&mut self, c: usize) {
        let enable = HW_CHANNEL_ENABLE_MASK[c % 4];
        let mut term = self.registers.get(R_AUDTERM) | enable;
        let panned = c == CHAN8 || (c < CHAN5 && self.channels[c + CHAN5].sound_id == 0);
        if panned {
            term = self.registers.get(R_AUDTERM) & !enable | self.stereo_panning & enable;
        }
        self.write_register(R_AUDTERM, term);
    }

    /// `AudioN_ApplyDutyCycleAndSoundLength`: the note's delay counter doubles as the hardware
    /// sound length, with the duty cycle in the two bits above it. Channel 3 has no duty cycle, so
    /// all eight bits are its length.
    fn apply_duty_cycle_and_sound_length(&mut self, c: usize) {
        let mut d = self.channels[c].note_delay_counter;
        if c != CHAN3 && c != CHAN7 {
            d = d & 0x3F | self.channels[c].duty_cycle;
        }
        self.write_channel_register(c, REG_DUTY_SOUND_LEN, d);
    }

    /// `AudioN_ApplyWavePatternAndFrequency`: on channel 3 copy the instrument into wave RAM, then
    /// on any channel write the frequency with the length counter enabled and the note triggered.
    fn apply_wave_pattern_and_frequency(&mut self, c: usize, d: u8, e: u8) {
        if c == CHAN3 || c == CHAN7 {
            let instrument = if c == CHAN3 { self.music_wave_instrument } else { self.sfx_wave_instrument };
            let samples = self.bank.wave_sample(instrument);
            self.write_register(R_AUD3ENA, 0);
            for (i, byte) in samples.into_iter().enumerate() {
                self.write_register(AUD3WAVERAM + i as u16, byte);
            }
            debug_assert_eq!(samples.len() as u16, AUD3WAVE_SIZE);
            self.write_register(R_AUD3ENA, AUD3ENA_ON);
        }
        let d = (d | 0x80) & 0xC7;
        self.write_channel_register(c, REG_FREQUENCY_LO, e);
        self.write_channel_register(c, REG_FREQUENCY_LO + 1, d);
        // The bug the plan keeps: engines 1 and 3 detune every channel while a cry is playing,
        // music included, because they do not ask which channel this is. `AUDIO_2` does.
        if self.bank != AudioBank::Two || c >= CHAN5 {
            self.apply_frequency_modifier(c, d, e);
        }
    }

    /// `AudioN_SetSfxTempo`: a cry stretches by its own modifier, anything else plays at `$0100`.
    fn set_sfx_tempo(&mut self) {
        if self.cry_or_battle_sfx() {
            let (low, carry) = self.tempo_modifier.overflowing_add(0x80);
            self.sfx_tempo = [carry as u8, low];
        } else {
            self.sfx_tempo = [1, 0];
        }
    }

    /// `AudioN_ApplyFrequencyModifier`: a cry's detune, added to the frequency just written.
    fn apply_frequency_modifier(&mut self, c: usize, d: u8, e: u8) {
        if !self.cry_or_battle_sfx() {
            return;
        }
        let (low, carry) = self.frequency_modifier.overflowing_add(e);
        let high = if carry { d.wrapping_add(1) } else { d };
        self.write_channel_register(c, REG_FREQUENCY_LO, low);
        self.write_channel_register(c, REG_FREQUENCY_LO + 1, high);
    }

    /// `AudioN_IsCry`.
    fn is_cry(&self) -> bool {
        (CRY_SFX_START.0..CRY_SFX_END.0).contains(&self.channels[CHAN5].sound_id)
    }

    /// `AudioN_IsBattleSFX`, which only `AUDIO_2` has, and which asks about channels 5 and 8 at
    /// once by *or*-ing their ids together rather than testing either.
    fn is_battle_sfx(&self) -> bool {
        let id = self.channels[CHAN8].sound_id | self.channels[CHAN5].sound_id;
        (BATTLE_SFX_START.0..BATTLE_SFX_END.0).contains(&id)
    }

    fn cry_or_battle_sfx(&self) -> bool {
        self.is_cry() || (self.bank == AudioBank::Two && self.is_battle_sfx())
    }

    /// `AudioN_GoBackOneCommandIfCry`: step the pointer back onto the `sound_ret` it just read, so
    /// the channel reads it again next time.
    fn go_back_one_command_if_cry(&mut self, c: usize) -> bool {
        if !self.is_cry() {
            return false;
        }
        self.channels[c].command_pointer = self.channels[c].command_pointer.wrapping_sub(1);
        true
    }

    /// `AudioN_CalculateFrequency`: the note's entry in `AudioN_Pitches` shifted right once per
    /// octave, arithmetically, with `8` added to what comes out of the high byte. Returns the high
    /// and low bytes of the frequency.
    fn calculate_frequency(&self, note: u8, octave: u8) -> (u8, u8) {
        let mut value = self.bank.pitch(note) as i16;
        let mut a = octave;
        while a != 7 {
            value >>= 1;
            a = a.wrapping_add(1);
        }
        let [e, d] = (value as u16).to_le_bytes();
        (8u8.wrapping_add(d), e)
    }

    /// `AudioN_ApplyPitchSlide`: one step towards the target frequency, or the end of the slide.
    fn apply_pitch_slide(&mut self, c: usize) {
        let channel = &mut self.channels[c];
        let steps = channel.pitch_slide_frequency_steps;
        let (mut d, mut e);
        if channel.test(BIT_PITCH_SLIDE_DECREASING) {
            let (low, borrow) = channel.pitch_slide_current_frequency_low_byte.overflowing_sub(steps);
            e = low;
            d = channel.pitch_slide_current_frequency_high_byte.wrapping_sub(borrow as u8);
            // The fractional step is doubled rather than accumulated on the way down, and its carry
            // is what the frequency borrows.
            let (fractional, carry) = channel
                .pitch_slide_frequency_steps_fractional_part
                .overflowing_add(channel.pitch_slide_frequency_steps_fractional_part);
            channel.pitch_slide_frequency_steps_fractional_part = fractional;
            let (low, borrow) = e.overflowing_sub(carry as u8);
            e = low;
            d = d.wrapping_sub(borrow as u8);
            let reached = d < channel.pitch_slide_target_frequency_high_byte
                || (d == channel.pitch_slide_target_frequency_high_byte
                    && e < channel.pitch_slide_target_frequency_low_byte);
            if reached {
                channel.res(BIT_PITCH_SLIDE_ON);
                channel.res(BIT_PITCH_SLIDE_DECREASING);
                return;
            }
        } else {
            let current = u16::from_be_bytes([
                channel.pitch_slide_current_frequency_high_byte,
                channel.pitch_slide_current_frequency_low_byte,
            ]);
            let stepped = current.wrapping_add(steps as u16);
            let (fractional, carry) = channel
                .pitch_slide_current_frequency_fractional_part
                .overflowing_add(channel.pitch_slide_frequency_steps_fractional_part);
            channel.pitch_slide_current_frequency_fractional_part = fractional;
            let stepped = stepped.wrapping_add(carry as u16);
            [d, e] = stepped.to_be_bytes();
            let reached = channel.pitch_slide_target_frequency_high_byte < d
                || (channel.pitch_slide_target_frequency_high_byte == d
                    && channel.pitch_slide_target_frequency_low_byte < e);
            if reached {
                channel.res(BIT_PITCH_SLIDE_ON);
                channel.res(BIT_PITCH_SLIDE_DECREASING);
                return;
            }
        }
        channel.pitch_slide_current_frequency_low_byte = e;
        channel.pitch_slide_current_frequency_high_byte = d;
        self.write_channel_register(c, REG_FREQUENCY_LO, e);
        self.write_channel_register(c, REG_FREQUENCY_LO + 1, d);
    }

    /// `AudioN_InitPitchSlideVars`: divide the distance to the target by how many frames the slide
    /// has, to get the step and its fractional part. Returns the two registers it leaves behind,
    /// which the caller goes on to treat as a frequency.
    fn init_pitch_slide_vars(&mut self, c: usize, d: u8, e: u8) -> (u8, u8) {
        let channel = &mut self.channels[c];
        channel.pitch_slide_current_frequency_high_byte = d;
        channel.pitch_slide_current_frequency_low_byte = e;
        // The length modifier is how many frames of the note are left after the slide's own count.
        let length = channel.note_delay_counter.checked_sub(channel.pitch_slide_length_modifier).unwrap_or(1);
        channel.pitch_slide_length_modifier = length;

        let (low, borrow) = e.overflowing_sub(channel.pitch_slide_target_frequency_low_byte);
        let high = d.wrapping_sub(borrow as u8);
        let (mut difference_high, target_greater) =
            high.overflowing_sub(channel.pitch_slide_target_frequency_high_byte);
        let mut difference_low = low;
        if target_greater {
            let (low, borrow) = channel.pitch_slide_target_frequency_low_byte
                .overflowing_sub(channel.pitch_slide_current_frequency_low_byte);
            difference_low = low;
            // The bug: the borrow comes off the *current* frequency's high byte rather than the
            // target's, so a slide upwards whose low byte has to borrow comes out $200 too far.
            let borrowed = channel.pitch_slide_current_frequency_high_byte.wrapping_sub(borrow as u8);
            difference_high = channel.pitch_slide_target_frequency_high_byte.wrapping_sub(borrowed);
            channel.res(BIT_PITCH_SLIDE_DECREASING);
        } else {
            channel.set(BIT_PITCH_SLIDE_DECREASING);
        }

        let (mut d, mut e, mut quotient) = (difference_high, difference_low, 0u8);
        loop {
            quotient = quotient.wrapping_add(1);
            let (low, borrow) = e.overflowing_sub(length);
            e = low;
            if !borrow {
                continue;
            }
            if d == 0 {
                break;
            }
            d -= 1;
        }
        let remainder = e.wrapping_add(length);
        channel.pitch_slide_frequency_steps = quotient;
        channel.pitch_slide_frequency_steps_fractional_part = remainder;
        channel.pitch_slide_current_frequency_fractional_part = remainder;
        // The divide runs in the same two registers the note's frequency arrived in, and the
        // caller saves them only after this returns. What comes back is therefore what the note
        // plays for its first frame; see `note_pitch`.
        (quotient, e)
    }

    /// `AudioN_PlaySound`: the engine's own entry point, which `PlaySound` reaches once it has
    /// dealt with any fade.
    pub fn engine_play_sound(&mut self, id: SoundId) {
        self.sound_id = id.0;
        if id == SoundId::STOP_ALL_MUSIC {
            return self.stop_all_audio();
        }
        if id <= self.bank.max_sfx_id() || id.0 > 0xFE {
            return self.play_sfx(id);
        }
        self.play_music_(id);
    }

    /// `AudioN_PlaySound.playMusic`: every channel back to nothing, the hardware reset, and then
    /// the header's channels started.
    fn play_music_(&mut self, id: SoundId) {
        self.unused_music_byte = 0;
        self.disable_channel_output_when_sfx_ends = 0;
        self.music_tempo = [1, 0];
        self.music_wave_instrument = 0;
        self.sfx_wave_instrument = 0;
        // Only the four music channels are reset. `.FillMem` is given `NUM_MUSIC_CHANS` for every
        // array, and eight bytes for the two pointer arrays, which is four channels' worth of two:
        // a sound effect playing over the music keeps its own channel and carries on.
        for c in 0..CHAN5 {
            self.reset_channel(c);
        }
        self.stereo_panning = 0xFF;
        self.write_register(R_AUDVOL, 0);
        self.write_register(R_AUD1SWEEP, AUD1SWEEP_DOWN);
        self.write_register(R_AUDTERM, 0);
        self.write_register(R_AUD3ENA, 0);
        self.write_register(R_AUD3ENA, AUD3ENA_ON);
        self.write_register(R_AUDVOL, 0x77);
        self.play_sound_common(id);
    }

    /// `AudioN_PlaySound.playSfx`: the header's channels are checked in reverse and the whole sound
    /// abandoned the moment one of them is busy with something of its own — after the ones already
    /// checked have been cleared.
    fn play_sfx(&mut self, id: SoundId) {
        let header = SoundHeader::read(self.bank, id);
        for entry in header.channels.iter().rev() {
            let c = entry.channel;
            if self.channels[c].sound_id != 0 {
                let current = SoundId(self.channels[c].sound_id);
                if c == CHAN8 {
                    if id < NOISE_INSTRUMENTS_END {
                        // A drum never interrupts anything already on the noise channel.
                        return;
                    }
                    if current > NOISE_INSTRUMENTS_END && id > current {
                        return;
                    }
                } else if id > current {
                    return;
                }
            }
            self.reset_channel(c);
            if c == CHAN5 {
                self.write_register(R_AUD1SWEEP, AUD1SWEEP_DOWN);
            }
        }
        self.play_sound_common(id);
    }

    /// `AudioN_PlaySound.playSoundCommon`: point each of the header's channels at its commands. A
    /// cry claims channels 5 to 8 whole, parks the sound effect wave channel on a bare `sound_ret`,
    /// and turns the volume up, remembering what it was.
    fn play_sound_common(&mut self, id: SoundId) {
        let header = SoundHeader::read(self.bank, id);
        for entry in &header.channels {
            let c = entry.channel;
            self.channels[c].sound_id = id.0;
            if c >= CHAN4 {
                self.channels[c].set(BIT_NOISE_OR_SFX);
            }
            self.channels[c].command_pointer = entry.address;
        }
        if !(CRY_SFX_START.0..CRY_SFX_END.0).contains(&id.0) {
            return;
        }
        for c in CHAN5..NUM_CHANNELS {
            self.channels[c].sound_id = id.0;
        }
        self.channels[CHAN7].command_pointer = self.cry_ret();
        if self.saved_volume == 0 {
            self.saved_volume = self.registers.get(R_AUDVOL);
            self.write_register(R_AUDVOL, 0x77);
        }
    }

    /// `AudioN_CryRet`: the one byte in the bank that is nothing but a `sound_ret`.
    fn cry_ret(&self) -> u16 {
        use poke_core::symbols::pokered_symbols;
        match self.bank {
            AudioBank::One => pokered_symbols::Audio1_CryRet.address,
            AudioBank::Two => pokered_symbols::Audio2_CryRet.address,
            AudioBank::Three => pokered_symbols::Audio3_CryRet.address,
        }
    }

    /// `AudioN_PlaySound.stopAllAudio`.
    fn stop_all_audio(&mut self) {
        self.write_register(R_AUDENA, AUDENA_ON);
        self.write_register(R_AUD3ENA, AUDENA_ON);
        self.write_register(R_AUDTERM, 0);
        self.write_register(R_AUD3LEVEL, 0);
        self.write_register(R_AUD1SWEEP, AUD1SWEEP_DOWN);
        self.write_register(R_AUD1ENV, AUD1SWEEP_DOWN);
        self.write_register(R_AUD2ENV, AUD1SWEEP_DOWN);
        self.write_register(R_AUD4ENV, AUD1SWEEP_DOWN);
        self.write_register(R_AUD1HIGH, AUD1HIGH_LENGTH_ON);
        self.write_register(R_AUD2HIGH, AUD1HIGH_LENGTH_ON);
        self.write_register(R_AUD4GO, AUD1HIGH_LENGTH_ON);
        self.write_register(R_AUDVOL, 0x77);
        self.unused_music_byte = 0;
        self.disable_channel_output_when_sfx_ends = 0;
        self.mute_audio_and_pause_music = 0;
        self.music_tempo = [1, 0];
        self.sfx_tempo = [1, 0];
        self.music_wave_instrument = 0;
        self.sfx_wave_instrument = 0;
        // `$a0` bytes from `wChannelCommandPointers` stops at the end of
        // `wChannelPitchSlideCurrentFrequencyLowBytes`, so both pitch slide *target* arrays survive
        // a stop, as do the fractional delays, the octaves and the volumes. Then `$18` bytes of
        // ones, which is the three counters and not the fractional part beside them.
        for c in 0..NUM_CHANNELS {
            let kept = self.channels[c];
            self.channels[c] = ChannelState {
                loop_counter: 1,
                note_delay_counter: 1,
                note_speed: 1,
                pitch_slide_target_frequency_high_byte: kept.pitch_slide_target_frequency_high_byte,
                pitch_slide_target_frequency_low_byte: kept.pitch_slide_target_frequency_low_byte,
                note_delay_counter_fractional_part: kept.note_delay_counter_fractional_part,
                octave: kept.octave,
                volume: kept.volume,
                ..ChannelState::default()
            };
        }
        self.stereo_panning = 0xFF;
    }

    /// `FadeOutAudio`: a step of the master volume every `reload` frames, and when it reaches zero
    /// everything stops and whatever id the fade was carrying is played in the saved bank.
    fn fade_out_audio(&mut self) {
        if self.audio_fade_out_control == 0 {
            if !self.no_audio_fade_out {
                self.write_register(R_AUDVOL, 0x77);
            }
            return;
        }
        if self.audio_fade_out_counter != 0 {
            self.audio_fade_out_counter -= 1;
            return;
        }
        self.audio_fade_out_counter = self.audio_fade_out_counter_reload_value;
        let volume = self.registers.get(R_AUDVOL);
        if volume != 0 {
            // Both nybbles step down together, and neither borrows into the other.
            let stepped = (volume & 0xF0).wrapping_sub(0x10) & 0xF0 | (volume & 0xF).wrapping_sub(1) & 0xF;
            self.write_register(R_AUDVOL, stepped);
            return;
        }
        let next = self.audio_fade_out_control;
        self.audio_fade_out_control = 0;
        self.new_sound_id = SoundId::STOP_ALL_MUSIC.0;
        self.play_sound(SoundId::STOP_ALL_MUSIC);
        self.bank = self.saved_bank;
        self.new_sound_id = next;
        self.play_sound(SoundId(next));
    }

    /// `Music_DoLowHealthAlarm`, which only `AUDIO_2`'s VBlank runs. It writes channel 1's
    /// registers itself, over whatever the engine had there, and holds channel 5 so nothing takes
    /// it back.
    fn music_do_low_health_alarm(&mut self) {
        const TONE_HI: [u8; 4] = [0xA0, 0xE2, 0x50, 0x87];
        const TONE_LO: [u8; 4] = [0xB0, 0xE2, 0xEE, 0x86];
        const TONE_SILENCE: [u8; 4] = [0x00, 0x00, 0x00, 0x80];

        if self.low_health_alarm == DISABLE_LOW_HEALTH_ALARM {
            self.low_health_alarm = 0;
            self.channels[CHAN5].sound_id = 0;
            return self.play_alarm_tone(TONE_SILENCE);
        }
        if self.low_health_alarm & 1 << BIT_LOW_HEALTH_ALARM == 0 {
            return;
        }
        let timer = self.low_health_alarm & LOW_HEALTH_TIMER_MASK;
        let next = if timer == 0 {
            self.play_alarm_tone(TONE_HI);
            30
        } else {
            if timer == 20 {
                self.play_alarm_tone(TONE_LO);
            }
            // Holding the id at `CRY_SFX_END` keeps channel 5 claimed without it reading as a cry.
            self.channels[CHAN5].sound_id = CRY_SFX_END.0;
            timer - 1
        };
        self.low_health_alarm = next | 1 << BIT_LOW_HEALTH_ALARM;
    }

    /// The alarm's five registers, `rAUD1SWEEP` always zeroed and the other four from the table.
    fn play_alarm_tone(&mut self, tone: [u8; 4]) {
        self.write_register(R_AUD1SWEEP, 0);
        for (i, byte) in tone.into_iter().enumerate() {
            self.write_register(R_AUD1SWEEP + 1 + i as u16, byte);
        }
    }
}

/// What a harvested trace was taken of: which copy of the engine, and which sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceInput {
    pub bank: AudioBank,
    pub id: SoundId,
}

/// One frame of writes as hex, two characters of register and two of byte. The register is named
/// by its low byte, which is unique across the sound registers, and the byte is the one the
/// hardware keeps, so a trace says nothing about bits the hardware throws away.
pub fn encode_writes(writes: &[Write]) -> String {
    writes
        .iter()
        .map(|write| {
            let (address, value) = write.register();
            format!("{:02X}{value:02X}", address as u8)
        })
        .collect()
}

/// `AudioN_MultiplyAdd`: `l + a * de`, sixteen bits, wrapping.
fn multiply_add(l: u8, mut a: u8, mut de: u16) -> u16 {
    let mut hl = l as u16;
    loop {
        let add = a & 1 != 0;
        a >>= 1;
        if add {
            hl = hl.wrapping_add(de);
        }
        de = de.wrapping_shl(1);
        if a == 0 {
            return hl;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::data::sounds;
    use super::*;

    fn playing(sound: Sound) -> AudioEngine {
        let mut engine = AudioEngine::new(sound.bank);
        engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
        engine.play_music(sound);
        engine
    }

    #[test]
    fn multiply_add_is_l_plus_a_times_de() {
        assert_eq!(multiply_add(0, 12, 160), 1920);
        assert_eq!(multiply_add(0x80, 1, 0x0100), 0x0180);
        assert_eq!(multiply_add(0, 0, 0x1234), 0, "a zero multiplier adds nothing");
        assert_eq!(multiply_add(0, 255, 0x0200), 0xFE00, "and it wraps");
    }

    /// Octave 7 is the table itself and every octave below it is one arithmetic shift further
    /// right, with eight added to the high byte.
    #[test]
    fn a_note_is_its_pitch_shifted_once_an_octave() {
        let engine = AudioEngine::new(AudioBank::One);
        assert_eq!(engine.calculate_frequency(0, 8 - 8), (0x07, 0xF0), "C_ at octave 8");
        assert_eq!(engine.calculate_frequency(0, 8 - 1), (0x00, 0x2C), "C_ at octave 1");
        let (high, _) = engine.calculate_frequency(11, 8 - 4);
        assert!(high < 8, "a frequency is eleven bits");
    }

    #[test]
    fn starting_a_song_points_its_channels_at_its_commands() {
        let engine = playing(sounds::MUSIC_PALLET_TOWN);
        assert_eq!(engine.channels[0].command_pointer, 0x67C5);
        assert_eq!(engine.channels[0].sound_id, sounds::MUSIC_PALLET_TOWN.id.0);
        assert_eq!(engine.channels[3].sound_id, 0, "Pallet Town has three channels");
        assert_eq!(engine.registers.get(R_AUDVOL), 0x77);
    }

    /// The first frame executes the song's header commands and then the first note of each
    /// channel, which is what the delay counters are left holding.
    #[test]
    fn the_first_frame_reaches_the_first_note_of_every_channel() {
        let mut engine = playing(sounds::MUSIC_PALLET_TOWN);
        let writes = engine.frame();
        assert!(!writes.is_empty(), "the first frame is silent");
        assert_eq!(u16::from_be_bytes(engine.music_tempo), 160, "tempo 160");
        for c in 0..3 {
            assert!(engine.channels[c].note_delay_counter > 0, "channel {c} has no note");
        }
    }

    /// Every song and sound effect in every bank plays for a second without the interpreter
    /// reaching a byte it cannot read or a state it cannot leave.
    #[test]
    fn every_sound_in_the_cartridge_plays() {
        for bank in AudioBank::ALL {
            for id in bank.sound_ids() {
                let mut engine = AudioEngine::new(bank);
                engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
                engine.engine_play_sound(id);
                let played = (0..60).any(|_| !engine.frame().is_empty());
                assert!(played, "{bank:?} {id:?} never wrote a register");
            }
        }
    }

    /// Safari Zone opens with `pitch_slide 1, 4, A_` and then `note C_, 1`. C_ at octave 3 is
    /// `$060B`, and `$0C` is what the low byte would be once perfect pitch has raised it — but what
    /// reaches the hardware is `$FF`, the remains of the slide's own division.
    #[test]
    fn a_note_that_arms_a_pitch_slide_sounds_the_divides_leftovers_first() {
        use super::super::Channel;
        let mut engine = playing(sounds::MUSIC_SAFARI_ZONE);
        let lows: Vec<u8> = engine
            .frame()
            .iter()
            .filter_map(|write| match write {
                Write::PeriodLow { channel: Channel::Pulse1, low } => Some(*low),
                _ => None,
            })
            .collect();
        assert_eq!(lows, [0xFF]);
    }

    /// Every sound in the cartridge, in all three copies of the engine, against the registers the
    /// cartridge's own engine wrote: 362 traces of 120 frames each.
    #[test]
    fn every_harvested_trace_replays() {
        for jsonl in [
            include_str!("../../fixtures/audio/audio_1.jsonl"),
            include_str!("../../fixtures/audio/audio_2.jsonl"),
            include_str!("../../fixtures/audio/audio_3.jsonl"),
        ] {
            for (input, frames, _) in crate::fixtures::cases::<TraceInput, Vec<String>>(jsonl) {
                let mut engine = AudioEngine::new(input.bank);
                engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
                engine.engine_play_sound(input.id);
                engine.take_writes();
                for (frame, expected) in frames.iter().enumerate() {
                    assert_eq!(&encode_writes(&engine.frame()), expected, "{input:?}, frame {frame}");
                }
            }
        }
    }

    /// A sound effect claims its music channel's hardware, and the music channel goes quiet until
    /// the effect ends rather than fighting it for the registers.
    #[test]
    fn a_sound_effect_pre_empts_the_music_channel_under_it() {
        let mut engine = playing(sounds::MUSIC_PALLET_TOWN);
        for _ in 0..10 {
            engine.frame();
        }
        engine.play_sound(sounds::SFX_COLLISION);
        assert_ne!(engine.channels[CHAN5].sound_id, 0, "the effect took channel 5");
        assert_ne!(engine.channels[0].sound_id, 0, "the music still holds channel 1");
        let mut frames = 0;
        while engine.channels[CHAN5].sound_id != 0 && frames < 600 {
            engine.frame();
            frames += 1;
        }
        assert!(frames < 600, "the effect never finished");
        assert!(!engine.sound_finished() || engine.channels[CHAN8].sound_id == 0);
    }

    /// A lower id wins, and a drum never interrupts anything already on the noise channel.
    #[test]
    fn a_busier_channel_refuses_a_lesser_sound() {
        let mut engine = AudioEngine::new(AudioBank::One);
        engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
        engine.engine_play_sound(sounds::SFX_COLLISION);
        let held = engine.channels[CHAN5].sound_id;
        engine.engine_play_sound(sounds::SFX_SAVE);
        assert_eq!(engine.channels[CHAN5].sound_id, held, "a higher id does not interrupt");
        engine.engine_play_sound(sounds::SFX_TINK);
        assert_eq!(engine.channels[CHAN5].sound_id, sounds::SFX_TINK.0, "a lower one does");
    }

    /// A cry claims all four sound effect channels, parks the wave one on `sound_ret`, and turns
    /// the volume up with the old one put by.
    #[test]
    fn a_cry_claims_every_sound_effect_channel() {
        let mut engine = AudioEngine::new(AudioBank::One);
        engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
        engine.play_cry(1);
        assert!(engine.is_cry());
        for c in CHAN5..NUM_CHANNELS {
            assert_ne!(engine.channels[c].sound_id, 0, "channel {c}");
        }
        assert_eq!(engine.channels[CHAN7].command_pointer, engine.cry_ret());
        assert_eq!(engine.saved_volume, 0x77);
        assert_eq!(engine.tempo_modifier, 0x80, "Rhydon is not stretched");
    }

    /// The fade steps both nybbles of the master volume down together and then plays what it was
    /// given, in the bank that was put by for it.
    #[test]
    fn a_fade_steps_the_volume_down_and_then_starts_the_next_song() {
        let mut engine = playing(sounds::MUSIC_PALLET_TOWN);
        engine.frame();
        engine.last_music_sound_id = sounds::MUSIC_PALLET_TOWN.id.0;
        engine.fade_out(4);
        engine.play_music(sounds::MUSIC_POKECENTER);
        assert_eq!(engine.registers.get(R_AUDVOL), 0x77);
        let mut frames = 0;
        while engine.channels[0].sound_id != sounds::MUSIC_POKECENTER.id.0 && frames < 600 {
            engine.frame();
            frames += 1;
        }
        assert!(frames < 600, "the fade never finished");
        assert_eq!(engine.bank, AudioBank::One);
    }

    /// Nothing the engine writes is held anywhere but the registers, so a round trip through a save
    /// is the same engine.
    #[test]
    fn an_engine_survives_a_round_trip() {
        let mut engine = playing(sounds::MUSIC_CITIES1);
        for _ in 0..100 {
            engine.frame();
        }
        let bytes = rmp_serde::to_vec_named(&engine).unwrap();
        let mut restored: AudioEngine = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(restored, engine);
        assert_eq!(restored.frame(), engine.frame());
    }
}
