//! `/api/audio`: the APU's output as Opus, encoded **once** for every listener.
//!
//! The screen has been on the wire since W2 and the sound has not. `Audio::read_samples_f32`
//! (`src/audio/mod.rs`) had exactly one caller — the SDL desktop loop — so a headless run
//! synthesised audio every instruction and dropped it: `BlipBuffer::end_frame` clears its own
//! backlog when it fills, which is why that was safe rather than a leak.
//!
//! ## Why a codec at all
//!
//! The design deferred as §12 of the plan was raw i16 stereo PCM at 24 kHz — **768 kbit/s**, no
//! encoder. That is 36x `/api/video`, which spent a whole bench file getting from 565 kbit/s to 21.
//! Opus at 24 kbit/s mono puts the sound back at roughly the price of the picture, and it costs one
//! dependency with no C in it.
//!
//! ## ⚠️ 48 kHz, and it is not a preference
//!
//! `opus-rs` 0.1.32 is **wrong at 24 kHz**. Measured before this module was written, on a synthetic
//! chiptune: encoded at 24 kHz mono and decoded — by the crate's own decoder *and* by the system's
//! real libopus 1.6 — the output came back at roughly the right loudness with the spectrum
//! destroyed. Input tones at +29.7 / +39.2 / +36.5 dB returned as -3.1 / +3.2 / +2.5 dB, and it did
//! not improve with bitrate. At **48 kHz** the same signal round-trips through real libopus to
//! within **0.3 dB on every tone**, with zero packets rejected, at exactly the requested bitrate.
//! 16 kHz is fine as well; 24 kHz is the one rate that fails.
//!
//! The control that proves the *measurement* rather than the crate: libopus to libopus at
//! 32 kbit/s reproduced the same input to within 0.2 dB.
//!
//! So the encoder is fed at 48 kHz. Nothing is lost by it — Opus's bitrate is independent of its
//! input rate, 48 kHz is its native rate, and it is what WebCodecs hands the page back anyway.
//! **Anyone "tidying" `SAMPLE_RATE` to match the 24 kbit/s figure breaks the sound in the worst
//! available way: it stays the right loudness and stops being the right sound.**
//! [`tests::the_packets_are_ones_a_browser_can_decode`] is what pins it.
//!
//! ## The wire format
//!
//! There is barely one. Every connection opens with [`header`] and everything after it is a bare
//! Opus packet; `src/web/mod.rs` puts a `u32`-LE length in front of each and sends a zero length as
//! a keep-alive, exactly as it does for video. Two things are deliberately *absent*:
//!
//! - ⚠️ **No container.** The W3C WebCodecs Opus registration makes `AudioDecoderConfig.description`
//!   optional, and says that without one the bitstream is raw Opus packets. So there is no Ogg
//!   muxer here and no `OpusHead` — the twelve bytes of [`header`] carry the two numbers the
//!   browser needs to `configure()` and nothing else.
//! - ⚠️ **No deflate**, which is the one place this path diverges from the video one. Opus is
//!   already entropy-coded and a second compressor only adds framing;
//!   `bench_audio_the_shipped_stack_and_what_it_beat` measures that rather than asserting it.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use opus_rs::{Application, OpusEncoder};

/// ⚠️ **Not 24 kHz.** See the module docs — the crate is measurably wrong at that rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// The Game Boy is stereo and this is not, which is a bandwidth choice rather than a technical one:
/// `NR51` panning is mostly centred in this cartridge and a second channel is not worth doubling
/// the stream for. [`AudioEncoder::push`] does the downmix.
pub const CHANNELS: u8 = 1;
/// One Opus frame. 20 ms is the codec's default and the value every browser decoder is happiest
/// with; it also makes a packet ~60 bytes at the shipped bitrate, so the `u32` length prefix is the
/// only framing overhead worth naming.
pub const FRAME_MS: u32 = 20;
/// Samples of **mono** audio in one frame.
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

/// The first message on every connection: enough for the page to `configure()` its decoder without
/// knowing anything this module might change.
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
///
/// Lives on [`crate::host::EmulatorHost`] beside `VideoEncoder`, and is driven from the emulator
/// thread for the same reason: that is the only thread a `GameBoy` exists on. ⚠️ **Encoding there is
/// measured, not assumed** — 48 kHz mono at complexity 9 costs **0.031 ms per 20 ms frame, 0.15% of
/// one core**, against a `MAX_CATCHUP` of 250 ms. There is no argument for a second thread and a
/// channel at four orders of magnitude under the budget.
pub struct AudioEncoder {
    /// ⚠️ **`None` once it has failed, and it never comes back.** See [`Self::push`].
    opus: Option<OpusEncoder>,
    bitrate: i32,
    /// Mono samples not yet a whole frame. ⚠️ **The normal case, not an edge case**: the host ticks
    /// at ~1 ms and a frame is 20 ms, so most pushes emit nothing at all.
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
        // Measured identical to complexity 5 on this content (0.031 ms/frame either way), so there
        // is nothing to buy by turning it down.
        opus.complexity = 9;
        // VBR: a Game Boy is silent for a good part of a run — menus, dialogue, a parked run — and
        // CBR would spend the full bitrate saying so.
        opus.use_cbr = false;
        // ⚠️ **Both off, deliberately, and the reason is the transport.** This is a chunked HTTP
        // response over TCP: a packet either arrives or the connection is gone. Forward error
        // correction spends a real fraction of the bitrate on redundancy that can never be used,
        // and loss concealment conceals a loss that cannot happen. They are the crate's defaults,
        // set here so that stays a decision rather than an inheritance.
        opus.use_inband_fec = false;
        opus.packet_loss_perc = 0;
        Some(opus)
    }

    /// Whether audio has given up for the rest of this process.
    pub fn silenced(&self) -> bool {
        self.opus.is_none()
    }

    /// Packets emitted since this encoder was built. Read by the host's tests and by the bench;
    /// ⚠️ **not reset by [`Self::restart`]**, which is about the audio and not about the counters.
    pub fn packets(&self) -> u64 {
        self.packets
    }

    /// Payload bytes emitted, before the wire's `u32` length prefix.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Accumulate interleaved stereo and append one packet per whole frame.
    ///
    /// ⚠️ **Guarded by `catch_unwind`, and the guard is the reason this dependency was acceptable.**
    /// `opus-rs` is weeks old and has ~183 `unsafe` sites; a panic inside it would unwind the
    /// **emulator** thread and take the run's checkpoint with it. On the first panic the encoder is
    /// dropped outright — which is also what makes the unwind safe, since no half-updated codec
    /// state is ever re-entered — and audio is silent for the rest of the process while everything
    /// else carries on. The caller reports it once; see `EmulatorHost::publish_audio`.
    pub fn push(&mut self, interleaved_stereo: &[f32], out: &mut Vec<Arc<[u8]>>) {
        if self.opus.is_none() {
            return;
        }
        // The downmix. Averaged rather than summed, or a centred note clips at exactly the moment
        // the cartridge is loudest. ⚠️ **Clamped, and not as a formality**: `accum_to_f32` can
        // overshoot on a sharp transient, and `encode` takes an unvalidated `&[f32]` straight into
        // a codec whose first month this is.
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
    ///
    /// Called on a new run — beside `VideoEncoder::restart` — and every tick that nobody is
    /// listening, so the first listener is not handed a frame stitched from two different minutes.
    /// ⚠️ **Unlike the video encoder there is no sequence number to preserve**: `/api/audio` has no
    /// keyframe and drops nothing by sequence, because every Opus packet stands on its own.
    ///
    /// ⚠️ **A silenced encoder stays silenced.** The panic above is a property of the codec and the
    /// input that reached it, not of the run, so rebuilding here would re-enter it 50 times a second.
    pub fn restart(&mut self) {
        self.pending.clear();
        if self.opus.is_some() {
            self.opus = self.build();
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "bench"))]
mod bench;
