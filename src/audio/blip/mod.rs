//! Band-limited synthesis and resampling for the Game Boy APU.
//!
//! A Rust port of Blip_Buffer 0.4.0 by Shay Green (blargg), <http://www.slack.net/~ant/>.
//! Copyright (C) 2003-2006 Shay Green. Distributed under the GNU Lesser General Public License,
//! version 2.1 or later; see `tools/blip-golden/vendor/LGPL.txt` for the full text, alongside the
//! original C++ this was translated from.
//!
//! ## What this replaces, and why
//!
//! The obvious way to get 1 048 576 Hz down to 44 100 Hz is a polyphase sinc filter, which is what
//! this emulator used to do (rubato, 256 taps × 256 phases). That treats the APU as a *sampled
//! signal* and reconstructs it — but the APU's output is not sampled, it is a piecewise-constant
//! staircase whose step times are known exactly. Blip_Buffer works from those steps directly: hand
//! it "the amplitude changed by this much at this clock" and it adds a pre-computed band-limited
//! step response straight into a buffer that is already at the output rate.
//!
//! That is cheaper (the Game Boy's mixed output only actually changes a few tens of thousands of
//! times a second, and unchanged means no work at all), lower latency (8 output samples of kernel
//! tail, versus a 1024-frame input chunk plus a 256-tap kernel), and needs no FFT — so no
//! dependencies.
//!
//! ## Layout
//!
//! - [`eq`] — the windowed-sinc kernel generator, the only floating-point in the pipeline.
//! - [`synth`] — [`BlipSynth`], which owns the impulse table and scatter-adds transitions.
//! - [`buffer`] — [`BlipBuffer`], the delta array and its integrating reader.
//! - [`BlipStereo`] — the pair of the above that the rest of the emulator actually talks to.
//!
//! Nothing here knows about SDL, or about any particular sink: [`BlipStereo::read_interleaved_f32`]
//! and [`BlipStereo::read_interleaved_i16`] fill a caller-owned slice, so an audio queue, a file
//! writer and a network stream are all the same to it.

pub mod buffer;
pub mod eq;
pub mod synth;

#[cfg(test)]
mod tests;

use crate::audio::sample::AudioSample;
use buffer::BlipBuffer;
use eq::BlipEq;
use synth::BlipSynth;

/// Bits of sub-sample phase the impulse table is indexed by. Fewer than 6 audibly adds broadband
/// noise to high-frequency square waves.
pub const BLIP_PHASE_BITS: u32 = 6;
/// Number of sub-sample phases: 64.
pub const BLIP_RES: usize = 1 << BLIP_PHASE_BITS;
/// Fractional bits in the resampling ratio and the time cursor.
pub const BLIP_BUFFER_ACCURACY: u32 = 16;
/// Widest kernel any supported quality uses, in output samples.
pub const BLIP_WIDEST_IMPULSE: usize = 16;
/// Internal accumulator headroom, in bits.
pub const BLIP_SAMPLE_BITS: u32 = 30;

/// Kernel width in output samples. 12 is the library's `blip_good_quality`.
pub const QUALITY: usize = 12;

/// Amplitude span the synth is scaled for: a waveform running from `-RANGE/2` to `+RANGE/2`.
///
/// Must be a power of two no greater than 32768 for the gain to be exact, and 16384 is the largest
/// value that still leaves `delta_factor >= 2` — below that the kernel gets attenuated instead and
/// loses tap precision. See the amplitude discussion in [`synth`].
pub const SYNTH_RANGE: i32 = 16384;

/// Scale applied to a mixed sample in [-1, 1] to reach the synth's integer amplitude domain.
pub const AMP_SCALE: f32 = 8192.0;

/// Treble level in dB at half the sample rate. The library default.
pub const DEFAULT_TREBLE_DB: f64 = -8.0;

/// Bass corner in Hz.
///
/// Reproduces the DMG capacitor high-pass the APU used to run at 1.048 MHz (coefficient
/// 0.999832011, i.e. a corner of ~28 Hz). The shift-based coefficient here resolves 28 Hz at
/// 44.1 kHz to a shift of 8, an actual corner of ~27.4 Hz.
pub const DEFAULT_BASS_HZ: u32 = 28;

/// Output rate assumed until the sink says otherwise.
pub const DEFAULT_SAMPLE_RATE: u32 = 44100;

/// How much audio the buffer holds before a caller that never drains starts losing the backlog.
pub const BUFFER_MS: u32 = 100;

/// A stereo pair of buffers and synths — what `Audio` owns and what the SDL layer reads from.
#[derive(Debug, Clone)]
pub struct BlipStereo {
    left: BlipBuffer,
    right: BlipBuffer,
    left_synth: BlipSynth<QUALITY>,
    right_synth: BlipSynth<QUALITY>,
    /// Transition log for the golden-fixture generator in `audio::reference`. Test-only, and free
    /// to live here because `BlipStereo` is excluded from the emulator's serialised state.
    #[cfg(test)]
    capture: Option<Vec<(u16, i16, i16)>>,
}

impl BlipStereo {
    pub fn new(clock_rate: u32, sample_rate: u32) -> Self {
        let eq = BlipEq::new(DEFAULT_TREBLE_DB);
        Self {
            left: BlipBuffer::new(clock_rate, sample_rate, BUFFER_MS, DEFAULT_BASS_HZ),
            right: BlipBuffer::new(clock_rate, sample_rate, BUFFER_MS, DEFAULT_BASS_HZ),
            left_synth: BlipSynth::new(eq, 1.0),
            right_synth: BlipSynth::new(eq, 1.0),
            #[cfg(test)]
            capture: None,
        }
    }

    /// Start logging the transitions handed to [`Self::update`], for fixture generation.
    #[cfg(test)]
    pub fn start_capture(&mut self) {
        self.capture = Some(Vec::new());
    }

    /// Stop logging and return the transitions, run-length merged.
    #[cfg(test)]
    pub fn take_capture(&mut self) -> Vec<(u16, i16, i16)> {
        let raw = self.capture.take().unwrap_or_default();
        let mut merged: Vec<(u16, i16, i16)> = Vec::with_capacity(raw.len());
        for (clocks, l, r) in raw {
            match merged.last_mut() {
                Some((c, pl, pr)) if *pl == l && *pr == r && c.checked_add(clocks).is_some() => *c += clocks,
                _ => merged.push((clocks, l, r)),
            }
        }
        merged
    }

    /// Retune for a new output rate, discarding anything buffered.
    ///
    /// The synths are untouched: with a treble-only equalisation the kernel does not depend on the
    /// sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.left.sample_rate() {
            return;
        }
        self.left.set_sample_rate(sample_rate, BUFFER_MS);
        self.right.set_sample_rate(sample_rate, BUFFER_MS);
        self.left_synth.reset_amplitude();
        self.right_synth.reset_amplitude();
    }

    pub fn sample_rate(&self) -> u32 {
        self.left.sample_rate()
    }

    pub fn set_bass_freq(&mut self, hz: u32) {
        self.left.set_bass_freq(hz);
        self.right.set_bass_freq(hz);
    }

    pub fn set_treble_db(&mut self, db: f64) {
        let eq = BlipEq::new(db);
        self.left_synth.set_treble_eq(eq);
        self.right_synth.set_treble_eq(eq);
    }

    /// Report the mixed output level at the start of the current frame.
    ///
    /// Emits a transition only when the level has actually moved, which for Game Boy audio is a few
    /// tens of thousands of times a second rather than a million.
    pub fn update(&mut self, sample: AudioSample) {
        let (left, right) = (quantise(sample.left), quantise(sample.right));
        #[cfg(test)]
        if let Some(log) = &mut self.capture {
            log.push((0, left as i16, right as i16));
        }
        self.left_synth.update(0, left, &mut self.left);
        self.right_synth.update(0, right, &mut self.right);
    }

    /// Advance both channels by `clocks` source clocks, making the samples that fall in that span
    /// readable.
    pub fn end_frame(&mut self, clocks: u32) {
        #[cfg(test)]
        if let Some(log) = &mut self.capture {
            if let Some(last) = log.last_mut() {
                last.0 = last.0.saturating_add(clocks as u16);
            }
        }
        self.left.end_frame(clocks);
        self.right.end_frame(clocks);
    }

    /// Stereo frames ready to be read.
    pub fn frames_avail(&self) -> usize {
        self.left.samples_avail().min(self.right.samples_avail())
    }

    /// Fill `out` with interleaved L/R `f32` frames, returning the number of *frames* written.
    pub fn read_interleaved_f32(&mut self, out: &mut [f32]) -> usize {
        let frames = self.frames_avail().min(out.len() / 2);
        self.left.read_raw(frames, |i, accum| out[i * 2] = buffer::accum_to_f32(accum));
        self.right.read_raw(frames, |i, accum| out[i * 2 + 1] = buffer::accum_to_f32(accum));
        frames
    }

    /// Fill `out` with interleaved L/R 16-bit frames, returning the number of *frames* written.
    pub fn read_interleaved_i16(&mut self, out: &mut [i16]) -> usize {
        let frames = self.frames_avail().min(out.len() / 2);
        self.left.read_raw(frames, |i, accum| out[i * 2] = buffer::accum_to_i16(accum));
        self.right.read_raw(frames, |i, accum| out[i * 2 + 1] = buffer::accum_to_i16(accum));
        frames
    }

    pub fn clear(&mut self) {
        self.left.clear();
        self.right.clear();
        self.left_synth.reset_amplitude();
        self.right_synth.reset_amplitude();
    }
}

impl Default for BlipStereo {
    fn default() -> Self {
        Self::new(crate::audio::GB_SAMPLE_RATE as u32, DEFAULT_SAMPLE_RATE)
    }
}

/// Quantise a mixed sample in [-1, 1] to the synth's integer amplitude domain.
pub fn quantise(sample: f32) -> i32 {
    (sample * AMP_SCALE).round() as i32
}
