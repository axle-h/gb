// Golden-vector generator for the Rust port of Blip_Buffer.
//
// Links the *real* C++ library in vendor/ and writes its output to src/audio/data/*.bin, which the
// Rust tests in src/audio/blip/tests.rs then assert against bit-exactly. Blip_Buffer ships no test
// suite of its own — only interactive SDL demos — so this is where the reference behaviour comes
// from.
//
// Every case is deterministic and uses no libc rand(): the pseudo-random case runs an explicit LCG
// that src/audio/blip/tests.rs reimplements line for line. Anything sourced from the host (time,
// address layout, locale) would make the goldens unreproducible.
//
// Build and run from the repo root:  tools/blip-golden/build.sh

#include "vendor/Blip_Buffer.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vector>

// ---------------------------------------------------------------------------------------------
// Parameters shared with the Rust side. Any change here must be mirrored in src/audio/blip/.
// ---------------------------------------------------------------------------------------------

static const long   CLOCK_RATE  = 1048576; // Game Boy M-cycle rate
static const long   SAMPLE_RATE = 44100;
static const int    BUFFER_MS   = 100;
static const int    QUALITY     = blip_good_quality; // 12
static const int    RANGE       = 16384;             // gives delta_factor == 2 exactly
static const int    FULL_SCALE  = RANGE / 2;         // amplitude at mixed == +1.0
static const int    GB_BASS_HZ  = 28;                // matches the old CapacitanceFilter corner

typedef Blip_Synth<blip_good_quality, 16384> GbSynth;

// ---------------------------------------------------------------------------------------------
// Output helpers. Every file is: u32 little-endian count, then `count` little-endian i16 samples.
// ---------------------------------------------------------------------------------------------

static const char* OUT_DIR = "src/audio/data";

static void write_samples(const char* name, const std::vector<blip_sample_t>& samples)
{
    char path[512];
    snprintf(path, sizeof path, "%s/%s", OUT_DIR, name);
    FILE* f = fopen(path, "wb");
    if (!f) { fprintf(stderr, "cannot open %s\n", path); exit(1); }

    unsigned long n = (unsigned long) samples.size();
    unsigned char header[4] = {
        (unsigned char)(n & 0xff), (unsigned char)((n >> 8) & 0xff),
        (unsigned char)((n >> 16) & 0xff), (unsigned char)((n >> 24) & 0xff),
    };
    fwrite(header, 1, 4, f);
    for (size_t i = 0; i < samples.size(); i++) {
        unsigned short v = (unsigned short) samples[i];
        unsigned char b[2] = { (unsigned char)(v & 0xff), (unsigned char)((v >> 8) & 0xff) };
        fwrite(b, 1, 2, f);
    }
    fclose(f);
    printf("  %-28s %6lu samples\n", name, n);
}

// Drain everything currently available.
static void drain(Blip_Buffer& buf, std::vector<blip_sample_t>& out)
{
    long avail = buf.samples_avail();
    if (avail <= 0) return;
    size_t at = out.size();
    out.resize(at + avail);
    long got = buf.read_samples(&out[at], avail);
    out.resize(at + got);
}

static void setup(Blip_Buffer& buf, GbSynth& synth, int bass_hz)
{
    if (buf.set_sample_rate(SAMPLE_RATE, BUFFER_MS)) { fprintf(stderr, "out of memory\n"); exit(1); }
    buf.clock_rate(CLOCK_RATE);
    buf.bass_freq(bass_hz);
    synth.volume(1.0);
    synth.output(&buf);
}

// ---------------------------------------------------------------------------------------------
// (a) The impulse table itself.
//
// Blip_Synth keeps `impulses` private, but it is the first non-static data member of a class with
// no bases and no virtuals, so it sits at offset 0. Rather than trust that silently, the table is
// validated against the invariant adjust_impulse() exists to guarantee — every phase's taps sum to
// exactly kernel_unit — which fails loudly if the cast is ever wrong.
// ---------------------------------------------------------------------------------------------

static void gen_impulses(void)
{
    Blip_Buffer buf;
    GbSynth synth;
    setup(buf, synth, GB_BASS_HZ);

    const short* impulses = reinterpret_cast<const short*>(&synth);
    const int size = blip_res / 2 * QUALITY + 1;
    const long kernel_unit = 32768; // base_unit, un-shifted because delta_factor >= 2 here

    // Validate the cast: for each phase, the forward and reverse halves together must sum to
    // kernel_unit. This mirrors exactly how offset_resampled walks the table.
    for (int phase = 0; phase < blip_res; phase++) {
        const short* fwd = impulses + blip_res - phase;
        const short* rev = impulses + phase;
        long sum = 0;
        for (int k = 0; k <= QUALITY / 2 - 1; k++) sum += fwd[k * blip_res];
        for (int r = 0; r <= QUALITY / 2 - 1; r++) sum += rev[r * blip_res];
        if (sum != kernel_unit) {
            fprintf(stderr, "impulse table sum mismatch at phase %d: %ld != %ld\n", phase, sum, kernel_unit);
            fprintf(stderr, "(the offset-0 cast onto Blip_Synth is wrong on this compiler)\n");
            exit(1);
        }
    }

    // Three i16 of header, then the taps: quality, half of kernel_unit (32768 does not fit in i16),
    // and the tap count.
    std::vector<blip_sample_t> out;
    out.push_back((blip_sample_t) QUALITY);
    out.push_back((blip_sample_t) (kernel_unit / 2));
    out.push_back((blip_sample_t) size);
    for (int i = 0; i < size; i++) out.push_back(impulses[i]);
    write_samples("blip_impulses.bin", out);
}

// ---------------------------------------------------------------------------------------------
// (b) Step response at every one of the 64 sub-sample phases.
//
// bass_freq(0) makes bass_shift 31, and the accumulator here never reaches 2^31, so the leak term
// is exactly zero and each read is a clean prefix sum of the kernel. This pins offset_resampled's
// phase-symmetry indexing and the reader integration together.
//
// The step is half full scale: a full-scale step settles at exactly 32768, which the i16 clamp
// would flatten to 32767 and mask a real difference. The clamp gets its own coverage in (d).
// ---------------------------------------------------------------------------------------------

static void gen_step_response(void)
{
    Blip_Buffer buf;
    GbSynth synth;
    setup(buf, synth, 0);

    const int samples_per_phase = 32;
    std::vector<blip_sample_t> out;

    for (int phase = 0; phase < blip_res; phase++) {
        buf.clear();
        blip_resampled_time_t t = (blip_resampled_time_t) phase << (BLIP_BUFFER_ACCURACY - BLIP_PHASE_BITS);
        synth.offset_resampled(t, FULL_SCALE / 2, &buf);
        buf.end_frame(800); // ~33 samples' worth of clocks

        size_t at = out.size();
        out.resize(at + samples_per_phase);
        long got = buf.read_samples(&out[at], samples_per_phase);
        if (got != samples_per_phase) { fprintf(stderr, "short read in step response\n"); exit(1); }
    }
    write_samples("blip_step.bin", out);
}

// ---------------------------------------------------------------------------------------------
// (c) A square wave read back at irregular sizes.
//
// The awkward read sizes are the point: they leave a partial backlog in the buffer so that
// remove_samples' memmove and the reader_accum carry across calls are actually exercised.
// ---------------------------------------------------------------------------------------------

static void gen_square(const char* name, int bass_hz)
{
    Blip_Buffer buf;
    GbSynth synth;
    setup(buf, synth, bass_hz);

    static const long read_sizes[] = { 37, 512, 1, 4096, 129 };
    const long frame_clocks = 20000;
    std::vector<blip_sample_t> out;
    int amplitude = FULL_SCALE / 3;

    for (int frame = 0; frame < 5; frame++) {
        // Period shortens each frame, sweeping the wave up in pitch.
        long period = 400 - frame * 60;
        for (long t = 0; t < frame_clocks; t += period) {
            amplitude = -amplitude;
            synth.update(t, amplitude);
        }
        buf.end_frame(frame_clocks);

        long want = read_sizes[frame];
        if (want > buf.samples_avail()) want = buf.samples_avail();
        size_t at = out.size();
        out.resize(at + want);
        long got = buf.read_samples(&out[at], want);
        out.resize(at + got);
    }
    drain(buf, out);
    write_samples(name, out);
}

// ---------------------------------------------------------------------------------------------
// (d) Pseudo-random amplitude storm.
//
// Numerical Recipes LCG, reimplemented identically in Rust. Amplitudes span the full +/-RANGE, i.e.
// twice full scale, so the i16 clamp in read_samples gets hit in both directions.
// ---------------------------------------------------------------------------------------------

static unsigned long lcg_state = 12345;
static unsigned long lcg_next(void)
{
    lcg_state = (lcg_state * 1664525UL + 1013904223UL) & 0xffffffffUL;
    return lcg_state;
}

static void gen_lcg(void)
{
    Blip_Buffer buf;
    GbSynth synth;
    setup(buf, synth, GB_BASS_HZ);
    lcg_state = 12345;

    const long frame_clocks = 20000;
    std::vector<blip_sample_t> out;

    for (int frame = 0; frame < 5; frame++) {
        long t = 0;
        for (;;) {
            t += 1 + (long) (lcg_next() % 64);
            if (t >= frame_clocks) break;
            int amp = (int) (lcg_next() % (2 * RANGE + 1)) - RANGE;
            synth.update(t, amp);
        }
        buf.end_frame(frame_clocks);
        drain(buf, out);
    }
    write_samples("blip_lcg.bin", out);
}

// ---------------------------------------------------------------------------------------------
// (e) The real thing: 30 ms of captured Pokemon Red APU output.
//
// Input is src/audio/data/apu_capture_in.bin, produced by the ignored Rust test
// audio::reference::tests::capture_golden_input. Format: u32 run count, then per run
// (u16 clocks, i16 left, i16 right) — already quantised to the synth's integer amplitude domain,
// so this comparison is independent of the f32 mixing path.
//
// The update(0, amp) + end_frame(clocks) pattern here is exactly what Audio::push_sample does.
// ---------------------------------------------------------------------------------------------

static void gen_apu_capture(void)
{
    char path[512];
    snprintf(path, sizeof path, "%s/apu_capture_in.bin", OUT_DIR);
    FILE* f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "cannot open %s\n", path);
        fprintf(stderr, "run: cargo test --release --bin gb -- "
                        "audio::reference::tests::capture_golden_input --exact --ignored\n");
        exit(1);
    }
    unsigned char header[4];
    if (fread(header, 1, 4, f) != 4) { fprintf(stderr, "short read\n"); exit(1); }
    unsigned long runs = header[0] | (header[1] << 8) | ((unsigned long) header[2] << 16)
                       | ((unsigned long) header[3] << 24);

    Blip_Buffer left, right;
    GbSynth left_synth, right_synth;
    setup(left, left_synth, GB_BASS_HZ);
    setup(right, right_synth, GB_BASS_HZ);

    std::vector<blip_sample_t> l_out, r_out;
    for (unsigned long i = 0; i < runs; i++) {
        unsigned char rec[6];
        if (fread(rec, 1, 6, f) != 6) { fprintf(stderr, "truncated capture at run %lu\n", i); exit(1); }
        int clocks = rec[0] | (rec[1] << 8);
        short l = (short) (rec[2] | (rec[3] << 8));
        short r = (short) (rec[4] | (rec[5] << 8));

        left_synth.update(0, l);
        right_synth.update(0, r);
        left.end_frame(clocks);
        right.end_frame(clocks);

        // Keep well clear of the buffer limit even though 30 ms would fit comfortably.
        if (left.samples_avail() > 2048) { drain(left, l_out); drain(right, r_out); }
    }
    fclose(f);
    drain(left, l_out);
    drain(right, r_out);

    if (l_out.size() != r_out.size()) { fprintf(stderr, "channel length mismatch\n"); exit(1); }
    std::vector<blip_sample_t> out;
    out.reserve(l_out.size() * 2);
    for (size_t i = 0; i < l_out.size(); i++) { out.push_back(l_out[i]); out.push_back(r_out[i]); }
    write_samples("blip_apu.bin", out);
}

int main(void)
{
    printf("generating Blip_Buffer goldens into %s/\n", OUT_DIR);
    gen_impulses();
    gen_step_response();
    gen_square("blip_square.bin", GB_BASS_HZ);
    gen_square("blip_bass0.bin", 0);
    gen_square("blip_bass16.bin", 16);
    gen_square("blip_bass461.bin", 461);
    gen_lcg();
    gen_apu_capture();
    printf("done\n");
    return 0;
}
