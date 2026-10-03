//! The recreated game's hardware-shaped backends, which need `gb`.

pub mod apu;

use pokered::Game;
use pokered::audio::Voices;
use pokered::audio::synth::Synth;

/// A synth to play `game` through from this frame on. The audio engine never writes `NR52`, since
/// the cartridge powers the APU on outside it, and a new backend is told what the engine is
/// holding, or a note playing across the switch stays silent.
pub fn synth_for(game: &Game) -> Synth {
    let mut synth = Synth::new();
    synth.write(pokered::audio::Write::Power(true));
    for write in game.audio().standing_writes() {
        synth.write(write);
    }
    synth
}
