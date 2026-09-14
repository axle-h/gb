use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::ram::{RAM, ROM};
use pokered::audio::data::{sounds, SoundId};
use pokered::audio::engine::{encode_writes, AudioEngine};
use pokered::audio::{Voices, Write};
use crate::native::apu::GbApu;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer};
use super::to_vblank;

const SAMPLE_RATE: u32 = 48_000;

fn drain(read: &mut dyn FnMut(&mut [f32]) -> usize, into: &mut Vec<f32>) {
    let mut buffer = [0.0; 4096];
    loop {
        let frames = read(&mut buffer);
        if frames == 0 {
            return;
        }
        into.extend_from_slice(&buffer[..frames * 2]);
    }
}

/// The cartridge's own sound writes, replayed through `Write` into a copy of its APU, play the same
/// music: within 2% RMS, since the cartridge's writes and level changes land on instruction
/// boundaries a replay cannot see.
#[test]
fn the_cartridge_s_writes_replayed_as_writes_play_the_same_music() {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/pallet-town-state.bin")).unwrap();
    gb.core_mut().mmu_mut().audio_mut().set_output_sample_rate(SAMPLE_RATE);
    to_vblank(&mut gb);
    drain(&mut |out| gb.core_mut().mmu_mut().audio_mut().read_samples_f32(out), &mut vec![]);

    let mut apu = GbApu::from_machine(gb.core().mmu());
    gb.core_mut().mmu_mut().capture_sound_writes();
    let (mut cartridge, mut frames) = (vec![], vec![]);
    for _ in 0..300 {
        to_vblank(&mut gb);
        frames.push(gb.core().mmu().now());
        drain(&mut |out| gb.core_mut().mmu_mut().audio_mut().read_samples_f32(out), &mut cartridge);
    }
    let writes = gb.core_mut().mmu_mut().take_sound_writes();
    assert!(writes.len() > 300, "the music wrote only {} times", writes.len());

    let mut replayed = vec![];
    let mut writes = writes.into_iter().peekable();
    for end in frames {
        while let Some(&(at, address, value)) = writes.peek().filter(|(at, _, _)| *at < end) {
            apu.advance_to(at);
            apu.write(Write::decode(address, value).expect("a sound register"));
            writes.next();
        }
        apu.advance_to(end);
        drain(&mut |out| apu.read_samples(out), &mut replayed);
    }
    assert!(cartridge.iter().any(|&s| s.abs() > 0.01), "the cartridge was silent");
    assert_eq!(cartridge.len(), replayed.len());
    let rms = |samples: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = samples.fold((0.0f64, 0usize), |(sum, n), s| (sum + (s as f64).powi(2), n + 1));
        (sum / n as f64).sqrt()
    };
    let error = rms(&mut cartridge.iter().zip(&replayed).map(|(a, b)| a - b));
    let signal = rms(&mut cartridge.iter().copied());
    assert!(error < signal * 0.02, "the replay is off by {error} RMS against {signal}");
}

/// Where a called routine returns to: unusable memory, so nothing else ever executes there.
const TRAP: u16 = 0xFEA0;

/// `call routine` with `a` and `c` set, on a machine that is mid-run, put back exactly as it was
/// found so the game carries on afterwards.
fn call(gb: &mut GameBoy, routine: DmgPointer, a: u8, c: u8) {
    let saved = gb.core().registers().clone();
    let sp = saved.sp - 2;
    gb.core_mut().mmu_mut().write_u16_le(sp, TRAP);
    let registers = gb.core_mut().registers_mut();
    *registers = saved.clone();
    registers.a = a;
    registers.c = c;
    registers.sp = sp;
    registers.pc = routine.address;
    let trap = Breakpoint::new(0, TRAP);
    let (stop, _) = gb.run_until(&[trap], MachineCycles::PER_FRAME * 60);
    assert_eq!(stop, Stop::Breakpoint(trap), "{routine} did not return");
    *gb.core_mut().registers_mut() = saved;
}

/// Tier 2: the recreation's engine beside the cartridge's, with the cartridge running for real.
/// The song is started through the game's own `PlayMusic`, the audio engine is driven by the real
/// VBlank interrupt in the bank VBlank switches to, and the overworld goes on executing between
/// frames. Every register write matches, frame for frame, for five seconds.
#[test]
fn the_recreation_writes_what_the_running_cartridge_writes() {
    let song = sounds::MUSIC_PALLET_TOWN;
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/pallet-town-state.bin")).unwrap();
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    to_vblank(&mut gb);

    // Both sides start from the same audio state. `stopAllAudio` leaves the fractional note delays,
    // the octaves and the volumes as it found them, so without this the song already playing would
    // decide where the recreation's first notes land.
    for address in pokered_symbols::wMuteAudioAndPauseMusic.address..=pokered_symbols::wTempoModifier.address {
        gb.core_mut().mmu_mut().write(address, 0);
    }
    // `wAudioROMBank` is inside that block and `PlaySound` dispatches on it, so it has to be put
    // back: left at zero the MBC maps bank 1 and the call lands in the middle of it.
    for symbol in [pokered_symbols::wAudioROMBank, pokered_symbols::wAudioSavedROMBank] {
        gb.core_mut().mmu_mut().write(symbol.address, song.bank.rom_bank());
    }
    // `FadeOutAudio` reads `BIT_NO_AUDIO_FADE_OUT`, bit 1 of `wStatusFlags2`, which belongs to the
    // game rather than to the audio engine, so it is read across rather than zeroed.
    let no_audio_fade_out = gb.core().mmu().read(pokered_symbols::wStatusFlags2.address) & 1 << 1 != 0;

    let mut engine = AudioEngine::new(song.bank);
    engine.no_audio_fade_out = no_audio_fade_out;
    engine.play_sound(SoundId::STOP_ALL_MUSIC);
    engine.play_music(song);

    gb.core_mut().mmu_mut().capture_sound_writes();
    call(&mut gb, pokered_symbols::PlaySound, SoundId::STOP_ALL_MUSIC.0, 0);
    call(&mut gb, pokered_symbols::PlayMusic, song.id.0, song.bank.rom_bank());

    let mut played = 0;
    for frame in 0..300 {
        to_vblank(&mut gb);
        let cartridge: Vec<Write> = gb
            .core_mut()
            .mmu_mut()
            .take_sound_writes()
            .into_iter()
            .filter_map(|(_, address, value)| Write::decode(address, value))
            .collect();
        played += cartridge.len();
        assert_eq!(encode_writes(&engine.frame()), encode_writes(&cartridge), "frame {frame}");
    }
    assert!(played > 300, "the cartridge only wrote {played} registers");
}
