//! Capture harness for the resampler: pulls real Pokémon Red audio out of the emulator, either as
//! the transition stream the Blip synth sees or as a rendered WAV.
//!
//! Two jobs, both `#[ignore]`d because they are generators and ear checks rather than assertions:
//!
//! 1. **Golden input.** [`tests::capture_golden_input`] freezes 30 ms of real APU output as
//!    `data/apu_capture_in.bin`, already quantised to the synth's integer amplitude domain. That
//!    file is the realistic-signal input for both `tools/blip-golden/gen_golden.cpp` and the Rust
//!    test that checks against its output, so regenerating one means regenerating the other.
//! 2. **Ear check.** [`tests::render_reference_wav`] renders a few seconds to
//!    `target/test-artifacts/`, for listening to against `rubato-reference.wav` — which was
//!    captured through the old resampler before it was removed.

use std::path::Path;
use std::time::Duration;

use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;

/// A mid-game fixture with overworld music playing, so the captured seconds are actually audible.
pub const CAPTURE_FIXTURE: &[u8] = include_bytes!("../pokemon/data/at-celadon.bin");

/// Emulated seconds to render into the ear-check WAV.
pub const CAPTURE_SECONDS: u64 = 6;

/// Emulated milliseconds to freeze as the golden-test input fixture.
pub const GOLDEN_INPUT_MILLIS: u64 = 20;

pub fn artifact_dir() -> &'static Path {
    Path::new("target/test-artifacts")
}

fn load_fixture(save_state: &[u8]) -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(save_state).expect("failed to load save state");
    gb
}

/// Run the emulator and log every amplitude transition the synth is handed, run-length merged.
///
/// The runs are the instruction boundaries the APU actually changes level on — a few tens of
/// thousands per second rather than the full 1 048 576 Hz, which is exactly why the band-limited
/// approach is cheap.
pub fn capture_transitions(save_state: &[u8], game_time: Duration) -> Vec<(u16, i16, i16)> {
    let mut gb = load_fixture(save_state);
    gb.core_mut().mmu_mut().audio_mut().output.start_capture();

    let target = MachineCycles::from_duration(game_time);
    let mut elapsed = MachineCycles::ZERO;
    let slice = MachineCycles::from_duration(Duration::from_millis(10));
    while elapsed < target {
        elapsed += gb.run(slice);
    }
    gb.core_mut().mmu_mut().audio_mut().output.take_capture()
}

/// Run the emulator and collect the resampled output as interleaved stereo `f32`.
pub fn render(save_state: &[u8], game_time: Duration) -> Vec<f32> {
    let mut gb = load_fixture(save_state);
    let target = MachineCycles::from_duration(game_time);
    let mut elapsed = MachineCycles::ZERO;
    let slice = MachineCycles::from_duration(Duration::from_millis(10));

    let mut out = Vec::new();
    let mut scratch = vec![0.0f32; 8192];
    while elapsed < target {
        elapsed += gb.run(slice);
        let frames = gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut scratch);
        out.extend_from_slice(&scratch[..frames * 2]);
    }
    out
}

/// Encode transitions as `u32 run_count`, then `run_count` × (`u16` clocks, `i16` left,
/// `i16` right), all little-endian. Read back by [`rle_decode`] and by `gen_golden.cpp`.
pub fn encode_runs(runs: &[(u16, i16, i16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + runs.len() * 6);
    out.extend_from_slice(&(runs.len() as u32).to_le_bytes());
    for (clocks, left, right) in runs {
        out.extend_from_slice(&clocks.to_le_bytes());
        out.extend_from_slice(&left.to_le_bytes());
        out.extend_from_slice(&right.to_le_bytes());
    }
    out
}

/// Decode what [`encode_runs`] produced.
pub fn rle_decode(bytes: &[u8]) -> Vec<(u16, i16, i16)> {
    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    (0..count)
        .map(|i| {
            let at = 4 + i * 6;
            (
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()),
                i16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap()),
                i16::from_le_bytes(bytes[at + 4..at + 6].try_into().unwrap()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::blip::AMP_SCALE;
    use crate::audio::wav::{f32_to_i16, write_wav_i16};

    /// Freeze 30 ms of real APU output as the golden test's input signal.
    ///
    /// Writes `src/audio/data/apu_capture_in.bin`. After running this, regenerate the matching
    /// output goldens with `tools/blip-golden/build.sh` — they are computed from this file.
    ///
    /// `cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored --nocapture`
    #[test]
    #[ignore = "fixture generator, not a test; run with --ignored"]
    fn capture_golden_input() {
        let runs = capture_transitions(CAPTURE_FIXTURE, Duration::from_millis(GOLDEN_INPUT_MILLIS));

        // A capture that is silent, or stuck on a handful of levels, would let the golden test look
        // like it passes while exercising almost nothing.
        let levels: std::collections::BTreeSet<i16> = runs.iter().flat_map(|(_, l, r)| [*l, *r]).collect();
        let peak = levels.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let clocks: u32 = runs.iter().map(|(c, ..)| *c as u32).sum();
        assert!(runs.len() > 500, "only {} transitions — capture is too short", runs.len());
        assert!(levels.len() > 16, "only {} distinct levels — capture is not representative", levels.len());

        let encoded = encode_runs(&runs);
        let path = Path::new("src/audio/data/apu_capture_in.bin");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &encoded).unwrap();
        println!("wrote {} ({} runs, {} bytes)", path.display(), runs.len(), encoded.len());
        println!(
            "  {} distinct levels, peak {peak}/{}, {clocks} clocks, mean run {:.1}",
            levels.len(),
            AMP_SCALE as i32,
            clocks as f64 / runs.len() as f64,
        );
    }

    /// Render `CAPTURE_SECONDS` of game audio for an A/B listen against `rubato-reference.wav`,
    /// which was captured through the old resampler before it was removed.
    ///
    /// `cargo test --release --bin gb -- audio::reference::tests::render_reference_wav --exact --ignored --nocapture`
    #[test]
    #[ignore = "ear check, not a test; run with --ignored"]
    fn render_reference_wav() {
        let out = render(CAPTURE_FIXTURE, Duration::from_secs(CAPTURE_SECONDS));
        let rate = crate::audio::blip::DEFAULT_SAMPLE_RATE;
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));

        let path = artifact_dir().join("blip-reference.wav");
        write_wav_i16(&path, rate, 2, &f32_to_i16(&out)).unwrap();
        println!(
            "wrote {} ({} frames, {:.2}s, peak {:.4} = {:.1} dBFS)",
            path.display(),
            out.len() / 2,
            out.len() as f64 / 2.0 / rate as f64,
            peak,
            20.0 * peak.log10(),
        );
    }
}
