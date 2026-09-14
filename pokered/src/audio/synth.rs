//! The synth backend: four voices at 48 kHz, with no emulator under it.
//!
//! **In**: the same [`Write`]s the audio engine hands any backend, applied between frames, and one
//! [`Voices::end_frame`] per VBlank. **Out**: interleaved stereo `f32` at [`SAMPLE_RATE`], read
//! through [`Voices::read_samples`]. Full scale is the hardware's own: four channels at full
//! amplitude with `NR50` at 7 reach ±1.
//!
//! **Exact**: the four channels' own algorithms, from the registers the cartridge writes — duty
//! position and period, the envelope at 64 Hz, the sweep at 128 Hz, the length counters at 256 Hz,
//! the wave channel's 32 nibbles and its output level, and the noise channel's shift register in
//! both widths. The sequencer's eight steps sit where a DMG's do, 2048 M-cycles apart, so a note
//! started on one frame decays on the same tick it would on hardware. Frame counts are exact
//! because a frame is exactly 17,556 M-cycles and the engine's writes all land between two of
//! them: there is no intra-frame timing for a backend to lose.
//!
//! **Recreates**: `NR10`-`NR52` and wave RAM, as `hardware.inc` names them. There is no cartridge
//! label here — the source this chunk ports is the sound hardware the audio engine is written
//! against, and what pins it is the emulator's APU over the harvested register traces.
//!
//! **How the samples are made.** Not by sampling the waveform, which would fold a pulse channel's
//! harmonics down into the audible band: every voice reports the level it is holding and the
//! moment it can next change, so the renderer integrates a constant level over each interval and
//! the result is the waveform's exact area over one internal sample. Those run at
//! [`OVERSAMPLE`]× 48 kHz and a linear-phase windowed sinc decimates them, which puts the
//! remaining images far enough up that they arrive attenuated. Then the output stage: a one-pole
//! treble rolloff standing in for the amplifier and speaker a DMG puts between its DACs and the
//! air ([`TREBLE_ROLLOFF_HZ`]), and a one-pole high pass at 28 Hz where its output capacitor is —
//! a channel whose DAC is on but silent holds a level, so without the second one every note would
//! arrive on a step of its own.
//!
//! **Faithful rather than exact**, both for the sake of the comparison against the emulator's APU:
//! a trigger resets the duty position, which a DMG leaves where it stands — inaudible either way,
//! since a periodic wave sounds the same at any phase, but a phase difference is the one thing a
//! sample-by-sample comparison cannot see past. And the master volume and panning take effect at
//! the internal sample they land in rather than the exact M-cycle, which is 5 µs of slack on a
//! register the engine writes once a frame with the same value.
//!
//! **Three places this and the emulator's APU disagree, where the hardware is what was followed.**
//! A trigger's sweep calculation is made for the overflow check and thrown away, so the note sounds
//! at the period that was written until the first 128 Hz iteration moves it. `NR32` shifts the
//! 4-bit sample rather than the byte holding it, so a low nibble does not take the bit above it.
//! And the level is read from the duty position continuously rather than held between edges, so a
//! note is heard from the trigger rather than a period later and an envelope step lands where it
//! falls. All three are measured in `lockstep::synth` in `poke-agent`, which is also where what
//! they cost is written down.

mod voices;

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use voices::{Noise, Pulse, Wave};
use super::{Channel, Voices, Write};

/// The rate the synth is built at. The web host's Opus stream and the SDL sink are both 48 kHz,
/// and the sub-unit arithmetic below is exact only at this rate.
pub const SAMPLE_RATE: u32 = 48_000;

/// Internal samples per output sample. Four puts the first image at 192 kHz, where a pulse
/// channel has next to nothing left.
const OVERSAMPLE: u64 = 4;

/// The renderer's clock: 750 of these to an M-cycle, which makes an output sample exactly 16,384
/// of them (1,048,576 / 48,000 = 8192/375) and the wave channel's half-M-cycle tick a whole
/// number too.
const SUBS_PER_MCYCLE: u64 = 750;
const SUBS_PER_SAMPLE: u64 = 16_384;
const SUBS_PER_INTERNAL_SAMPLE: u64 = SUBS_PER_SAMPLE / OVERSAMPLE;

/// One video frame: 154 scanlines of 456 t-cycles.
const M_CYCLES_PER_FRAME: u64 = 17_556;
const SUBS_PER_FRAME: u64 = M_CYCLES_PER_FRAME * SUBS_PER_MCYCLE;

/// The frame sequencer's 512 Hz step, which is DIV bit 4 falling.
const SUBS_PER_SEQUENCER_STEP: u64 = 2048 * SUBS_PER_MCYCLE;

/// Taps in the decimating filter, odd so that it is symmetric about one of them.
const TAPS: usize = 257;
/// Where the filter is to have given up. A pulse channel's harmonics run the whole way up, so
/// cutting early is audible as a duller instrument rather than as a missing top octave; the
/// transition sits astride the top of the band instead.
const CUTOFF_HZ: f32 = 22_500.0;

/// The pole of the output high pass: `1 - 2^-8` at 48 kHz is a corner just under 30 Hz, which is
/// where a DMG's own is.
const HIGH_PASS_POLE: f32 = 1.0 - 1.0 / 256.0;

/// The output stage, which is not the DAC. A DMG has an amplifier and a speaker between its DACs
/// and the air, and both roll treble off; the emulator's resampler stands in for them with a tilt
/// of 8 dB at half the sample rate, and one pole here is that same curve to within a few percent
/// across the band. Without it the synth is flat, which is *brighter* than the cartridge has ever
/// sounded through anything, and a backend that can be swapped under the same host had better not
/// change the character of the sound when it is.
const TREBLE_ROLLOFF_HZ: f32 = 8_250.0;

/// A DMG's DAC: digital 0 is the top of the swing and 15 the bottom.
fn dac(level: u8) -> f32 {
    1.0 - level as f32 * (2.0 / 15.0)
}

/// `NR50`'s three bits per side: 0 is an eighth of the way up rather than silence.
fn master_volume(volume: u8) -> f32 {
    (volume + 1) as f32 / 8.0
}

/// The decimating filter: a sinc at [`CUTOFF_HZ`] under a Blackman window, scaled to unity at DC.
fn filter() -> &'static [f32; TAPS] {
    static FILTER: OnceLock<[f32; TAPS]> = OnceLock::new();
    FILTER.get_or_init(|| {
        let rate = SAMPLE_RATE as f32 * OVERSAMPLE as f32;
        let cutoff = CUTOFF_HZ / rate;
        let middle = (TAPS - 1) as f32 / 2.0;
        let mut taps = [0.0; TAPS];
        for (n, tap) in taps.iter_mut().enumerate() {
            let x = n as f32 - middle;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
            };
            let phase = 2.0 * std::f32::consts::PI * n as f32 / (TAPS - 1) as f32;
            let blackman = 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
            *tap = sinc * blackman;
        }
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|tap| *tap /= sum);
        taps
    })
}

/// A one-pole low pass, one per side: the output stage above.
#[derive(Debug, Clone, Copy)]
struct Treble {
    coefficient: f32,
    last: f32,
}

impl Treble {
    fn new() -> Self {
        let angle = 2.0 * std::f32::consts::PI * TREBLE_ROLLOFF_HZ / SAMPLE_RATE as f32;
        Self { coefficient: 1.0 - (-angle).exp(), last: 0.0 }
    }

    fn apply(&mut self, sample: f32) -> f32 {
        self.last += self.coefficient * (sample - self.last);
        self.last
    }
}

/// A one-pole high pass, one per side.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct HighPass {
    last_in: f32,
    last_out: f32,
}

impl HighPass {
    fn apply(&mut self, sample: f32) -> f32 {
        let out = sample - self.last_in + HIGH_PASS_POLE * self.last_out;
        self.last_in = sample;
        self.last_out = out;
        out
    }
}

/// The counts a test reads to pin the sequencer's rates.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
struct Clocks {
    envelope: u32,
    sweep: u32,
    length: u32,
}

/// Four voices, the mixer they run into and the resampler that ends at 48 kHz.
pub struct Synth {
    pulse1: Pulse,
    pulse2: Pulse,
    wave: Wave,
    noise: Noise,
    powered: bool,
    /// `NR51`, a bit per channel and side.
    panning: u8,
    volume_left: u8,
    volume_right: u8,
    /// The frame sequencer's step, 0 to 7, and how long until the next one.
    step: u8,
    subs_to_step: u64,
    /// The level integrated so far into the internal sample being made, and how long it still has
    /// to run.
    accumulated: [f64; 2],
    subs_to_internal_sample: u64,
    /// The last [`TAPS`] internal samples, oldest at `oldest`.
    history: [[f32; 2]; TAPS],
    oldest: usize,
    since_output: u64,
    treble: [Treble; 2],
    high_pass: [HighPass; 2],
    /// Finished frames, interleaved, and how far a reader has got through them.
    out: Vec<f32>,
    read_at: usize,
    #[cfg(test)]
    clocks: Clocks,
}

impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}

impl Synth {
    pub fn new() -> Self {
        Self {
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            wave: Wave::new(),
            noise: Noise::new(),
            // A cartridge writes `NR52` before anything else, but nothing obliges it to, and a
            // backend that starts deaf is a backend that loses the first sound of a run.
            powered: true,
            panning: 0,
            volume_left: 0,
            volume_right: 0,
            step: 7,
            subs_to_step: SUBS_PER_SEQUENCER_STEP,
            accumulated: [0.0; 2],
            subs_to_internal_sample: SUBS_PER_INTERNAL_SAMPLE,
            history: [[0.0; 2]; TAPS],
            oldest: 0,
            since_output: 0,
            treble: [Treble::new(); 2],
            high_pass: [HighPass::default(); 2],
            out: Vec::new(),
            read_at: 0,
            #[cfg(test)]
            clocks: Clocks::default(),
        }
    }

    /// Whether the sequencer step now standing is one of the four that clock a length counter.
    /// Enabling a counter on one, or triggering an expired channel there, costs a tick at once.
    fn on_length_step(&self) -> bool {
        matches!(self.step, 0 | 2 | 4 | 6)
    }

    /// The mixer: every channel whose DAC is on, panned, averaged and scaled by `NR50`. With all
    /// four DACs off the volume units are disconnected from the output rather than fed silence.
    fn mix(&self) -> (f32, f32) {
        let levels = [self.pulse1.level(), self.pulse2.level(), self.wave.level_out(), self.noise.level()];
        if levels.iter().all(Option::is_none) {
            return (0.0, 0.0);
        }
        let (mut left, mut right) = (0.0, 0.0);
        for (channel, level) in levels.into_iter().enumerate() {
            let Some(level) = level else { continue };
            let sample = dac(level);
            if self.panning & 0x10 << channel != 0 {
                left += sample;
            }
            if self.panning & 1 << channel != 0 {
                right += sample;
            }
        }
        (left / 4.0 * master_volume(self.volume_left), right / 4.0 * master_volume(self.volume_right))
    }

    /// How long every voice is guaranteed to hold the level it is holding.
    fn subs_to_transition(&self) -> u64 {
        [
            self.pulse1.subs_to_next(),
            self.pulse2.subs_to_next(),
            self.wave.subs_to_next(),
            self.noise.subs_to_next(),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(u64::MAX)
    }

    fn advance_voices(&mut self, subs: u64) {
        self.pulse1.advance(subs);
        self.pulse2.advance(subs);
        self.wave.advance(subs);
        self.noise.advance(subs);
    }

    fn sequencer_step(&mut self) {
        self.step = self.step + 1 & 7;
        if self.on_length_step() {
            self.pulse1.clock_length();
            self.pulse2.clock_length();
            self.wave.clock_length();
            self.noise.clock_length();
            #[cfg(test)]
            {
                self.clocks.length += 1;
            }
        }
        if matches!(self.step, 2 | 6) {
            self.pulse1.clock_sweep();
            #[cfg(test)]
            {
                self.clocks.sweep += 1;
            }
        }
        if self.step == 7 {
            self.pulse1.clock_envelope();
            self.pulse2.clock_envelope();
            self.noise.clock_envelope();
            #[cfg(test)]
            {
                self.clocks.envelope += 1;
            }
        }
    }

    /// One internal sample: the area under the mixed output over its window.
    fn finish_internal_sample(&mut self) {
        let scale = SUBS_PER_INTERNAL_SAMPLE as f64;
        let sample = [(self.accumulated[0] / scale) as f32, (self.accumulated[1] / scale) as f32];
        self.accumulated = [0.0; 2];
        self.history[self.oldest] = sample;
        self.oldest = (self.oldest + 1) % TAPS;
        self.since_output += 1;
        if self.since_output == OVERSAMPLE {
            self.since_output = 0;
            self.finish_sample();
        }
    }

    fn finish_sample(&mut self) {
        let taps = filter();
        let (mut left, mut right) = (0.0, 0.0);
        for (tap, sample) in taps.iter().zip(self.history[self.oldest..].iter().chain(&self.history[..self.oldest])) {
            left += tap * sample[0];
            right += tap * sample[1];
        }
        let (left, right) = (self.treble[0].apply(left), self.treble[1].apply(right));
        self.out.push(self.high_pass[0].apply(left));
        self.out.push(self.high_pass[1].apply(right));
    }

    /// Play `subs` of sound, splitting it at every moment a voice, the sequencer or an internal
    /// sample says something changes.
    fn render(&mut self, mut subs: u64) {
        while subs > 0 {
            let (left, right) = self.mix();
            let span = subs
                .min(self.subs_to_internal_sample)
                .min(self.subs_to_step)
                .min(self.subs_to_transition());
            debug_assert!(span > 0, "a voice asked to be woken in no time at all");
            self.accumulated[0] += left as f64 * span as f64;
            self.accumulated[1] += right as f64 * span as f64;
            self.advance_voices(span);
            subs -= span;
            self.subs_to_step -= span;
            if self.subs_to_step == 0 {
                self.subs_to_step = SUBS_PER_SEQUENCER_STEP;
                self.sequencer_step();
            }
            self.subs_to_internal_sample -= span;
            if self.subs_to_internal_sample == 0 {
                self.subs_to_internal_sample = SUBS_PER_INTERNAL_SAMPLE;
                self.finish_internal_sample();
            }
        }
    }

    /// `NR52`: powering off clears every channel and register, and powering on restarts the
    /// sequencer a step short of zero. Wave RAM survives both.
    fn set_power(&mut self, on: bool) {
        if on == self.powered {
            return;
        }
        self.powered = on;
        if !on {
            self.pulse1 = Pulse::new(true);
            self.pulse2 = Pulse::new(false);
            self.wave.power_off();
            self.noise = Noise::new();
            self.panning = 0;
            self.volume_left = 0;
            self.volume_right = 0;
            self.step = 0;
        } else {
            self.step = 7;
        }
    }
}

impl Voices for Synth {
    fn write(&mut self, write: Write) {
        // With the power off only the length counters, wave RAM and `NR52` itself are writable.
        if !self.powered && !matches!(write, Write::Power(_) | Write::WaveRam { .. } | Write::Length { .. }) {
            return;
        }
        let on_length_step = self.on_length_step();
        match write {
            Write::Sweep { pace, decrease, step } => self.pulse1.set_sweep(pace, decrease, step),
            Write::Length { channel, duty, length } => match channel {
                Channel::Pulse1 | Channel::Pulse2 => {
                    let pulse = if channel == Channel::Pulse1 { &mut self.pulse1 } else { &mut self.pulse2 };
                    if self.powered {
                        pulse.set_duty(duty);
                    }
                    pulse.set_length(length);
                }
                Channel::Wave => self.wave.set_length(length),
                Channel::Noise => self.noise.set_length(length),
            },
            Write::Envelope { channel, volume, increase, pace } => match channel {
                Channel::Pulse1 => self.pulse1.set_envelope(volume, increase, pace),
                Channel::Pulse2 => self.pulse2.set_envelope(volume, increase, pace),
                Channel::Noise => self.noise.set_envelope(volume, increase, pace),
                // There is no `NR32` envelope; the wave channel's level is its own register.
                Channel::Wave => {}
            },
            Write::PeriodLow { channel, low } => match channel {
                Channel::Pulse1 => self.pulse1.set_period_low(low),
                Channel::Pulse2 => self.pulse2.set_period_low(low),
                Channel::Wave => self.wave.set_period_low(low),
                Channel::Noise => {}
            },
            Write::PeriodHigh { channel, high, trigger, length_enable } => match channel {
                Channel::Pulse1 => self.pulse1.set_period_high(high, trigger, length_enable, on_length_step),
                Channel::Pulse2 => self.pulse2.set_period_high(high, trigger, length_enable, on_length_step),
                Channel::Wave => self.wave.set_period_high(high, trigger, length_enable, on_length_step),
                Channel::Noise => self.noise.set_control(trigger, length_enable, on_length_step),
            },
            Write::WaveDac(on) => self.wave.set_dac(on),
            Write::WaveLevel(level) => self.wave.set_level(level),
            Write::Noise { shift, short, divisor } => self.noise.set_noise(shift, short, divisor),
            Write::MasterVolume { left, right, .. } => {
                self.volume_left = left;
                self.volume_right = right;
            }
            Write::Panning(panning) => self.panning = panning,
            Write::Power(on) => self.set_power(on),
            Write::WaveRam { index, samples } => self.wave.write_ram(index, samples),
        }
    }

    fn end_frame(&mut self) {
        self.render(SUBS_PER_FRAME);
    }

    fn read_samples(&mut self, out: &mut [f32]) -> usize {
        let frames = ((self.out.len() - self.read_at) / 2).min(out.len() / 2);
        out[..frames * 2].copy_from_slice(&self.out[self.read_at..self.read_at + frames * 2]);
        self.read_at += frames * 2;
        if self.read_at == self.out.len() {
            self.out.clear();
            self.read_at = 0;
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::TraceInput;

    /// Left and right, one per output frame.
    fn play(writes: &[Write], frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut synth = Synth::new();
        for &write in writes {
            synth.write(write);
        }
        let (mut left, mut right) = (vec![], vec![]);
        let mut buffer = [0.0; 4096];
        for _ in 0..frames {
            synth.end_frame();
            loop {
                let read = synth.read_samples(&mut buffer);
                if read == 0 {
                    break;
                }
                left.extend(buffer[..read * 2].iter().step_by(2));
                right.extend(buffer[1..read * 2].iter().step_by(2));
            }
        }
        (left, right)
    }

    /// Both sides on, both volumes up: the one preamble every test here wants.
    fn audible() -> Vec<Write> {
        vec![
            Write::Power(true),
            Write::MasterVolume { left: 7, right: 7, vin_left: false, vin_right: false },
            Write::Panning(0xFF),
        ]
    }

    fn note(channel: Channel, period: u16) -> Vec<Write> {
        vec![
            Write::Length { channel, duty: 2, length: 0 },
            Write::Envelope { channel, volume: 15, increase: false, pace: 0 },
            Write::PeriodLow { channel, low: period as u8 },
            Write::PeriodHigh { channel, high: (period >> 8) as u8, trigger: true, length_enable: false },
        ]
    }

    /// Zero crossings over a second, halved: a periodic wave crosses twice a cycle.
    fn frequency(samples: &[f32]) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        crossings as f32 / 2.0 * SAMPLE_RATE as f32 / samples.len() as f32
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The filter passes DC untouched and is symmetric, which is what makes it linear phase.
    #[test]
    fn the_decimating_filter_is_symmetric_and_unity_at_dc() {
        let taps = filter();
        assert!((taps.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        // The window is computed rather than tabulated, so the two halves agree to the last bit
        // or two rather than exactly.
        for n in 0..TAPS / 2 {
            assert!((taps[n] - taps[TAPS - 1 - n]).abs() < 1e-6, "tap {n}");
        }
    }

    /// The curve the output stage is there to be: untouched where the notes are, a fifth of a
    /// decibel down by 500 Hz, and progressively further off towards the top of the band. The
    /// three points are where the emulator's own resampler measures, harmonic by harmonic, on a
    /// steady note — see `lockstep::synth` in `poke-agent`, which is what fixed the pole.
    #[test]
    fn the_output_stage_rolls_treble_off_towards_nyquist() {
        let gain = |frequency: f32| {
            let mut treble = Treble::new();
            let step = 2.0 * std::f32::consts::PI * frequency / SAMPLE_RATE as f32;
            // A cosine, not a sine: a sine at exactly Nyquist is sampled at every zero crossing
            // and is a silent test signal.
            let played: Vec<f32> = (0..4096).map(|n| (step * n as f32).cos()).collect();
            let out: Vec<f32> = played.iter().map(|&sample| treble.apply(sample)).collect();
            out[1024..].iter().fold(0.0f32, |peak, &sample| peak.max(sample.abs()))
        };
        assert!(gain(500.0) > 0.99, "{} at 500 Hz", gain(500.0));
        assert!((gain(6_800.0) - 0.817).abs() < 0.03, "{} at 6.8 kHz", gain(6_800.0));
        assert!((gain(13_100.0) - 0.597).abs() < 0.03, "{} at 13.1 kHz", gain(13_100.0));
    }

    /// The sub-unit clock is exact at 48 kHz: a whole number to the M-cycle, the output sample and
    /// the wave channel's half-M-cycle tick alike.
    #[test]
    fn the_clock_divides_evenly() {
        assert_eq!(SUBS_PER_SAMPLE * SAMPLE_RATE as u64, SUBS_PER_MCYCLE * 1_048_576);
        assert_eq!(SUBS_PER_SAMPLE % OVERSAMPLE, 0);
        assert_eq!(SUBS_PER_MCYCLE % 2, 0);
    }

    /// A second of frames is a second of sound, to the sample.
    #[test]
    fn a_frame_is_a_frame_s_worth_of_samples() {
        let (left, right) = play(&audible(), 60);
        let expected = (60 * SUBS_PER_FRAME / SUBS_PER_SAMPLE) as usize;
        assert!(left.len().abs_diff(expected) <= 1, "{} samples, not {expected}", left.len());
        assert_eq!(left.len(), right.len());
    }

    /// The registry's rates, counted: the envelope at 64 Hz, the sweep at 128 and the length
    /// counters at 256, off a sequencer stepping 512 times a second.
    #[test]
    fn the_sequencer_clocks_the_envelope_at_64_hz_and_the_sweep_at_128() {
        let mut synth = Synth::new();
        for _ in 0..60 {
            synth.end_frame();
        }
        let seconds = 60.0 * SUBS_PER_FRAME as f64 / (SUBS_PER_MCYCLE as f64 * 1_048_576.0);
        let per_second = |count: u32| count as f64 / seconds;
        assert!((per_second(synth.clocks.envelope) - 64.0).abs() < 1.0, "{:?}", synth.clocks);
        assert!((per_second(synth.clocks.sweep) - 128.0).abs() < 1.5, "{:?}", synth.clocks);
        assert!((per_second(synth.clocks.length) - 256.0).abs() < 2.0, "{:?}", synth.clocks);
    }

    /// A pulse channel plays the note its period register asks for: 131072 / (2048 - period).
    #[test]
    fn a_pulse_note_sounds_at_the_frequency_its_period_asks_for() {
        for period in [1546u16, 1798, 1926] {
            let mut writes = audible();
            writes.extend(note(Channel::Pulse2, period));
            let (left, _) = play(&writes, 60);
            let expected = 131_072.0 / (2048 - period) as f32;
            let measured = frequency(&left[SAMPLE_RATE as usize / 10..]);
            assert!((measured - expected).abs() < expected * 0.01, "period {period}: {measured} not {expected}");
        }
    }

    /// The envelope is what silences a note, one step every 64th of a second, so a full-volume
    /// note on pace 1 is gone in a quarter of a second and not before.
    #[test]
    fn a_decaying_note_lasts_as_long_as_its_envelope() {
        let mut writes = audible();
        writes.extend(note(Channel::Pulse2, 1798));
        writes[4] = Write::Envelope { channel: Channel::Pulse2, volume: 15, increase: false, pace: 1 };
        let (left, _) = play(&writes, 60);
        let window = SAMPLE_RATE as usize / 100;
        let at = |seconds: f32| {
            let start = (seconds * SAMPLE_RATE as f32) as usize;
            rms(&left[start..start + window])
        };
        assert!(at(0.05) > 0.02, "the note never sounded");
        assert!(at(0.2) < at(0.05), "the envelope is not decaying");
        assert!(at(0.3) < 0.001, "still sounding a quarter of a second in");
    }

    /// The wave channel plays wave RAM: a ramp comes out at the frequency the period asks for,
    /// which is twice a pulse channel's for the same register.
    #[test]
    fn the_wave_channel_plays_its_own_samples() {
        let period = 1798;
        let mut writes = audible();
        writes.push(Write::WaveDac(false));
        for index in 0..16 {
            // A ramp up and back down: one cycle of a triangle in the 32 nibbles.
            let nibble = |n: u8| if n < 16 { n } else { 31 - n };
            writes.push(Write::WaveRam { index, samples: nibble(index * 2) << 4 | nibble(index * 2 + 1) });
        }
        writes.push(Write::WaveDac(true));
        writes.push(Write::WaveLevel(1));
        writes.push(Write::PeriodLow { channel: Channel::Wave, low: period as u8 });
        writes.push(Write::PeriodHigh {
            channel: Channel::Wave,
            high: (period >> 8) as u8,
            trigger: true,
            length_enable: false,
        });
        let (left, _) = play(&writes, 60);
        let expected = 65_536.0 / (2048 - period) as f32;
        let measured = frequency(&left[SAMPLE_RATE as usize / 10..]);
        assert!((measured - expected).abs() < expected * 0.02, "{measured} not {expected}");
    }

    /// `NR32` is a shift: half and quarter volume are the same waveform, quieter.
    #[test]
    fn the_wave_level_halves_and_quarters_what_it_plays() {
        let level_rms = |level: u8| {
            let mut writes = audible();
            writes.push(Write::WaveDac(false));
            for index in 0..16 {
                writes.push(Write::WaveRam { index, samples: 0xF0 });
            }
            writes.push(Write::WaveDac(true));
            writes.push(Write::WaveLevel(level));
            writes.push(Write::PeriodLow { channel: Channel::Wave, low: 0 });
            writes.push(Write::PeriodHigh { channel: Channel::Wave, high: 4, trigger: true, length_enable: false });
            let (left, _) = play(&writes, 30);
            rms(&left[SAMPLE_RATE as usize / 10..])
        };
        let (full, half, quarter, silent) = (level_rms(1), level_rms(2), level_rms(3), level_rms(0));
        // The level shifts the 4-bit sample, not the swing the DAC makes of it, so a pattern of
        // 15s comes out at 7/15 and 3/15 of full rather than a half and a quarter.
        assert!((half / full - 7.0 / 15.0).abs() < 0.02, "half is {}", half / full);
        assert!((quarter / full - 3.0 / 15.0).abs() < 0.02, "quarter is {}", quarter / full);
        assert!(silent < full / 100.0, "level 0 still sounded");
    }

    /// Both widths of the shift register make noise, and they are not the same noise: the narrow
    /// one repeats 127 shifts in and is heard as a pitch.
    #[test]
    fn both_noise_widths_sound_and_differ() {
        let render = |short: bool| {
            let mut writes = audible();
            writes.push(Write::Envelope { channel: Channel::Noise, volume: 15, increase: false, pace: 0 });
            writes.push(Write::Noise { shift: 2, short, divisor: 4 });
            writes.push(Write::PeriodHigh { channel: Channel::Noise, high: 0, trigger: true, length_enable: false });
            play(&writes, 30).0
        };
        let (wide, narrow) = (render(false), render(true));
        assert!(rms(&wide) > 0.02 && rms(&narrow) > 0.02, "one of the widths was silent");
        let difference = wide.iter().zip(&narrow).map(|(a, b)| (a - b).powi(2)).sum::<f32>();
        assert!(difference.sqrt() > 1.0, "the two widths played the same thing");
    }

    /// With the power off the registers are deaf, and turning it back on leaves a silent machine
    /// rather than the one it was.
    #[test]
    fn the_power_switch_clears_the_registers() {
        let mut writes = audible();
        writes.extend(note(Channel::Pulse2, 1798));
        writes.push(Write::Power(false));
        writes.push(Write::Power(true));
        writes.extend(note(Channel::Pulse2, 1798));
        let (left, _) = play(&writes, 30);
        // `NR50` and `NR51` were cleared by the power cycle and the note never asked for them
        // back, so nothing reaches the mixer.
        assert!(rms(&left[SAMPLE_RATE as usize / 10..]) < 0.001, "a deaf machine made a sound");
    }

    fn trace_writes(frame: &str) -> Vec<Write> {
        frame
            .as_bytes()
            .chunks(4)
            .map(|pair| {
                let byte = |at: usize| u8::from_str_radix(std::str::from_utf8(&pair[at..at + 2]).unwrap(), 16).unwrap();
                Write::decode(0xFF00 | byte(0) as u16, byte(2)).expect("a sound register")
            })
            .collect()
    }

    fn traces() -> Vec<(TraceInput, Vec<String>)> {
        [
            include_str!("../../fixtures/audio/audio_1.jsonl"),
            include_str!("../../fixtures/audio/audio_2.jsonl"),
            include_str!("../../fixtures/audio/audio_3.jsonl"),
        ]
        .into_iter()
        .flat_map(|jsonl| crate::fixtures::cases::<TraceInput, Vec<String>>(jsonl))
        .map(|(input, frames, _)| (input, frames))
        .collect()
    }

    /// Every harvested trace, played: nothing the cartridge can write leaves the synth silent for
    /// its whole length, off the end of the scale, or on a number that is not one.
    fn play_traces(traces: &[(TraceInput, Vec<String>)]) {
        for (input, frames) in traces {
            let mut synth = Synth::new();
            let mut samples = vec![];
            let mut buffer = [0.0; 4096];
            for frame in frames {
                for write in trace_writes(frame) {
                    synth.write(write);
                }
                synth.end_frame();
                loop {
                    let read = synth.read_samples(&mut buffer);
                    if read == 0 {
                        break;
                    }
                    samples.extend_from_slice(&buffer[..read * 2]);
                }
            }
            let expected = (frames.len() as u64 * SUBS_PER_FRAME / SUBS_PER_SAMPLE) as usize * 2;
            assert!(samples.len().abs_diff(expected) <= 2, "{input:?}: {} samples, not {expected}", samples.len());
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{input:?} left the scale");
            assert!(samples.iter().any(|s| s.abs() > 0.001), "{input:?} was silent for two seconds");
        }
    }

    #[test]
    fn the_first_traces_of_every_bank_play() {
        let all = traces();
        let sample: Vec<_> = all.chunks(40).map(|chunk| chunk[0].clone()).collect();
        play_traces(&sample);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn every_harvested_trace_plays() {
        play_traces(&traces());
    }
}
