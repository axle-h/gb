//! The regression net for the wire format. The TypeScript decoder in the SPA is a direct port of
//! [`VideoDecoder`] and is checked by eye; *this* is what stops the format drifting.

use super::*;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::pokemon::agent::{AGENT_RESOLUTION, PokemonAgent};
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::{PokemonApi, roms};

/// Real frames, from the emulator actually playing — a synthetic gradient would exercise none of
/// what makes this codec cheap (long runs, a handful of colours, most blocks unchanged between
/// frames) and would not catch a regression that only shows up on a sprite edge.
///
/// The agent walks Red around his bedroom under `RandomPolicy`, so the frames genuinely differ:
/// sprite animation, a menu opening, the screen scrolling.
fn recorded_frames(count: usize) -> Vec<Box<Frame>> {
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(crate::pokemon::data::START_OF_GAME).expect("fixture should load");
    let mut agent = PokemonAgent::default();
    let mut cache = MapMetadataCache::default();

    let mut frames = Vec::with_capacity(count);
    while frames.len() < count {
        // Two agent ticks per captured frame ≈ 25 fps of emulated time, close to what the host
        // publishes at.
        let mut ran = MachineCycles::ZERO;
        for _ in 0..2 {
            ran += gb.run(AGENT_RESOLUTION);
        }
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).expect("agent should not fail in Red's bedroom");
        frames.push(Box::new(*gb.core().mmu().ppu().lcd()));
    }
    frames
}

/// A frame of `distinct` different colours, laid out so consecutive blocks differ — the shape that
/// exhausts a palette, which no real Game Boy frame does.
fn synthetic_frame(distinct: usize, offset: u32) -> Box<Frame> {
    let mut frame = Box::new([LcdColor::default(); PIXELS]);
    for (p, pixel) in frame.iter_mut().enumerate() {
        let n = offset + (p / 4 % distinct) as u32;
        *pixel = LcdColor::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
    }
    frame
}

#[test]
fn roundtrip_recorded_frames() {
    let frames = recorded_frames(120);
    let mut encoder = VideoEncoder::default();
    let mut decoder = VideoDecoder::default();

    let mut messages = 0;
    for (n, frame) in frames.iter().enumerate() {
        if let Some(encoded) = encoder.encode(frame) {
            messages += 1;
            decoder.apply(&encoded.bytes).unwrap_or_else(|e| panic!("frame {n}: {e}"));
        }
        assert_eq!(decoder.pixels(), frame.as_ref(), "frame {n} did not reconstruct exactly");
    }
    assert!(messages > 2, "only {messages} messages — the capture never changed, so this proved nothing");
}

/// The whole reason for a diff format: a screen nobody is touching costs zero bytes, not a frame's
/// worth every 33 ms.
#[test]
fn an_unchanged_frame_sends_nothing() {
    let frames = recorded_frames(1);
    let mut encoder = VideoEncoder::default();
    assert!(encoder.encode(&frames[0]).is_some(), "the first frame is always a keyframe");
    assert_eq!(encoder.encode(&frames[0]), None);
    assert_eq!(encoder.encode(&frames[0]), None);
}

/// §5.2's late-joiner handshake rests on this: a keyframe taken from the encoder's current state
/// leaves a fresh decoder holding *exactly* what a decoder that had followed every delta holds —
/// same pixels **and** the same palette, so the next delta lines up for both.
#[test]
fn a_keyframe_catches_a_fresh_decoder_up_exactly() {
    let frames = recorded_frames(40);
    let mut encoder = VideoEncoder::default();
    let mut following = VideoDecoder::default();
    for frame in &frames[..30] {
        if let Some(encoded) = encoder.encode(frame) {
            following.apply(&encoded.bytes).unwrap();
        }
    }

    let mut joining = VideoDecoder::default();
    let keyframe = encoder.keyframe().expect("something has been encoded");
    joining.apply(&keyframe.bytes).unwrap();
    assert_eq!(joining.pixels(), following.pixels(), "the keyframe did not catch the joiner up");

    // …and both stay in step across the deltas that follow, which is the part a keyframe carrying
    // only the colours it needed would have broken.
    for (n, frame) in frames[30..].iter().enumerate() {
        if let Some(encoded) = encoder.encode(frame) {
            following.apply(&encoded.bytes).unwrap();
            joining.apply(&encoded.bytes).unwrap();
        }
        assert_eq!(joining.pixels(), frame.as_ref(), "joiner diverged at frame {n} after the keyframe");
        assert_eq!(following.pixels(), frame.as_ref(), "follower diverged at frame {n}");
    }
}

/// The safety valve. Two frames that share no colours need 500 palette entries between them, so the
/// second must arrive as a keyframe with a fresh palette rather than as a delta that runs out of
/// indices half way through.
#[test]
fn palette_exhaustion_forces_a_keyframe() {
    let first = synthetic_frame(200, 0);
    let second = synthetic_frame(200, 1_000_000);

    let mut encoder = VideoEncoder::default();
    let mut decoder = VideoDecoder::default();

    let opening = encoder.encode(&first).expect("first frame");
    assert!(opening.keyframe);
    decoder.apply(&opening.bytes).unwrap();
    assert_eq!(decoder.pixels(), first.as_ref());

    let reset = encoder.encode(&second).expect("second frame");
    assert!(reset.keyframe, "a palette reset must be announced as a keyframe");
    decoder.apply(&reset.bytes).unwrap();
    assert_eq!(decoder.pixels(), second.as_ref(), "reconstruction survived the palette reset");

    // A decoder that only ever sees the reset — a viewer who joined one frame ago — must land in the
    // same place, which is only true because the keyframe replaced the palette rather than appending
    // to it.
    let mut fresh = VideoDecoder::default();
    fresh.apply(&reset.bytes).unwrap();
    assert_eq!(fresh.pixels(), second.as_ref());
}

/// A palette filled to the brim, and then overflowed within a single frame — the one case the
/// encoder cannot solve by spending a keyframe, because there is no smaller palette to reset to.
///
/// Two things must hold. The length byte must not wrap (it is a `u8`, which is why the cap is 255
/// and not 256), and the encoder must stay **self-consistent**: it tracks what the decoder holds,
/// not what the frame contained, so a keyframe taken afterwards has to describe the approximation
/// rather than the original. If `last_sent` held the true colours instead, `keyframe()` would look
/// up palette entries that were never sent and paint index 0.
#[test]
fn a_frame_that_overflows_the_palette_degrades_without_desynchronising() {
    let overflowing = synthetic_frame(400, 0);
    let mut encoder = VideoEncoder::default();
    let mut following = VideoDecoder::default();

    let encoded = encoder.encode(&overflowing).expect("first frame");
    assert_eq!(encoded.bytes[4], 255, "the palette filled to the cap and the length byte held it");
    following.apply(&encoded.bytes).unwrap();

    // Lossy by construction — 400 colours will not fit in 255 slots — so this is *not* pixel-exact
    // against the source, and asserting that it were would be asserting a lie.
    assert_ne!(following.pixels(), overflowing.as_ref());

    // …but re-offering the same frame must produce nothing, i.e. the encoder knows what it sent.
    assert_eq!(encoder.encode(&overflowing), None, "the encoder does not know what it already sent");

    // …and a joiner arriving now lands exactly where the follower is, approximation included.
    let mut joining = VideoDecoder::default();
    joining.apply(&encoder.keyframe().unwrap().bytes).unwrap();
    assert_eq!(joining.pixels(), following.pixels(), "the keyframe described the source, not what was sent");
}

/// All three modes have to be exercised by something, and each wins on a different shape of block.
#[test]
fn every_block_mode_is_chosen_when_it_is_the_smallest() {
    // Five or more colours per block, changing every pixel: RLE would spend 128 bytes and packing
    // is not available at all, so raw's flat 64 has to win.
    let mut noisy = Box::new([LcdColor::default(); PIXELS]);
    for (p, pixel) in noisy.iter_mut().enumerate() {
        let n = (p % 60) as u32 * 4;
        *pixel = LcdColor::rgb(n as u8, (n >> 1) as u8, (n >> 2) as u8);
    }
    let mut encoder = VideoEncoder::default();
    let encoded = encoder.encode(&noisy).expect("first frame");
    let counts = count_modes(&encoded.bytes);
    assert!(counts[MODE_RAW as usize] > 0, "no block chose raw mode, so mode 1 is untested");

    let mut decoder = VideoDecoder::default();
    decoder.apply(&encoded.bytes).unwrap();
    assert_eq!(decoder.pixels(), noisy.as_ref());

    // A real frame is four DMG shades: flat blocks go RLE, detailed ones pack, and nothing needs raw.
    let frames = recorded_frames(1);
    let mut plain = VideoEncoder::default();
    let real = plain.encode(&frames[0]).unwrap();
    let counts = count_modes(&real.bytes);
    assert!(counts[MODE_RLE as usize] > 0, "no block chose RLE on a real frame");
    assert!(counts[MODE_PACKED as usize] > 0, "no block packed on a real frame — mode 2 is dead weight");
    assert_eq!(counts[MODE_RAW as usize], 0, "a four-shade frame should never need raw");

    // The point of mode 2: without it this frame would cost meaningfully more. 20 bytes of payload
    // against RLE's measured ~40 for the same blocks.
    let packed_bytes: usize = counts[MODE_PACKED as usize] * (PACKED_COLOURS + BLOCK_PIXELS / 4);
    assert!(packed_bytes < real.bytes.len(), "sanity: packed blocks are part of this message");
}

/// Walk a message's block list counting each mode. Deliberately a second, dumber parse than
/// [`VideoDecoder`] so a bug shared with the decoder cannot hide here too — and the length arithmetic
/// per mode is exactly what a wire-format change breaks.
fn count_modes(message: &[u8]) -> [usize; 3] {
    let palette_len = message[4] as usize;
    let mut at = 5 + palette_len * 3;
    let blocks = u16::from_le_bytes([message[at], message[at + 1]]) as usize;
    at += 2;
    let mut counts = [0usize; 3];
    for _ in 0..blocks {
        at += 2; // block index
        let mode = message[at];
        at += 1;
        counts[mode as usize] += 1;
        match mode {
            MODE_RLE => {
                let mut covered = 0;
                while covered < BLOCK_PIXELS {
                    covered += message[at] as usize;
                    at += 2;
                }
            }
            MODE_RAW => at += BLOCK_PIXELS,
            MODE_PACKED => at += PACKED_COLOURS + BLOCK_PIXELS / 4,
            other => panic!("unknown mode {other}"),
        }
    }
    assert_eq!(at, message.len(), "trailing bytes after {blocks} blocks");
    counts
}

#[test]
fn a_corrupt_message_is_an_error_not_a_panic() {
    let frames = recorded_frames(1);
    let mut encoder = VideoEncoder::default();
    let good = encoder.encode(&frames[0]).unwrap().bytes;

    let mut wrong_version = good.clone();
    wrong_version[0] = VERSION + 1;
    assert!(VideoDecoder::default().apply(&wrong_version).is_err());

    // Truncation at every length: nothing may panic, and nothing may silently succeed.
    for cut in 0..good.len() {
        let mut decoder = VideoDecoder::default();
        assert!(decoder.apply(&good[..cut]).is_err(), "truncating to {cut} bytes was accepted");
    }

    // A delta whose palette index has never been sent is a desynchronised stream, not a black pixel.
    let mut orphan = vec![VERSION, 0, 1, 0, 0];
    orphan.extend_from_slice(&1u16.to_le_bytes());
    orphan.extend_from_slice(&0u16.to_le_bytes());
    orphan.push(MODE_RLE);
    orphan.extend_from_slice(&[BLOCK_PIXELS as u8, 7]);
    assert!(VideoDecoder::default().apply(&orphan).is_err());
}
