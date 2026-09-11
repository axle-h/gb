use std::sync::OnceLock;

use super::*;
use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use poke_agent::pokemon::agent::PokemonAgent;
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::policy::RandomPolicy;
use poke_agent::pokemon::{PokemonApi, roms};

const SECONDS: u32 = 60;
/// The emulator is stepped at the host's own cadence so the APU is drained the way `tick` drains
/// it.
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
    // Exactly what the host's `tune_audio` does, and for the same reason: a state carries neither.
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

/// Built once per process and shared, so every test below measures the same audio.
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

fn kbits(bytes: usize, seconds: f64) -> f64 {
    bytes as f64 * 8.0 / seconds / 1000.0
}

/// Encode a whole capture at one bitrate, returning (payload bytes, packets).
fn encode(capture: &Capture, bitrate: i32) -> (usize, usize) {
    let mut encoder = AudioEncoder::new(bitrate);
    let mut out = Vec::new();
    // Pushed in host-sized bites rather than one slab, so the accumulator is exercised the way
    // the run exercises it.
    for chunk in capture.pcm.chunks(SAMPLE_RATE as usize / 50 * 2) {
        encoder.push(chunk, &mut out);
    }
    (encoder.bytes() as usize, out.len())
}

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

// ── The headline
// ─────────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "benchmark: what the Opus stream costs, and what it beat"]
fn bench_audio_the_shipped_stack_and_what_it_beat() {
    println!("\n=== {SECONDS}s per capture, {SAMPLE_RATE} Hz, kbit/s on the wire ===");
    let mut totals = [0f64; 7];
    let mut seconds = 0.0;

    for capture in captures() {
        let mono = to_mono(&capture.pcm);
        let span = capture.seconds();
        seconds += span;

        // Every row is what the *wire* carries: the payload plus our 4-byte length prefix per
        // message, which for Opus is 50 messages a second and for the raw rows is one per host
        // tick.
        let messages_per_second = 1000.0 / STEP_MS as f64;
        let framing = |per_second: f64| (4.0 * per_second * span) as usize;

        let row = [
            capture.pcm.len() * 4 + framing(messages_per_second),        // f32 stereo, as the APU makes it
            capture.pcm.len() * 2 + framing(messages_per_second),        // i16 stereo
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
        "raw i16 mono (= 24 kHz stereo)      ",
        "IMA ADPCM mono (the fallback)       ",
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
