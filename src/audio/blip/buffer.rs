//! The delta buffer and its integrating reader — `Blip_Buffer` from Blip_Buffer 0.4.0.
//!
//! Stores *first differences* at the output sample rate rather than samples. A band-limited step
//! extends to infinity, which a finite buffer cannot hold; storing the differences means a
//! transition only ever touches a handful of slots around its own position, and the extension falls
//! out of the running sum the reader keeps. That running sum leaks slightly on every sample, and
//! that leak is the bass high-pass — there is no separate DC blocker.
//!
//! Time is tracked as a 16.16 fixed-point cursor (`offset`), advanced by `clocks * factor` where
//! `factor = round(sample_rate / clock_rate * 2^16)`. The ratio is therefore not exact, but it is
//! *consistent*: whatever it rounds to is what you get, every second, forever.

use super::{BLIP_BUFFER_ACCURACY, BLIP_SAMPLE_BITS, BLIP_WIDEST_IMPULSE};

/// Slack past the nominal buffer length, so a transition landing on the last valid sample still has
/// somewhere to put its tail.
const BUFFER_EXTRA: usize = BLIP_WIDEST_IMPULSE + 2;

/// Bits the accumulator is shifted down by to reach 16-bit output.
const SAMPLE_SHIFT: u32 = BLIP_SAMPLE_BITS - 16;

/// Accumulator value corresponding to full output scale, i.e. what `read_i16` maps to 32768.
const FULL_SCALE_ACCUM: f32 = (1u64 << (SAMPLE_SHIFT + 15)) as f32;

#[derive(Debug, Clone)]
pub struct BlipBuffer {
    /// 16.16 fixed-point output samples per source clock.
    factor: u64,
    /// 16.16 fixed-point cursor: how far into the buffer the current frame has reached.
    offset: u64,
    /// First differences. `size + BUFFER_EXTRA` long.
    deltas: Vec<i64>,
    size: usize,
    reader_accum: i64,
    bass_shift: u32,
    sample_rate: u32,
    clock_rate: u32,
    bass_freq: u32,
}

impl BlipBuffer {
    pub fn new(clock_rate: u32, sample_rate: u32, buffer_ms: u32, bass_freq: u32) -> Self {
        let mut buf = Self {
            factor: 0,
            offset: 0,
            deltas: Vec::new(),
            size: 0,
            reader_accum: 0,
            bass_shift: 0,
            sample_rate: 0,
            clock_rate,
            bass_freq,
        };
        buf.set_sample_rate(sample_rate, buffer_ms);
        buf
    }

    /// Resize for a new output rate and clear. `buffer_ms` is how much audio the buffer can hold
    /// before a non-draining caller starts losing the backlog.
    pub fn set_sample_rate(&mut self, sample_rate: u32, buffer_ms: u32) {
        assert!(sample_rate > 0, "sample rate must be positive");
        // Same sizing arithmetic as the original, which is what makes `buffer_ms` come back out of
        // a length query unchanged.
        let size = ((sample_rate as u64 * (buffer_ms as u64 + 1) + 999) / 1000) as usize;
        self.size = size;
        self.deltas.clear();
        self.deltas.resize(size + BUFFER_EXTRA, 0);
        self.sample_rate = sample_rate;
        self.set_clock_rate(self.clock_rate);
        self.set_bass_freq(self.bass_freq);
        self.clear();
    }

    pub fn set_clock_rate(&mut self, clock_rate: u32) {
        self.clock_rate = clock_rate;
        let ratio = self.sample_rate as f64 / clock_rate as f64;
        let factor = (ratio * (1u64 << BLIP_BUFFER_ACCURACY) as f64 + 0.5).floor();
        assert!(factor > 0.0, "clock rate to sample rate ratio is too large");
        self.factor = factor as u64;
    }

    /// Set the corner below which bass response starts to fall away. 0 disables it entirely.
    ///
    /// The coefficient is a shift rather than a multiply, so the achievable corners are quantised to
    /// powers of two: 28 Hz at 44.1 kHz resolves to a shift of 8, i.e. an actual corner of ~27.4 Hz.
    pub fn set_bass_freq(&mut self, freq: u32) {
        self.bass_freq = freq;
        let mut shift = 31u32;
        if freq > 0 {
            shift = 13;
            let mut f = ((freq as u64) << 16) / self.sample_rate as u64;
            loop {
                f >>= 1;
                if f == 0 {
                    break;
                }
                shift -= 1;
                if shift == 0 {
                    break;
                }
            }
        }
        self.bass_shift = shift;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }

    pub fn factor(&self) -> u64 {
        self.factor
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn deltas_mut(&mut self) -> &mut [i64] {
        &mut self.deltas
    }

    pub fn samples_avail(&self) -> usize {
        (self.offset >> BLIP_BUFFER_ACCURACY) as usize
    }

    /// Discard everything and reset to silence.
    pub fn clear(&mut self) {
        self.offset = 0;
        self.reader_accum = 0;
        self.deltas.fill(0);
    }

    /// Drop the pending backlog without clearing the whole array — the cheap path for a buffer that
    /// nobody is draining. Everything past `samples_avail` is already zero.
    fn clear_avail(&mut self) {
        let count = (self.samples_avail() + BUFFER_EXTRA).min(self.deltas.len());
        self.deltas[..count].fill(0);
        self.offset = 0;
        self.reader_accum = 0;
    }

    /// End the current time frame, making its samples readable and starting the next frame at what
    /// was `clocks`.
    ///
    /// The fractional part of the cursor carries across calls, so a caller can end a frame every
    /// instruction with `time = 0` transitions and still land every transition on the right
    /// sub-sample phase.
    pub fn end_frame(&mut self, clocks: u32) {
        self.offset += clocks as u64 * self.factor;

        // Nothing is obliged to read us — the headless integration tests run twenty minutes of
        // emulated time with no audio consumer at all. Drop the backlog rather than let a
        // transition scatter past the end of the array.
        if self.samples_avail() >= self.size {
            self.clear_avail();
        }
    }

    /// Integrate `count` samples out of the buffer, handing each raw accumulator value to `sink`,
    /// then remove them.
    ///
    /// Callers pick their own output format and destination stride — which is how stereo
    /// interleaving happens without a scratch buffer. `count` must not exceed
    /// [`Self::samples_avail`].
    pub fn read_raw(&mut self, count: usize, mut sink: impl FnMut(usize, i64)) {
        debug_assert!(count <= self.samples_avail());
        if count == 0 {
            return;
        }
        let mut accum = self.reader_accum;
        for i in 0..count {
            sink(i, accum);
            accum -= accum >> self.bass_shift;
            accum += self.deltas[i];
        }
        self.reader_accum = accum;
        self.remove_samples(count);
    }

    /// Read out at most `out.len()` samples as 16-bit PCM, removing them from the buffer.
    pub fn read_i16(&mut self, out: &mut [i16]) -> usize {
        let count = self.samples_avail().min(out.len());
        self.read_raw(count, |i, accum| out[i] = accum_to_i16(accum));
        count
    }

    /// Read out at most `out.len()` samples as `f32` in [-1, 1].
    pub fn read_f32(&mut self, out: &mut [f32]) -> usize {
        let count = self.samples_avail().min(out.len());
        self.read_raw(count, |i, accum| out[i] = accum_to_f32(accum));
        count
    }

    /// Drop `count` already-read samples, sliding the remainder down to the front.
    fn remove_samples(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.offset -= (count as u64) << BLIP_BUFFER_ACCURACY;
        let remain = self.samples_avail() + BUFFER_EXTRA;
        self.deltas.copy_within(count..count + remain, 0);
        self.deltas[remain..remain + count].fill(0);
    }
}

/// Convert a raw reader accumulator to 16-bit PCM, saturating instead of wrapping.
///
/// The bit-exact port of the clamp in `Blip_Buffer::read_samples`: `s >> 24` carries the sign, so
/// this yields 32767 on positive overflow and -32768 on negative.
pub fn accum_to_i16(accum: i64) -> i16 {
    let s = accum >> SAMPLE_SHIFT;
    if s as i16 as i64 != s {
        (0x7FFF - (s >> 24)) as i16
    } else {
        s as i16
    }
}

/// Convert a raw reader accumulator to `f32` in [-1, 1].
///
/// Scales the full 30-bit accumulator rather than the truncated 16-bit sample, so this carries more
/// resolution than the original's reader — worth having when the sink is float anyway.
pub fn accum_to_f32(accum: i64) -> f32 {
    (accum as f32 / FULL_SCALE_ACCUM).clamp(-1.0, 1.0)
}
