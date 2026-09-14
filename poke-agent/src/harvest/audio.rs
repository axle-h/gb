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
use gb::ram::RAM;
use pokered::audio::data::{AudioBank, SoundId};
use pokered::audio::engine::{encode_writes, AudioEngine, TraceInput};
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
        Self { gb, stack, bank }
    }

    /// `call routine` with `a` set, run until it returns.
    fn call(&mut self, routine: DmgPointer, a: u8) {
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
        let trap = Breakpoint::new(0, TRAP);
        let (stop, _) = self.gb.run_until(&[trap], MachineCycles::PER_FRAME * 60);
        assert_eq!(stop, Stop::Breakpoint(trap), "{routine} did not return");
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
