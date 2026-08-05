//! Capture harness for the resampler: pulls real Pokémon Red audio out of the emulator as the
//! transition stream the Blip synth sees.
//!
//! One job, `#[ignore]`d because it is a fixture generator rather than an assertion:
//! [`tests::capture_golden_input`] freezes 30 ms of real APU output as `data/apu_capture_in.bin`,
//! already quantised to the synth's integer amplitude domain. That file is the realistic-signal
//! input for both `tools/blip-golden/gen_golden.cpp` and the Rust test that checks against its
//! output, so regenerating one means regenerating the other.
//!
//! A WAV "ear check" used to live here too — it rendered a few seconds for a listen against
//! `rubato-reference.wav`. It was a listening aid, not an assertion, and the invariant tests in
//! `blip/tests.rs` are the real regression net, so it was removed rather than left in the ignored
//! list pretending to be a test.

use std::path::Path;
use std::time::Duration;

use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;

/// A mid-game fixture with overworld music playing, so the captured seconds are actually audible.
pub const CAPTURE_FIXTURE: &[u8] = include_bytes!("../pokemon/data/at-celadon.bin");

/// Emulated milliseconds to freeze as the golden-test input fixture.
pub const GOLDEN_INPUT_MILLIS: u64 = 20;

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

    /// Freeze 30 ms of real APU output as the golden test's input signal.
    ///
    /// Writes `src/audio/data/apu_capture_in.bin`. After running this, regenerate the matching
    /// output goldens with `tools/blip-golden/build.sh` — they are computed from this file.
    ///
    /// `cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored --nocapture`
    #[test]
    #[cfg(feature = "diagnostics")]
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

}
