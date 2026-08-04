# gb vs gambatte — compatibility & accuracy guides

A comparison of **`gb`** (`/home/alex/projects/gb`, Rust) against **gambatte**
(`/home/alex/projects/gambatte`, C++), written as an implementation backlog for future agents.

Produced 2026-08-04 by seven parallel research agents reading both codebases, plus benchmarks of
both cores on the same machine. The gambatte tree was treated as **read-only** throughout and was
never modified. No emulator code was changed while writing these — they are pure research plus a
derived implementation plan.

> **Reference tree.** These documents cite `/home/alex/projects/gambatte` throughout — an upstream
> sinamas gambatte checkout (git `efa674a9`), **not** gambatte-speedrun or the libretro core. It is
> not vendored here and is not a submodule. If it is absent, `file:line` citations to it cannot be
> re-verified; see [`06-features-and-robustness.md` §0](06-features-and-robustness.md#0-which-gambatte-is-this--read-first)
> for exactly which variant it is and what it does *not* contain.

---

## ▶ Doing the work? Start here

**[`10-implementation-plan.md`](10-implementation-plan.md)** is the executable, phased plan —
Stabilise → CGB → Performance → Missing hardware. It carries the **agent protocol**, a status board,
and an append-only ledger for handing off between sessions. If you are implementing rather than
researching, read that document and ignore the rest of this file except as reference.

The guides below are the **research** those phases were derived from.

## The guides

| Doc | Covers | Headline |
|---|---|---|
| [`01-architecture.md`](01-architecture.md) | Scheduling model, memory hot path, performance, save states, public API | **gambatte is ~18× faster.** 81% of `gb`'s CPU dispatches are HALT, each paying a full peripheral update |
| [`02-cpu.md`](02-cpu.md) | Timing granularity, opcodes, interrupts, DIV/TIMA, HALT, STOP | **No sub-instruction timing.** Plus three live bugs: dead `MMU::restart()`, STOP kills DIV/APU permanently, illegal opcodes freeze the whole machine |
| [`03-ppu.md`](03-ppu.md) | Rendering model, mode timing, STAT, window, sprites, CGB video | **A one-line bug at `src/ppu.rs:230` makes the pixel x-advance quadratic** — worth ~4.5× in raster fidelity |
| [`04-apu.md`](04-apu.md) | Frame sequencer, length/envelope/sweep, wave RAM, mixing | Strongest subsystem — blargg 9/9 pass. But **duty patterns 1–3 are rotated one step**, and channel-off clicks |
| [`05-mmu-cartridge.md`](05-mmu-cartridge.md) | MBC matrix, RTC, memory-map corners, OAM DMA, boot state | **No MBC abstraction at all** — one hardcoded pseudo-mapper for every cartridge |
| [`06-features-and-robustness.md`](06-features-and-robustness.md) | Model support, API features, panic audit | **`Core::reset()` is `todo!()`**; three guest-reachable panic hazards |
| [`07-testing.md`](07-testing.md) | Current tests, gambatte's 3524-ROM suite, external suites, roadmap | **19 more blargg ROMs run with zero new code.** Then mooneye for ~90 named defects |

---

## Executive summary

`gb` is a **good DMG emulator with one architectural limitation that gates almost everything else**:
it advances time one *instruction* at a time, whereas gambatte threads an exact T-cycle timestamp
through every memory access.

```rust
// src/game_boy.rs:29-36 — the whole run loop
while cycles < min_cycles {
    let opcode = self.core.fetch();
    cycles += self.core.execute(opcode);   // peripherals updated AFTER the instruction
}
```

```cpp
// cpu.cpp:141-148 — gambatte's equivalent
#define READ(dest, addr) do { (dest) = mem_.read(addr, cycleCounter); cycleCounter += 4; } while (0)
```

Everything follows from that: `LDH A,(n)` reads its I/O byte 8 T-cycles early; the PPU can't model a
variable mode-3; the APU decimates its own transitions; and 65% of emulated cycles (HALT) are
simulated at full price instead of skipped.

**But the ordering below deliberately front-loads the cheap wins**, because roughly a dozen genuine
bugs are independently fixable and several are ~1–4 lines each.

### What `gb` already does well — don't "fix" these

- **Blargg `cpu_instrs` 11/11, `instr_timing`, `dmg-acid2`** genuinely pass (verified by running
  them: 31 tests, 0.56s)
- **Blargg `dmg_sound` 9/9 of those wired** — the APU's register masks, frame-sequencer steps, and
  length/sweep quirks are all correct
- **Sprite selection and DMG priority** are correct, including the subtle "off-screen sprites still
  consume one of the 10 slots" rule
- **Per-opcode cycle totals**, `DAA`, `ADD SP,r8` flags, `ADC`/`SBC` half-carry, interrupt priority
- **Savestates exclude the ROM, are lz4-compressed, and validate ROM identity** — all three better
  than gambatte
- **Explicit save/load** rather than gambatte's destructor-time writes
- The **blip resampler** is fine; it is merely *driven* wrong

---

## Cross-cutting roadmap

Ordered by value per unit of effort, not by document. Tier 1 is roughly a day's work total.

### Tier 1 — cheap, independent, immediately verifiable

| # | Fix | Where | Lines |
|---|---|---|---|
| 1 | **Quadratic pixel x-advance** | `src/ppu.rs:230` | 1 ⚠️ *changes every screenshot — see the [fixture warning](03-ppu.md#fixture-warning)* |
| 2 | **Duty patterns 1–3 rotated one step** | `src/audio/square_channel.rs:218` | 3 |
| 3 | **Channel-off DAC click** (square + noise; wave already correct) | `src/audio/square_channel.rs:140`, `noise_channel.rs:105` | 4 |
| 4 | **Noise LFSR doesn't stop at shift ≥ 14** | `src/audio/noise_channel.rs:137` | 2 |
| 5 | **`Sweep::set_nr10` re-arms an idle sweep** | `src/audio/sweep.rs:42` | −3 |
| 6 | **Dead `initialised: true`** disables the first-duty-step quirk | `src/audio/square_channel.rs:53` | 1 |
| 7 | **OAM-DMA access gate is inverted** (makes VRAM/OAM *more* accessible during DMA) | `src/ppu.rs:90,103,109,118` | ~6 |
| 8 | **STOP permanently kills DIV/TIMA/APU** — `MMU::restart()` is never called | `src/core.rs:486` | ~4 |
| 9 | **Illegal opcodes freeze the whole machine**, not just the CPU | `src/core.rs:472` | 3 |
| 10 | **I/O read-back masks**: STAT.7, IF, TAC, SC, P1, IE, `FF46` | `src/mmu.rs:310-332` | ~8 |
| 11 | **`0xFEA0-0xFEFF` should read `0x00` on DMG**, not `0xFF` | `src/mmu.rs:333` | 1 |
| 12 | **`Core::reset()` is `todo!()`** | `src/core.rs:42` | ~20 |
| 13 | **19 more blargg ROMs**, zero new harness code | `src/game_boy.rs` | ~30 |
| 14 | **Fix the benchmark command** in `CLAUDE.md` (currently matches zero tests) and the stale "27 fixtures" figure (it's 91) | `CLAUDE.md` | 2 |

### Tier 2 — self-contained, high value

| # | Fix | Doc |
|---|---|---|
| 15 | **Mooneye harness** (~30 lines) → ~90 *named* timing defects | [07 §4](07-testing.md#4-prioritised-adoption-roadmap) |
| 16 | **Versioned savestate envelope** — retires the "never add a field to `Audio`" rule with zero fixture churn | [01 §6.1](01-architecture.md#61-migration-plan--five-steps-each-shippable) |
| 17 | **STAT as an OR-ed level with edge detection** — buys `stat_irq_blocking` and most `intr_*` | [03 §3](03-ppu.md#3-stat-interrupts) |
| 18 | **Rewrite the timer as a DIV-derived edge detector** | [02 §5](02-cpu.md#5-div--timer--highest-accuracy-win-per-line) |
| 19 | **LCD enable/disable state reset** — removes a latent "VRAM locked forever" hazard | [03 §4](03-ppu.md#4-lcd-enabledisable) |
| 20 | **ROM padding + bank masking** — removes the only guest-reachable panic in the memory hot path | [05 §1](05-mmu-cartridge.md#1-mbc-support-matrix) |
| 21 | **Sprite-height OOB** on a mid-scanline LCDC write | [06 §4.4](06-features-and-robustness.md#44-three-genuine-guest-reachable-hazards) |
| 22 | **DIV-write frame-sequencer clock** | [04 §2](04-apu.md#2-frame-sequencer) |
| 23 | **Envelope zombie mode** | [04 §4](04-apu.md#4-envelope-unit--zombie-mode-is-missing) |
| 24 | **Wave RAM lock while active** — re-enables blargg `09`/`12` | [04 §6](04-apu.md#6-wave-channel--the-biggest-concrete-gap) |
| 25 | **`trait Mbc`** + per-mapper dispatch | [05 §1](05-mmu-cartridge.md#1-mbc-support-matrix) |
| 26 | **`initstate` table** (F=`0xB0`, LCDC=`0x91`, BGP=`0xFC`, HRAM/OAM dumps, SRAM=`0xFF`) | [05 §7](05-mmu-cartridge.md#7-boot-state) |

### Tier 3 — the architectural work

| # | Fix | Payoff |
|---|---|---|
| 27 | **Thread a clock through the bus** — `tick()`/`bus_read()`/`bus_write()` | Gates everything below; migrate incrementally with `machine_cycles` as a `debug_assert!` oracle |
| 28 | **Event scheduler + HALT fast-path** | ~2× performance, and the structure accuracy needs |
| 29 | **Real PPU pixel pipeline with variable mode 3** | The window quirks and accurate access windows |
| 30 | **Memory page table** | 5–10%, and makes cycle-accurate access blocking affordable |
| 31 | **gambatte's hwtests as a regression ratchet** | The endgame, not the opening move |

### Explicitly not recommended

- **CGB support** — 20 features, a colour-type rewrite of the pixel pipeline, and zero value for a
  DMG-only Pokémon Red agent
- **`target-cpu=native`** — already measured slower, twice
- **Optimising the agent layer** — it is 13% of the loop; the emulator core is the other 87%
- **RTC** — Pokémon Red's cartridge (`0x13`) has no timer
- OSD overlay, savestate thumbnails, ZIP loading, multicart heuristics, Game Genie/Shark

---

## Two corrections to widely-held beliefs

Both were assumed during this research and turned out to be wrong. Recorded so nobody re-derives
them:

1. **Blargg's *Game Boy* test ROMs have no `$A000` / `$DE $B0 $61` magic signature.** That is his
   *NES* suite. `cpu_instrs/readme.txt` says outright that there is no well-defined programmatic
   result location. The serial-substring approach `gb` already uses is the de-facto standard and is
   correct.
2. **This gambatte checkout has no boot-ROM support, no `setTimeMode`, no `setRtcDivisorOffset`, no
   `loadGbcBios`, and no CGB title-checksum palette table.** It is upstream sinamas ~0.5.0-era, not
   gambatte-speedrun. See
   [`06-features-and-robustness.md` §0](06-features-and-robustness.md#0-which-gambatte-is-this--read-first).

Also worth knowing: **neither emulator supports the Super Game Boy or a link cable** (both
grep-verified).

---

## Licence note

**gambatte is GPL-2.0 only**, whole tree, including `test/hwtests/**` and the 413 reference PNGs
(no per-file headers; they inherit the project licence). `gb` ships **no LICENSE** and already
contains an LGPL-2.1+ port of blargg's Blip_Buffer.

If gambatte's test corpus is adopted, **use a git submodule exactly as `pokered/` is handled** — no
GPL source enters `gb`'s history, and the corpus is never linked into the binary. Better still, use
[`c-sp/game-boy-test-roms`](https://github.com/c-sp/game-boy-test-roms) v7.0, which ships all 3524
gambatte ROMs prebuilt alongside 15 other suites.

---

## How to use these documents

Every guide has the same shape: a **ranked gap table**, then per-topic sections with *How gambatte
does it* (real code, `file:line`), *How gb does it* (real code, `file:line`), *Gap*, *Symptom /
failing tests*, and *Fix sketch* — plus checkbox task lists you can work through directly.

Every claim carries a `file:line` reference on both sides. Where something could not be verified, it
says so. Where a "gap" turned out to be correct behaviour, that is recorded too — several of `gb`'s
subsystems are right and the documents say which, so nobody spends effort "fixing" them.

**Start with Tier 1.** It is about a day of work, roughly a dozen genuine bug fixes, and every item
is independently verifiable against test ROMs already in the repo.
