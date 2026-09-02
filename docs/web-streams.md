# The video and audio streams

Read before touching `src/web/{video,audio}*` or `web/src/{stream,video,audio}.ts`. Every kbit/s
figure in the README comes from `src/web/video/bench.rs` and `src/web/audio/bench.rs`
(`--features bench`) and nowhere else. Quote the number for a moving screen; an idle one costs
almost nothing and was quoted as the stream's cost for a long time.

## Video (`/api/video`)

- The deflate compressor is per connection, which is why `VideoMessage` carries plain bytes and
  the stream cannot be compressed once in `Published` for everyone.
- `VideoStream::frame` flushes after every message. Without it the encoder holds a livestream back
  into bursts. `the_video_stream_is_one_deflate_stream_of_length_prefixed_messages` inflates
  incrementally for exactly that reason.
- The deflate is the `Content-Type`, not a `Content-Encoding`: a declared encoding invites a proxy
  to re-encode and buffer. The client inflates with `DecompressionStream('deflate')`.
- Never base64 what you will compress: 33% before compression, 69–113% after, because it shifts a
  repeating pattern into three phases and LZ77 stops matching. That is what took SSE off the table,
  and the same mechanism is why an unaligned bit width is not a saving; widths are 1, 2, 4 or 8.
- `gb serve` runs a DMG unless `GB_HARDWARE=cgb`, so the screen is four shades. A CGB is six and
  measured 1.63× on the wire.
- Four codec invariants that fail silently: a keyframe replaces the decoder's palette with the
  encoder's whole palette, or late joiners desync for ever; the keyframe is stored before the delta
  is broadcast and kept current (`late_joiner_never_misses_a_delta`); `new_palette_len` is a `u8`,
  so the palette caps at 255; the encoder tracks what the decoder holds and decides "changed" after
  interning, or the lossy path emits a keyframe every tick.
- No keyframe goes on the wire in steady state, so a keyframe interval is not a lever.

## Audio (`/api/audio`)

- 48 kHz, not 24. `opus-rs` at 24 kHz keeps the loudness and destroys the spectrum, through its own
  decoder and real libopus alike; at 48 it round-trips within 0.3 dB. The guard
  `the_packets_are_ones_a_browser_can_decode` is spectral (Goertzel), never a waveform SNR, because
  CELT does not preserve phase and a waveform comparison reports failure on correct output.
  `a_packet_says_mono_twenty_milliseconds_on_its_face` reads the TOC byte by hand, the one check
  that is about the bitstream rather than the library.
- No deflate, despite the page above it in `src/web/mod.rs` arguing for it on video: Opus is
  already range-coded and a per-message flush puts a block boundary round every ~60-byte packet.
  `bench_audio_deflate_is_not_worth_a_byte` asserts the sign. 40 ms frames are not the fix either:
  CELT-only, which the encoder picks at this bitrate, stops at 20 ms.
- `set_output_sample_rate` and `set_emulation_speed` are derived state every `load_state` drops.
  `host::tune_audio` is called from both load sites on the emulator thread (`EmulatorHost::new`
  and `start_new_run`); the SDL UI's F9 handler in `src/sdl/render.rs` applies its own. Miss one
  and the resampler falls back to 44.1 kHz under a header saying 48.
  `the_sample_rate_and_the_speed_survive_every_state_that_is_loaded`.
- Nothing is encoded while nobody listens: `drain_audio` returns on `audio_listeners() == 0`, and
  it reads an edge, throwing away the backlog and the part-built frame on 0→1 so a new listener
  does not hear a fragment that may be hours old.
- Encoding is on the emulator thread (~0.03 ms per 20 ms frame) and wrapped in `catch_unwind`. On
  the first panic the encoder is dropped, a `Notice` is published once, and it is never rebuilt, or
  `restart` would re-enter the same panic fifty times a second.
- A `Lagged` subscriber is skipped; that is the whole handling. `AUDIO_CAPACITY` (64, ~1.3 s) is
  deliberately small. A `MAX_CATCHUP` gap is left as a gap for the client's underrun path. On a
  park the 2 s keepalive is the entire connection.
- The 12-byte `GBA1` header is ours and must never become an `OpusHead`: WebCodecs treats a
  supplied `description` as Ogg encapsulation. It carries `sampleRate` and `numberOfChannels` for
  `configure()` and is never handed to the decoder.

## The client

- `web/src/stream.ts` is the transport both streams share: the framing, one `AbortController` per
  attempt chained to the caller's, and the `STALE_MS` watchdog (`api.ts`, 4× the server's 2 s
  keepalive). The identity-tap `ended` trick is deflate-specific. A 503 (`GB_AUDIO_BITRATE=0`) or a
  404 (a build without audio) stops the retry loop.
- `audio.ts`'s `schedule` is a pure function. It trims `playbackRate` by at most ±0.5%, and it has
  to: the emulator's clock and the browser's drift about 2.5 s an hour in a healthy run, which
  without the trim forces an audible cut every ten minutes. Real discontinuities get one fade, and
  the fade-out is armed in advance on every scheduled frame because an underrun is detected after
  the DAC has run dry. Never `cancelScheduledValues` without pinning the current value first. Do not
  `suspend()` the `AudioContext` on a park: it freezes `currentTime` and invalidates every stored
  deadline.
- Every connection re-sends the header, so `AudioPlayer.connectionChanged` clears `format` on
  `'live'`: fed to the decoder, `G` (0x47) reads as a stereo TOC byte and kills the rest of that
  connection's packets. `nextAt` is deliberately kept (a reconnect is not a discontinuity) and the
  decoder is not rebuilt (its timestamp counter must stay monotonic); only a changed format earns a
  new decoder, and `buildDecoder` closes the old one.
- `AudioContext.resume()` without user activation never settles, so it is raced against
  `RESUME_GRACE_MS`, and `SoundButton` assigns `player.current` only once the context is running,
  with a `starting` latch. A synthetic `.click()` confers no activation, so a harness always hits
  this path.
- `AudioData.sampleRate` is the authority for the buffer and the frame duration: Chrome decodes at
  48 kHz whatever was configured. Every packet is a `'key'` chunk. A decode error rebuilds the
  decoder and keeps the connection, the opposite of `Screen`, where an error means the palette is
  suspect and only a fresh keyframe repairs it.
