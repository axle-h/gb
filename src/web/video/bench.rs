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

use std::collections::HashSet;
use std::sync::OnceLock;

use super::*;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::policy::RandomPolicy;
use crate::pokemon::{PokemonApi, roms};

const FPS: u32 = 30;
const SECONDS: u32 = 60;
/// Pokémon Red on a DMG renders four shades and nothing else — `bench_video_palette_width` asserts
/// it rather than assuming it, because the whole format rests on it.
const SHADES: usize = 4;

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
            capture("bedroom", crate::pokemon::data::START_OF_GAME, 1),
            capture("route-1", include_bytes!("../../pokemon/data/route1-state.bin"), 2),
            capture("viridian-forest", include_bytes!("../../pokemon/data/viridian-forest.bin"), 3),
            capture("celadon", include_bytes!("../../pokemon/data/at-celadon.bin"), 4),
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

/// ⚠️ The fact the format is built on: **the screen is four shades**. `gb serve` runs
/// `GameBoy::dmg`, so the stream palette never grows past the DMG ramp — which is what makes 2 bits
/// per pixel not a compromise but the exact width of the data, and what made v1's per-block
/// sub-palette (4 bytes of every 23) pure overhead.
#[test]
fn bench_video_palette_width() {
    println!("\n=== how wide does a palette index need to be? ===");
    for capture in captures() {
        let mut stream: HashSet<LcdColor> = HashSet::new();
        let mut worst_block = 0;
        for frame in &capture.frames {
            stream.extend(frame.iter().copied());
            for block in 0..BLOCK_COUNT {
                let distinct: HashSet<LcdColor> = block_pixels(block).map(|p| frame[p]).collect();
                worst_block = worst_block.max(distinct.len());
            }
        }
        println!(
            "  {:<16} {} colours in {SECONDS}s of play, worst 8x8 block {}",
            capture.name,
            stream.len(),
            worst_block
        );
        assert!(stream.len() <= SHADES, "a DMG stream is four shades; this one had {}", stream.len());
    }
}

/// **The simplest thing that could possibly work**, kept because "obviously too expensive" is
/// exactly the kind of claim this file exists to check: no diff at all, the whole 160×144 screen as
/// 5760 bytes of 2bpp every frame, and let the compressor find the redundancy.
///
/// It loses — but only by 2×, not by the 60× its uncompressed size suggests. Worth knowing: most of
/// what the block diff earns, a deflate window would have earned anyway.
#[test]
fn bench_video_the_diff_is_worth_less_than_it_looks() {
    println!("\n=== the block diff against sending the whole screen every frame ===");
    let (mut diffed, mut whole, mut frames) = (0usize, 0usize, 0usize);
    for capture in captures() {
        let mut encoder = VideoEncoder::default();
        let mut diff_stream = DeflateStream::default();
        let mut whole_stream = DeflateStream::default();
        let mut palette: Vec<LcdColor> = Vec::with_capacity(SHADES);
        let (mut a, mut b) = (0usize, 0usize);

        for frame in &capture.frames {
            frames += 1;
            if let Some(encoded) = encoder.encode(frame) {
                a += diff_stream.push(&encoded.bytes);
            }
            let mut message = Vec::with_capacity(PIXELS / 4);
            for chunk in frame.chunks(4) {
                message.push(chunk.iter().enumerate().fold(0u8, |acc, (i, colour)| {
                    let index = palette.iter().position(|c| c == colour).unwrap_or_else(|| {
                        palette.push(*colour);
                        palette.len() - 1
                    });
                    acc | ((index as u8) << (i * 2))
                }));
            }
            b += whole_stream.push(&message);
        }
        println!(
            "  {:<16} block diff {:>6.1} kbit/s | whole screen every frame {:>6.1} kbit/s",
            capture.name,
            kbits(a, capture.frames.len()),
            kbits(b, capture.frames.len()),
        );
        diffed += a;
        whole += b;
    }
    println!(
        "  {:<16} block diff {:>6.1} kbit/s | whole screen every frame {:>6.1} kbit/s",
        "all four",
        kbits(diffed, frames),
        kbits(whole, frames)
    );
}

/// Two structural wins neither the block diff nor a byte compressor can reach on its own, measured
/// as *opportunity* rather than built: a block that repeats one already on screen (a "copy block N"
/// mode), and a background that has simply scrolled (a global motion vector).
///
/// Both are real and neither has been built, because deflate across the connection already collects
/// most of the first and the second is worth its complexity only if the bitrate ever matters again.
#[test]
fn bench_video_redundancy_still_on_the_table() {
    println!("\n=== redundancy the block diff cannot express ===");
    for capture in captures() {
        let mut changed_blocks = 0usize;
        let (mut repeat_in_frame, mut repeat_on_screen) = (0usize, 0usize);
        let (mut scroll_frames, mut scroll_covered, mut motion_frames) = (0usize, 0usize, 0usize);

        for pair in capture.frames.windows(2) {
            let (last, frame) = (&pair[0], &pair[1]);
            let changed: Vec<usize> = (0..BLOCK_COUNT)
                .filter(|&b| block_pixels(b).any(|p| frame[p] != last[p]))
                .collect();
            if changed.is_empty() {
                continue;
            }
            changed_blocks += changed.len();

            let key_of = |source: &Frame, b: usize| {
                let mut key = [LcdColor::default(); 64];
                for (slot, p) in block_pixels(b).enumerate() {
                    key[slot] = source[p];
                }
                key
            };
            let on_screen: HashSet<[LcdColor; 64]> =
                (0..BLOCK_COUNT).map(|b| key_of(last, b)).collect();
            let mut seen: HashSet<[LcdColor; 64]> = HashSet::new();
            for &b in &changed {
                let key = key_of(frame, b);
                if on_screen.contains(&key) {
                    repeat_on_screen += 1;
                } else if seen.contains(&key) {
                    repeat_in_frame += 1;
                }
                seen.insert(key);
            }

            // Global motion: is this frame the last one shifted? The window is what a Game Boy can
            // scroll in 33 ms.
            motion_frames += 1;
            let still = (0..PIXELS).filter(|&p| frame[p] == last[p]).count();
            let mut best = 0usize;
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let mut matched = 0usize;
                    for y in 0..LCD_HEIGHT as i32 {
                        let sy = y + dy;
                        if !(0..LCD_HEIGHT as i32).contains(&sy) {
                            continue;
                        }
                        for x in 0..LCD_WIDTH as i32 {
                            let sx = x + dx;
                            if !(0..LCD_WIDTH as i32).contains(&sx) {
                                continue;
                            }
                            matched += (frame[y as usize * LCD_WIDTH + x as usize]
                                == last[sy as usize * LCD_WIDTH + sx as usize])
                                as usize;
                        }
                    }
                    best = best.max(matched);
                }
            }
            if best > still {
                scroll_frames += 1;
                scroll_covered += best - still;
            }
        }

        println!(
            "  {:<16} {} changed blocks: {:.0}% repeat one already on screen, a further {:.0}% repeat one in the same message",
            capture.name,
            changed_blocks,
            repeat_on_screen as f64 * 100.0 / changed_blocks.max(1) as f64,
            repeat_in_frame as f64 * 100.0 / changed_blocks.max(1) as f64,
        );
        println!(
            "  {:<16} a global scroll vector beats a straight diff on {:.0}% of moving frames, recovering {:.0} of {PIXELS} pixels each",
            "",
            scroll_frames as f64 * 100.0 / motion_frames.max(1) as f64,
            scroll_covered as f64 / scroll_frames.max(1) as f64,
        );
    }
}

/// Dump the captures as raw 8-bit greyscale so the ffmpeg comparison can be redone. Not a codec
/// test: it exists so "would a real video codec do better?" is answered with a number.
#[test]
fn bench_video_dump_for_ffmpeg() {
    let dir = std::path::Path::new("target/test-artifacts/video-bench");
    std::fs::create_dir_all(dir).expect("artifact dir");
    println!("\n=== raw greyscale dumps for ffmpeg ===");
    for capture in captures() {
        let mut bytes = Vec::with_capacity(capture.frames.len() * PIXELS);
        for frame in &capture.frames {
            bytes.extend(frame.iter().map(|c| c.to_rgb().0[0]));
        }
        let path = dir.join(format!("{}.gray", capture.name));
        std::fs::write(&path, &bytes).expect("write dump");
        println!("  {} ({} frames, {} B)", path.display(), capture.frames.len(), bytes.len());
    }
    println!(
        "\n  ffmpeg -f rawvideo -pix_fmt gray -s 160x144 -r {FPS} -i <file>.gray \\\n    \
         -c:v libx264 -preset veryslow -tune animation -g 300 -crf 0 -f h264 - | wc -c\n  \
         measured 2026-08-11: 45 kbit/s at -crf 0, 25 at -crf 28 — both worse than 21."
    );
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
