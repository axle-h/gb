//! Where the video bitrate actually goes — measured, not guessed. This file is why the codec and
//! the transport look the way they do, and it is the thing to re-run before changing either.
//!
//! Behind the `bench` feature:
//! `cargo test --release --features bench --bin gb -- video::bench --nocapture`
//!
//! Everything here reads real frames out of the emulator actually playing, from four fixtures
//! chosen for different screen behaviour: a bedroom (a room to walk around), a route (the whole
//! background scrolls), a forest (grass, encounters, battle) and a city (busy sprites). The policy
//! is **seeded**, so two runs compare like with like — an unseeded `RandomPolicy` moved the headline
//! number by 2× between two tests in the same process.
//!
//! ## What it found (2026-08-11, four captures of 60 s each)
//!
//! The stream had been described as "about 19 kbit/s" since W2. That was an **idle screen**. Under
//! ordinary play, as deployed, it was **565 kbit/s** — 30× the claim, and enough to matter to
//! someone watching on a phone.
//!
//! | | kbit/s |
//! |---|---|
//! | v1 block diff, base64, SSE — **as deployed** | 565 |
//! | v1 block diff, binary, deflated per message | 108 |
//! | v1 block diff, binary, deflated across the connection | 68 |
//! | v2 block diff, base64, deflated across the connection | 47 |
//! | **v2 block diff, binary, deflated across the connection — what ships** | **21** |
//! | *for comparison:* x264 `-crf 0`, lossless | 45 |
//! | *for comparison:* x264 `-crf 28`, visibly lossy on a 4-shade screen | 25 |
//!
//! Three conclusions, in the order they are worth the most:
//!
//! 1. **Compress the connection, not the message.** A Game Boy screen is built of repeated 8×8
//!    tiles, so identical payload bytes recur within a frame and across frames; a deflate window
//!    that spans the whole stream sees all of it and a per-message one sees almost none. Worth 5×.
//! 2. **Do not base64 anything you are going to compress.** The 33% it costs before compression is
//!    the number everyone knows; after compression it costs **69–113%**, because it shifts a
//!    repeating byte pattern into three alphabet phases and LZ77 stops recognising it as a repeat.
//! 3. **A real video codec is not the answer.** x264 at *lossless* is twice the size of this, and
//!    even at a quality that visibly mangles 4-shade pixel art it does not catch up — a
//!    macroblock DCT has nothing to offer a screen whose pixels take four values. That is before
//!    the ffmpeg dependency, the WebCodecs decoder and the GOP latency.

use std::sync::OnceLock;

use super::*;
use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use poke_agent::pokemon::agent::PokemonAgent;
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::policy::RandomPolicy;
use poke_agent::pokemon::{PokemonApi, roms};

const FPS: u32 = 30;
const SECONDS: u32 = 60;

struct Capture {
    name: &'static str,
    frames: Vec<Box<Frame>>,
}

fn capture(name: &'static str, state: &[u8], seed: u64) -> Capture {
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(state).expect("fixture should load");
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));
    let mut cache = MapMetadataCache::default();

    let step = MachineCycles::from_duration(std::time::Duration::from_nanos(1_000_000_000 / FPS as u64));
    let count = (FPS * SECONDS) as usize;
    let mut frames = Vec::with_capacity(count);
    while frames.len() < count {
        let ran = gb.run(step);
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        let _ = agent.update(&mut api, ran);
        frames.push(Box::new(*gb.core().mmu().ppu().lcd()));
    }
    Capture { name, frames }
}

/// Built **once per process** and shared, so every test below measures the same frames and the
/// numbers can be read against each other.
fn captures() -> &'static [Capture] {
    static CAPTURES: OnceLock<Vec<Capture>> = OnceLock::new();
    CAPTURES.get_or_init(|| {
        vec![
            capture("bedroom", poke_agent::pokemon::data::START_OF_GAME, 1),
            capture("route-1", include_bytes!("../../../../poke-agent/src/pokemon/data/route1-state.bin"), 2),
            capture("viridian-forest", include_bytes!("../../../../poke-agent/src/pokemon/data/viridian-forest.bin"), 3),
            capture("celadon", include_bytes!("../../../../poke-agent/src/pokemon/data/at-celadon.bin"), 4),
        ]
    })
}

/// Bytes over the emulated seconds those frames span.
fn kbits(bytes: usize, frames: usize) -> f64 {
    bytes as f64 * 8.0 / (frames as f64 / FPS as f64) / 1000.0
}

// ── The stack that ships ─────────────────────────────────────────────────────────────────────────

/// The headline number, and the four alternatives it was chosen over. **Re-run this before changing
/// the codec or the transport.**
#[test]
fn bench_video_the_shipped_stack_and_what_it_beat() {
    println!("\n=== {SECONDS}s per capture at {FPS} fps, kbit/s ===");
    let (mut totals, mut all_frames) = ([0usize; 5], 0usize);
    const LABELS: [&str; 5] = [
        "codec payload (uncompressed)",
        "  + base64 on SSE           ",
        "  + deflate per message     ",
        "  + base64, deflated stream ",
        "  BINARY, DEFLATED STREAM   ",
    ];

    for capture in captures() {
        all_frames += capture.frames.len();
        let mut encoder = VideoEncoder::default();
        let mut binary = DeflateStream::default();
        let mut base64ed = DeflateStream::default();
        let mut row = [0usize; 5];
        let mut blocks = 0usize;
        let mut silent = 0usize;

        for frame in &capture.frames {
            let Some(encoded) = encoder.encode(frame) else {
                silent += 1;
                continue;
            };
            let line = format!(
                "data: {}\n\n",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &encoded.bytes)
            );
            row[0] += encoded.bytes.len();
            row[1] += line.len();
            row[2] += deflate(&encoded.bytes).len();
            row[3] += base64ed.push(line.as_bytes());
            row[4] += binary.push(&encoded.bytes);
            blocks += changed_blocks(&encoded.bytes);
        }

        println!(
            "\n-- {} ({:.0}% of frames changed nothing, {:.0} of 360 blocks moved when they did) --",
            capture.name,
            silent as f64 * 100.0 / capture.frames.len() as f64,
            blocks as f64 / (capture.frames.len() - silent).max(1) as f64,
        );
        for (i, label) in LABELS.iter().enumerate() {
            println!("  {label} {:>8.1}", kbits(row[i], capture.frames.len()));
            totals[i] += row[i];
        }
    }

    println!("\n== all four captures ==");
    for (i, label) in LABELS.iter().enumerate() {
        println!(
            "  {label} {:>8.1}   ({:.1}× the shipped stack)",
            kbits(totals[i], all_frames),
            totals[i] as f64 / totals[4] as f64
        );
    }
}

/// How many blocks a delta carried, read from its header alone.
fn changed_blocks(message: &[u8]) -> usize {
    if message[1] & FLAG_KEYFRAME != 0 {
        return BLOCK_COUNT;
    }
    let at = 6 + message[5] as usize * 3;
    if message[1] & FLAG_BITMAP != 0 {
        message[at..at + BITMAP_BYTES].iter().map(|b| b.count_ones() as usize).sum()
    } else {
        u16::from_le_bytes([message[at], message[at + 1]]) as usize
    }
}

// ── Compressors ──────────────────────────────────────────────────────────────────────────────────

fn deflate(bytes: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder.write_all(bytes).expect("in-memory");
    encoder.finish().expect("in-memory")
}

/// The same thing `src/web/mod.rs`'s `VideoStream` does on a live connection: one deflate stream,
/// flushed after every message, so the window is shared but the latency is not.
#[derive(Default)]
struct DeflateStream {
    encoder: Option<flate2::write::ZlibEncoder<Vec<u8>>>,
}

impl DeflateStream {
    fn push(&mut self, bytes: &[u8]) -> usize {
        use std::io::Write;
        let encoder = self.encoder.get_or_insert_with(|| {
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6))
        });
        let before = encoder.get_ref().len();
        encoder.write_all(bytes).expect("in-memory");
        encoder.flush().expect("in-memory");
        encoder.get_ref().len() - before
    }
}
