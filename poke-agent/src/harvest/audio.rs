//! Register-write traces from the cartridge's own audio engine, as tier-1 fixtures for `pokered`.
//!
//! A trace is what one of the three copies of the engine writes to the sound registers, frame by
//! frame, for one sound. Interrupts are masked and `FadeOutAudio`, `Music_DoLowHealthAlarm` and
//! `AudioN_UpdateMusic` are called by hand in `VBlank`'s own order, so a trace is those three
//! routines alone: nothing else on the machine is running, and nothing else touches a register.
//!
//! The machine is normalised first — every sound register zeroed and every audio global the engine
//! reads with it — so a trace starts from the state `AudioEngine::default` is, and a fixture needs
//! to carry no initial conditions beyond which bank and which sound.

use gb::core::CoreMode;
use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::ram::{RAM, ROM};
use pokered::audio::data::{AudioBank, SoundId};
use pokered::audio::engine::{encode_writes, AudioEngine, Cue, CueInput, CueTrace, TraceInput};
use pokered::audio::Write;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// Where a called routine returns to: unusable memory, so nothing else ever executes there.
const TRAP: u16 = 0xFEA0;
const INTERRUPT_ENABLE: u16 = 0xFFFF;
const MBC_ROM_BANK: u16 = 0x2000;

/// Two seconds of each sound: long enough to reach the end of every sound effect and cry, several
/// bars of every song, and the first `sound_loop` of the shorter ones.
const FRAMES: usize = 120;

/// The cartridge's audio engine, driven a frame at a time with nothing else running.
struct Cartridge {
    gb: GameBoy,
    stack: u16,
    bank: AudioBank,
    /// `wAudioFadeOutControl` after each VBlank, non-zero while a fade runs.
    fading: Vec<bool>,
}

/// One fixture file per copy of the engine.
fn fixture_name(bank: AudioBank) -> &'static str {
    match bank {
        AudioBank::One => "audio_1",
        AudioBank::Two => "audio_2",
        AudioBank::Three => "audio_3",
    }
}

/// `AudioN_PlaySound` and `AudioN_UpdateMusic` for one of the three copies.
fn routines(bank: AudioBank) -> (DmgPointer, DmgPointer) {
    match bank {
        AudioBank::One => (pokered_symbols::Audio1_PlaySound, pokered_symbols::Audio1_UpdateMusic),
        AudioBank::Two => (pokered_symbols::Audio2_PlaySound, pokered_symbols::Audio2_UpdateMusic),
        AudioBank::Three => (pokered_symbols::Audio3_PlaySound, pokered_symbols::Audio3_UpdateMusic),
    }
}

impl Cartridge {
    fn new(bank: AudioBank) -> Self {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-celadon.bin")).expect("load the state");
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        for _ in 0..1_000_000 {
            if gb.core().mode() == CoreMode::Normal {
                break;
            }
            gb.run_until(&[], MachineCycles::from_m(1));
        }
        assert_eq!(gb.core().mode(), CoreMode::Normal, "the CPU never woke");
        gb.core_mut().mmu_mut().write(INTERRUPT_ENABLE, 0);
        let stack = gb.core().registers().sp;
        Self { gb, stack, bank, fading: Vec::new() }
    }

    /// `call routine` with `a` set, run until it returns.
    fn call(&mut self, routine: DmgPointer, a: u8) {
        self.call_with(routine, a, 0);
    }

    /// `call routine` with `a` and `c` set, run until it returns; what it left in `a`.
    fn call_with(&mut self, routine: DmgPointer, a: u8, c: u8) -> u8 {
        let mmu = self.gb.core_mut().mmu_mut();
        if let DmgBank::ROM { bank } = routine.bank && routine.address >= 0x4000 {
            mmu.write(MBC_ROM_BANK, bank);
            mmu.write(pokered_symbols::hLoadedROMBank.address, bank);
        }
        let sp = self.stack - 2;
        mmu.write_u16_le(sp, TRAP);
        let registers = self.gb.core_mut().registers_mut();
        registers.sp = sp;
        registers.pc = routine.address;
        registers.a = a;
        registers.c = c;
        let trap = Breakpoint::new(0, TRAP);
        let (stop, _) = self.gb.run_until(&[trap], MachineCycles::PER_FRAME * 60);
        assert_eq!(stop, Stop::Breakpoint(trap), "{routine} did not return");
        self.gb.core().registers().a
    }

    /// Every sound register to zero and every audio global the engine reads with it.
    fn normalise(&mut self) {
        let mmu = self.gb.core_mut().mmu_mut();
        // The APU off and on again clears the registers; they are then writable to be sure of it.
        mmu.write(0xFF26, 0x00);
        mmu.write(0xFF26, 0x80);
        for address in (0xFF10..=0xFF25).chain(0xFF30..=0xFF3F) {
            mmu.write(address, 0);
        }
        // The whole audio block, every `wChannel*` array included. `stopAllAudio` leaves the
        // fractional note delays, the octaves, the volumes and the pitch slide targets exactly as
        // it found them, so without this a save state's leftovers put a cry's first note a frame
        // out and nothing says why.
        for address in pokered_symbols::wMuteAudioAndPauseMusic.address..=pokered_symbols::wTempoModifier.address {
            mmu.write(address, 0);
        }
        // The four that live away from the block; `FadeOutAudio` reads `BIT_NO_AUDIO_FADE_OUT` out
        // of the last of them.
        for global in [
            pokered_symbols::wLowHealthAlarm,
            pokered_symbols::wAudioFadeOutControl,
            pokered_symbols::wAudioFadeOutCounter,
            pokered_symbols::wAudioFadeOutCounterReloadValue,
            pokered_symbols::wStatusFlags2,
            pokered_symbols::wLastMusicSoundID,
        ] {
            mmu.write(global.address, 0);
        }
        mmu.write(pokered_symbols::wAudioROMBank.address, self.bank.rom_bank());
        mmu.write(pokered_symbols::wAudioSavedROMBank.address, self.bank.rom_bank());
    }

    /// One sound from a normalised machine: what the engine writes over `FRAMES` frames.
    fn trace(&mut self, id: SoundId) -> Vec<String> {
        let (play_sound, update_music) = routines(self.bank);
        self.normalise();
        self.call(play_sound, SoundId::STOP_ALL_MUSIC.0);
        self.call(play_sound, id.0);
        self.gb.core_mut().mmu_mut().capture_sound_writes();
        self.gb.core_mut().mmu_mut().take_sound_writes();
        (0..FRAMES)
            .map(|_| {
                self.call(pokered_symbols::FadeOutAudio, 0);
                if self.bank == AudioBank::Two {
                    self.call(pokered_symbols::Music_DoLowHealthAlarm, 0);
                }
                self.call(update_music, 0);
                let writes: Vec<Write> = self
                    .gb
                    .core_mut()
                    .mmu_mut()
                    .take_sound_writes()
                    .into_iter()
                    .filter_map(|(_, address, value)| Write::decode(address, value))
                    .collect();
                encode_writes(&writes)
            })
            .collect()
    }
}

impl Cartridge {
    /// The writes of one frame: `VBlank`'s audio, in its order, in whichever bank
    /// `wAudioROMBank` names by then.
    fn vblank(&mut self, trace: &mut CueTrace) {
        self.call(pokered_symbols::FadeOutAudio, 0);
        let bank = self.gb.core().mmu().read(pokered_symbols::wAudioROMBank.address);
        let bank = AudioBank::from_rom_bank(bank).expect("an audio bank");
        let (_, update_music) = routines(bank);
        if bank == AudioBank::Two {
            self.call(pokered_symbols::Music_DoLowHealthAlarm, 0);
        }
        self.call(update_music, 0);
        let writes: Vec<Write> = self
            .gb
            .core_mut()
            .mmu_mut()
            .take_sound_writes()
            .into_iter()
            .filter_map(|(_, address, value)| Write::decode(address, value))
            .collect();
        let mmu = self.gb.core().mmu();
        let ids = pokered_symbols::wChannelSoundIDs.address;
        let alarm = mmu.read(pokered_symbols::wLowHealthAlarm.address) & 0x80 != 0;
        let finished = alarm || [4, 5, 7].iter().all(|&c| mmu.read(ids + c) == 0);
        self.fading.push(mmu.read(pokered_symbols::wAudioFadeOutControl.address) != 0);
        trace.push(encode_writes(&writes), finished);
    }

    /// A sequence of cues through the game's own home routines, from a normalised machine.
    fn cues(&mut self, input: &CueInput) -> CueTrace {
        self.bank = input.bank;
        let (play_sound, _) = routines(self.bank);
        self.normalise();
        self.call(play_sound, SoundId::STOP_ALL_MUSIC.0);
        self.gb.core_mut().mmu_mut().capture_sound_writes();
        self.gb.core_mut().mmu_mut().take_sound_writes();
        let mut trace = CueTrace::default();
        for &cue in &input.cues {
            match cue {
                Cue::Music(sound) => {
                    self.call_with(pokered_symbols::PlayMusic, sound.id.0, sound.bank.rom_bank());
                }
                Cue::Sound(id) => {
                    self.call(pokered_symbols::PlaySound, id.0);
                }
                Cue::Cry(index) => {
                    let id = self.call_with(pokered_symbols::GetCryData, index, 0);
                    self.call(pokered_symbols::PlaySound, id);
                }
                Cue::LowHealthAlarm(on) => {
                    let mmu = self.gb.core_mut().mmu_mut();
                    let alarm = pokered_symbols::wLowHealthAlarm.address;
                    let value = if on { mmu.read(alarm) | 0x80 } else { 0xFF };
                    mmu.write(alarm, value);
                }
                Cue::EndLowHealthAlarm => {
                    let mmu = self.gb.core_mut().mmu_mut();
                    mmu.write(pokered_symbols::wLowHealthAlarm.address, 0);
                    mmu.write(pokered_symbols::wChannelSoundIDs.address + 4, 0);
                }
                Cue::Modifiers(frequency, tempo) => {
                    let mmu = self.gb.core_mut().mmu_mut();
                    mmu.write(pokered_symbols::wFrequencyModifier.address, frequency);
                    mmu.write(pokered_symbols::wTempoModifier.address, tempo);
                }
                Cue::StopMusic(frames) => {
                    // `StopMusic` up to its wait, which a harness with interrupts masked never leaves.
                    let mmu = self.gb.core_mut().mmu_mut();
                    mmu.write(pokered_symbols::wAudioFadeOutControl.address, frames);
                    mmu.write(pokered_symbols::wNewSoundID.address, SoundId::STOP_ALL_MUSIC.0);
                    self.call(pokered_symbols::PlaySound, SoundId::STOP_ALL_MUSIC.0);
                }
                Cue::FadeOutToSilence(frames) => {
                    // `Music_Cities1AlternateTempo`'s three writes, before its `DelayFrames 100`.
                    let mmu = self.gb.core_mut().mmu_mut();
                    mmu.write(pokered_symbols::wAudioFadeOutCounterReloadValue.address, frames);
                    mmu.write(pokered_symbols::wAudioFadeOutCounter.address, frames);
                    mmu.write(pokered_symbols::wAudioFadeOutControl.address, SoundId::STOP_ALL_MUSIC.0);
                }
                Cue::StopAllSounds => self.call(pokered_symbols::StopAllSounds, 0),
                Cue::Frames(n) => {
                    for _ in 0..n {
                        self.vblank(&mut trace);
                    }
                }
            }
        }
        trace
    }
}

fn compare_cues(cartridge: &mut Cartridge, input: &CueInput) {
    let expected = cartridge.cues(input);
    let actual = input.play();
    assert_eq!(expected.writes.len(), actual.writes.len());
    assert!(expected.writes.iter().any(|frame| !frame.is_empty()), "{input:?} is silent on the cartridge");
    for (frame, (expected, actual)) in expected.writes.iter().zip(&actual.writes).enumerate() {
        assert_eq!(actual, expected, "{input:?}, frame {frame}");
    }
    assert_eq!(actual.finished, expected.finished, "{input:?}: when the sound finishes");
}

/// The recreation playing the same sound the same way, for the same frames.
fn recreation(bank: AudioBank, id: SoundId) -> Vec<String> {
    let mut engine = AudioEngine::new(bank);
    engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
    engine.engine_play_sound(id);
    engine.take_writes();
    (0..FRAMES).map(|_| encode_writes(&engine.frame())).collect()
}

fn compare(bank: AudioBank, ids: &[SoundId]) {
    let mut cartridge = Cartridge::new(bank);
    for &id in ids {
        let expected = cartridge.trace(id);
        let actual = recreation(bank, id);
        for (frame, (expected, actual)) in expected.iter().zip(&actual).enumerate() {
            assert_eq!(actual, expected, "{bank:?} {id:?}, frame {frame}");
        }
    }
}

/// The default tier's share: one song, one sound effect and one cry, against the cartridge.
#[test]
fn the_port_matches_the_cartridge_on_a_song_a_sound_effect_and_a_cry() {
    use pokered::audio::data::sounds;
    compare(AudioBank::One, &[
        sounds::MUSIC_PALLET_TOWN.id,
        sounds::SFX_COLLISION,
        sounds::SFX_CRY_00,
    ]);
}

/// A cry, then the low health alarm, then a sound effect, over the battle music: the alarm holds
/// channel 5 by writing its id behind the engine's back.
fn alarm_after_a_cry() -> CueInput {
    use pokered::audio::data::sounds;
    use poke_core::species::PokemonSpecies;
    CueInput {
        bank: AudioBank::Two,
        cues: vec![
            Cue::Music(sounds::MUSIC_WILD_BATTLE),
            Cue::Cry(PokemonSpecies::Pidgey as u8),
            Cue::Frames(80),
            Cue::LowHealthAlarm(true),
            Cue::Frames(40),
            Cue::Sound(sounds::SFX_PRESS_AB),
            Cue::Frames(120),
        ],
    }
}

/// Every caller that writes `wFrequencyModifier` and `wTempoModifier` before its `PlaySound`, in
/// the bank it plays in, each after a cry has left its own modifiers behind.
fn modifier_callers() -> Vec<CueInput> {
    use pokered::audio::data::{sounds, Cry};
    use poke_core::species::PokemonSpecies;
    let marowak = PokemonSpecies::Marowak as u8;
    let after_a_cry = |cues: &[Cue]| {
        let mut all = vec![Cue::Music(sounds::MUSIC_WILD_BATTLE), Cue::Cry(marowak), Cue::Frames(60)];
        all.extend_from_slice(cues);
        CueInput { bank: AudioBank::Two, cues: all }
    };
    // `GetMoveSound` for GROWL: the user's cry, with the move's row added to the cry's modifiers.
    let growl = Cry::of_species_index(PokemonSpecies::Pidgey as u8);
    vec![
        // `PrintEnemyMonAppearedText`'s `.playSFX`, the unveiled ghost's included.
        after_a_cry(&[Cue::Modifiers(0x00, 0x80), Cue::Sound(sounds::SFX_TRAINER_APPEARED), Cue::Frames(260)]),
        // The same sound left at the cry's modifiers, which is what a missing write sounds like.
        after_a_cry(&[Cue::Sound(sounds::SFX_TRAINER_APPEARED), Cue::Frames(300)]),
        // `HandleEnemyMonFainted`'s fall, then the thud at whatever the fall left.
        after_a_cry(&[
            Cue::Modifiers(0x00, 0x00),
            Cue::Sound(sounds::SFX_FAINT_FALL),
            Cue::Frames(60),
            Cue::Sound(sounds::SFX_FAINT_THUD),
            Cue::Frames(60),
        ]),
        // `PlayApplyingAttackSound`'s three.
        after_a_cry(&[Cue::Modifiers(0x20, 0x30), Cue::Sound(sounds::SFX_DAMAGE), Cue::Frames(60)]),
        after_a_cry(&[Cue::Modifiers(0xE0, 0xFF), Cue::Sound(sounds::SFX_SUPER_EFFECTIVE), Cue::Frames(90)]),
        after_a_cry(&[Cue::Modifiers(0x50, 0x01), Cue::Sound(sounds::SFX_NOT_VERY_EFFECTIVE), Cue::Frames(60)]),
        // `GetMoveSound`: a row of `MoveSoundTable` (MEGA_PUNCH), and a cry move.
        after_a_cry(&[Cue::Modifiers(0x00, 0x40), Cue::Sound(sounds::SFX_BATTLE_0D), Cue::Frames(90)]),
        after_a_cry(&[
            Cue::Modifiers(growl.frequency_modifier, growl.tempo_modifier.wrapping_add(0xC0)),
            Cue::Sound(growl.sound),
            Cue::Frames(90),
        ]),
    ]
}

/// `Music_Cities1AlternateTempo`'s fade to silence and the song after it, and `HandleBlackOut`'s
/// `StopMusic` then `StopAllSounds`.
fn fades() -> Vec<CueInput> {
    use pokered::audio::data::sounds;
    vec![
        CueInput {
            bank: AudioBank::One,
            cues: vec![Cue::Music(sounds::MUSIC_CITIES1), Cue::Frames(40), Cue::FadeOutToSilence(10), Cue::Frames(100),
                Cue::Music(sounds::MUSIC_CITIES1), Cue::Frames(60)],
        },
        CueInput {
            bank: AudioBank::One,
            cues: vec![Cue::Music(sounds::MUSIC_PALLET_TOWN), Cue::Frames(40), Cue::StopMusic(8), Cue::Frames(90),
                Cue::StopAllSounds, Cue::Frames(30)],
        },
    ]
}

/// The writes of each fade, the volume's steps among them, and the frame it ends on.
#[test]
fn a_fade_to_silence_and_stop_music_match_the_cartridge() {
    let mut cartridge = Cartridge::new(AudioBank::One);
    for input in fades() {
        cartridge.fading.clear();
        compare_cues(&mut cartridge, &input);
        let mut engine = AudioEngine::new(input.bank);
        engine.engine_play_sound(SoundId::STOP_ALL_MUSIC);
        let mut fading = Vec::new();
        for &cue in &input.cues {
            match cue {
                Cue::Music(sound) => engine.play_music(sound),
                Cue::StopMusic(frames) => engine.stop_music(frames),
                Cue::FadeOutToSilence(frames) => engine.fade_out_to_silence(frames),
                Cue::StopAllSounds => engine.stop_all_sounds(),
                Cue::Frames(n) => (0..n).for_each(|_| {
                    engine.frame();
                    fading.push(engine.fading_out());
                }),
                other => unreachable!("{other:?}"),
            }
        }
        assert_eq!(fading, cartridge.fading, "{input:?}: when the fade ends");
        assert!(fading.iter().any(|&f| f) && !fading.last().unwrap(), "{input:?} fades and stops");
    }
}

#[test]
fn a_sound_effect_after_a_cry_with_the_alarm_on_matches_the_cartridge() {
    compare_cues(&mut Cartridge::new(AudioBank::Two), &alarm_after_a_cry());
}

/// The modifiers are the caller's to write, and the trainer-appeared sound's length is theirs: at
/// `$00`/`$80` it is 17 frames, and left at Marowak's cry's `$60` it is 271, because a tempo below
/// `$0100` rounds a one-frame note's delay down to 0, which counts down from 255.
#[test]
fn every_caller_of_the_frequency_and_tempo_modifiers_matches_the_cartridge() {
    let mut cartridge = Cartridge::new(AudioBank::Two);
    let callers = modifier_callers();
    for input in &callers {
        compare_cues(&mut cartridge, input);
    }
    // Both open with 60 frames of cry.
    let length = |input: &CueInput| input.play().finished[60..].find('1').expect("the sound finishes");
    assert_eq!(length(&callers[0]), 17);
    assert_eq!(length(&callers[1]), 271);
}

/// Every sound effect in `bank`, played over a song after a cry with the low health alarm on, at
/// a few offsets from the alarm's tones, then the alarm turned off both ways the battle does it.
/// The alarm runs only in `AUDIO_2`'s `VBlank`; in the other two the byte is inert and the sweep
/// says so.
#[cfg(feature = "slow-tests")]
fn alarm_sweep(bank: AudioBank) -> Vec<CueInput> {
    use pokered::audio::data::sounds;
    use poke_core::species::PokemonSpecies;
    let song = match bank {
        AudioBank::One => sounds::MUSIC_ROUTES1,
        AudioBank::Two => sounds::MUSIC_WILD_BATTLE,
        AudioBank::Three => sounds::MUSIC_DUNGEON1,
    };
    // Frames of cry before the alarm, and frames of alarm before the sound: the first tick plays
    // the high tone, the eleventh the low one, and the thirty-first starts again.
    let offsets: &[(u16, u16, bool)] = match bank {
        AudioBank::Two => &[(80, 0, false), (80, 11, true), (80, 31, false), (10, 5, true)],
        _ => &[(80, 0, false), (10, 5, true)],
    };
    let effects = bank.sound_ids().into_iter().filter(|&id| id <= bank.max_sfx_id());
    effects
        .flat_map(|id| {
            offsets.iter().map(move |&(cry, alarm, end)| CueInput {
                bank,
                cues: vec![
                    Cue::Music(song),
                    Cue::Cry(PokemonSpecies::Pidgey as u8),
                    Cue::Frames(cry),
                    Cue::LowHealthAlarm(true),
                    Cue::Frames(alarm),
                    Cue::Sound(id),
                    Cue::Frames(60),
                    if end { Cue::EndLowHealthAlarm } else { Cue::LowHealthAlarm(false) },
                    Cue::Frames(30),
                ],
            })
        })
        .collect()
}

#[test]
#[cfg(feature = "slow-tests")]
fn every_sound_effect_after_a_cry_with_the_alarm_on_matches_the_cartridge() {
    for bank in AudioBank::ALL {
        let mut cartridge = Cartridge::new(bank);
        let inputs = alarm_sweep(bank);
        for input in &inputs {
            compare_cues(&mut cartridge, input);
        }
        println!("{bank:?}: {} sequences", inputs.len());
    }
}

/// Every sound in the cartridge, in all three copies of the engine: 362 of them.
#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    for bank in AudioBank::ALL {
        compare(bank, &bank.sound_ids());
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/audio/cues.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_cue_traces() {
    use super::{write_fixture, Case};
    let mut inputs = vec![alarm_after_a_cry()];
    inputs.extend(fades());
    inputs.extend(modifier_callers());
    // A seventh of the sweep, which lands on every offset and both ways of ending the alarm.
    for bank in AudioBank::ALL {
        inputs.extend(alarm_sweep(bank).into_iter().step_by(7));
    }
    let mut cartridge = Cartridge::new(AudioBank::Two);
    let cases: Vec<Case<CueInput, CueTrace>> = inputs
        .into_iter()
        .map(|input| Case { output: cartridge.cues(&input), input, rng: vec![] })
        .collect();
    write_fixture("audio", "cues", &cases);
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/audio/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_audio_traces() {
    use super::{write_fixture, Case};
    for bank in AudioBank::ALL {
        let mut cartridge = Cartridge::new(bank);
        let cases: Vec<Case<TraceInput, Vec<String>>> = bank
            .sound_ids()
            .into_iter()
            .map(|id| Case { input: TraceInput { bank, id }, output: cartridge.trace(id), rng: vec![] })
            .collect();
        write_fixture("audio", fixture_name(bank), &cases);
    }
}
