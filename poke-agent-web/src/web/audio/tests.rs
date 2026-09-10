//! What the encoder has to keep being true.
//!
//! The important one is [`the_packets_are_ones_a_browser_can_decode`], and its shape is the whole
//! lesson of this module — see the comment on it before writing anything similar.

use super::*;

/// 48 kHz, and every tone the tests below listen for is a harmonic-free choice well inside the band.
const TONES: [f32; 4] = [110.0, 440.0, 659.0, 1320.0];

/// A stand-in for what the cartridge actually produces: square waves and a little noise.
fn chiptune(samples: usize) -> Vec<f32> {
    (0..samples)
        .map(|n| {
            let t = n as f32 / SAMPLE_RATE as f32;
            let square = |f: f32| if (t * f).fract() < 0.5 { 1.0f32 } else { -1.0 };
            let noise =
                ((n as u32).wrapping_mul(1664525).wrapping_add(1013904223) >> 16) as f32 / 32768.0 - 1.0;
            0.30 * square(440.0) + 0.22 * square(659.0) + 0.10 * square(110.0) + 0.05 * noise
        })
        .collect()
}

/// The same thing as interleaved stereo, with the two channels equal so a downmix is a no-op.
fn centred(mono: &[f32]) -> Vec<f32> {
    mono.iter().flat_map(|s| [*s, *s]).collect()
}

/// Energy at one frequency, in dB, by Goertzel.
///
/// ⚠️ **Phase-insensitive, and that is the entire point.** See the ⚠️ on the round-trip test.
fn tone_db(samples: &[f32], frequency: f32) -> f64 {
    let w = 2.0 * std::f64::consts::PI * frequency as f64 / SAMPLE_RATE as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, sample) in samples.iter().enumerate() {
        re += *sample as f64 * (w * n as f64).cos();
        im += *sample as f64 * (w * n as f64).sin();
    }
    10.0 * ((re * re + im * im) / samples.len() as f64).max(1e-30).log10()
}

fn packets_from(encoder: &mut AudioEncoder, stereo: &[f32]) -> Vec<Arc<[u8]>> {
    let mut out = Vec::new();
    encoder.push(stereo, &mut out);
    out
}

// ── The one that matters ─────────────────────────────────────────────────────────────────────────

/// The packets this module puts on the wire are ones a browser will decode into the sound the
/// cartridge made — pinned by decoding them back and listening for the tones that went in.
///
/// ⚠️ **Measured as spectral energy, never as a sample-wise SNR.** Opus is a transform codec: it has
/// ~6.5 ms of lookahead, and CELT does not preserve phase at all. A waveform comparison therefore
/// reports total failure on a perfectly healthy codec — the first version of this check did exactly
/// that, reporting -3 dB SNR on output that was in fact correct — which is the worst possible answer
/// to get from the test that guards a young dependency, because it is indistinguishable from the
/// real bug below.
///
/// ⚠️ **This is the test that catches `opus-rs`'s 24 kHz bug**, so it pins [`SAMPLE_RATE`] along
/// with everything else. At 24 kHz these same assertions fail by ~36 dB while the *loudness* stays
/// right, which is precisely the failure a listener would struggle to describe and a waveform test
/// would miss. Run it against any change to the rate, the crate or its version.
#[test]
fn the_packets_are_ones_a_browser_can_decode() {
    let mono = chiptune(SAMPLE_RATE as usize * 2);
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let packets = packets_from(&mut encoder, &centred(&mono));
    assert!(!packets.is_empty(), "two seconds of audio produced no packets at all");

    let mut decoder =
        opus_rs::OpusDecoder::new(SAMPLE_RATE as i32, CHANNELS as usize).expect("decoder");
    let mut frame = vec![0.0f32; FRAME_SAMPLES];
    let mut decoded = Vec::with_capacity(mono.len());
    for packet in &packets {
        let samples = decoder.decode(packet, FRAME_SAMPLES, &mut frame).expect("decode");
        assert_eq!(samples, FRAME_SAMPLES, "a packet decoded to the wrong length");
        decoded.extend_from_slice(&frame);
    }

    // The first half-second holds the encoder's lookahead and the decoder's warm-up.
    let skip = SAMPLE_RATE as usize / 2;
    let (before, after) = (&mono[skip..decoded.len()], &decoded[skip..]);

    for tone in TONES {
        let (went_in, came_out) = (tone_db(before, tone), tone_db(after, tone));
        assert!(
            (went_in - came_out).abs() < 3.0,
            "{tone} Hz went in at {went_in:.1} dB and came out at {came_out:.1} dB. \
             A gap this size is not lossy coding, it is the wrong bitstream — check SAMPLE_RATE \
             against the ⚠️ in the module docs before believing anything else.",
        );
    }
}

// ── Framing ──────────────────────────────────────────────────────────────────────────────────────

/// A frame is 20 ms and a host tick is ~1 ms, so the accumulator spanning pushes is the ordinary
/// case. Pushed at ragged sizes, nothing may be lost at the seams and nothing may be emitted early.
#[test]
fn the_encoder_frames_across_ragged_pushes() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let mono = chiptune(FRAME_SAMPLES * 10);
    let stereo = centred(&mono);

    let mut out = Vec::new();
    let mut at = 0;
    // Deliberately co-prime-ish with FRAME_SAMPLES so no push lands on a frame boundary.
    for step in [7usize, 1000, 3, 4001, 11, 2048, 9, 5000, 1, 6000, 13, 7000].iter().cycle() {
        if at >= stereo.len() {
            break;
        }
        let end = (at + step * 2).min(stereo.len());
        encoder.push(&stereo[at..end], &mut out);
        at = end;
    }
    assert_eq!(out.len(), 10, "ten frames of samples should be exactly ten packets");
    assert!(out.iter().all(|packet| !packet.is_empty() && packet.len() <= MAX_PACKET));
}

/// A partial frame is held, not emitted and not dropped.
#[test]
fn a_part_frame_waits_for_the_rest_of_it() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let mono = chiptune(FRAME_SAMPLES);
    let stereo = centred(&mono);
    let split = (FRAME_SAMPLES - 1) * 2;

    assert!(packets_from(&mut encoder, &stereo[..split]).is_empty(), "emitted a part frame");
    assert_eq!(packets_from(&mut encoder, &stereo[split..]).len(), 1, "lost the held samples");
}

/// ⚠️ **Silence has to reach the wire.** `publish_video` may return without sending when nothing
/// moved on screen; audio may not do the equivalent, because the client's jitter buffer starves on
/// a stream that merely stops.
#[test]
fn a_silent_run_still_produces_packets() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let silence = vec![0.0f32; FRAME_SAMPLES * 50 * 2];
    let packets = packets_from(&mut encoder, &silence);
    assert_eq!(packets.len(), 50, "one second of silence should still be fifty packets");
    assert!(packets.iter().all(|packet| !packet.is_empty()), "an empty packet is a keep-alive");
}

/// The two channels are averaged rather than summed, or a centred note clips exactly when the
/// cartridge is loudest.
#[test]
fn stereo_is_downmixed_by_averaging_rather_than_summing() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let loud: Vec<f32> = (0..FRAME_SAMPLES * 40).flat_map(|_| [0.9f32, 0.9]).collect();
    let packets = packets_from(&mut encoder, &loud);

    let mut decoder =
        opus_rs::OpusDecoder::new(SAMPLE_RATE as i32, CHANNELS as usize).expect("decoder");
    let mut frame = vec![0.0f32; FRAME_SAMPLES];
    let mut peak = 0.0f32;
    for packet in &packets {
        decoder.decode(packet, FRAME_SAMPLES, &mut frame).expect("decode");
        peak = peak.max(frame.iter().fold(0.0f32, |a, s| a.max(s.abs())));
    }
    assert!(peak < 1.2, "a hard-panned-centre DC level came back at {peak:.2}; that is a sum");
}

// ── Lifecycle ────────────────────────────────────────────────────────────────────────────────────

/// A new run gets a fresh codec, and the partial frame the old run left is not glued to the front
/// of it.
#[test]
fn restart_drops_the_frame_the_previous_run_was_halfway_through() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let mono = chiptune(FRAME_SAMPLES);
    let stereo = centred(&mono);
    let split = (FRAME_SAMPLES / 2) * 2;

    assert!(packets_from(&mut encoder, &stereo[..split]).is_empty());
    encoder.restart();
    // Half a frame was thrown away, so half a frame is no longer enough to complete one.
    assert!(packets_from(&mut encoder, &stereo[split..]).is_empty(), "restart kept the old samples");
    assert!(!encoder.silenced());
}

/// The header is what the page configures its decoder from, so its shape is a contract with
/// `web/src/audio.ts` and not an internal detail.
#[test]
fn the_header_says_what_the_page_needs_to_configure_a_decoder() {
    let header = header();
    assert_eq!(&header[..4], MAGIC);
    assert_eq!(header[4], VERSION);
    assert_eq!(header[5], CHANNELS);
    assert_eq!(u32::from_le_bytes(header[6..10].try_into().unwrap()), SAMPLE_RATE);
    assert_eq!(u16::from_le_bytes(header[10..12].try_into().unwrap()), FRAME_MS as u16);
}

/// ⚠️ A rate the crate is wrong at must not be reachable by editing one constant and running the
/// suite: the round-trip test above is the alarm, and this is the label on it.
#[test]
fn the_sample_rate_is_one_the_encoder_is_known_good_at() {
    assert_eq!(SAMPLE_RATE, 48_000, "see the ⚠️ in the module docs — 24 kHz is measurably broken");
    assert_eq!(FRAME_SAMPLES, 960);
}

// ── The bitstream, structurally ──────────────────────────────────────────────────────────────────

/// What a packet says about itself, read by hand out of its first byte.
///
/// ⚠️ **This is the only check here that is evidence about the *bitstream* rather than about the
/// library.** Everything above round-trips `opus-rs` through `opus-rs`, which a self-consistently
/// wrong codec passes — and a self-consistently wrong codec is exactly what the 24 kHz bug is. RFC
/// 6716 §3.1 lays the TOC byte out as `config:5 | s:1 | c:2`, so config 12–15 is CELT-only at 20 ms,
/// `s` is the stereo flag and `c` is the frame-count code. Fifteen lines, no dependency, and it
/// fails if the encoder ever quietly starts emitting stereo, a different frame length, or several
/// frames per packet — each of which would leave the round-trip test perfectly green and the page's
/// `AudioDecoder` reading 20 ms of audio out of every 40 ms it was handed.
#[test]
fn a_packet_says_mono_twenty_milliseconds_on_its_face() {
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let packets = packets_from(&mut encoder, &centred(&chiptune(SAMPLE_RATE as usize)));
    assert!(!packets.is_empty());

    for packet in &packets {
        let toc = packet[0];
        let (config, stereo, frames) = (toc >> 3, (toc >> 2) & 1, toc & 3);
        assert_eq!(stereo, 0, "the stereo bit is set on a stream that says it is mono");
        assert_eq!(frames, 0, "code {frames}: more than one frame in a packet the page reads as one");
        // 20 ms is the fourth frame size in each of the SILK, hybrid and CELT config blocks, so it
        // is the configs congruent to 3 (mod 4) below 16, and 15 in the CELT block above it.
        let twenty_ms = matches!(config, 3 | 7 | 11 | 15 | 19 | 23 | 27 | 31);
        assert!(twenty_ms, "config {config} is not a 20 ms configuration");
    }
}
