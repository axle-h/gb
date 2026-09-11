//! `/api/audio`: the APU's output as Opus, encoded once for every listener.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use opus_rs::{Application, OpusEncoder};

/// Not 24 kHz. See the module docs — the crate is measurably wrong at that rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// The Game Boy is stereo and this is not, which is a bandwidth choice rather than a technical
/// one: `NR51` panning is mostly centred in this cartridge and a second channel is not worth
/// doubling the stream for. [`AudioEncoder::push`] does the downmix.
pub const CHANNELS: u8 = 1;
/// One Opus frame.
pub const FRAME_MS: u32 = 20;
/// Samples of mono audio in one frame.
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE as usize / 1000) * FRAME_MS as usize;
/// RFC 6716's ceiling on a single packet.
pub const MAX_PACKET: usize = 1276;
/// `b"GBA1"` | version | channels | u32-LE sample rate | u16-LE frame ms.
pub const HEADER_LEN: usize = 12;
const MAGIC: &[u8; 4] = b"GBA1";
const VERSION: u8 = 1;

/// What `GB_AUDIO_BITRATE` defaults to, and what the README quotes.
pub const DEFAULT_BITRATE: i32 = 24_000;
/// The range `GB_AUDIO_BITRATE` is accepted in. Refused rather than clamped, for the reason
/// `GB_COMPACT_ABOVE` is: a value outside this is a misunderstanding, and silently playing
/// something other than what was asked for is the worse answer.
pub const MIN_BITRATE: i32 = 6_000;
pub const MAX_BITRATE: i32 = 128_000;

/// The first message on every connection: enough for the page to `configure()` its decoder
/// without knowing anything this module might change.
pub fn header() -> [u8; HEADER_LEN] {
    let mut out = [0u8; HEADER_LEN];
    out[..4].copy_from_slice(MAGIC);
    out[4] = VERSION;
    out[5] = CHANNELS;
    out[6..10].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    out[10..12].copy_from_slice(&(FRAME_MS as u16).to_le_bytes());
    out
}

/// Interleaved stereo `f32` in, whole Opus packets out.
pub struct AudioEncoder {
    /// `None` once it has failed, and it never comes back.
    opus: Option<OpusEncoder>,
    bitrate: i32,
    /// Mono samples not yet a whole frame.
    pending: Vec<f32>,
    /// Reused across every packet, so a run allocates nothing per frame.
    packet: Vec<u8>,
    packets: u64,
    bytes: u64,
}

impl AudioEncoder {
    pub fn new(bitrate: i32) -> Self {
        let mut encoder = Self {
            opus: None,
            bitrate: bitrate.clamp(MIN_BITRATE, MAX_BITRATE),
            pending: Vec::with_capacity(FRAME_SAMPLES * 2),
            packet: vec![0u8; MAX_PACKET],
            packets: 0,
            bytes: 0,
        };
        encoder.opus = encoder.build();
        encoder
    }

    fn build(&self) -> Option<OpusEncoder> {
        // `Application::Audio` rather than `Voip`: this is music, and the voice model at a low
        // bitrate is what makes a chiptune sound like a modem.
        let mut opus = OpusEncoder::new(SAMPLE_RATE as i32, CHANNELS as usize, Application::Audio)
            .map_err(|failure| eprintln!("gb serve — the Opus encoder would not start: {failure}"))
            .ok()?;
        opus.bitrate_bps = self.bitrate;
        // Measured identical to complexity 5 on this content (0.031 ms/frame either way), so
        // there is nothing to buy by turning it down.
        opus.complexity = 9;
        // VBR: a Game Boy is silent for a good part of a run — menus, dialogue, a parked run —
        // and CBR would spend the full bitrate saying so.
        opus.use_cbr = false;
        // Both off, deliberately, and the reason is the transport.
        opus.use_inband_fec = false;
        opus.packet_loss_perc = 0;
        Some(opus)
    }

    /// Whether audio has given up for the rest of this process.
    pub fn silenced(&self) -> bool {
        self.opus.is_none()
    }

    /// Packets emitted since this encoder was built. Read by the host's tests and by the bench;
    /// not reset by [`Self::restart`], which is about the audio and not about the counters.
    pub fn packets(&self) -> u64 {
        self.packets
    }

    #[cfg(feature = "slow-tests")]
    /// Payload bytes emitted, before the wire's `u32` length prefix.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Accumulate interleaved stereo and append one packet per whole frame.
    pub fn push(&mut self, interleaved_stereo: &[f32], out: &mut Vec<Arc<[u8]>>) {
        if self.opus.is_none() {
            return;
        }
        // The downmix.
        self.pending.extend(
            interleaved_stereo.chunks_exact(2).map(|lr| ((lr[0] + lr[1]) * 0.5).clamp(-1.0, 1.0)),
        );

        let mut at = 0;
        while at + FRAME_SAMPLES <= self.pending.len() {
            let frame = &self.pending[at..at + FRAME_SAMPLES];
            let opus = self.opus.as_mut().expect("checked above and only cleared below");
            let packet = &mut self.packet;
            let encoded = std::panic::catch_unwind(AssertUnwindSafe(move || {
                opus.encode(frame, FRAME_SAMPLES, packet)
            }));
            match encoded {
                Ok(Ok(length)) => {
                    self.packets += 1;
                    self.bytes += length as u64;
                    out.push(Arc::from(&self.packet[..length]));
                }
                Ok(Err(failure)) => {
                    eprintln!("gb serve — the Opus encoder refused a frame: {failure}");
                    self.opus = None;
                    break;
                }
                Err(_) => {
                    eprintln!("gb serve — the Opus encoder panicked; audio is off for this process");
                    self.opus = None;
                    break;
                }
            }
            at += FRAME_SAMPLES;
        }
        self.pending.drain(..at.min(self.pending.len()));
        if self.opus.is_none() {
            self.pending.clear();
        }
    }

    /// A fresh codec and an empty accumulator.
    pub fn restart(&mut self) {
        self.pending.clear();
        if self.opus.is_some() {
            self.opus = self.build();
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "slow-tests"))]
mod bench;
