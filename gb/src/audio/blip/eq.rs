//! Low-pass equalisation for the band-limited step kernel.

use std::f64::consts::PI;

/// Width of the scratch kernel `Blip_Synth::treble_eq` works in, in `f32` entries.
pub const FIMPULSE_LEN: usize = super::BLIP_RES / 2 * (super::BLIP_WIDEST_IMPULSE - 1) + super::BLIP_RES * 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlipEq {
    /// Treble level in dB at half the sampling rate. 0.0 is flat, negative values roll treble
    /// off.
    pub treble_db: f64,
}

impl Default for BlipEq {
    fn default() -> Self {
        Self { treble_db: super::DEFAULT_TREBLE_DB }
    }
}

impl BlipEq {
    pub fn new(treble_db: f64) -> Self {
        Self { treble_db }
    }

    /// Fill `out` with the half-kernel, then apply half a Hamming window to it.
    pub fn generate(&self, out: &mut [f32]) {
        let count = out.len();
        // With no rolloff frequency there is no cutoff, and `oversample` reduces to this.
        let oversample = super::BLIP_RES as f64 * 2.25 / count as f64 + 0.85;
        let cutoff = 0.0;

        gen_sinc(out, super::BLIP_RES as f64 * oversample, self.treble_db, cutoff);

        // Half a Hamming window.
        let to_fraction = PI / (count - 1) as f64;
        for i in (0..count).rev() {
            out[i] = (out[i] as f64 * (0.54 - 0.46 * (i as f64 * to_fraction).cos())) as f32;
        }
    }
}

/// The windowed-sinc generator itself.
fn gen_sinc(out: &mut [f32], oversample: f64, treble: f64, cutoff: f64) {
    let cutoff = if cutoff >= 0.999 { 0.999 } else { cutoff };
    let treble = treble.clamp(-300.0, 5.0);

    let count = out.len() as i32;
    let maxh = 4096.0f64;
    let rolloff = 10.0f64.powf(1.0 / (maxh * 20.0) * treble / (1.0 - cutoff));
    let pow_a_n = rolloff.powf(maxh - maxh * cutoff);
    let to_angle = PI / 2.0 / maxh / oversample;

    for i in 0..count {
        let angle = (((i - count) * 2 + 1) as f64) * to_angle;
        let mut c = rolloff * ((maxh - 1.0) * angle).cos() - (maxh * angle).cos();
        let cos_nc_angle = (maxh * cutoff * angle).cos();
        let cos_nc1_angle = ((maxh * cutoff - 1.0) * angle).cos();
        let cos_angle = angle.cos();

        c = c * pow_a_n - rolloff * cos_nc1_angle + cos_nc_angle;
        let d = 1.0 + rolloff * (rolloff - cos_angle - cos_angle);
        let b = 2.0 - cos_angle - cos_angle;
        let a = 1.0 - cos_angle - cos_nc_angle + cos_nc1_angle;

        out[i as usize] = ((a * d + c * b) / (b * d)) as f32;
    }
}
