# Architecture, performance & state management

`gb` (`/home/alex/projects/gb`) vs **gambatte** (`/home/alex/projects/gambatte`) — the structural
comparison. This guide underpins all the others: most per-peripheral accuracy gaps documented in
[`02-cpu.md`](02-cpu.md), [`03-ppu.md`](03-ppu.md) and [`04-apu.md`](04-apu.md) are downstream of
the scheduling model described here.

**Measurements in this document were taken on 2026-08-04** on an AMD Ryzen 9 7900X. `gb` was built
with its committed release profile (`lto="thin"`, `codegen-units=1`); gambatte was compiled
out-of-tree into a scratchpad with `g++ 16.1.1 -O2 -fomit-frame-pointer -fno-exceptions -fno-rtti`.
The gambatte tree itself was never modified.

---

## 0. Measured performance

### 0.1 Headline throughput

| # | Core | Workload | Wall | Emulated | Realtime | t-cycles/s |
|---|---|---|---|---|---|---|
| 1 | **gb** | `bench_emulation_throughput`, raw `GameBoy::run` from `at-celadon.bin` | 1.214 s | 30 s | **24.7×** | 104 M |
| 2 | **gb** | same bench, full `agent.step()` | 2.743 s | 60 s | **21.9×** | 92 M |
| 3 | **gambatte** | Pokémon Red DMG, **in-game** (loaded `gb`'s own `pokemon-red.sav`) | 0.132 s | 60.3 s | **457.6×** | 1919 M |
| 4 | **gambatte** | Pokémon Red DMG, from power-on | 0.108 s | 60.2 s | **558.6×** | 2343 M |
| 5 | **gambatte** | `cpu_instrs.gb` (busy-loops, **never HALTs**) | 0.141 s | 60.3 s | **428.8×** | 1799 M |
| 6 | **gambatte** | `dmg-acid2.gb` | 0.097 s | 60.3 s | **622.7×** | 2612 M |

**gambatte is roughly 18× faster.** Per emulated m-cycle: `gb` ≈ 208 host cycles, gambatte ≈ 11.

**Independent confirmation run** (same machine, no other load):

```
[full agent.step]  60.0s game in 2.463s → 24.4x realtime (3000 steps)
test result: ok. 1 passed; 0 failed; finished in 3.54s
```

24.4× with the agent, versus row 2's 21.9× — run-to-run variance is ±6% and rows 1–2 were taken
while six research agents were competing for the machine. **Treat `gb` as ~24–25× realtime and the
gap as 18–25×**; the exact multiplier is not the point, the order of magnitude is.

Two honest caveats:

- The two Pokémon workloads are not identical — `gb`'s bench walks and battles from
  `at-celadon.bin`; the gambatte harness lands on a party-status screen (verified by dumping its
  framebuffer). Row 5 is the control: `cpu_instrs.gb` **never HALTs**, so idle-skipping buys
  nothing there, and gambatte still runs at 428×. **The gap is not only idle-skipping.**
- The agent layer costs ~13%, closely matching the ~11% already documented in `CLAUDE.md`. That
  number holds up; **do not spend effort optimising the agent layer.**

### 0.2 Ablation — where `gb`'s time goes

`perf` and `valgrind` are not installed on this machine, so there is **no instruction-level
profile**. Instead, a scratchpad copy of the tree was instrumented with env-gated early-returns.
Best of 3, 30 s of game time. This ablation matrix *is* the profile — coarser than `perf record`,
but real measurement.

| Config | Wall | Δ | Share |
|---|---|---|---|
| baseline | 1.145 s | — | 100% |
| `ABLATE_AUDIO` (whole `Audio::update` no-op) | 0.719 s | −0.426 s | **APU = 37.2%** |
| `ABLATE_RENDER` (PPU pixel loop skipped, state machine kept) | 0.776 s | −0.369 s | **pixel loop = 32.2%** |
| both | 0.346 s | −0.799 s | residual (CPU+MMU+timers) = **30.2%** |
| `ABLATE_MIX` (skip f32 pan/mix, still push a sample) | 1.025 s | −0.120 s | float mixing = 10.5% |
| `ABLATE_BLIP` (skip `BlipStereo::update`/`end_frame`) | 1.044 s | −0.101 s | resampler = 8.8% |
| `ABLATE_CHTIMER` (`PhaseTimer::update` → 0 iterations) | 1.057 s | −0.088 s | phase timers = 7.7% |

### 0.3 The HALT profile — the single biggest finding

Instrumenting `Core::fetch`/`Core::execute`:

```
[halt profile] instrs   normal=4,738,963  halt=20,442,126  (81.2% halt)
               m-cycles normal=11,025,032 halt=20,443,919  (65.0% halt)
```

**81% of `gb`'s CPU dispatches, and 65% of emulated m-cycles, are HALT.** Each one executes a
virtual `Nop` *and* a full `MMU::update` that ticks every peripheral. Gambatte skips the entire idle
span with one addition.

(The counts were identical across all six ablation builds — independent evidence the core is
deterministic.)

### 0.4 A documentation bug found along the way

The benchmark command in `CLAUDE.md`:

```
cargo test --release --bin gb -- bench_emulation_throughput --exact --ignored --nocapture
```

**matches zero tests.** `--exact` requires the full module path:

```
cargo test --release --bin gb -- pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture
```

- [ ] Fix the command in `CLAUDE.md`.

---

## 1. Ranked findings

| # | Finding | Impact | Effort |
|---|---|---|---|
| **F1** | **No event scheduler.** `Core::execute` calls `MMU::update(cycles)` after every instruction, unconditionally ticking PPU + 4 APU channels + timer + divider + serial + DMA + a 5-entry interrupt poll | Structural; enables F2–F4 | High |
| **F2** | **HALT emulated at full price** — 65% of cycles, 81% of dispatches | up to ~2–3× | Medium |
| **F3** | **APU = 37%**, driven per-instruction; `PhaseTimer::update` loops once per m-cycle per channel | ~25–30% wall | Low–Med |
| **F4** | **PPU pixel loop = 32%**, run in slivers, with a `Vec` allocated per scanline and a `sorted_by_key` **inside the innermost pixel loop** | ~15–25% wall | Medium |
| **F5** | Memory access is a 25-arm `match` over `u16` ranges with bounds-checked `Vec` indexing | 5–10%, and a prerequisite for cycle-accurate access timing | Medium |
| **F6** | `read`/`write` carry **no timestamp** — every access in an instruction happens at the same logical instant | Structural blocker | High |
| **F7** | Save state has **no magic, no version, no field skipping** — adding one field invalidates **91** committed fixtures (1.4 MB) | Unblocks all `Audio`/PPU work | Low–Med |
| **F8** | `GameBoy::reset()` is `todo!()`; `MMU::from_rom` `println!`s; illegal opcodes `println!` from the hot path | Embedding correctness | Low |
| **F9** | No "frame ready" signal — the SDL loop blits on a wall clock, so it can tear | API | Low |
| **F10** | `load_state` clones the whole 1 MB ROM (`game_boy.rs:82`) | Minor | Trivial |
| **F11** | `MachineCycles(usize)` with **saturating** `Sub` hides ordering bugs | Latent correctness | Low |

---

## 2. The scheduling model

### How gambatte does it

One global t-cycle counter plus **absolute** event timestamps. The primitive is
`MinKeeper<ids>` — a template-unrolled tournament tree (`minkeeper.h:44-60`) where `setValue<id>`
writes a leaf and walks `ceil(log2(ids))` parents as straight-line code, and `min()`/`minValue()`
are O(1) field reads.

Nine event ids (`interruptrequester.h:29-36`): `intevent_unhalt, _end, _blit, _serial, _oam, _dma,
_tima, _video, _interrupts`. "Disabled" is a **sentinel, not a flag** —
`enum { disabled_time = 0xfffffffful };` (`counterdef.h:7`) — so a disabled event simply never wins
the min, with no branch.

The run loop (`cpu.cpp:511-537`):

```cpp
void CPU::process(unsigned long const cycles) {
    mem_.setEndtime(cycleCounter_, cycles);
    mem_.updateInput();
    while (mem_.isActive()) {
        if (mem_.halted()) {
            if (cycleCounter < mem_.nextEventTime()) {
                unsigned long cycles = mem_.nextEventTime() - cycleCounter;
                cycleCounter += cycles + (-cycles & 3);      // skip the whole idle span
            }
        } else while (cycleCounter < mem_.nextEventTime()) {
            /* fetch + giant switch; only PC_READ/READ/WRITE touch memory */
        }
        cycleCounter = mem_.event(cycleCounter);
    }
}
```

Three things to notice:

1. **The inner instruction loop contains no peripheral calls at all.** Only when `cycleCounter`
   reaches `nextEventTime()` does `Memory::event()` (`memory.cpp:178-273`) dispatch on
   `intreq_.minEventId()` and service *the one* peripheral that is due.
2. **HALT is a single addition.**
3. **The end of the caller's slice is itself an event** (`intevent_end`, `memory.cpp:142-149`), so
   the loop needs no separate bound check.

Peripherals own their timing and publish a next-event time. `LCD::update` is pure lazy catch-up
(`video.cpp:857-867`). The APU is the same shape — `SoundUnit` holds `counter_` = the absolute time
of its next event, and `DutyUnit` derives position **in closed form, with no loop**:

```cpp
// sound/duty_unit.cpp:51-58
void DutyUnit::updatePos(unsigned long const cc) {
    if (cc >= nextPosUpdate_) {
        unsigned long const inc = (cc - nextPosUpdate_) / period_ + 1;   // no loop
        nextPosUpdate_ += period_ * inc;
        pos_ = (pos_ + inc) % duty_pattern_len;
        high_ = toOutState(duty_, pos_);
    }
}
```

**The invariant on every register write is: catch up → mutate → reschedule.**
`PSG::setNr13(data)` → `ch1_.setNr3(data, cycleCounter_)` → `DutyUnit::nr3Change` → `updatePos(cc)`
then `period_ = …` then `setCounter()`.

### How gb does it

```rust
// src/game_boy.rs:29-36
pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
    let mut cycles = MachineCycles::ZERO;
    while cycles < min_cycles {
        let opcode = self.core.fetch();
        cycles += self.core.execute(opcode);
    }
    cycles
}
```

`Core::execute` ends with (`src/core.rs:479-502`):

```rust
let cycles = MachineCycles::from_m(opcode.machine_cycles(condition_met));
let interrupt_cycles = match self.mode {
    CoreMode::Normal | CoreMode::Halt => { self.mmu.update(cycles); self.interrupt() }
    /* ... */
};
self.mmu.update(interrupt_cycles);
```

and `MMU::update` (`src/mmu.rs:216-249`) unconditionally does, **per instruction**: DMA update
(plus a 160-byte copy if firing), `serial.update`, `divider.update`, `timer.update`, `ppu.update`
(including the per-pixel render loop), `audio.update` (4 channels each with a per-m-cycle `for`
loop, plus a full f32 pan/mix/divide and blip `update` + `end_frame`), then a 5-iteration
`for interrupt in InterruptType::all()` poll with an `Activation` trait call each.

**There is no absolute clock anywhere** — only private accumulators
(`Divider::cycles_since_tick`, `Timer::cycles`, `PPU::current_ticks`). And in HALT the CPU still
walks all of it once per m-cycle:

```rust
// src/core.rs:143-151
pub fn fetch(&mut self) -> OpCode {
    if self.mode == CoreMode::Normal { OpCode::parse(self) }
    else { OpCode::Nop }   // "keeps the clocks ticking"
}
```

### Gap

| | gambatte | gb |
|---|---|---|
| Clock | one absolute t-cycle counter threaded everywhere | none; per-peripheral deltas |
| Idle cycles | skipped in O(1) | fully simulated — 65% of all cycles |
| Peripheral work | only on its own event | every instruction, every peripheral |
| Next event | `MinKeeper`, O(1) query | n/a |
| Register write | catch up → mutate → reschedule | mutate only |
| Sub-instruction timing | modelled | not modelled |

### Fix sketch — retrofitting an event scheduler in Rust

**Step 0 — an absolute clock.** Add `now: u64` (m-cycles) to `MMU`; `Core` advances it. This avoids
changing the `ROM`/`RAM` traits on day one.

**Step 1 — the schedule.** With ~8 event kinds a flat `[u64; N]` min **beats a heap and beats
`MinKeeper` in simplicity** — the scan is 8 `cmp`+`cmov` pairs and auto-vectorises.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[repr(u8)]
pub enum Ev { EndOfSlice = 0, Video, Timer, Divider, Serial, OamDma, Apu, Interrupt }
const N_EV: usize = 8;
pub const DISABLED: u64 = u64::MAX;

#[derive(Clone, Copy, Encode, Decode)]
pub struct Schedule { when: [u64; N_EV], next: u64, next_id: u8 }

impl Schedule {
    #[inline(always)]
    pub fn set(&mut self, e: Ev, t: u64) {
        self.when[e as usize] = t;
        if t <= self.next { self.next = t; self.next_id = e as u8; }
        else if self.next_id == e as u8 { self.recompute(); }   // we relaxed the current min
    }
    #[inline]
    fn recompute(&mut self) {
        let (mut best, mut id) = (DISABLED, 0u8);
        for (i, &t) in self.when.iter().enumerate() {
            if t < best { best = t; id = i as u8; }
        }
        self.next = best; self.next_id = id;
    }
    #[inline(always)] pub fn next(&self) -> u64 { self.next }
}
```

> Port `MinKeeper` only if `recompute` ever shows up in a profile. With N=8 it is ~4 ns and `set`
> usually takes the fast branch.

**Step 2 — the run loop**, with `EndOfSlice` as an event exactly as gambatte does:

```rust
pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
    let start = self.mmu.now;
    let end = start + min_cycles.m_cycles() as u64;
    self.mmu.sched.set(Ev::EndOfSlice, end);

    while self.mmu.now < end {
        if self.mode == CoreMode::Halt {
            self.mmu.now = self.mmu.sched.next().min(end);   // F2: one add, not 20M dispatches
        } else {
            while self.mmu.now < self.mmu.sched.next() {
                let op = self.fetch();
                self.mmu.now += self.execute(op).m_cycles() as u64;   // NO mmu.update here
            }
        }
        self.mmu.service_events();   // catch up + reschedule exactly what is due
    }
    MachineCycles::from_m((self.mmu.now - start) as usize)
}
```

**Step 3 — `catch_up` / `next_event` per peripheral.** Each loses its delta accumulator and gains an
absolute "next":

```rust
pub trait Peripheral {
    fn catch_up(&mut self, now: u64, irq: &mut InterruptFlags);
    fn next_event(&self) -> u64;                    // DISABLED == u64::MAX
}

impl Peripheral for Timer {
    fn catch_up(&mut self, now: u64, irq: &mut InterruptFlags) {
        while self.next_tick <= now {
            self.next_tick += self.period;
            if self.value == 0xFF { self.value = self.modulo; irq.set(InterruptType::Timer); }
            else { self.value += 1; }
        }
    }
    fn next_event(&self) -> u64 { if self.enabled { self.next_tick } else { DISABLED } }
}
```

**Step 4 — register writes reschedule.** This is the rule that keeps it correct:

```rust
0xFF07 => {                                   // TAC
    self.timer.catch_up(self.now, &mut self.interrupt_request);
    self.timer.set_control(value);
    self.sched.set(Ev::Timer, self.timer.next_event());
}
0xFF13 | 0xFF14 => {                          // NR13 / NR14
    self.audio.catch_up(self.now);
    self.audio.write(address, value);
    self.sched.set(Ev::Apu, self.audio.next_event());
}
```

Reads of timed values need the catch-up half, which requires `&mut self` — so widen
`ROM::read` to `fn read(&mut self, address: u16) -> u8` and add a separate
`fn peek(&self, address: u16) -> u8` for the Pokémon RAM-inspection layer, which must not perturb
timing.

**Step 5 — the interrupt poll disappears.** Peripherals set `interrupt_request` inside their own
`catch_up` (gambatte's `InterruptRequester::flagIrq`), replacing the 5-way poll at
`src/mmu.rs:237-248` — **and removing `src/activation.rs` entirely**, which exists only because a
peripheral currently cannot reach the interrupt register.

**Migration order that keeps the fixtures alive.** Steps 0 and 2 can land first and be
behaviour-preserving if each `catch_up` is exactly today's `update` fed a bigger delta. `Timer`,
`Divider` and `Serial` already are (`while cycles >= period` loops); only `PPU` and `Audio` need
real work.

### Tasks

- [ ] Add `now: u64` to `MMU`
- [ ] Add a dependency-free `src/schedule.rs` with `Ev` / `Schedule` and unit tests
- [ ] Convert `Timer`, `Divider`, `Serial` to `catch_up`/`next_event` (behaviour-preserving)
- [ ] Convert `PPU` and `Audio` (the real work — coordinate with
      [`03-ppu.md`](03-ppu.md) and [`04-apu.md`](04-apu.md))
- [ ] Add the HALT fast-path
- [ ] Route every timed register write through catch-up → mutate → reschedule
- [ ] Delete `src/activation.rs`

---

## 3. Memory-access hot path

### gambatte

`MemPtrs` (`mem/memptrs.h:82-93`) keeps two 16-entry tables of raw pointers, one per 4 KB region,
**pre-biased** so no per-access subtraction is needed:

```cpp
// mem/memptrs.cpp:118-122
void MemPtrs::setRombank(unsigned bank) {
    romdata_[1] = romdata() + bank * rombank_size() - mm_rom1_begin;   // bias by 0x4000
    rmem_[0x7] = rmem_[0x6] = rmem_[0x5] = rmem_[0x4] = romdata_[1];
    disconnectOamDmaAreas();
}
```

so the access is a table load, an add, a load:

```cpp
// memory.h:73-90
unsigned read(unsigned p, unsigned long cc) {
    return cart_.rmem(p >> 12) ? cart_.rmem(p >> 12)[p] : nontrivial_read(p, cc);
}
void write(unsigned p, unsigned data, unsigned long cc) {
    if (cart_.wmem(p >> 12)) cart_.wmem(p >> 12)[p] = data;
    else nontrivial_write(p, data, cc);
}
```

A **null entry is the slow-path marker**, and gambatte reuses it as a *mechanism*: during OAM DMA,
`disconnectOamDmaAreas` nulls the conflicting regions so reads automatically fall into
`nontrivial_read`, which models the bus conflict. Region-null also covers VRAM (mode 3), SRAM
(disabled/RTC), OAM and I/O. All of ROM/VRAM/SRAM/WRAM lives in **one** `SimpleArray<unsigned char>
memchunk_` (`mem/memptrs.cpp:88-101`).

### gb

`impl ROM for MMU` (`src/mmu.rs:284-338`) — 25 arms, several with guards:

```rust
match address {
    0x0000..=0x3FFF => self.data[address as usize],
    0x4000..=0x7FFF => {
        let bank_offset = self.rom_bank_register * ROM_BANK_SIZE;
        self.data[bank_offset + (address - 0x4000) as usize]
    }
    0x8000..=0x9FFF => self.ppu.read_vram(address - 0x8000),
    0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks() > 0 => { /* ... */ }
    /* ... 20 more single-address arms ... */
    _ => 0xFF,
}
```

Costs: `self.data` is a `Vec<u8>` → load ptr, load len, bounds-check, load byte (**two dependent
loads before the data**, vs gambatte's one); `rom_bank_register * ROM_BANK_SIZE` is re-derived per
banked access instead of cached as a base; `ram_banks: Vec<[u8; 0x2000]>` adds a second indirection
*and* a second bounds check; the guarded arms make the match non-dense, so the cold high-`0xFFxx`
arms sit on the same comparison chain as the hot WRAM/ROM arms; and because `read` is `&self` with
no timestamp, DMA-conflict and mode-3 blocking have to live inside each peripheral
(`PPU::read_vram` re-checks the LCD mode on **every** access, `src/ppu.rs:89-93`).

### Fix sketch — safe, no `unsafe`, no self-referential pointers

Mirror `memptrs` with a single owned buffer plus an **offset** table (offsets, not pointers, so the
struct stays movable, `Clone`, and `Encode`/`Decode`-able):

```rust
const SLOW: u32 = u32::MAX;              // region not directly mapped -> fall through
const REGIONS: usize = 16;               // 4 KB each

pub struct MemMap {
    chunk: Box<[u8]>,        // ROM ++ VRAM ++ SRAM ++ WRAM, one allocation
    rbase: [u32; REGIONS],   // pre-biased: chunk[(rbase[p >> 12] + p as u32) as usize]
    wbase: [u32; REGIONS],
}

impl MMU {
    #[inline(always)]
    pub fn read(&mut self, addr: u16) -> u8 {
        let base = self.map.rbase[(addr >> 12) as usize];
        if base != SLOW {
            self.map.chunk[(base.wrapping_add(addr as u32)) as usize]
        } else {
            self.read_slow(addr)          // VRAM/SRAM-disabled/OAM/IO/HRAM/DMA-conflict
        }
    }
}
```

Notes:
- HRAM deserves its own branch — Pokémon Red's hot loops live there.
- Echo RAM is free: point `rbase[0xE]` at WRAM biased by `-0x2000`, and keep `rbase[0xF]` `SLOW`
  because `0xFE00..` is OAM/IO.
- **VRAM/SRAM blocking becomes "set the region to `SLOW`"**, so the fast path never tests for it —
  this is the mechanism that makes cycle-accurate access blocking affordable.
- To keep the ROM out of the serialised chunk, hold it as `Arc<[u8]>` and add a `rspace: [u8; 16]`
  selector — still one branch.
- `[*const u8; 16]` would need `unsafe`; the offset table is the safe equivalent, and the extra
  bounds check is usually hoisted since `chunk.len()` is loop-invariant.

---

## 4. Counter arithmetic and overflow

**gambatte** uses absolute `unsigned long` timestamps with `disabled_time = 0xfffffffful`. Because
32-bit wraps after ~17 minutes of game time, it **rebases periodically** (`cpu.cpp:45-53`,
`Memory::resetCounters` at `memory.cpp:448-470`): every subsystem implements
`resetCc(oldCc, newCc)`. Cost: once per ~0.5 s of game time — negligible, and it buys 32-bit
timestamps that keep `MinKeeper`'s values in one or two cache lines.

**gb** has no absolute timestamps at all, and `MachineCycles(usize)` is 64-bit here, so no rebasing
is needed. Two real concerns:

1. **Saturating subtraction hides bugs** (`src/cycles.rs:76-86`). `a - b` with `b > a` is always a
   logic error in cycle accounting; saturating it to zero turns it into a silent timing skew. Only
   two call sites actually want clamping (`src/sdl/render.rs:236`, `:239`).
2. **`usize`, not `u64`.** `t_cycles() = self.0 * 4` overflows after ~17 min of game time on a
   32-bit target, and `full_playthrough` runs 20 minutes.
3. `from_hz` does two truncating divisions — exact for all current constants, silently lossy for a
   non-divisor.

**Verdict: sound on 64-bit today**, but the saturating `Sub` is a smell and `usize` is an
unnecessary portability trap. With `u64` absolute timestamps from §2, **no rebasing is ever
needed** — a genuine simplification over gambatte, which only needed `resetCounters` because C++98
gave it no portable 64-bit type.

- [ ] Change `MachineCycles` to wrap `u64`
- [ ] Replace saturating `Sub`/`SubAssign` with `debug_assert!(self.0 >= other.0)` plus explicit
      `saturating_sub` at the two `render.rs` sites

---

## 5. Ranked optimisations

| Rank | Optimisation | Where | Payoff | Risk |
|---|---|---|---|---|
| **1** | **HALT fast-path** — jump `now` to the next event instead of 20 M virtual NOPs | `game_boy.rs:29`, `core.rs:143-151` | 65% of cycles stop costing a full `MMU::update`; **≈2×** once peripherals are delta-agnostic | Medium |
| **2** | **Closed-form `PhaseTimer::update`** — replace `for _ in 0..ticks` with `(ticks + period - counter) / period` and a modular phase advance, exactly `DutyUnit::updatePos` | `src/audio/timer.rs:52-64` | measured 7.7% at delta=1; grows superlinearly with rank 1 | Low — blargg `dmg_sound` guards it |
| **3** | **Mix only when something changed** — `Audio::update` recomputes 4 `output_f32()`, 4 pans, a volume multiply and a `/4.0` every instruction | `src/audio/mod.rs:110-137` | measured 10.5% | Low |
| **4** | **Render whole scanlines** — move the pixel loop out of `PPU::update`'s `Drawing` arm to the mode-3→HBlank transition | `src/ppu.rs:221-273` | 32.2% is all pixel work; expect a third to a half back from loop setup alone | Medium — dmg-acid2 guards it |
| **5** | **Hoist the sprite search out of the pixel loop.** `src/ppu.rs:257-266` runs `.filter().map().filter().sorted_by_key().next()` **per pixel** — `sorted_by_key` **allocates a `Vec` in the innermost loop.** Pre-sort by `x` once per scanline (gambatte: `insertionSort`, `video/sprite_mapper.cpp:180`) then scan linearly | `src/ppu.rs` | likely the largest line-item inside rank 4 | Low |
| **6** | **Kill the per-scanline `Vec`.** `scanline_sprites` is `.collect()`ed every OAM→Drawing transition (`src/ppu.rs:210-219`) — 8640 allocations/s. The DMG hard limit is 10; use `[Sprite; 10]` + `len` | `src/ppu.rs` | 1–3% | Trivial |
| **7** | **Memory page table** (§3) | `src/mmu.rs` | 5–10% | Medium |
| **8** | **Cheaper decode.** `OpCode::parse` builds a fat enum, then `machine_cycles(condition_met)` re-matches the whole enum afterwards (`opcode.rs:577-635`) — **two full dispatches per instruction**. Use a `[u8; 256]` cycle table | `opcode.rs`, `core.rs:479` | 2–5% | Low |
| **9** | **Drop the per-instruction interrupt poll** (`mmu.rs:237-248` iterates 5 types with an `Activation` call each; `interrupt_pending` at `:251-258` iterates again). Make it `(ie & iflag).trailing_zeros()` | `src/mmu.rs` | 1–3% | Low |
| **10** | Try `lto = "fat"` | `Cargo.toml` | unknown, 0–3% | Trivial |

> ⚠️ **Caution on rank 1.** The HALT fast-path is **not free money on its own.** With today's
> peripherals, `catch_up(delta)` is still O(delta) in `PhaseTimer` and the PPU mode loop. Ranks 2
> and 4 must land with or before it, or the win collapses to saved dispatch overhead (~15%, not 2×).

**Do not pursue:** `target-cpu=native` (already measured slower — the `Cargo.toml` comment records
26.3× vs 29.7×, and the measurements here agree with that ordering), or optimising the agent layer
(13% of the loop).

---

## 6. Save states and the fixture freeze

### How gambatte does it

**(a) State is a struct, not the live objects.** `savestate.h` declares a flat `SaveState` of plain
scalars plus `Ptr<T>` views into the live buffers. `Memory::setStatePtrs(state)` wires them once;
save/load then only copy scalars. The serialisable surface is an explicit, reviewable list, and the
live objects are free to hold non-serialisable members.

**(b) The file format is label-addressed and skip-tolerant.** `statesaver.cpp:216-335` builds a
sorted `SaverList` of `{label, save_fn, load_fn}`, each a short ASCII tag (`"cc"`, `"pc"`,
`"dut1ctr"`, …). Save writes `label\0` + a 3-byte length + payload. Load:

```cpp
// statesaver.cpp:424-441
while (file.good() && done != list.end()) {
    file.getline(labelbuf, list.maxLabelsize(), NUL);
    SaverList::const_iterator it = done;
    if (std::strcmp(labelbuf, it->label)) {
        it = std::lower_bound(it + 1, list.end(), labelbufSaver);
        if (it == list.end() || std::strcmp(labelbuf, it->label)) {
            file.ignore(get24(file));      // UNKNOWN LABEL: skip it and carry on
            continue;
        }
    } else ++done;
    (*it->load)(file, state);
}
```

A field added in a new version is simply **absent** from old files (its `setInitState` value
survives); a field removed is **skipped** in old files. Endian-independent too — everything is
written byte-by-byte MSB-first, never `memcpy`d.

### How gb does it

```rust
// src/game_boy.rs:60-65
pub fn save_state(&self) -> Result<Vec<u8>, String> {
    let serialized = bincode::encode_to_vec(self, bincode::config::standard())?;
    Ok(lz4_flex::compress_prepend_size(&serialized))
}
```

`bincode::config::standard()` is **positional and non-self-describing** — field order and presence
*are* the schema. There is no magic, so a loader cannot tell what it is holding. Consistency is
checked only by `header()` equality *after* decoding (`src/game_boy.rs:78-80`), which catches "wrong
ROM" but not "wrong layout". The hand-written `Encode`/`Decode` for `MMU` and `Audio`
(`src/mmu.rs:395-460`, `src/audio/mod.rs:357-370`) exist purely to *omit* fields.

**Hence the CLAUDE.md freeze.** And the current fixture population is **91 files / 1.4 MB** —
`CLAUDE.md` says 27, **stale by more than 3×**, so the cost of a layout break is three times what
the doc claims.

Determinism is good: the whole `GameBoy` is `PartialEq + Eq`, and `save_and_load_state`
(`src/game_boy.rs:106-126`) asserts round-trip identity.

| | gambatte | gb |
|---|---|---|
| Version | 2-byte header + label tolerance | none |
| Unknown field on load | skipped | stream desync |
| Missing field on load | left at init value | truncated decode error |
| State vs runtime | separate `SaveState` | hand-written impls on the live struct |
| Cost of adding a field | zero | **invalidates 91 fixtures** |

### 6.1 Migration plan — five steps, each shippable

**Step 1 — versioned envelope with legacy sniffing (zero fixture churn).**

```rust
const MAGIC: [u8; 4] = *b"GBST";
pub const STATE_VERSION: u16 = 1;

pub fn save_state(&self) -> Result<Vec<u8>, String> {
    let payload = bincode::encode_to_vec(self, bincode::config::standard())?;
    let mut out = Vec::with_capacity(payload.len() / 2 + 8);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&STATE_VERSION.to_le_bytes());
    out.extend_from_slice(&lz4_flex::compress_prepend_size(&payload));
    Ok(out)
}

pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
    let (version, body) = if data.starts_with(&MAGIC) {
        (u16::from_le_bytes([data[4], data[5]]), &data[6..])
    } else {
        (0, data)          // legacy: bare lz4-with-size-prefix, exactly today's format
    };
    if version > STATE_VERSION {
        return Err(format!("save state v{version} is newer than this build (v{STATE_VERSION})"));
    }
    let raw = lz4_flex::decompress_size_prepended(body)?;
    let (gb, _): (GameBoy, usize) = bincode::decode_from_slice_with_context(
        &raw, bincode::config::standard(), StateVersion(version))?;
    /* ... */
}
```

**The sniff is provably safe:** today's first four bytes are an LE `u32` decompressed length, which
for a ≤2 MB state is `xx xx 0x 00` and can never be `47 42 53 54`. All 91 fixtures keep loading with
**zero re-encoding**.

**Step 2 — version as a bincode decode context.** `MMU` already implements `Decode<__Context>`
generically (`src/mmu.rs:417`); concretise it:

```rust
#[derive(Clone, Copy)]
pub struct StateVersion(pub u16);

impl Decode<StateVersion> for Audio {
    fn decode<D: Decoder<Context = StateVersion>>(d: &mut D) -> Result<Self, DecodeError> {
        let mut me = Audio {
            enabled:         Decode::decode(d)?,
            panning:         Decode::decode(d)?,
            master_volume:   Decode::decode(d)?,
            frame_sequencer: Decode::decode(d)?,
            channel1:        Decode::decode(d)?,
            channel2:        Decode::decode(d)?,
            channel3:        Decode::decode(d)?,
            channel4:        Decode::decode(d)?,
            output:          BlipStereo::default(),   // never serialised, by design
        };
        if d.context().0 >= 1 {          // v1 additions, absent from v0 fixtures
            me.pcm12 = Decode::decode(d)?;
        }
        Ok(me)
    }
}
```

This is the Rust analogue of label-skipping, and it is **the whole answer to the freeze**: the rule
becomes *"you may add fields, appended at the end, behind `if version >= N`"* instead of *"you may
never add fields"*.

**Step 3 — a re-encode tool, so the legacy path can eventually be dropped.**

```rust
#[test]
#[ignore = "maintenance tool"]
#[cfg(feature = "regen-fixtures")]
fn reencode_fixtures() {
    for entry in std::fs::read_dir("src/pokemon/data").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "bin") { continue; }
        let mut gb = GameBoy::dmg(crate::pokemon::roms::ROM);
        gb.load_state_from_file(path.to_str().unwrap()).unwrap();   // reads v0 or v1
        gb.save_state_to_file(path.to_str().unwrap()).unwrap();     // writes current
    }
}
```

**Step 4 — a guard test in the *default* tier.** 1.4 MB total; decoding is milliseconds.

```rust
#[test]
fn every_committed_fixture_decodes() {
    for (name, bytes) in ALL_FIXTURES {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::ROM);
        gb.load_state(bytes).unwrap_or_else(|e| panic!(
            "fixture {name} no longer decodes: {e}\n\
             If intentional: bump STATE_VERSION, guard new fields with `if version >= N`, \
             and run `reencode_fixtures`."));
    }
}
```

Today a layout break surfaces hours later as a confusing `slow-tests` failure. This makes it a
2-second failure with the fix in the panic text.

**Step 5 — separate state from runtime (the real cure).**

```rust
#[derive(Encode, Decode, PartialEq, Eq)]
pub struct AudioState { enabled: bool, panning: Panning, /* … */ channel4: NoiseChannel }

pub struct Audio { state: AudioState, output: BlipStereo }   // runtime member, not serialised
```

`#[derive(Encode, Decode, PartialEq)]` works again, the hand-written impls and their drift risk
disappear, and **the CLAUDE.md constraint evaporates** — you can add anything to `Audio`, because it
is not in `AudioState`. Do the same for `MMU`: its hand-written impls exist only to skip
`data: Vec<u8>`, and moving the ROM to `rom: Arc<[u8]>` outside `MmuState` removes both the impls
*and* the 1 MB clone in `load_state` (F10).

### Tasks

- [ ] Step 1: versioned envelope + legacy sniffing
- [ ] Step 2: `StateVersion` decode context
- [ ] Step 3: `reencode_fixtures` maintenance test
- [ ] Step 4: `every_committed_fixture_decodes` guard in the default tier
- [ ] Step 5: split `AudioState` from `Audio`, and `MmuState` from `MMU`
- [ ] Update the stale "27 committed fixtures" figure in `CLAUDE.md` to 91

---

## 7. Public API and embedding ergonomics

### gambatte's contract

```cpp
// include/gambatte.h:81-82
std::ptrdiff_t runFor(uint_least32_t *videoBuf, std::ptrdiff_t pitch,
                      uint_least32_t *audioBuf, std::size_t &samples);
```

Runs until **either** `samples` stereo samples are produced **or** a video frame completes. 35112
samples/frame, may overrun by ≤2064. `samples` is in/out. **The return value is the sample offset at
which the frame completed**, or −1 — this is what makes A/V sync possible without heuristics.
`videoBuf` may be **0** (headless). `load()` returns a `LoadRes` enum, never throws, never prints.

### What `gb` doesn't expose that an accurate core needs

1. **Frame-completion signal.** None. `src/sdl/render.rs:266` blits when
   `since_last_render >= TARGET_FRAME_TIME` — wall clock, not VBlank, so it can tear or repeat at
   1×. `PPU` already sets `vblank_interrupt_pending` (`src/ppu.rs:19`); it just isn't surfaced.
   ```rust
   pub struct RunOutcome { pub cycles: MachineCycles, pub frame_completed: Option<MachineCycles> }
   pub fn run(&mut self, min_cycles: MachineCycles) -> RunOutcome;   // or run_until_vblank()
   ```
2. **Audio/video budget coupling** — a `run_for_samples(n)` is the natural API for a headless
   recorder or a frame-locked frontend.
3. **`reset()` is `todo!()` and panics** (`src/core.rs:41-43`).
4. **Headless mode.** `PPU::update` always renders into `lcd: [DMGColor; 160*144]`. The
   `ABLATE_RENDER` measurement *is* this experiment: **26.2× → 38.5×**. `PokemonTextReader` reads
   VRAM rather than the framebuffer, so a `set_video_enabled(false)` may well be viable for most of
   the test suite (dmg-acid2 and `screenshot()`-based artifacts would keep it on). **Worth a real
   measurement.**
5. **Load error taxonomy.** `MMU::from_rom` returns `Result<_, String>` but `Core::dmg`
   `.expect()`s it (`src/core.rs:34`) — a bad ROM aborts the process. It also `println!`s the header
   unconditionally (`src/mmu.rs:45`); a library should not write to stdout. Same for the
   illegal-opcode `println!` in the hot path (`src/core.rs:473`).
6. **Encapsulation.** `core_mut().mmu_mut().…` exposes everything, which is what made the Pokémon
   layer possible — but it leaves no stable contract to hold while §2 and §3 restructure the
   internals. Suggest a thin facade (`press`, `framebuffer`, `read_samples`, `peek`, `wram`) with
   `core_mut()` retained as an explicitly-unstable escape hatch.
7. **`load_state` requires a live `GameBoy` with the ROM already loaded** and clones 1 MB
   (`src/game_boy.rs:82-84`). An `Arc<[u8]>` ROM makes this free and enables
   `GameBoy::from_state(rom, bytes)`.

---

## 8. Idioms worth stealing

1. **Peripherals own their timing and publish `next_event()`.** Every mutator takes `cc` —
   `Tima::setTac(unsigned tac, unsigned long cc, TimaInterruptRequester)` (`tima.h:45`). The
   *discipline* matters more than the trait: today `Timer::set_control` mutates with no notion of
   when.
2. **The interrupt requester as an injected capability, not a global.** `TimaInterruptRequester`
   (`tima.h:26-37`) is a one-field wrapper handed to `Tima`'s methods. In Rust, pass
   `&mut InterruptFlags` into `catch_up` — this sidesteps the borrow problem `gb` currently solves
   by polling `Activation`s.
3. **`disabled_time` as a sentinel, not an `Option`.** `0xFFFFFFFF` sorts last, so "disabled" needs
   no branch in the min. In Rust use `u64::MAX` — **resist `Option<u64>`**, which doubles the array
   and adds a branch per comparison.
4. **State structs separate from logic** (`savestate.h`) — §6.1 step 5.
5. **Lazy derivation over eager ticking.** `DutyUnit::updatePos`'s `(cc - nextPosUpdate_) / period_
   + 1` is the template for every counter in `gb` that currently loops (`PhaseTimer::update`,
   `NoiseChannel::update`, `Divider::update`, `Timer::update`). **Store *when the next thing
   happens*, not *how long since the last thing happened*.**
6. **Deltas, not samples, in the audio buffer.** `Channel1::update` writes `out - prevOut_` at
   transitions only; `PSG::fillBuffer` integrates in one unrolled pass. `gb`'s `BlipStereo` already
   has this insight — but `Audio::update` *calls* it once per instruction rather than once per
   transition. **The blip layer is fine; the driving is wrong.**
7. **Shared micro-primitives with no dependencies** — `minkeeper.h`, `insertion_sort.h`,
   `counterdef.h`.
8. **One contiguous memory chunk** rather than four scattered allocations (`src/mmu.rs:23-30`).

**One structural observation specific to `gb`:** `MMU` is a god-object — cartridge, banking, WRAM,
HRAM, *plus* `PPU`, `Serial`, `Divider`, `Timer`, `Audio`, `JoypadRegister` and both interrupt
registers (`src/mmu.rs:22-39`). Gambatte's `Memory` owns the same set, so this is not wrong per
se — but gambatte gives each member its own clock discipline, whereas `gb`'s `MMU::update` is the
one place that drives everything and therefore the one place that cannot be made selective.
Splitting into `Bus` (address decode + memory) and `Devices` (timed peripherals + `Schedule`) would
let `Bus::read` stay `&self`-cheap while `Devices` carries `now`.

---

## References

- Pan Docs — <https://gbdev.io/pandocs/>
- gambatte scheduling: `libgambatte/src/minkeeper.h`, `counterdef.h`, `cpu.cpp:511-537`,
  `memory.cpp:178-273`
- gambatte state: `libgambatte/src/savestate.h`, `statesaver.cpp`
- gambatte memory: `libgambatte/src/mem/memptrs.{h,cpp}`, `memory.h:73-90`
- bincode decode contexts — <https://docs.rs/bincode/latest/bincode/de/trait.Decode.html>
