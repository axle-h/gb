---
name: web-streams
description: "The /api/video block-delta codec and the /api/audio Opus stream, server and client: the four silent codec invariants, why video deflates per connection and audio deflates never, 48 kHz against opus-rs, and the browser-side jitter buffer, drift trim and reconnect rules. Load before touching src/web/{video,audio}* or web/src/{stream,video,audio}.ts."
---

# The video and audio streams

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## The video stream

The README has the numbers (565 → 21 kbit/s), the three levers and the alternatives measured and rejected;
`src/web/video/bench.rs` (`--features bench`, seeded `RandomPolicy` over four fixtures with different screen
behaviour) is the only source for any of them. ⚠️ **Quote the number for a screen that is *moving*** — W2 measured
536 kbit/s walking against ~8 idle and what reached the README was "about 19", which is neither. What a change to
the stack has to not break:

- ⚠️ **The compressor is per connection**, which is the price of deflating the stream rather than the message. It
  cannot be done once in `Published` for everyone, which is why `VideoMessage` carries plain bytes.
- ⚠️ **`VideoStream::frame` flushes after every message and must keep doing so.** Without the flush the encoder holds
  messages back until its buffer fills — correct for a file, and a livestream that arrives in bursts seconds apart.
  `the_video_stream_is_one_deflate_stream_of_length_prefixed_messages` inflates incrementally for exactly this
  reason; inflating at the end would pass either way.
- ⚠️ **The deflate is `Content-Type`, not `Content-Encoding`.** A declared encoding invites a proxy to inflate and
  re-deflate, which buffers whole messages and shows up as stutter only in production. The client inflates with
  `DecompressionStream('deflate')`.
- ⚠️ **Never base64 something you are going to compress** — 33% before compression, **69–113% after**, because it
  shifts a repeating byte pattern into three alphabet phases and LZ77 stops seeing the repeat. That single fact is
  what took SSE off the table: SSE cannot carry binary.

⚠️ **`gb serve` runs `GameBoy::dmg` unless `GB_HARDWARE=cgb`, so the screen is four shades — the format is built on
it.** On a CGB it is six (compatibility mode: BG and OBJ1 share the red ramp, OBJ0 the green one), which widens
`bits_per_pixel` from 2 to 4 and measured **1.63× on the wire** — less than the 2× it costs before deflate, because
the extra bits repeat what the compressor is already matching. Nothing in the format changed to allow it: the width
has always been per message (1/2/4/8, wide enough for the palette *after* this message's new entries). ⚠️ **A
non-power-of-two width is not the saving it looks like** — 3bpp for six colours is 25% fewer raw bits but only
**13%** fewer on the wire, because an unaligned bitstream shifts an identical tile payload into eight phases and
LZ77 stops matching it. Same mechanism as base64.

⚠️ **Four codec invariants that fail silently rather than loudly**, all found building v1 and all still true:

- **A keyframe must REPLACE the decoder's palette with the encoder's WHOLE palette**, not append only
  the colours its own blocks need, or every late joiner silently desynchronises — no error, just a
  corner of the screen wrong for ever. That is what makes `VideoEncoder::keyframe()` pure and
  publishable beside every delta.
- **Store the keyframe BEFORE broadcasting the delta.** The other order loses a delta for anyone who
  subscribes in the gap; `late_joiner_never_misses_a_delta` loops over the size of that window. ⚠️ And
  it must stay *current* — a stale stored keyframe loses the deltas published between it and the
  joiner's subscribe point, so it cannot be rebuilt every N frames to save CPU (it is ~0.06% of a core).
- **`new_palette_len` is a `u8`, so the palette caps at 255, not 256.** A 256th entry encodes its
  length as 0 and the decoder reads the block list as palette bytes.
- **The encoder must track what the DECODER holds, not what the frame contained**, and must decide
  "changed" *after* interning — otherwise the lossy path (>255 colours) emits a full keyframe every
  tick for ever for a screen nobody is touching.

⚠️ **No keyframe goes on the wire in steady state**, so an adaptive keyframe interval is not a lever;
the stored one is only for joiners and lag recovery.

Two things left on the table, both measured in `bench_video_redundancy_still_on_the_table` and neither built: 12–19%
of changed blocks duplicate a block already on screen, and a global scroll vector beats a straight diff on half to
four fifths of moving frames. Deflate already collects most of the first.

## The audio stream

The README has the format, the bitrate table's headline and the `opus-rs` story; `src/web/audio/bench.rs` is the only
source for the kbit/s figures in either file. What a change has to not break:

⚠️ **48 kHz, not 24, and the difference is a bug in the dependency rather than a preference.** Anyone "tidying"
`SAMPLE_RATE` to match the 24 kbit/s figure breaks the sound in the worst available way: it stays the right loudness
and stops being the right sound. (`opus-rs` 0.1.32 at 24 kHz decodes — through its own decoder *and* through real
libopus 1.6 — with the spectrum destroyed, tones at +29.7/+39.2/+36.5 dB coming back at −3.1/+3.2/+2.5, and no better
at a higher bitrate; at 48 kHz the same signal round-trips to within 0.3 dB, with libopus→libopus at 0.2 dB as the
control that proves the measurement rather than the crate. 16 kHz is fine; 24 is the one rate that fails.)

⚠️ **The guard is spectral, never a sample-wise SNR, and the first version got this wrong.** Opus has ~6.5 ms of
lookahead and CELT does not preserve phase, so a waveform comparison read −3 dB SNR on output that was correct —
indistinguishable from the real bug, which is the worst possible answer from the test guarding a three-week-old
dependency. `the_packets_are_ones_a_browser_can_decode` uses Goertzel energies and was watched going red at 24 kHz
before being trusted at 48. ⚠️ Beside it, `a_packet_says_mono_twenty_milliseconds_on_its_face` reads the TOC byte by
hand (RFC 6716 §3.1) — the only check that is evidence about the *bitstream* rather than the library, since a
self-consistently wrong codec passes every round-trip.

⚠️ **No deflate, and the section above it in `src/web/mod.rs` argues the opposite at length.** That is the trap:
`/api/video` spends a page establishing that deflating the connection is worth 5×, so the next reader is primed to
apply it here, where it is wrong twice over — Opus is already range-coded, and `frame` must flush per message, which
puts a block boundary around every ~60-byte packet. `bench_audio_deflate_is_not_worth_a_byte` asserts the sign, so a
"consistency" fix fails. ⚠️ **Nor are 40 ms frames the fix for the wire overhead**: at this bitrate the encoder picks
CELT-only, whose frame sizes stop at 20 ms. `bench_audio_what_a_packet_costs_the_wire` prints the refusal rather than
extrapolating.

⚠️ **`set_output_sample_rate` and `set_emulation_speed` are derived state that every `load_state` drops.** There are
exactly two load sites on the emulator thread — `EmulatorHost::new` and `start_new_run` — plus `render.rs`'s `F9` in
the SDL UI, and `host::tune_audio` is the one function all of them call. A missed one does not announce itself: the
resampler falls back to 44.1 kHz while the stream's header keeps saying 48, so the game plays at the wrong pitch for
the rest of the run, and a missed *speed* makes a `target_speed` 40 host synthesise forty seconds of audio per second.
`the_sample_rate_and_the_speed_survive_every_state_that_is_loaded` asserts both directly rather than inferring them
from how much audio arrived, and was watched failing at `(44100, 1.0)`.

⚠️ **Nothing is encoded while nobody is listening** — `drain_audio` returns on `audio_listeners() == 0` without even
draining, and `BlipBuffer::end_frame` drops its own backlog, so that is what a headless run always did. ⚠️ **It reads
an edge, not a level**: the 0→1 transition throws away both the backlog (up to 100 ms) and the part-built frame, or
the first thing a new listener hears is a fragment of a moment that may be hours old.

⚠️ **Encoding is on the emulator thread and that was measured** — **0.032 ms per 20 ms frame, 0.16% of one core**,
against a `MAX_CATCHUP` of 250 ms. There is no argument for a thread and a channel four orders of magnitude under the
budget, and encoding once for everyone is only possible *because* it is there. ⚠️ But the call is wrapped in
`catch_unwind`, which is the whole reason a three-week-old codec with ~183 `unsafe` sites was acceptable: a panic
would unwind the emulator thread and take the run's checkpoint with it. On the first panic the encoder is dropped
outright — which is what makes the unwind safe, since no half-updated codec state is re-entered — a `Notice` is
published once, and the game plays on in silence. ⚠️ **A silenced encoder is never rebuilt**, or `restart` would
re-enter the same panic fifty times a second.

⚠️ **A `Lagged` subscriber is skipped and that is the whole handling.** A lagged *video* subscriber keeps a stale
screen for ever unless handed a keyframe; a lagged audio subscriber has missed sound that is already gone.
`AUDIO_CAPACITY` is 64 (~1.3 s) and deliberately not generous for the same reason: a bigger ring hands over a longer
burst of stale audio the client must throw away to reach live.

⚠️ **A park is silence for hours and the 2 s keep-alive is the entire connection** — an idle screen is still a screen,
but a parked run produces no packets at all, so it is the only thing between a listening page and `STALE_MS`. Equally,
⚠️ **a `MAX_CATCHUP` gap is a real gap and is left as one**: nothing inserts silence, and the client's underrun path
is the whole of the handling for both.

⚠️ **The header is ours and must never become an `OpusHead`.** The W3C WebCodecs Opus registration makes
`AudioDecoderConfig.description` optional and says that *supplying* one means the bitstream is Ogg-encapsulated — so
"we have a header, let us use the standard one" would configure the page's decoder into the wrong mode for the bare
packets it is about to receive. The twelve bytes carry `sampleRate` and `numberOfChannels` for `configure()` and are
never handed to the decoder.

### On the client

`web/src/stream.ts` holds the transport both streams share — the framing, the per-attempt `AbortController` chain and
the `STALE_MS` watchdog — extracted from `video.ts` rather than copied, because those ~90 lines are almost entirely
the subtle parts. ⚠️ **The identity-tap `ended` trick is deflate-specific**: the server never *finishes* the deflate
stream, so with `inflate: false` a clean close is simply `done` and `Disconnected` is raised directly. ⚠️ **A 503 or a
404 stops the retry loop**, and they are different answers — 503 is `GB_AUDIO_BITRATE=0` on a build that has audio,
404 is a build that does not.

⚠️ **The drift trim is not a refinement.** `web/src/audio.ts`'s `schedule` is a pure function on purpose, so the whole
algorithm can be read without an `AudioContext`. It trims `playbackRate` by at most ±0.5%, and it has to: the
wall/emulated ratio's 1.0007× ceiling separates the emulator's clock from the browser's by ~2.5 s an hour in a
perfectly healthy run, and a sound card is independently off by up to 0.1% again, so without the trim that drift
*alone* forces an audible cut every ten minutes. ±0.5% closes a 100 ms error in 20 s and is inaudible on chiptune.
The hard resync is kept for real discontinuities, and ⚠️ **the fade-out is armed in advance** on every scheduled
frame: by the time an underrun is detected the DAC has already run dry and the click cannot be undone. ⚠️ **Never
`cancelScheduledValues` without pinning the current value first** — it drops the gain to whatever was last set, which
is the click the fades exist to avoid. ⚠️ **Do not "handle" a park by suspending the `AudioContext`**: `suspend()`
freezes `currentTime`, which makes every deadline already stored wrong on resume, and the underrun path handles it
unchanged.

⚠️ **Every connection re-sends the header, so the client has to forget the format on each one.** A stream is twelve
bytes of `GBA1` followed by bare Opus packets, told apart by **position and nothing else**, so a reconnect that kept
the format it already had fed the header to the decoder as audio — and it is not rejected: `G` is `0x47`, which reads
as a TOC byte claiming a stereo stream and a frame-count code of 3. Reproduced in Chrome across a server bounce:
`EncodingError: Decoding error.` and **11 of 22 frames lost**, because the decoder dies at the header and takes the
rest of that connection's packets with it. `AudioPlayer.connectionChanged` clears `format` on `'live'`, which
`subscribeFramed` reports after the fetch succeeds and before the first message. ⚠️ **`nextAt` is deliberately *not*
cleared with it** — a reconnect is not automatically a discontinuity, a blip repaired inside the jitter buffer should
stay inaudible, and a gap long enough to matter drains `nextAt` past `UNDERRUN_S` and anchors on its own. ⚠️ **Nor is
the decoder rebuilt**, because its timestamp counter has to stay monotonic across the gap; only a format that has
genuinely moved under a deploy earns a new one, and `buildDecoder` closes the old one rather than dropping it.

⚠️ **`AudioContext.resume()` does not reject without user activation — it never settles at all**, so `await`ing it is a
hang rather than a failure, and that is reachable in ordinary use: the stored-preference path calls `AudioPlayer.start`
at mount, where there has been no gesture yet. Left unraced it hung for the life of the page, and — because
`player.current` had already been assigned — the viewer's *real* click then short-circuited on a player that was never
going to connect, leaving the speaker on `sound connecting…` for ever. `RESUME_GRACE_MS` (1 s, far more than a
permitted resume needs) races it; `SoundButton` assigns `player.current` only once the context is genuinely running,
with a `starting` latch so a second click cannot stack a second context. ⚠️ **A synthetic `.click()` does not confer
activation either**, so this path is what a test harness driving the page from JavaScript will always hit — that is how
it was found, and it is not a reason to think it is only a harness problem.

⚠️ **Never assume the decoder's output rate matches what was configured.** Chrome decodes Opus at 48 kHz whatever
`sampleRate` says, so `AudioData.sampleRate` is the authority for both the buffer and the frame's duration. ⚠️ **Every
packet is a `'key'` chunk** — Opus has no delta frames and WebCodecs rejects a stream whose first chunk is not one; the
timestamp is a local counter and is deliberately *not* used for scheduling, since there is no shared clock with the
server and the emulated one legitimately gaps.

⚠️ **A decode error rebuilds the decoder and keeps the connection, which is the opposite of what `Screen` does.** A
video decode error means the palette and the pixels are both suspect and only a fresh keyframe repairs them; the next
Opus packet repairs itself, so reconnecting would throw away the jitter buffer to fix nothing.

⚠️ **The speaker lives in the header beside the trophy, not on the screen**, borrowing `header .trophy`'s box exactly
and saying it is on with `.pill.live`'s accent border rather than by the glyph alone. On the screen it had to be dimmed
to avoid covering 160×144 of four-shade pixel art, and a control the viewer is going out of their way to look for
should not hide from them. ⚠️ **It stays in the header on a phone**, where the `width < 640px` block drops the context
gauge and the links: those are a desk's questions and sound is not, the same argument that keeps the trophy there.

