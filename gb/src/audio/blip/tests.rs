//! Verification for the Blip_Buffer port.
//!
//! Two independent layers, because they fail in different ways:
//!
//! **Golden vectors.** Blip_Buffer ships no test suite — only interactive SDL demos — so the
//! reference behaviour is produced by linking the real C++ in `tools/blip-golden/` and freezing its
//! output into `../data/blip_*.bin`. Those comparisons are bit-exact. Regenerate with
//! `tools/blip-golden/build.sh` after any deliberate change.
//!
//! **Invariants.** Properties the algorithm must hold whatever the reference did — every phase's
//! taps summing to `kernel_unit`, a step depositing exactly its own amplitude of DC, no sample-rate
//! drift, no aliasing. These are what catch a change that is self-consistently wrong, and they need
//! no C++ toolchain to run.
//!
//! Everything below mirrors `tools/blip-golden/gen_golden.cpp` case for case. If you change a setup
//! here, change it there.

use super::buffer::BlipBuffer;
use super::eq::BlipEq;
use super::synth::BlipSynth;
use super::*;
use crate::audio::reference::rle_decode;

// Mirrors the constants at the top of gen_golden.cpp.
const CLOCK_RATE: u32 = 1_048_576;
const SAMPLE_RATE: u32 = 44_100;
const FULL_SCALE: i32 = SYNTH_RANGE / 2; // amplitude at mixed == +1.0
const GB_BASS_HZ: u32 = 28;

/// The equalisation the fixtures were generated at, spelled out rather than read from
/// [`DEFAULT_TREBLE_DB`]. The two happen to be equal today, but tone is a taste knob and the port's
/// correctness is not: changing what the emulator ships should not silently invalidate the goldens.
const GOLDEN_TREBLE_DB: f64 = -8.0;

const GOLDEN_IMPULSES: &[u8] = include_bytes!("../data/blip_impulses.bin");
const GOLDEN_STEP: &[u8] = include_bytes!("../data/blip_step.bin");
const GOLDEN_SQUARE: &[u8] = include_bytes!("../data/blip_square.bin");
const GOLDEN_BASS0: &[u8] = include_bytes!("../data/blip_bass0.bin");
const GOLDEN_BASS16: &[u8] = include_bytes!("../data/blip_bass16.bin");
const GOLDEN_BASS461: &[u8] = include_bytes!("../data/blip_bass461.bin");
const GOLDEN_LCG: &[u8] = include_bytes!("../data/blip_lcg.bin");
const GOLDEN_APU: &[u8] = include_bytes!("../data/blip_apu.bin");
const APU_CAPTURE: &[u8] = include_bytes!("../data/apu_capture_in.bin");

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// Golden file format: `u32` count, then that many little-endian `i16`.
fn parse_golden(bytes: &[u8]) -> Vec<i16> {
    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    (0..count)
        .map(|i| i16::from_le_bytes(bytes[4 + i * 2..6 + i * 2].try_into().unwrap()))
        .collect()
}

/// Compare against a golden, and on failure say *where* and *by how much* rather than dumping
/// thousands of samples into the test log.
fn assert_matches_golden(name: &str, actual: &[i16], expected: &[i16]) {
    if actual == expected {
        return;
    }
    let first = actual.iter().zip(expected).position(|(a, b)| a != b);
    let max_diff = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| (*a as i32 - *b as i32).abs())
        .max()
        .unwrap_or(0);

    // Both series land under target/ so they can be diffed without polluting the working tree.
    let dir = std::path::Path::new("target/test-artifacts");
    std::fs::create_dir_all(dir).ok();
    let dump = |suffix: &str, data: &[i16]| {
        let bytes: Vec<u8> = data.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(dir.join(format!("{name}_{suffix}.bin")), bytes).ok();
    };
    dump("actual", actual);
    dump("expected", expected);

    let context = first
        .map(|i| {
            let lo = i.saturating_sub(3);
            let hi = (i + 4).min(actual.len()).min(expected.len());
            format!("\n  at {i}: actual {:?}\n         expected {:?}", &actual[lo..hi], &expected[lo..hi])
        })
        .unwrap_or_default();

    panic!(
        "{name} does not match the C++ golden\n  lengths: actual {} expected {}\n  first difference: {first:?}\n  max abs difference: {max_diff}{context}\n  dumps written to {}",
        actual.len(),
        expected.len(),
        dir.display(),
    );
}

fn gb_buffer(bass_hz: u32) -> BlipBuffer {
    BlipBuffer::new(CLOCK_RATE, SAMPLE_RATE, BUFFER_MS, bass_hz)
}

fn gb_synth() -> BlipSynth<QUALITY> {
    BlipSynth::new(BlipEq::new(GOLDEN_TREBLE_DB), 1.0)
}

/// Read everything currently available.
fn drain(buf: &mut BlipBuffer, out: &mut Vec<i16>) {
    let avail = buf.samples_avail();
    if avail == 0 {
        return;
    }
    let at = out.len();
    out.resize(at + avail, 0);
    buf.read_i16(&mut out[at..]);
}

// ---------------------------------------------------------------------------------------------
// (a) The impulse table
// ---------------------------------------------------------------------------------------------

/// The canary for every other golden test.
///
/// This is the only place floating point enters the pipeline, and the only place where this
/// platform's libm could disagree with the one that built the fixtures. If this passes, the integer
/// path below is being fed an identical table and any failure there is a real porting bug; if this
/// is the *only* failure, see `step_response_matches_cpp_with_cpp_table`.
#[test]
fn impulse_table_matches_cpp() {
    let golden = parse_golden(GOLDEN_IMPULSES);
    let (quality, half_kernel_unit, tap_count) = (golden[0], golden[1], golden[2]);
    let taps = &golden[3..];

    assert_eq!(quality as usize, QUALITY);
    assert_eq!(tap_count as usize, taps.len());

    let synth = gb_synth();
    assert_eq!(synth.kernel_unit(), half_kernel_unit as i64 * 2, "kernel_unit");
    assert_eq!(synth.delta_factor(), 2, "delta_factor must be exactly 2 for SYNTH_RANGE");
    assert_matches_golden("impulses", synth.impulses(), taps);
}

/// The property `adjust_impulse` exists to guarantee: whatever phase a transition lands on, its
/// taps sum to exactly `kernel_unit`. Without this a step deposits slightly the wrong amount of DC
/// and the error accumulates over a playthrough.
#[test]
fn every_phase_sums_to_kernel_unit() {
    let synth = gb_synth();
    let taps = synth.impulses();
    let mid = QUALITY / 2 - 1;
    for phase in 0..BLIP_RES {
        let mut sum = 0i64;
        for k in 0..=mid {
            sum += taps[BLIP_RES - phase + k * BLIP_RES] as i64;
            sum += taps[phase + k * BLIP_RES] as i64;
        }
        assert_eq!(sum, synth.kernel_unit(), "phase {phase} taps sum to {sum}");
    }
}

// ---------------------------------------------------------------------------------------------
// (b) Step response at all 64 sub-sample phases
// ---------------------------------------------------------------------------------------------

fn step_response(synth: &BlipSynth<QUALITY>) -> Vec<i16> {
    const SAMPLES_PER_PHASE: usize = 32;
    let mut buf = gb_buffer(0);
    let mut out = vec![0i16; BLIP_RES * SAMPLES_PER_PHASE];

    for phase in 0..BLIP_RES {
        buf.clear();
        let t = (phase as u64) << (BLIP_BUFFER_ACCURACY - BLIP_PHASE_BITS);
        synth.offset_resampled(t, FULL_SCALE / 2, &mut buf);
        buf.end_frame(800);
        let at = phase * SAMPLES_PER_PHASE;
        let got = buf.read_i16(&mut out[at..at + SAMPLES_PER_PHASE]);
        assert_eq!(got, SAMPLES_PER_PHASE, "short read at phase {phase}");
    }
    out
}

#[test]
fn step_response_matches_cpp() {
    assert_matches_golden("step", &step_response(&gb_synth()), &parse_golden(GOLDEN_STEP));
}

/// The libm escape hatch.
///
/// Loads the C++-generated impulse table straight into the Rust synth, so this exercises the
/// integer DSP — the phase-symmetry indexing, the scatter-add, the reader integration and the
/// clamp — with the floating-point kernel generator taken out of the picture entirely. It holds
/// bit-exactly whether or not this platform's `cos`/`pow` agree with the fixture host's.
#[test]
fn step_response_matches_cpp_with_cpp_table() {
    let golden = parse_golden(GOLDEN_IMPULSES);
    let (half_kernel_unit, taps) = (golden[1], &golden[3..]);

    let mut synth = gb_synth();
    synth.set_raw_impulses(taps, half_kernel_unit as i64 * 2, 2);
    assert_matches_golden("step_cpp_table", &step_response(&synth), &parse_golden(GOLDEN_STEP));
}

// ---------------------------------------------------------------------------------------------
// (c) / (f) Square wave, read back at irregular sizes, across bass settings
// ---------------------------------------------------------------------------------------------

fn square_wave(bass_hz: u32) -> Vec<i16> {
    const READ_SIZES: [usize; 5] = [37, 512, 1, 4096, 129];
    const FRAME_CLOCKS: u32 = 20000;

    let mut buf = gb_buffer(bass_hz);
    let mut synth = gb_synth();
    let mut out: Vec<i16> = Vec::new();
    let mut amplitude = FULL_SCALE / 3;

    for frame in 0..5u32 {
        let period = 400 - frame * 60;
        let mut t = 0;
        while t < FRAME_CLOCKS {
            amplitude = -amplitude;
            synth.update(t, amplitude, &mut buf);
            t += period;
        }
        buf.end_frame(FRAME_CLOCKS);

        // Awkward read sizes on purpose: they leave a partial backlog, which is what exercises the
        // slide-down in remove_samples and the reader accumulator carrying across calls.
        let want = READ_SIZES[frame as usize].min(buf.samples_avail());
        let at = out.len();
        out.resize(at + want, 0);
        buf.read_i16(&mut out[at..]);
    }
    drain(&mut buf, &mut out);
    out
}

#[test]
fn square_matches_cpp() {
    assert_matches_golden("square", &square_wave(GB_BASS_HZ), &parse_golden(GOLDEN_SQUARE));
}

/// Pins the shift-search in `set_bass_freq`, which quantises the requested corner to a power of two.
#[test]
fn bass_variants_match_cpp() {
    for (hz, golden) in [(0u32, GOLDEN_BASS0), (16, GOLDEN_BASS16), (461, GOLDEN_BASS461)] {
        assert_matches_golden(&format!("bass{hz}"), &square_wave(hz), &parse_golden(golden));
    }
}

// ---------------------------------------------------------------------------------------------
// (d) Pseudo-random amplitude storm
// ---------------------------------------------------------------------------------------------

/// The same Numerical Recipes LCG gen_golden.cpp runs. Deliberately not `rand`: libc's generator
/// differs between platforms, and the fixtures have to be reproducible.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

/// Amplitudes span the full ±SYNTH_RANGE, i.e. twice full scale, so the i16 clamp in the reader is
/// driven into saturation in both directions.
#[test]
fn lcg_matches_cpp() {
    const FRAME_CLOCKS: u32 = 20000;
    let mut buf = gb_buffer(GB_BASS_HZ);
    let mut synth = gb_synth();
    let mut lcg = Lcg(12345);
    let mut out: Vec<i16> = Vec::new();

    for _ in 0..5 {
        let mut t = 0u32;
        loop {
            t += 1 + (lcg.next() % 64);
            if t >= FRAME_CLOCKS {
                break;
            }
            let amp = (lcg.next() % (2 * SYNTH_RANGE as u32 + 1)) as i32 - SYNTH_RANGE;
            synth.update(t, amp, &mut buf);
        }
        buf.end_frame(FRAME_CLOCKS);
        drain(&mut buf, &mut out);
    }

    let golden = parse_golden(GOLDEN_LCG);
    assert!(
        golden.iter().any(|s| *s == 32767) && golden.iter().any(|s| *s == -32768),
        "the LCG golden should saturate in both directions — it is the only clamp coverage"
    );
    assert_matches_golden("lcg", &out, &golden);
}

// ---------------------------------------------------------------------------------------------
// (e) Real captured APU output, end to end through BlipStereo
// ---------------------------------------------------------------------------------------------

/// The integration proof: 30 ms of actual Pokémon Red audio, driven through exactly the
/// `update` + `end_frame` pattern `Audio::push_sample` uses.
///
/// The capture is stored already quantised to the synth's integer amplitude domain, so this
/// compares the resampler alone and is not perturbed by the `f32` mixing path.
#[test]
fn apu_capture_matches_cpp() {
    let runs = rle_decode(APU_CAPTURE);
    assert!(runs.len() > 500, "capture fixture looks truncated: {} runs", runs.len());

    let mut left = gb_buffer(GB_BASS_HZ);
    let mut right = gb_buffer(GB_BASS_HZ);
    let mut left_synth = gb_synth();
    let mut right_synth = gb_synth();
    let (mut l_out, mut r_out): (Vec<i16>, Vec<i16>) = (Vec::new(), Vec::new());

    for (clocks, l, r) in runs {
        left_synth.update(0, l as i32, &mut left);
        right_synth.update(0, r as i32, &mut right);
        left.end_frame(clocks as u32);
        right.end_frame(clocks as u32);
        if left.samples_avail() > 2048 {
            drain(&mut left, &mut l_out);
            drain(&mut right, &mut r_out);
        }
    }
    drain(&mut left, &mut l_out);
    drain(&mut right, &mut r_out);

    let interleaved: Vec<i16> = l_out.iter().zip(&r_out).flat_map(|(l, r)| [*l, *r]).collect();
    assert_matches_golden("apu", &interleaved, &parse_golden(GOLDEN_APU));
}

/// The same signal through the public `BlipStereo` façade must produce the same frames — i.e. the
/// interleaving wrapper does not perturb what the two buffers do.
#[test]
fn blip_stereo_matches_raw_buffers() {
    let runs = rle_decode(APU_CAPTURE);
    let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
    let mut out: Vec<i16> = Vec::new();
    let mut scratch = vec![0i16; 8192];

    for (clocks, l, r) in &runs {
        stereo.update(crate::audio::sample::AudioSample::new(
            *l as f32 / AMP_SCALE,
            *r as f32 / AMP_SCALE,
        ));
        stereo.end_frame(*clocks as u32);
        if stereo.frames_avail() > 2048 {
            let frames = stereo.read_interleaved_i16(&mut scratch);
            out.extend_from_slice(&scratch[..frames * 2]);
        }
    }
    let frames = stereo.read_interleaved_i16(&mut scratch);
    out.extend_from_slice(&scratch[..frames * 2]);

    assert_matches_golden("apu_stereo", &out, &parse_golden(GOLDEN_APU));
}

// ---------------------------------------------------------------------------------------------
// Invariants — no C++ toolchain required
// ---------------------------------------------------------------------------------------------

/// A step up followed by an equal step down must leave the integrator exactly where it started.
/// Any asymmetry here would show up as DC drift over a long session.
#[test]
fn step_up_then_down_is_dc_neutral() {
    let mut buf = gb_buffer(0); // no leak, so the accumulator must return to precisely zero
    let mut synth = gb_synth();
    let mut out = Vec::new();

    synth.update(100, FULL_SCALE, &mut buf);
    synth.update(5000, 0, &mut buf);
    buf.end_frame(20000);
    drain(&mut buf, &mut out);

    let tail = &out[out.len() - 64..];
    assert!(tail.iter().all(|s| *s == 0), "settled back to {:?}, not silence", &tail[..8]);
}

/// A step of amplitude `a` must settle at exactly `a * delta_factor * kernel_unit >> 14`, which for
/// this configuration is `a * 4`. This is the gain contract the amplitude domain is built on.
#[test]
fn step_settles_at_unity_gain() {
    let mut buf = gb_buffer(0);
    let mut synth = gb_synth();
    let mut out = Vec::new();

    let amp = FULL_SCALE / 4;
    synth.update(0, amp, &mut buf);
    buf.end_frame(20000);
    drain(&mut buf, &mut out);

    assert_eq!(*out.last().unwrap() as i32, amp * 4);
    // And a full-scale step reaches full output scale (the clamp turns 32768 into 32767).
    assert_eq!(FULL_SCALE * 4, 32768);
}

/// The resampling ratio is inexact by design — it is rounded to 1/65536 — but it must be *exact*
/// with respect to whatever it rounded to: the fractional cursor has to carry across every frame
/// boundary and every partial read, with no accumulating loss.
///
/// So this asserts two separate things. First, that the realised rate is close enough to the
/// requested one to be inaudible. Second, and much more strictly, that the sample count is
/// *precisely* what the rounded ratio predicts after ten minutes — not merely close to it.
#[test]
fn sample_count_does_not_drift() {
    let mut buf = gb_buffer(GB_BASS_HZ);
    let mut scratch = vec![0i16; 8192];
    let mut produced = 0u64;
    let clocks_per_frame = 17556u32; // one Game Boy video frame

    // 44100/1048576 * 65536 = 2756.25, which rounds to 2756 — an actual output rate of 44096 Hz.
    // That is 4 Hz low, about 1.6 cents of pitch, and it never gets any worse.
    let realised_rate = buf.factor() as f64 * CLOCK_RATE as f64 / (1u64 << BLIP_BUFFER_ACCURACY) as f64;
    let error = (realised_rate - SAMPLE_RATE as f64).abs() / SAMPLE_RATE as f64;
    assert!(error < 1e-4, "realised rate {realised_rate} is {:.4}% off nominal", error * 100.0);

    const FRAMES: u64 = 60 * 60 * 10; // ten minutes of emulated video frames
    for _ in 0..FRAMES {
        buf.end_frame(clocks_per_frame);
        produced += buf.read_i16(&mut scratch) as u64;
    }

    let total_clocks = FRAMES * clocks_per_frame as u64;
    let expected = (total_clocks * buf.factor()) >> BLIP_BUFFER_ACCURACY;
    assert_eq!(produced, expected, "sample production drifted from the resampling ratio");
}

/// Nothing may panic or write out of bounds across the full phase and amplitude space, at any
/// supported quality.
#[test]
fn no_panic_across_phases_and_qualities() {
    fn sweep<const Q: usize>() {
        let mut buf = BlipBuffer::new(CLOCK_RATE, SAMPLE_RATE, BUFFER_MS, DEFAULT_BASS_HZ);
        let synth: BlipSynth<Q> = BlipSynth::new(BlipEq::new(GOLDEN_TREBLE_DB), 1.0);
        let mut scratch = vec![0i16; 8192];
        for phase in 0..BLIP_RES as u64 {
            for amp in [-SYNTH_RANGE, -1, 0, 1, SYNTH_RANGE] {
                buf.clear();
                let t = phase << (BLIP_BUFFER_ACCURACY - BLIP_PHASE_BITS);
                synth.offset_resampled(t, amp, &mut buf);
                buf.end_frame(20000);
                buf.read_i16(&mut scratch);
            }
        }
    }
    sweep::<8>();
    sweep::<12>();
    sweep::<16>();
}

/// The headless integration tests run up to twenty minutes of emulated time with no audio consumer
/// at all. That must neither panic nor grow without bound.
#[test]
fn survives_a_minute_with_no_reader() {
    let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
    let mut lcg = Lcg(7);
    let mut elapsed = 0u64;
    let total = CLOCK_RATE as u64 * 60;

    while elapsed < total {
        let amp = (lcg.next() % 2048) as f32 / 2048.0 - 0.5;
        stereo.update(crate::audio::sample::AudioSample::new(amp, -amp));
        let clocks = 1 + lcg.next() % 6;
        stereo.end_frame(clocks);
        elapsed += clocks as u64;
        assert!(stereo.frames_avail() < SAMPLE_RATE as usize, "backlog grew unbounded");
    }
}

/// The `f32` reader must agree with the `i16` reader to within the latter's quantisation step —
/// it is the same accumulator, just scaled before truncation rather than after.
#[test]
fn f32_reader_agrees_with_i16_reader() {
    let runs = rle_decode(APU_CAPTURE);
    let render = |as_f32: bool| -> Vec<f32> {
        let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
        let mut out = Vec::new();
        let mut f32_scratch = vec![0.0f32; 8192];
        let mut i16_scratch = vec![0i16; 8192];
        for (clocks, l, r) in &runs {
            stereo.update(crate::audio::sample::AudioSample::new(
                *l as f32 / AMP_SCALE,
                *r as f32 / AMP_SCALE,
            ));
            stereo.end_frame(*clocks as u32);
        }
        if as_f32 {
            let n = stereo.read_interleaved_f32(&mut f32_scratch);
            out.extend_from_slice(&f32_scratch[..n * 2]);
        } else {
            let n = stereo.read_interleaved_i16(&mut i16_scratch);
            out.extend(i16_scratch[..n * 2].iter().map(|s| *s as f32 / 32768.0));
        }
        out
    };

    let floats = render(true);
    let ints = render(false);
    assert_eq!(floats.len(), ints.len());
    let worst = floats
        .iter()
        .zip(&ints)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst <= 1.0 / 32768.0, "readers disagree by {worst}, more than one i16 step");
}

/// Render a square wave of `freq` Hz through a `BlipStereo`, in buffer-sized frames.
fn render_square(stereo: &mut BlipStereo, freq: f64, seconds: f64) -> Vec<f32> {
    const FRAME_CLOCKS: u32 = 20000;
    let half_period = CLOCK_RATE as f64 / (2.0 * freq);
    let total_clocks = (CLOCK_RATE as f64 * seconds) as u32;
    let mut level = 0.25f32;
    let mut out: Vec<f32> = Vec::new();
    let mut scratch = vec![0.0f32; 16384];
    let (mut k, mut elapsed) = (1u32, 0u32);

    while elapsed < total_clocks {
        // BlipStereo only reports level changes at frame starts, so step one half-period per frame
        // when the period is longer than a frame, and otherwise sub-divide.
        let mut within = 0u32;
        while within < FRAME_CLOCKS {
            let t = (k as f64 * half_period).round() as u32;
            if t >= elapsed + FRAME_CLOCKS {
                break;
            }
            let step = t - elapsed - within;
            stereo.end_frame(step);
            within += step;
            level = -level;
            stereo.update(crate::audio::sample::AudioSample::new(level, level));
            k += 1;
        }
        stereo.end_frame(FRAME_CLOCKS - within);
        elapsed += FRAME_CLOCKS;
        let frames = stereo.read_interleaved_f32(&mut scratch);
        out.extend(scratch[..frames * 2].iter().step_by(2));
    }
    out
}

/// The tuning knobs have to actually do something, and `clear` has to actually clear. Cheaper to
/// assert that here than to discover a no-op setter while chasing a tone problem by ear.
#[test]
fn stereo_knobs_take_effect() {
    let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
    assert_eq!(stereo.sample_rate(), SAMPLE_RATE);

    // Rate: 50 ms of clocks yields proportionally more frames at a higher output rate.
    let frames_at = |stereo: &mut BlipStereo| {
        stereo.clear();
        stereo.end_frame(CLOCK_RATE / 20);
        stereo.frames_avail()
    };
    let at_44k = frames_at(&mut stereo);
    stereo.set_sample_rate(48_000);
    assert_eq!(stereo.sample_rate(), 48_000);
    let at_48k = frames_at(&mut stereo);
    assert!(at_48k > at_44k, "48 kHz produced {at_48k} frames, 44.1 kHz produced {at_44k}");

    // Clear really empties the buffer.
    stereo.clear();
    assert_eq!(stereo.frames_avail(), 0);

    // Treble: less rolloff means more energy left at 10 kHz.
    let treble_energy = |db: f64| {
        let mut s = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
        s.set_treble_db(db);
        let out = render_square(&mut s, 10_000.0, 0.2);
        magnitude_at(&out, SAMPLE_RATE as f32, 10_000.0)
    };
    let (flat, rolled) = (treble_energy(0.0), treble_energy(-8.0));
    assert!(flat > rolled * 1.05, "treble 0 dB ({flat}) is not brighter than -8 dB ({rolled})");

    // Bass: a corner up at 2 kHz strips far more of a 200 Hz tone than the default 28 Hz does.
    let bass_energy = |hz: u32| {
        let mut s = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
        s.set_bass_freq(hz);
        let out = render_square(&mut s, 200.0, 0.2);
        magnitude_at(&out, SAMPLE_RATE as f32, 200.0)
    };
    let (full, thin) = (bass_energy(DEFAULT_BASS_HZ), bass_energy(2000));
    assert!(full > thin * 2.0, "bass 28 Hz ({full}) barely differs from 2 kHz ({thin})");
}

/// Fast-forward contract: however fast the emulator runs, the *wall-clock* output rate must stay
/// at the sink's rate. That is what stops a sped-up emulator out-running the audio device.
///
/// Concretely — at N× speed one wall second contains N seconds of game time, so N seconds of game
/// time has to yield exactly one sink-second of samples.
#[test]
fn speed_keeps_the_wall_clock_output_rate_constant() {
    for speed in [1.0, 2.0, 3.006, 5.016, 0.5] {
        let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
        stereo.set_speed(speed);
        assert!((stereo.speed() - speed).abs() < 1e-3, "speed() reported {}", stereo.speed());

        // One wall-clock second of emulation, fed in 10 ms slices so the buffer is drained rather
        // than overflowed.
        let mut frames = 0usize;
        let mut scratch = vec![0.0f32; 16384];
        let game_clocks = (CLOCK_RATE as f64 * speed) as u32;
        let slice = game_clocks / 100;
        for _ in 0..100 {
            stereo.end_frame(slice);
            frames += stereo.read_interleaved_f32(&mut scratch);
        }

        let error = (frames as f64 - SAMPLE_RATE as f64).abs() / SAMPLE_RATE as f64;
        assert!(
            error < 0.005,
            "at {speed}x, one wall second produced {frames} frames, expected ~{SAMPLE_RATE}",
        );
    }
}

/// Changing speed must not disturb audio already buffered — a speed change should not click.
#[test]
fn speed_change_preserves_buffered_audio() {
    let mut stereo = BlipStereo::new(CLOCK_RATE, SAMPLE_RATE);
    stereo.update(crate::audio::sample::AudioSample::new(0.5, -0.5));
    stereo.end_frame(CLOCK_RATE / 100);
    let before = stereo.frames_avail();
    assert!(before > 0);

    stereo.set_speed(5.0);
    assert_eq!(stereo.frames_avail(), before, "a speed change discarded buffered frames");
}

// ---------------------------------------------------------------------------------------------
// Spectral behaviour — is this actually a good resampler, independent of the reference?
// ---------------------------------------------------------------------------------------------

/// Magnitude of `signal` at `freq`, via a Hann-windowed single-bin DFT.
///
/// A whole FFT would need a crate, and this crate has no dev-dependencies. Evaluating the two bins
/// the test cares about directly is cheaper anyway.
fn magnitude_at(signal: &[f32], sample_rate: f32, freq: f32) -> f32 {
    let n = signal.len();
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, s) in signal.iter().enumerate() {
        let window = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
        let angle = -2.0 * std::f64::consts::PI * freq as f64 * i as f64 / sample_rate as f64;
        let v = *s as f64 * window;
        re += v * angle.cos();
        im += v * angle.sin();
    }
    ((re * re + im * im).sqrt() / n as f64) as f32
}

/// The whole point of band-limited synthesis: a square wave whose harmonics all sit above Nyquist
/// must come out as a clean tone, not a mess of folded-back images.
///
/// A 15 kHz square at the Game Boy clock has its next harmonic at 45 kHz. Naive decimation would
/// fold that to |44100 - 45000| = 900 Hz — an audible whistle nowhere near the source frequency.
/// A correct band-limited synth leaves essentially nothing there.
#[test]
fn high_frequency_square_does_not_alias() {
    const FUNDAMENTAL: f32 = 15000.0;
    let mut buf = gb_buffer(GB_BASS_HZ);
    let mut synth = gb_synth();

    // Toggle at exact fractional half-periods so the wave is genuinely 15 kHz rather than snapped
    // to a whole number of clocks — snapping would itself be a source of spurious tones.
    //
    // Synthesised in 20 ms frames: the buffer holds 100 ms, and a transition landing past its end
    // is a hard error rather than something to discover from a bounds check.
    let half_period = CLOCK_RATE as f64 / (2.0 * FUNDAMENTAL as f64);
    const FRAME_CLOCKS: u32 = 20000;
    let total_clocks = CLOCK_RATE / 4; // 0.25 s
    let mut amplitude = FULL_SCALE / 2;
    let mut out: Vec<f32> = Vec::new();
    let mut scratch = vec![0.0f32; 8192];
    let (mut k, mut elapsed) = (1u32, 0u32);

    while elapsed < total_clocks {
        loop {
            let t = (k as f64 * half_period).round() as u32;
            if t >= elapsed + FRAME_CLOCKS {
                break;
            }
            amplitude = -amplitude;
            synth.update(t - elapsed, amplitude, &mut buf);
            k += 1;
        }
        buf.end_frame(FRAME_CLOCKS);
        elapsed += FRAME_CLOCKS;
        let n = buf.read_f32(&mut scratch);
        out.extend_from_slice(&scratch[..n]);
    }

    let rate = SAMPLE_RATE as f32;
    let fundamental = magnitude_at(&out, rate, FUNDAMENTAL);
    let alias = magnitude_at(&out, rate, 44100.0 - 3.0 * FUNDAMENTAL);

    assert!(fundamental > 0.01, "fundamental barely present ({fundamental})");
    let ratio = alias / fundamental;
    assert!(ratio < 0.01, "aliased image at 900 Hz is only {:.1} dB down", 20.0 * ratio.log10());
}
