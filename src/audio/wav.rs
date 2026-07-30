//! A 44-byte-header PCM WAV writer, used only by the ignored ear-check tests.
//!
//! The crate has no `[dev-dependencies]` (every test is an inline `#[cfg(test)] mod tests`), so
//! rather than pull in a crate to dump a few seconds of audio for a listen, this writes the header
//! by hand. Canonical 16-bit interleaved PCM — nothing here needs to be clever.

use std::io::Write;
use std::path::Path;

/// Write interleaved 16-bit PCM to `path`, creating parent directories as needed.
pub fn write_wav_i16(path: &Path, sample_rate: u32, channels: u16, interleaved: &[i16]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data_len = (interleaved.len() * 2) as u32;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format = PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in interleaved {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    std::fs::File::create(path)?.write_all(&out)
}

/// Convert an interleaved `f32` stream in [-1, 1] to 16-bit PCM, clamping rather than wrapping.
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect()
}
