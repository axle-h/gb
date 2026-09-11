use gb::game_boy::GameBoy;
use pokered::audio::{Voices, Write};
use crate::native::apu::GbApu;
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
