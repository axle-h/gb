//! Capture harness for the resampler: pulls real Pokémon Red audio out of the emulator as the
//! transition stream the Blip synth sees.

#[cfg(feature = "slow-tests")]
use std::path::Path;
#[cfg(feature = "slow-tests")]
use std::time::Duration;

#[cfg(feature = "slow-tests")]
use crate::cycles::MachineCycles;
#[cfg(feature = "slow-tests")]
use crate::game_boy::GameBoy;

#[cfg(feature = "slow-tests")]
/// A mid-game fixture with overworld music playing, so the captured seconds are actually audible.
pub const CAPTURE_FIXTURE: &[u8] = crate::test_fixtures::AT_CELADON;

#[cfg(feature = "slow-tests")]
/// Emulated milliseconds to freeze as the golden-test input fixture.
pub const GOLDEN_INPUT_MILLIS: u64 = 20;

#[cfg(feature = "slow-tests")]
fn load_fixture(save_state: &[u8]) -> GameBoy {
    let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
    gb.load_state(save_state).expect("failed to load save state");
    gb
}

#[cfg(feature = "slow-tests")]
/// Run the emulator and log every amplitude transition the synth is handed, run-length merged.
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

#[cfg(feature = "slow-tests")]
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
    #[cfg(feature = "slow-tests")]
    use super::*;
    #[cfg(feature = "slow-tests")]
    use crate::audio::blip::AMP_SCALE;

    /// Freeze 30 ms of real APU output as the golden test's input signal.
    #[test]
    #[cfg(feature = "slow-tests")]
    #[ignore = "fixture generator, not a test; run with --ignored"]
    fn capture_golden_input() {
        let runs = capture_transitions(CAPTURE_FIXTURE, Duration::from_millis(GOLDEN_INPUT_MILLIS));

        // A capture that is silent, or stuck on a handful of levels, would let the golden test
        // look like it passes while exercising almost nothing.
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
