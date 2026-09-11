# The video and audio streams

Read before touching `poke-agent-web/src/web/{video,audio}*` or the SPA's `stream.ts`, `video.ts`
and `audio.ts`. Every kbit/s figure comes from the two bench files and nowhere else, and the figure
to quote is for a moving screen: an idle one costs almost nothing.

## Video

- The deflate compressor is per connection, which is why a video message carries plain bytes and the
  stream cannot be compressed once for everyone. The stream flushes after every message, or the
  encoder holds a livestream back into bursts.
- The deflate is the `Content-Type`, not a `Content-Encoding`: a declared encoding invites a proxy to
  re-encode and buffer.
- Never base64 what you will compress: 33% before compression, 69-113% after, because it shifts a
  repeating pattern into three phases and LZ77 stops matching. That is what took SSE off the table,
  and the same mechanism is why an unaligned bit width is not a saving.
- Four codec invariants that fail silently: a keyframe replaces the decoder's palette with the
  encoder's whole palette, or late joiners desync for ever; the keyframe is stored before the delta
  is broadcast; the palette length is a `u8`, so it caps at 255; and the encoder tracks what the
  decoder holds and decides "changed" after interning, or the lossy path emits a keyframe every tick.
- No keyframe goes on the wire in steady state, so a keyframe interval is not a lever. A real video
  codec was measured rather than assumed away: x264 is 45 kbit/s lossless and 25 at a quality that
  mangles four-shade pixel art, against 21. The block diff itself beats sending the whole screen
  every frame by only about 2x — most of what it earns, the deflate window would have earned anyway.

## Audio

- 48 kHz, not 24. `opus-rs` at 24 kHz keeps the loudness and destroys the spectrum, through its own
  decoder and real libopus alike; at 48 it round-trips within 0.3 dB. The guard is spectral, never a
  waveform SNR, because CELT does not preserve phase and a waveform comparison reports failure on
  correct output. One check reads the TOC byte by hand, the only one about the bitstream rather than
  the library.
- No deflate, despite the argument for it on video: Opus is already range-coded and a per-message
  flush puts a block boundary round every ~60-byte packet. Measured at +16.6%. Longer frames are not
  the fix either — CELT-only stops at 20 ms — so the transport overhead around a packet is a floor.
- The output sample rate and the emulation speed are derived state every `load_state` drops, and
  both load sites plus the SDL UI's reload handler have to re-apply them. Miss one and the resampler
  falls back to 44.1 kHz under a header saying 48.
- Nothing is encoded while nobody listens, and the listener count is read as an edge: the backlog and
  the part-built frame are thrown away on 0→1 so a new listener does not hear a fragment hours old.
- Encoding is on the emulator thread and wrapped in `catch_unwind`. On the first panic the encoder is
  dropped and never rebuilt, or a restart re-enters the same panic fifty times a second.
- The 12-byte header is ours and must never become an `OpusHead`: WebCodecs treats a supplied
  description as Ogg encapsulation.

## The client

- `audio.ts`'s scheduler is a pure function. It trims playback rate by at most ±0.5%, and it has to:
  the two clocks drift a couple of seconds an hour in a healthy run, which without the trim forces an
  audible cut every ten minutes. Real discontinuities get one fade, armed in advance on every
  scheduled frame because an underrun is detected after the DAC has run dry. Never suspend the
  `AudioContext` on a park: it freezes `currentTime` and invalidates every stored deadline.
- Every connection re-sends the header, so the player clears its format on a fresh connection — fed
  to the decoder, `G` reads as a stereo TOC byte. The next timestamp is kept (a reconnect is not a
  discontinuity) and the decoder is not rebuilt, because its counter must stay monotonic.
- `AudioData.sampleRate` is the authority: Chrome decodes at 48 kHz whatever was configured. A decode
  error rebuilds the decoder and keeps the connection — the opposite of the video path, where an
  error means the palette is suspect and only a fresh keyframe repairs it.
