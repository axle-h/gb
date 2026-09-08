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

⚠️ **`BENCH_AUDIO=off` is the number that matters now, and it is a different number.** It gates the
APU's output side, which is the state every agent test tier and the deployment actually run in —
nothing has ever listened in either — and two of the three optimisations in §6 only exist there. The
table above is the *open* configuration, which is what a listener at the desktop or on the page gets.
Score work on the channels against the gated baseline and work on the mixer against the open one, and
say which you used; §6's tables do.

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

### 4.1 The same thing again, with the output side gated (2026-09-08)

Once §6.1 landed, the baseline moved and the shares moved with it. These are against
`BENCH_AUDIO=off` on the pokemon fixture, best of three, baseline **98.2x**:

| ablation | result | so that thing is |
|---|---|---|
| the four channels never advance | **122.8x** | the channel side = **20%** of the gated wall clock |
| the channels advance to a deadline, no correctness plumbing | **110.0x** | ⭐ what §6.2 was worth going after |

⚠️ **A third negative result, and the sharpest one here: flushing defensively in the powered-off
branch of `Audio::update` costs 1.5%.** That branch cannot carry a batch — `Audio::reset` clears it
outright and nothing else can turn the APU off — but even a *guarded* `if pending > 0 { sync() }`,
on a branch the pokemon fixture essentially never takes, measured **105.4–106.6x against
107.1–108.2x**. The cost is not the compare, it is the layout: this whole function inlines into
`MMU::update`. The invariant is enforced where the state is created and carries a `debug_assert`
where it is consumed, which is worth knowing before someone tidies that into a real check.

A fourth, taken after the fact and worth the same one line the other negative results get: `perf`
blames `flatten.rs` for **1.3%** of the gated run, all of it `soonest_channel_event`'s
`[..].into_iter().flatten().min()` once per flush. Folding the four `Option`s by hand measured
**108.0–108.7x against 107.7–108.5x**, which is no change: the compiler was already doing it and the
samples are attribution rather than work.

⚠️ **The crude second row is 110.0x and the shipped thing is 108.5x, and the gap is not overhead —
it is the correctness.** The ablation carried a batch straight into the frame sequencer's tick;
paying it off first is an extra `advance_channels` at 512 Hz and it is what makes the mechanism
exact. See the comment in `Audio::update`, and §6.2's noise-channel bug for what the shortcut cost.

## 5. What to do about it, ranked

1. ~~⭐ **Gate the APU's output side when nobody is listening.**~~ **Done, 2026-09-08 — §6.1.**
   Predicted +10%, delivered +10.2%.
2. ~~**Deadline-drive the APU's channel updates.**~~ **Done, 2026-09-08 — §6.2.** Predicted 12%,
   delivered **+10.4%** gated and **+18.2%** on `cpu_instrs`.
3. ⭐ **Let a listener have the batch too.** §6.2 only batches while the output side is gated,
   because with a listener attached the resampler has to be told *when* a level moved and not merely
   that it did. The crude ablation that ignored this measured the open configuration at **104x**
   against 89, so there is **~17%** here for the desktop UI and for anyone who presses the speaker.
   The shape is known and it is small: a level can only move at a flush, so between flushes the
   mixer is already doing nothing but `end_frame`, and `end_frame(a); end_frame(b)` is
   `end_frame(a+b)` — split the flush into `end_frame(pending - delta)`, `update(mixed)`,
   `end_frame(delta)` and every transition lands on the cycle it lands on today. What makes it work
   worth doing carefully rather than quickly is that **nothing here fails loudly**: the machine stays
   bit-identical either way, so `full_playthrough` and the blargg suites would all pass a version
   that had merely made the music slightly wrong. It needs a test that compares *samples*, and
   `src/audio/blip/tests.rs`'s spectral check is the closest thing to a template.
4. **Render a scanline in one pass, catching up only when a write demands it.** +5.6%. Medium-high
   difficulty for a modest return: correctness needs a catch-up on every write to VRAM, OAM, LCDC,
   SCX/SCY, WX/WY and the palettes, and `dmg-acid2`/`cgb-acid2` plus
   `the_halt_fast_path_matches_stepping_cycle_by_cycle` are what would have to hold.
5. **A base-cycle table for `OpCode::machine_cycles`.** 4.1% self, called once per instruction (twice
   for a taken branch) as a large match over a rich enum, where a `[u8; 256]` indexed by the raw
   opcode byte would do. Untested; the plumbing is that `OpCode` does not currently keep its byte.
6. **A combined "any interrupt pending" mask**, to replace the five-way poll every instruction.
   Unmeasured, plausibly 1-2%.

⚠️ **The pixel loop has no cheap wins left** — it is 35% of the run and the two obvious ideas came
out at +0.8% and −2.4%. It has already been hoisted hard (see the comment above the loop in
`ppu.rs`). Anything further is a restructuring — span-at-a-time rendering, sprite-free fast paths —
and deserves an ablation measuring its ceiling *before* anyone starts writing it.

## 6. What has been done

### 6.1 The APU's output side is gated when nobody is listening (2026-09-08, +10.2%)

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

### 6.2 The four channels are advanced to a deadline rather than per instruction (2026-09-08, +10.4%)

Ranked #2 above, and §4.1 is the ablation that sized it: with the output side already gated the four
channels were **20%** of the emulator, and almost all of it was four counters being decremented past
a moment that had not arrived. At the periods a game actually plays, the soonest of the four phase
timers is tens of M-cycles out and an instruction is two or three.

`Audio::next_event` — the bound C2's HALT skip has always respected — is exactly the "how long can
this be left alone" answer, so this is that same skip applied to the CPU's *running* cycles as well
as its idle ones. `Audio::update` accumulates `pending` and returns; when it reaches
`channel_deadline` the whole batch is handed to the channels in one call.

| `bench_core_throughput`, `BENCH_AUDIO=off` | before | after | |
|---|---|---|---|
| pokemon-red (fixture) | 96.6–98.1x | **107.6–108.5x** | **+10.4%** |
| cpu_instrs.gb | 58.5–59.3x | **69.2–69.6x** | **+18.2%** |
| dmg-acid2.gb | 193.4–194.7x | 194.0–196.2x | +0.6% |

⚠️ **Those are four rounds of the two binaries run *alternately*, not two runs an hour apart.** This
machine drifted 1–2% over the afternoon that produced them, which is the same size as the third
result below — long enough to invent a regression that was not there, and it did, twice. Keep the
old binary in its own `CARGO_TARGET_DIR` and interleave.

⭐ **The spread across the three is the mechanism explaining itself.** `cpu_instrs` never HALTs, so
before this it had no deadline-driving anywhere and gains the most. Pokémon HALTs for 65% of its
cycles, which were already skipped, so it gains on the other 35%. `dmg-acid2` powers the APU on and
never plays a note, so there is nothing to batch and nothing to gain — and, importantly, nothing lost
either. The open configuration (a listener attached) measured 90.0–91.8x against a baseline of
89.1–90.1x: unchanged, which is what it is meant to be.

**Four things had to be got right, and one of them was got wrong first.** All four are the same
question — *what can move a channel between now and the deadline?* — and the answer is: a frame
sequencer event, a register write, and nothing else.

- ⚠️ **Nothing may advance a phase inside a batch**, and that is what makes the mechanism exact
  rather than approximate. `channel_deadline` is the *minimum* over the four channels, so the flush
  finds every one of them still short of its next tick and the closed forms in `PhaseTimer::update`
  and `NoiseChannel::update` take their one-step path exactly as they did per instruction.
- ⚠️ **The batch is paid off *before* a frame-sequencer event, not with it.** This is the one that
  was wrong first. A channel applies its length counter, envelope and sweep at the *start* of the
  window it is handed and only then advances, so carrying the batch into the event's call moves
  cycles that belong before the tick to after it — and when the length counter deactivates the
  channel, `update` returns early and the whole batch is **dropped on the floor**. The symptom was
  an idle noise channel frozen 60 cycles later than it should have been, three thousand frames in.
- ⚠️ **A register write flushes first.** It is the one thing that can move a channel without the
  frame sequencer. `MMU::update` runs *after* the instruction's bus access, so everything
  outstanding at a write belongs to strictly earlier instructions and paying it off is exactly
  right. It also puts `access_offset` back on the instruction boundary that `WaveChannel::trigger`'s
  retrigger quirk measures from.
- ⚠️ **The DMG wave-RAM read aperture is compensated arithmetically, not by flushing**, because
  `Audio::read` takes `&self`. `access_offset` adds the outstanding batch; `fetch_at` subtracts a
  counter that is stale by the same amount, so the two cancel exactly.

Three more places had to learn to see past a batch, none of them hot: `Audio::next_event` nets it
off (**and must still answer `None` when nothing is clocking** — a stand-in `u64::MAX` wraps
`MMU::schedule`'s sum round to a deadline in the past and the HALT skip silently stops skipping),
and the `apu` save-state section and `PartialEq` both go through `Audio::settled`, which advances
*clones*. A save state carrying channels a few dozen cycles behind the CPU would restore a machine
that is subtly not the one that was saved, and `the_halt_fast_path_matches_stepping_cycle_by_cycle`
builds two machines batching differently on purpose.

⚠️ **Every APU test in this repo ran the un-batched path, so the accuracy suite proved nothing about
this.** blargg's `dmg_sound` tests run with the output side open, which is where batching is
switched off. `game_boy::tests::deadline_driving_the_channels_is_invisible_to_the_game` is the fix:
the whole twelve-sub-test suite run **gated**, against the same reference frame, with a second
machine beside it comparing APUs every slice — the screen answers "does the game still pass" and the
comparison answers "was it the same machine that passed". Registers, length counters, triggers,
sweep, and all three wave-RAM aperture tests are in there. It asserts it actually batched, and it
asserts the control machine never did. All three of the mechanisms above were mutation-tested
against it and all three failures were caught.

`full_playthrough` passes unchanged, which carries the same weight it carried in §6.1: it is a
golden RNG replay, and a change that moved the emulator by one cycle would fail it hundreds of steps
from wherever the change was. It also corroborates the bench from the other end — **236.65 s against
§6.1's 256.65 s**, which is the +8% a whole scripted playthrough sees, and the agent tiers run gated
for the same reason the deployment does. The default tier is 28.5 s and the slow tier 85 s, both
green.
