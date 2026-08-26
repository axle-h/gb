//! What the audio stream costs, and everything it was chosen over. **Measured, not claimed** — this
//! file is why `/api/audio` looks the way it does and is the thing to re-run before changing the
//! codec, the bitrate or the framing.
//!
//! Behind the `bench` feature:
//! `cargo test --release --features bench --bin gb -- audio::bench --nocapture`
//!
//! It reads real audio out of the emulator actually playing, from the same four fixtures
//! `video/bench.rs` uses and under the same seeded `RandomPolicy`, so the two streams' numbers can
//! be read against each other — which is the comparison that matters, since a viewer with sound on
//! pays for both.
//!
//! ⚠️ **It needs its own capture.** The video one collects LCD frames and never touches the APU;
//! this one has to tune the resampler and drain it per step, which is thirty lines rather than a
//! shared abstraction that would serve neither well.

use std::io::Write;
use std::sync::OnceLock;

use super::*;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::policy::RandomPolicy;
use crate::pokemon::{PokemonApi, roms};

const SECONDS: u32 = 60;
/// The emulator is stepped at the host's own cadence so the APU is drained the way `tick` drains it.
const STEP_MS: u64 = 20;

struct Capture {
    name: &'static str,
    /// Interleaved stereo, at [`SAMPLE_RATE`].
    pcm: Vec<f32>,
}

impl Capture {
    fn frames(&self) -> usize {
        self.pcm.len() / 2
    }

    fn seconds(&self) -> f64 {
        self.frames() as f64 / SAMPLE_RATE as f64
    }
}

fn capture(name: &'static str, state: &[u8], seed: u64) -> Capture {
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(state).expect("fixture should load");
    // Exactly what `EmulatorHost::tune_audio` does, and for the same reason: a state carries neither.
    gb.core_mut().mmu_mut().audio_mut().set_output_sample_rate(SAMPLE_RATE);
    gb.core_mut().mmu_mut().audio_mut().set_emulation_speed(1.0);

    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));
    let mut cache = MapMetadataCache::default();
    let step = MachineCycles::from_duration(std::time::Duration::from_millis(STEP_MS));
    let wanted = SAMPLE_RATE as usize * SECONDS as usize;

    let mut pcm = Vec::with_capacity(wanted * 2);
    let mut scratch = vec![0.0f32; SAMPLE_RATE as usize / 8 * 2];
    while pcm.len() / 2 < wanted {
        let ran = gb.run(step);
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        let _ = agent.update(&mut api, ran);
        loop {
            let frames = gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut scratch);
            if frames == 0 {
                break;
            }
            pcm.extend_from_slice(&scratch[..frames * 2]);
        }
    }
    Capture { name, pcm }
}

/// Built **once per process** and shared, so every test below measures the same audio.
fn captures() -> &'static [Capture] {
    static CAPTURES: OnceLock<Vec<Capture>> = OnceLock::new();
    CAPTURES.get_or_init(|| {
        vec![
            capture("bedroom", crate::pokemon::data::START_OF_GAME, 1),
            capture("route-1", include_bytes!("../../pokemon/data/route1-state.bin"), 2),
            capture("viridian-forest", include_bytes!("../../pokemon/data/viridian-forest.bin"), 3),
            capture("celadon", include_bytes!("../../pokemon/data/at-celadon.bin"), 4),
        ]
    })
}

fn kbits(bytes: usize, seconds: f64) -> f64 {
    bytes as f64 * 8.0 / seconds / 1000.0
}

/// Encode a whole capture at one bitrate, returning (payload bytes, packets).
fn encode(capture: &Capture, bitrate: i32) -> (usize, usize) {
    let mut encoder = AudioEncoder::new(bitrate);
    let mut out = Vec::new();
    // Pushed in host-sized bites rather than one slab, so the accumulator is exercised the way the
    // run exercises it.
    for chunk in capture.pcm.chunks(SAMPLE_RATE as usize / 50 * 2) {
        encoder.push(chunk, &mut out);
    }
    (encoder.bytes() as usize, out.len())
}

/// IMA ADPCM, 4 bits a sample — the "no dependency at all" answer, and the one §12 offered as the
/// fallback if raw PCM proved too fat. Here so the codec is compared against something rather than
/// against nothing.
fn ima_adpcm(mono: &[f32]) -> usize {
    const STEPS: [i32; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
        73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408,
        449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
        2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630,
        9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
        32767,
    ];
    const ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
    let (mut predicted, mut index) = (0i32, 0usize);
    for sample in mono {
        let target = (*sample * 32767.0) as i32;
        let step = STEPS[index];
        let mut delta = target - predicted;
        let sign = if delta < 0 { 8 } else { 0 };
        delta = delta.abs();
        let mut code = 0i32;
        let mut diff = step >> 3;
        for bit in [4, 2, 1] {
            let magnitude = step * bit as i32 / 2;
            if delta >= magnitude {
                code |= bit as i32;
                delta -= magnitude;
                diff += magnitude;
            }
        }
        predicted += if sign != 0 { -diff } else { diff };
        predicted = predicted.clamp(-32768, 32767);
        index = (index as i32 + ADJUST[code as usize]).clamp(0, 88) as usize;
    }
    // Four bits a sample, whatever the arithmetic above decided.
    mono.len().div_ceil(2)
}

fn to_mono(pcm: &[f32]) -> Vec<f32> {
    pcm.chunks_exact(2).map(|lr| ((lr[0] + lr[1]) * 0.5).clamp(-1.0, 1.0)).collect()
}

// ── The headline ─────────────────────────────────────────────────────────────────────────────────

/// **The number the README quotes**, and the four alternatives it was chosen over.
#[test]
fn bench_audio_the_shipped_stack_and_what_it_beat() {
    println!("\n=== {SECONDS}s per capture, {SAMPLE_RATE} Hz, kbit/s on the wire ===");
    let mut totals = [0f64; 7];
    let mut seconds = 0.0;

    for capture in captures() {
        let mono = to_mono(&capture.pcm);
        let span = capture.seconds();
        seconds += span;

        // Every row is what the *wire* carries: the payload plus our 4-byte length prefix per
        // message, which for Opus is 50 messages a second and for the raw rows is one per host tick.
        let messages_per_second = 1000.0 / STEP_MS as f64;
        let framing = |per_second: f64| (4.0 * per_second * span) as usize;

        let row = [
            capture.pcm.len() * 4 + framing(messages_per_second),        // f32 stereo, as the APU makes it
            capture.pcm.len() * 2 + framing(messages_per_second),        // i16 stereo
            // i16 mono at 48 kHz is the same 768 kbit/s §12's 24 kHz *stereo* plan came to.
            mono.len() * 2 + framing(messages_per_second),
            ima_adpcm(&mono) + framing(messages_per_second),             // IMA ADPCM mono
            encode(capture, 16_000).0 + framing(50.0),
            encode(capture, DEFAULT_BITRATE).0 + framing(50.0),
            encode(capture, 32_000).0 + framing(50.0),
        ];
        for (total, bytes) in totals.iter_mut().zip(row) {
            *total += bytes as f64;
        }
        println!(
            "  {:<16} f32 {:7.1} | i16 {:7.1} | i16 mono {:7.1} | adpcm {:6.1} | opus 16k {:5.1} | \
             opus 24k {:5.1} | opus 32k {:5.1}",
            capture.name,
            kbits(row[0], span),
            kbits(row[1], span),
            kbits(row[2], span),
            kbits(row[3], span),
            kbits(row[4], span),
            kbits(row[5], span),
            kbits(row[6], span),
        );
    }

    const LABELS: [&str; 7] = [
        "raw f32 stereo (what the APU makes) ",
        "raw i16 stereo                     ",
        "raw i16 mono (= §12's 24 kHz stereo)",
        "IMA ADPCM mono (§12's fallback)     ",
        "Opus mono @16k                      ",
        "OPUS MONO @24k — WHAT SHIPS         ",
        "Opus mono @32k                      ",
    ];
    println!("\n  overall:");
    for (label, total) in LABELS.iter().zip(totals) {
        println!("    {label} {:8.1} kbit/s", kbits(total as usize, seconds));
    }
    println!(
        "\n  for comparison, /api/video ships at 21 kbit/s — so sound roughly {} the bill.",
        if kbits(totals[5] as usize, seconds) > 21.0 { "doubles" } else { "adds half to" }
    );
}

/// ⚠️ **The guard on `audio_stream`'s "no deflate".** The section above it in `src/web/mod.rs`
/// spends a page establishing that deflating the *connection* is worth 5× on video, so the next
/// person to read both will be primed to apply it here.
#[test]
fn bench_audio_deflate_is_not_worth_a_byte() {
    println!("\n=== deflate on top of Opus ===");
    for capture in captures() {
        let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
        let mut packets = Vec::new();
        for chunk in capture.pcm.chunks(SAMPLE_RATE as usize / 50 * 2) {
            encoder.push(chunk, &mut packets);
        }
        let raw: usize = packets.iter().map(|packet| packet.len()).sum();

        // Across the connection, flushed per message — exactly what `VideoStream` does, which is the
        // only way it could be done here.
        let mut deflate = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        let mut across = 0usize;
        for packet in &packets {
            let _ = deflate.write_all(packet);
            let _ = deflate.flush();
            across += std::mem::take(deflate.get_mut()).len();
        }
        // And per message, for completeness.
        let per_message: usize = packets
            .iter()
            .map(|packet| {
                let mut one = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
                let _ = one.write_all(packet);
                one.finish().expect("in-memory").len()
            })
            .sum();

        println!(
            "  {:<16} raw {raw:7} B | deflated stream {across:7} B ({:+.1}%) | per message {per_message:7} B ({:+.1}%)",
            capture.name,
            (across as f64 / raw as f64 - 1.0) * 100.0,
            (per_message as f64 / raw as f64 - 1.0) * 100.0,
        );
        assert!(across >= raw, "deflate helped after all — re-read audio_stream's ⚠️ and this test");
    }
}

/// What one packet costs beyond its payload, at each frame length Opus offers.
///
/// **Measured by actually encoding at each length**, not extrapolated from the 20 ms row: a longer
/// frame codes slightly more efficiently as well as amortising the framing, and guessing at that
/// would be guessing at the half of the answer that argues for the change.
///
/// The wire carries our `u32` prefix (measured), plus HTTP chunked-transfer framing and a TCP
/// segment per write — ⚠️ **that second part is an estimate and is labelled as one**: ~8 bytes of
/// chunk header and ~52 of IPv4/TCP, more under TLS, and a network that coalesces writes pays less.
/// It is here because at 20 ms it is the larger half of the bill, which is not the sort of thing to
/// leave out because it is awkward to measure exactly.
///
/// This is the test that decides [`FRAME_MS`]. It prints rather than asserts, because the answer is
/// a trade against latency and not a fact.
#[test]
fn bench_audio_what_a_packet_costs_the_wire() {
    /// Our length prefix, measured. The rest is the estimate above.
    const PREFIX: usize = 4;
    const TRANSPORT: usize = 8 + 52;

    println!("\n=== per-packet overhead at {} kbit/s ===", DEFAULT_BITRATE / 1000);
    let capture = &captures()[1];
    let mono = to_mono(&capture.pcm);
    let span = capture.seconds();

    for frame_ms in [10u32, 20, 40, 60] {
        let frame = (SAMPLE_RATE as usize / 1000) * frame_ms as usize;
        let mut opus =
            opus_rs::OpusEncoder::new(SAMPLE_RATE as i32, CHANNELS as usize, opus_rs::Application::Audio)
                .expect("encoder");
        opus.bitrate_bps = DEFAULT_BITRATE;
        opus.complexity = 9;
        opus.use_cbr = false;
        let mut packet = vec![0u8; MAX_PACKET];

        let (mut payload, mut packets) = (0usize, 0usize);
        let mut refused = None;
        for chunk in mono.chunks_exact(frame) {
            match opus.encode(chunk, frame, &mut packet) {
                Ok(length) => {
                    payload += length;
                    packets += 1;
                }
                Err(why) => {
                    refused = Some(why);
                    break;
                }
            }
        }
        if let Some(why) = refused {
            println!("  {frame_ms:>2} ms: refused — {why}");
            continue;
        }
        println!(
            "  {frame_ms:>2} ms: {packets:5} packets, {:5.1} B payload | payload {:5.1} kbit/s | \
             + our prefix {:5.1} | + estimated transport {:5.1} kbit/s",
            payload as f64 / packets as f64,
            kbits(payload, span),
            kbits(payload + packets * PREFIX, span),
            kbits(payload + packets * (PREFIX + TRANSPORT), span),
        );
    }
    // ⚠️ **And this is the answer to "why not a longer frame".** At this bitrate the encoder picks
    // CELT-only, which RFC 6716 gives frame sizes of 2.5, 5, 10 and 20 ms — 40 and 60 ms exist only
    // in the SILK and hybrid configurations, which are the speech models and are not what should be
    // coding a chiptune. So 20 ms is not a latency preference, it is the longest frame available
    // here, and the transport overhead below it is a floor rather than something to tune away.
    println!("  ⚠️ {FRAME_MS} ms is the longest CELT-only frame Opus has; 40 and 60 are SILK's.");
}

/// What the encoder costs the **emulator thread**, which is the argument for it being there at all.
#[test]
fn bench_audio_encoder_costs_the_emulator_thread() {
    println!("\n=== encode cost, against MAX_CATCHUP's 250 ms ===");
    let capture = &captures()[1];
    let started = std::time::Instant::now();
    let (_, packets) = encode(capture, DEFAULT_BITRATE);
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "  {packets} packets in {elapsed:.1} ms — {:.3} ms per {FRAME_MS} ms frame, {:.2}% of one core",
        elapsed / packets as f64,
        elapsed / (capture.seconds() * 1000.0) * 100.0,
    );
    let per_frame = elapsed / packets as f64;
    assert!(
        per_frame < 5.0,
        "a frame costs {per_frame:.1} ms; at that price the encoder belongs off the emulator thread",
    );
}

/// ⚠️ **"An idle game costs nothing" is the exact claim the video stream had to retract**, so it is
/// measured here rather than asserted.
#[test]
fn bench_audio_what_silence_costs() {
    let silence = vec![0.0f32; SAMPLE_RATE as usize * 2 * 10];
    let mut encoder = AudioEncoder::new(DEFAULT_BITRATE);
    let mut out = Vec::new();
    encoder.push(&silence, &mut out);
    println!(
        "\n=== silence ===\n  {} packets over 10 s, {:.1} kbit/s — VBR, so a quiet game is nearly free",
        out.len(),
        kbits(encoder.bytes() as usize + out.len() * 4, 10.0),
    );
}
