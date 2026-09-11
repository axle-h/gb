# Emulator performance

Read before optimising `gb/src/{ppu,core,opcode,mmu}.rs` or `gb/src/audio/`, and before building a
rig to measure them — one was built and deleted, because the yardstick is a game *standing still*
and that profiles the same as one walking to within two points.

`game_boy::tests::bench_core_throughput` is the yardstick. Absolute numbers move with the machine;
the ratios are the point. Thin LTO and `codegen-units = 1` are the only build knobs that make
everything faster at once, and `-C target-cpu=native` on top of them measured *slower*.

## How to profile it

- Build into a separate `CARGO_TARGET_DIR`: profiling needs different `RUSTFLAGS`, and changing them
  in the working tree buys a full rebuild on the next ordinary test run.
  ```bash
  CARGO_TARGET_DIR=/tmp/prof CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C force-frame-pointers=yes" \
    cargo test --release -p gb --features slow-tests --no-run
  BENCH_FRAMES=60000 BENCH_ONLY=pokemon perf record -F 999 -g --call-graph fp -- \
    /tmp/prof/release/deps/gb-<hash> --exact --nocapture bench_core_throughput
  ```
- `perf report` never finishes on this binary with inline resolution on. Always `--no-inline`, prefer
  `-g none`, and use `--sort srcfile`; `--sort srcline` is unusably slow, so reach for
  `perf annotate -l -s <symbol>` instead.

## Where the time goes

Roughly: PPU 41%, CPU 18%, MMU and scheduling 13%, APU 13% *by attribution*. `perf stat` says what
kind of problem this is — IPC 3.77, branch misses 1.12%, cache misses negligible, since a Game Boy's
whole working set fits in L1/L2. It is neither memory- nor branch-bound: it runs about 53 host
instructions per emulated t-cycle, and the only way down is to execute fewer of them.

The file-level profile badly understates audio, because `Audio::update` inlines into `MMU::update`.
Ablation says 30%, not 13%. Trust the ablation: copy the tree to a scratch directory, disable one
thing with a hard-coded early return, rebuild into its own target dir, and take the best of three.

## Ceilings, and what has been tried

| ablation | share of wall clock | outcome |
|---|---|---|
| pixel rendering | 35% | bound only |
| the whole APU | 30% | bound only |
| the audio output side alone | 9% | **done** — gated when nobody is listening, +10.2% |
| the four channels advancing | 20% of the gated run | **done** — deadline-driven, +10.4% |
| letting a listener have the batch too | | **done**, +13.8% |
| render per scanline, not per instruction | 5.6% | not done: needs a catch-up on every write to VRAM, OAM, LCDC, the scroll registers and the palettes |
| a base-cycle table for `OpCode::machine_cycles` | 4.1% self | not done, untested; `OpCode` does not keep its byte |
| a combined "any interrupt pending" mask | ~1-2%, unmeasured | the ablation cannot be run — removing the poll stops the game |

Five things measured worse or made no difference, recorded so nobody spends an afternoon
rediscovering them: hoisting the tile-map base out of the per-pixel path (+0.8%, the compiler was
already doing it); resolving all eight of a tile row's pixels at fetch time (**−2.4%** — it inflates
the row enough that the cache-key compare costs more than the bit-twiddling saved); flushing
defensively in the powered-off branch of `Audio::update` (**−1.5%**, and the cost is the layout
rather than the compare, because the function inlines into `MMU::update`); folding
`soonest_channel_event`'s four `Option`s by hand (no change — the samples were attribution, not
work); and `-C target-cpu=native`.

The pixel loop has no cheap wins left: it is 35% of the run, it has already been hoisted hard, and
the two obvious ideas came out at +0.8% and −2.4%. Anything further is a restructuring, and deserves
an ablation measuring its ceiling *before* anyone starts writing it.
