# Emulator performance

Read before optimising `src/{ppu,core,opcode,mmu}.rs` or `src/audio/`, and before building a rig to
measure them — one was built and deleted, and §1 is why.

This is an **evidence** doc, like [deployed-run-defects](deployed-run-defects.md), not a rule index.
Everything below was measured on 2026-09-07, on a Ryzen 9 7900X, `--release` (thin LTO,
`codegen-units = 1`). Absolute numbers will move with the machine; the **ratios** are the point.

## 1. The workload is a game standing still, and that is deliberate

`game_boy::tests::bench_core_throughput` is the yardstick. It restores `at-celadon.bin`, presses
nothing, and runs a fixed number of frames:

```bash
cargo test --release --features bench --bin gb -- bench_core_throughput --nocapture
BENCH_FRAMES=60000 BENCH_ONLY=pokemon …          # long enough for perf to sample
```

```
workload                     realtime       t-cycles/s     frames
pokemon-red (fixture)           89.0x        373204810        600
cpu_instrs.gb                   53.5x        224202759        600     # never HALTs
dmg-acid2.gb                   189.1x        793088768        600     # HALTs heavily
```

⭐ **Pressing no buttons is not a weakness of that benchmark, and this was checked rather than
assumed.** A scripted joypad timeline was built for exactly this doubt — "at absolute emulated time
T, hold `Left` for 7.6 s" replayed against a bare `GameBoy`, with no agent anywhere in the loop —
driving two circuits: a 27-tile walking lap along `SilphCo3F`'s north corridor, and a loop in and
out of the Celadon Pokémon Centre's door. Three workloads, the same emulated game time each:

| | realtime | vs. its own control |
|---|---|---|
| idle (`SilphCo3F`, nothing held) | 90.6x | — |
| walking (56 tiles a lap) | 88.6x | 1.022x the wall clock |
| standing (`at-celadon`, nothing held) | 88.7x | — |
| doorway (two map loads a lap) | 93.0x | **0.954x** the wall clock |

The `perf` profiles came out the same three ways — within a couple of points on every source file.
So walking costs 2%, and **a map change makes the emulator faster**: a probe of the LCDC enable bit
put the screen *off* for 5.21% of a doorway lap and 0.00% of the other two, and a blanked screen is a
PPU that renders nothing. The rig had answered its question, so it was deleted rather than left to
rot. ⚠️ **Do not rebuild it to re-ask this.** If you need it for something else, it was one file
(`src/pokemon/input_bench.rs`, deleted 2026-09-07) and the traps it hit are worth knowing: a first
draft walked into a Rocket's line of sight three seconds in, and every leg of the working lap had to
end against a **wall** so the game itself re-synchronised the player's position each time.

## 2. How to profile it

- ⚠️ **Build into a separate `CARGO_TARGET_DIR`.** Profiling needs different `RUSTFLAGS`, and
  changing them in the working tree's `target/` buys a full two-minute rebuild on the next ordinary
  `cargo test`.
  ```bash
  CARGO_TARGET_DIR=/tmp/prof CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C force-frame-pointers=yes" \
    cargo test --release --features bench --bin gb --no-run
  BENCH_FRAMES=60000 BENCH_ONLY=pokemon perf record -F 999 -g --call-graph fp -- \
    /tmp/prof/release/deps/gb-<hash> --exact --nocapture game_boy::tests::bench_core_throughput
  ```
- ⚠️ **`perf report` never finishes on this binary with inline resolution on** — over two minutes on
  5 k samples, killed repeatedly. Always `--no-inline`, and prefer `-g none`. `--sort srcfile` is the
  most useful view; `--sort srcline` is unusably slow, so use `perf annotate -l -s <symbol>` for
  line detail.
- `perf_event_paranoid` is 2 on this machine, so only user-space (`:u`) counters work. That is
  enough for everything here.

## 3. Where the time goes

By source file (`--sort srcfile`, identical for idle, walking and doorway to within ~2 points):

| group | share | files |
|---|---|---|
| **PPU** | ~41% | `ppu.rs` 37.1, `lcd_control` 2.3, `lcd_palette` 0.7, `lcd_dma` 0.8 |
| **CPU** | ~18% | `core.rs` 10.1, `opcode.rs` 7.1, `interrupt.rs` 1.2 |
| **APU** | ~13% *(but see below)* | `square_channel` 4.5, `wave_channel` 2.5, `volume` 2.1, `noise_channel` 1.7, `blip/buffer` 1.2 |
| **MMU / scheduling** | ~13% | `mmu.rs` 7.6, `game_boy` 1.6, `timer` 1.1, `divider` 1.1, `serial` 0.5, `schedule` 0.5 |

By symbol: `PPU::draw_pixels_to` 36.0% self, `MMU::update` 19.4% self / 64.9% inclusive,
`PPU::update` 8.7%, `GameBoy::run` 6.3%, `Core::fetch` 5.8%, `SquareWaveChannel::update` 4.7%,
`Core::execute` 4.2%, `OpCode::machine_cycles` 4.1%, `Core::fetch_u8` 3.9%.

Inside `draw_pixels_to`: `ppu.rs:885` (the sprite-priority chain, with `top_sprite` inlined) 18.6%
of the function, `TileRow::pixel`'s bit extraction 13.1%, the framebuffer store 5.7%, the tile-cache
key compare 5.6%, `TileMapMode::base_address` 5.4%, `fetch_tile_row`'s address arithmetic 4.0%.

`perf stat` says what kind of problem this is: **IPC 3.77**, branch misses **1.12%**, cache misses
negligible (1.2 M in 5.7 s — a Game Boy's whole working set is in L1/L2). It is neither
memory-bound nor branch-bound; it runs **~53 host instructions per emulated t-cycle**, and the only
way down is to execute fewer of them.

## 4. Ceilings, measured by ablation

⚠️ **The file-level profile badly understates audio.** `Audio::update` is inlined into `MMU::update`,
so its cost lands in `mmu.rs`, `mod.rs` and `uint_macros.rs` — 13% by attribution, **30% by
ablation**. Trust the ablation.

The method: `rsync` the tree to a scratch directory (`--exclude target --exclude .git`), disable one
thing with a hard-coded `if true { return; }`, rebuild into its own target dir, and re-measure best
of three. Baseline for this table was the walking circuit at **88.5x**; against
`bench_core_throughput`'s 89.0x the ratios carry over unchanged.

| ablation | result | so that thing is | legal? |
|---|---|---|---|
| `draw_pixels_to` returns immediately | **137.2x** | pixel rendering = **35%** of wall clock | no — bounds only |
| `Audio::update` returns immediately | **126.5x** | the whole APU = **30%** | no — bounds only |
| APU channels run, mixing/blip/`end_frame` skipped | **97.3x** | the audio **output side alone = 9%** | ⭐ **done — see §6** |
| render once per scanline instead of per instruction | **93.8x** | `draw_pixels_to`'s per-call setup = **5.6%** | nearly — needs write-triggered catch-up |
| hoist the tile-map base out of the per-pixel path | 89.2x | +0.8%, at the noise floor | yes |
| expand `TileRow` to eight resolved colours per tile | 86.4x | ⚠️ a **2.4% regression** | yes, and don't |

⚠️ **Two negative results, recorded so nobody spends an afternoon rediscovering them.** Hoisting
`TileMapMode::base_address()` out of `tile_map_entry` — which `perf` blames for 5.4% of the pixel
loop — buys 0.8%, because the compiler was already doing most of it. And resolving all eight of a
tile row's pixels at fetch time, the obvious answer to `TileRow::pixel` being 13% of the function,
is **slower**: it inflates `TileRow` enough that the per-pixel cache-key compare and the struct copy
cost more than the bit-twiddling saved.

A third ablation could not be run at all: removing `MMU::update`'s five-way interrupt poll (6.5% of
that function) stops the game working, so its value is still unknown.

## 5. What to do about it, ranked

1. ~~⭐ **Gate the APU's output side when nobody is listening.**~~ **Done, 2026-09-08 — §6.**
   Predicted +10%, delivered +10.2%.
2. **Deadline-drive the APU's channel updates.** The other 21 points of the APU's 30%, so the
   ceiling is large. The four channels are advanced every instruction to move phase timers that fire
   far less often, and `Audio::next_deadline` — the bound C2's HALT skip already respects — is
   exactly the "how long can this be left alone" answer needed. High difficulty and high risk:
   the frame sequencer, length counters, envelopes and sweep all have to land on the same cycle they
   do now, and the blip clock is unforgiving.
3. **Render a scanline in one pass, catching up only when a write demands it.** +5.6%. Medium-high
   difficulty for a modest return: correctness needs a catch-up on every write to VRAM, OAM, LCDC,
   SCX/SCY, WX/WY and the palettes, and `dmg-acid2`/`cgb-acid2` plus
   `the_halt_fast_path_matches_stepping_cycle_by_cycle` are what would have to hold.
4. **A base-cycle table for `OpCode::machine_cycles`.** 4.1% self, called once per instruction (twice
   for a taken branch) as a large match over a rich enum, where a `[u8; 256]` indexed by the raw
   opcode byte would do. Untested; the plumbing is that `OpCode` does not currently keep its byte.
5. **A combined "any interrupt pending" mask**, to replace the five-way poll every instruction.
   Unmeasured, plausibly 1-2%.

⚠️ **The pixel loop has no cheap wins left** — it is 35% of the run and the two obvious ideas came
out at +0.8% and −2.4%. It has already been hoisted hard (see the comment above the loop in
`ppu.rs`). Anything further is a restructuring — span-at-a-time rendering, sprite-free fast paths —
and deserves an ablation measuring its ceiling *before* anyone starts writing it.

## 6. What has been done

### The APU's output side is gated when nobody is listening (2026-09-08, +10.2%)

Ranked #1 above, and it measured where the ablation said it would:

| `bench_core_throughput`, pokemon fixture | realtime |
|---|---|
| output side running (a listener is attached) | 88.7–89.0x |
| output side gated (nobody is) | **97.3–98.0x** |

Best of three each way, samples drained in the loop so the listening case pays what a listener
really costs it. The ablation in §4 predicted 97.3x, which is the low end of what the real thing
does — the flag is a `bool` in a register where the ablation was a `return`.

`Audio::set_output_enabled` stops the mixer, `BlipStereo::update` and `end_frame`. It stops nothing
else: the frame sequencer and all four channels keep clocking, because their registers are
CPU-visible through NR52 and the wave RAM, and `Audio::next_event` is the bound C2's HALT skip
respects. That "the machine cannot tell" is not an argument, it is a test —
`game_boy::tests::silencing_the_output_side_is_invisible_to_the_game` runs two machines 120 frames
side by side, gated and not, and compares state *and* framebuffer every frame, with both halves of
the vacuity check (the open one produced samples, the gated one produced none).

Two callers shut it:

- `host.rs`'s `drain_audio`, on the same signal that already decided not to encode. ⚠️ **Set every
  tick, not on the edge**, because `MMU::reset` replaces the whole `Audio` and a `load_state` does
  not carry derived state — a gate set once would silently reopen. It is a `bool` compare when
  nothing has moved.
- `TestFixture::with_policy` and the soak tier's own fixture, so **every agent test tier** takes
  the same mechanism: nothing in either harness has ever listened. ⚠️ **Not A/B'd per tier** — two
  attempts at a `full_playthrough` baseline were killed by memory pressure on this machine, so the
  +10.2% above is the bench's number and the only claim about the tiers is that they run the same
  gate. What is known is that `full_playthrough` **passes with it, in 256.65 s**.

⚠️ **Re-opening is a resync, not a resume**, and that is what `set_output_enabled(true)` pays for:
the resampler's 16.16 clock stops while the gate is shut, so whatever is still in the buffer belongs
to a moment that may be hours old and the synth's last amplitude is a level the machine has long
since left. Both are dropped, and `mix_dirty` makes the next update re-report from scratch. That is
the same 0→1 policy `drain_audio` already applied to the Opus encoder, which is why the two sit next
to each other.

⚠️ **It was safe to point at `full_playthrough` only because the machine is bit-identical.** That
test is a golden RNG replay: a change that moved the emulator by one cycle would fail it hundreds of
steps from wherever the change was. It passes unchanged, which is the strongest single statement
here: eight badges of scripted play, cycle-for-cycle the same game with the APU's output side off.
