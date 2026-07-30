//! The band-limited step synthesiser — `Blip_Synth` from Blip_Buffer 0.4.0.
//!
//! Holds a windowed-sinc **step response** sampled at [`BLIP_RES`] sub-sample phases. Adding an
//! amplitude transition means scatter-adding `QUALITY` scaled taps into the buffer's delta array at
//! the phase the transition falls on; the buffer's reader integrates those deltas back into
//! samples. Because the taps for every phase sum to exactly `kernel_unit`, a step of amplitude `a`
//! contributes exactly `a` of DC — no drift however many transitions go by.
//!
//! ## Amplitude domain
//!
//! `delta_factor = volume / RANGE * 2^30 / kernel_unit` is stored as an integer, so for exact gain
//! the ratio has to land on a whole number, and it must be at least 2 or [`Self::set_volume`]
//! attenuates the kernel itself to compensate — costing tap precision. With `volume = 1.0` and the
//! un-shifted `kernel_unit` of 32768 that means `RANGE` must be a power of two no greater than
//! 32768, and the largest such value with `delta_factor >= 2` is 16384. See [`super::SYNTH_RANGE`].

use super::buffer::BlipBuffer;
use super::eq::{BlipEq, FIMPULSE_LEN};
use super::{BLIP_BUFFER_ACCURACY, BLIP_PHASE_BITS, BLIP_RES, BLIP_SAMPLE_BITS, BLIP_WIDEST_IMPULSE};

/// Enough taps for the widest supported quality, so the table can be a plain array rather than a
/// heap allocation whose length depends on a const generic.
const MAX_IMPULSES: usize = BLIP_RES * (BLIP_WIDEST_IMPULSE / 2) + 1;

/// Base magnitude the kernel is normalised to before any volume-driven attenuation.
const BASE_UNIT: f64 = 32768.0;

#[derive(Debug, Clone)]
pub struct BlipSynth<const QUALITY: usize> {
    impulses: [i16; MAX_IMPULSES],
    volume_unit: f64,
    kernel_unit: i64,
    delta_factor: i32,
    last_amp: i32,
}

impl<const QUALITY: usize> BlipSynth<QUALITY> {
    /// Number of taps actually in use for this quality.
    const IMPULSES_LEN: usize = BLIP_RES / 2 * QUALITY + 1;
    /// First buffer slot the scatter-add touches, relative to the transition's sample index.
    const FWD: usize = (BLIP_WIDEST_IMPULSE - QUALITY) / 2;
    /// Last-but-one slot; the reverse half is written backwards from `REV + 1`.
    const REV: usize = Self::FWD + QUALITY - 2;
    /// Highest tap index in each half.
    const MID: usize = QUALITY / 2 - 1;

    pub fn new(eq: BlipEq, volume: f64) -> Self {
        assert!(QUALITY >= 4 && QUALITY <= BLIP_WIDEST_IMPULSE && QUALITY % 4 == 0,
            "quality must be 4, 8, 12 or 16");
        let mut synth = Self {
            impulses: [0; MAX_IMPULSES],
            volume_unit: 0.0,
            kernel_unit: 0,
            delta_factor: 0,
            last_amp: 0,
        };
        synth.set_treble_eq(eq);
        synth.set_volume(volume);
        synth
    }

    pub fn delta_factor(&self) -> i32 {
        self.delta_factor
    }

    pub fn kernel_unit(&self) -> i64 {
        self.kernel_unit
    }

    pub fn impulses(&self) -> &[i16] {
        &self.impulses[..Self::IMPULSES_LEN]
    }

    /// Reset the running amplitude without emitting a transition. Call whenever the buffer is
    /// cleared, or the next `update` will synthesise a step that the buffer has no history for.
    pub fn reset_amplitude(&mut self) {
        self.last_amp = 0;
    }

    /// Rebuild the impulse table for a new equalisation.
    ///
    /// Mirrors `Blip_Synth_::treble_eq`: generate a half kernel, mirror it about its centre,
    /// integrate and first-difference it into the phase-interleaved layout `offset` walks, then
    /// hand off to [`Self::adjust_impulse`] for the error correction.
    pub fn set_treble_eq(&mut self, eq: BlipEq) {
        // The original leaves the tail of this scratch array uninitialised; nothing reads past
        // `IMPULSES_LEN + BLIP_RES`, which the written region always covers.
        let mut fimpulse = [0.0f32; FIMPULSE_LEN];
        let half_size = BLIP_RES / 2 * (QUALITY - 1);
        eq.generate(&mut fimpulse[BLIP_RES..BLIP_RES + half_size]);

        // Mirror slightly past the centre — the calculation below needs it.
        for i in (0..BLIP_RES).rev() {
            fimpulse[BLIP_RES + half_size + i] = fimpulse[BLIP_RES + half_size - 1 - i];
        }
        for i in 0..BLIP_RES {
            fimpulse[i] = 0.0;
        }

        let mut total = 0.0f64;
        for i in 0..half_size {
            total += fimpulse[BLIP_RES + i] as f64;
        }
        let rescale = BASE_UNIT / 2.0 / total;
        self.kernel_unit = BASE_UNIT as i64;

        // Integrate, first-difference, rescale, round to i16.
        let mut sum = 0.0f64;
        let mut next = 0.0f64;
        for i in 0..Self::IMPULSES_LEN {
            self.impulses[i] = ((next - sum) * rescale + 0.5).floor() as i16;
            sum += fimpulse[i] as f64;
            next += fimpulse[i + BLIP_RES] as f64;
        }
        self.adjust_impulse();

        // Volume is expressed relative to the kernel, so it has to be reapplied.
        let volume = self.volume_unit;
        if volume != 0.0 {
            self.volume_unit = 0.0;
            self.set_volume_unit(volume);
        }
    }

    /// Distribute rounding error so that every phase's taps sum to exactly `kernel_unit`.
    ///
    /// This is the whole reason a step contributes no DC error. The loop bounds look wrong and are
    /// not: the original's `for (int p = blip_res; p-- >= blip_res / 2;)` runs its body with
    /// `p = 63 … 31`, and on the first iteration `p2 = BLIP_RES - 2 - p` is **-1**, so the paired
    /// index `i + p2` reaches tap 0. Getting either detail wrong shifts the whole table.
    fn adjust_impulse(&mut self) {
        let size = Self::IMPULSES_LEN as i32;
        for p in (BLIP_RES as i32 / 2 - 1..BLIP_RES as i32).rev() {
            let p2 = BLIP_RES as i32 - 2 - p;
            let mut error = self.kernel_unit;
            let mut i = 1i32;
            while i < size {
                error -= self.impulses[(i + p) as usize] as i64;
                error -= self.impulses[(i + p2) as usize] as i64;
                i += BLIP_RES as i32;
            }
            if p == p2 {
                error /= 2; // the half-phase impulse uses the same half for both sides
            }
            let at = (size - BLIP_RES as i32 + p) as usize;
            self.impulses[at] = (self.impulses[at] as i64 + error) as i16;
        }
    }

    /// Set overall volume, where 1.0 means an amplitude of `RANGE / 2` reaches full output scale.
    pub fn set_volume(&mut self, volume: f64) {
        self.set_volume_unit(volume * (1.0 / super::SYNTH_RANGE as f64));
    }

    /// `Blip_Synth_::volume_unit`. If the requested gain is so small that `delta_factor` would fall
    /// below 2, the kernel is attenuated instead so the multiply keeps its precision.
    fn set_volume_unit(&mut self, new_unit: f64) {
        if new_unit == self.volume_unit {
            return;
        }
        self.volume_unit = new_unit;
        let mut factor = new_unit * (1i64 << BLIP_SAMPLE_BITS) as f64 / self.kernel_unit as f64;

        if factor > 0.0 {
            let mut shift = 0u32;
            while factor < 2.0 {
                shift += 1;
                factor *= 2.0;
            }
            if shift > 0 {
                self.kernel_unit >>= shift;
                assert!(self.kernel_unit > 0, "volume unit too low");
                // Bias into positive territory first: a sign-preserving right shift rounds towards
                // negative infinity, which would skew the negative taps.
                let offset = 0x8000i64 + (1i64 << (shift - 1));
                let offset2 = 0x8000i64 >> shift;
                for i in 0..Self::IMPULSES_LEN {
                    self.impulses[i] = (((self.impulses[i] as i64 + offset) >> shift) - offset2) as i16;
                }
                self.adjust_impulse();
            }
        }
        self.delta_factor = (factor + 0.5).floor() as i32;
    }

    /// Track a waveform's absolute amplitude, emitting whatever transition it implies.
    ///
    /// The zero-delta early-out is what makes feeding this a sample per instruction cheap: the
    /// Game Boy's mixed output only actually changes a few tens of thousands of times a second.
    pub fn update(&mut self, time: u32, amp: i32, buf: &mut BlipBuffer) {
        let delta = amp - self.last_amp;
        self.last_amp = amp;
        if delta != 0 {
            self.offset(time, delta, buf);
        }
    }

    /// Add a transition of `delta` at source-clock `time` within the current frame.
    pub fn offset(&self, time: u32, delta: i32, buf: &mut BlipBuffer) {
        self.offset_resampled(time as u64 * buf.factor() + buf.offset(), delta, buf);
    }

    /// The scatter-add, in resampled (16.16 fixed-point) time.
    ///
    /// The original expresses this as two unrolled macro chains carrying an accumulator register
    /// between pairs of taps. Tracing that carry through shows it is a plain scatter-add over
    /// `QUALITY` distinct slots, with the kernel's second half recovered from its first by symmetry
    /// (`BLIP_RES - phase` forward, `phase` reverse) — so the two loops below are bit-identical to
    /// the original while being readable.
    ///
    /// One deliberate divergence: the original computes `tap * delta` in 32-bit `int`, which
    /// overflows above roughly 2^30. Doing it in `i64` is identical everywhere the original is
    /// well-defined and correct past the point where it is not.
    pub fn offset_resampled(&self, time: u64, delta: i32, buf: &mut BlipBuffer) {
        let delta = delta as i64 * self.delta_factor as i64;
        let phase = ((time >> (BLIP_BUFFER_ACCURACY - BLIP_PHASE_BITS)) & (BLIP_RES as u64 - 1)) as usize;
        let index = (time >> BLIP_BUFFER_ACCURACY) as usize;
        // Caller error rather than a runtime condition: it means a frame ran longer than the buffer
        // can hold without anyone calling end_frame. Kept as a hard assert (the original asserts
        // here too) because silently dropping the transition would leave a permanent DC error.
        assert!(
            index < buf.size(),
            "transition at sample {index} is past the end of a {}-sample buffer — end_frame more often",
            buf.size()
        );

        let fwd = &self.impulses[BLIP_RES - phase..];
        let rev = &self.impulses[phase..];
        let out = &mut buf.deltas_mut()[index..];

        for k in 0..=Self::MID {
            out[Self::FWD + k] += fwd[k * BLIP_RES] as i64 * delta;
        }
        for r in 0..=Self::MID {
            out[Self::REV + 1 - r] += rev[r * BLIP_RES] as i64 * delta;
        }
    }

    /// Install a table produced elsewhere — used by the golden tests to isolate the integer DSP
    /// path from any disagreement between this platform's libm and the one that built the fixtures.
    #[cfg(test)]
    pub fn set_raw_impulses(&mut self, taps: &[i16], kernel_unit: i64, delta_factor: i32) {
        assert_eq!(taps.len(), Self::IMPULSES_LEN);
        self.impulses[..taps.len()].copy_from_slice(taps);
        self.kernel_unit = kernel_unit;
        self.delta_factor = delta_factor;
    }
}
