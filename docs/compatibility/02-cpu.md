# CPU / timing-model compatibility guide

`gb` (`/home/alex/projects/gb`, Rust) vs **gambatte** (`/home/alex/projects/gambatte`, C++) —
`libgambatte/src/cpu.cpp`, `interrupter.cpp`, `interruptrequester.cpp`, `tima.cpp`, `memory.cpp`.

---

## Ranked gap summary

| # | Gap | Severity | Symptom |
|---|---|---|---|
| 1 | **No sub-instruction timing.** All bus accesses happen at T+0; peripherals advance *after* the whole instruction | Critical | Every LY/STAT/DIV/TIMA read is 4–20 T early. blargg `mem_timing`, `mem_timing-2`; mooneye `*_timing` |
| 2 | **Timer not derived from the DIV counter.** No 16-bit system counter, no falling-edge detector, no overflow reload window | Critical | mooneye `timer/tima_reload`, `tima_write_reloading`, `div_write`, `rapid_toggle`, `tim00`–`tim11` |
| 3 | **STOP permanently kills DIV + timer + APU frame sequencer.** `MMU::restart()` is dead code; STOP doesn't consume its second byte | Critical (latent) | Any ROM executing STOP loses DIV/TIMA/APU clocking forever; PC desync |
| 4 | **Interrupt dispatch atomic + mis-ordered.** No cancelled-interrupt quirk; `RETI` re-enables IME one instruction late | High | mooneye `interrupts/ie_push`, `intr_timing`; blargg `interrupt_time` |
| 5 | **HALT bug absent** — no PC-not-incremented double read | High | blargg `halt_bug.gb`; mooneye `halt_ime0_nointr_timing` |
| 6 | **Illegal opcodes freeze the whole machine**, not just the CPU | Med-High | Frozen LCD/audio hang instead of frozen-CPU-live-video |
| 7 | **OAM DMA instantaneous, access gates inverted**, transfer silently dropped in mode 2/3 | Med-High | mooneye `oam_dma*`; sprite corruption for non-VBlank DMA |
| 8 | **DIV write doesn't reset the sub-tick phase** | Medium | DIV-seeded RNG diverges (pokered's `Random` reads `rDIV`); mooneye `div_write` |
| 9 | **No KEY1 / double speed / CGB** | Low | DMG-only project |
| 10 | **Dispatch design**: decode-to-enum every execution, no cycle budget, halt spins per M-cycle | Low (perf) | ~2× dispatch overhead |

### What is already correct — do not touch

- **All per-opcode cycle totals** (`src/opcode.rs:577-634`), including all four conditional
  taken/not-taken pairs
- **`DAA`** (`src/core.rs:286-304`) — verified against gambatte `cpu.cpp:734-756` across the
  sign/half-carry matrix
- **`ADD SP,r8` / `LD HL,SP+r8` flags** — the `a^b^result` trick at `src/core.rs:546-552` is
  bit-identical to gambatte's `sp_plus_n` (`cpu.cpp:441-452`)
- **`ADC`/`SBC` half-carry**, **`BIT b,(HL)` = 12 T**, the 11-opcode illegal set, and
  **interrupt priority order** (`src/interrupt.rs:77-83`)

> The problem is *when* things happen inside an instruction, not how long the instruction is.
> That is exactly why blargg `instr_timing` passes (asserted at `src/game_boy.rs:188`) while
> `mem_timing` would not.

---

## 1. Timing granularity — the root cause

### How gambatte does it

One free-running T-cycle counter (`cpu.h:76`), timestamped into every access. `counterdef.h`
contains only `enum { disabled_time = 0xfffffffful };` — there is no tick-frequency constant,
because the counter *is* the clock.

```cpp
// cpu.cpp:141-148
#define READ(dest, addr)  do { (dest) = mem_.read(addr, cycleCounter); cycleCounter += 4; } while (0)
#define PC_READ(dest)     do { (dest) = mem_.read(pc, cycleCounter); pc = (pc+1)&0xFFFF; cycleCounter += 4; } while (0)
#define WRITE(addr, data) do { mem_.write(addr, data, cycleCounter); cycleCounter += 4; } while (0)
#define PC_MOD(data)      do { pc = data; cycleCounter += 4; } while (0)
```

Peripherals are lazily caught up *inside* the accessor (`nontrivial_ff_read`, `memory.cpp:557`,
calls `updateOamDma(cc)` / `tima_.tima(cc)` / `lcd_.getStat(..., cc)`), so a read at `cc+8` sees
state at exactly `cc+8`. The instruction body *is* the schedule:

```cpp
// cpu.cpp:854-863 — inc (hl), 12 cycles
case 0x34: { unsigned const addr = hl();
    READ(hf2, addr);          // bus read  @ cc+4
    zf = hf2 + 1;
    WRITE(addr, zf & 0xFF);   // bus write @ cc+8
    hf2 |= hf2_incf; } break;

// cpu.cpp:300-303 — push: idle cycle first, then two writes
#define push_rr(r1,r2) do { cycleCounter += 4; PUSH(r1,r2); } while (0)
```

### How gb does it

The entire instruction runs, then time is handed to the peripherals:

```rust
// src/core.rs:479-503
let cycles = MachineCycles::from_m(opcode.machine_cycles(condition_met));
let interrupt_cycles = match self.mode {
    CoreMode::Normal | CoreMode::Halt => { self.mmu.update(cycles); self.interrupt() }
    CoreMode::Stop => { /* no update at all */ MachineCycles::ZERO }
    CoreMode::Crash => MachineCycles::ZERO,
};
self.mmu.update(interrupt_cycles);
```

Operands are read even earlier, in `Core::fetch` → `OpCode::parse` (`src/core.rs:143`,
`src/opcode.rs:636`). `MMU::read` / `MMU::write` (`src/mmu.rs:285`, `src/mmu.rs:342`) take **no
cycle argument** — there is nowhere to put a timestamp.

### The concrete divergence

| Instruction | gambatte bus schedule (T from start) | gb |
|---|---|---|
| `LD (HL),A` 8 T | fetch@0, write@4 | write@0, +8 T after |
| `INC (HL)` 12 T | fetch@0, read@4, write@8 | read@0, write@0, +12 T after |
| `PUSH BC` 16 T | fetch@0, idle@4, w@8, w@12 | both writes @0 |
| `CALL nn` 24 T | fetch@0, imm@4, imm@8, idle@12, w@16, w@20 | all five @0 |
| `LDH A,(n)` 12 T | I/O read @8 | @0 — **8 T early** |
| `LD A,(nn)` 16 T | read @12 | @0 — **12 T early** |
| `SET n,(HL)` 16 T | read@8, write@12 | both @0 |

Second-order error: because `mmu.update` runs *after* the accesses, an instruction can never
observe an event caused by its own cycles — hardware reads on the last M-cycle and can. Third:
`MMU::update` (`src/mmu.rs:216-249`) advances DMA → serial → divider → timer → PPU → audio for the
whole delta in that fixed order, so intra-step inter-peripheral ordering is arbitrary.

### Tasks

- [ ] Add `tick()` / `bus_read()` / `bus_write()` helpers on `Core` that advance the MMU one
      M-cycle *before* each access (the eager convention mooneye assumes).
- [ ] Migrate opcode families incrementally, keeping `OpCode::machine_cycles` as a
      `debug_assert!` oracle so blargg `instr_timing` keeps passing throughout.
- [ ] Move operand fetch out of `OpCode::parse` into the timed path.

```rust
#[inline] fn tick(&mut self) { self.mmu.update(MachineCycles::ONE); }
#[inline] fn bus_read(&mut self, a: u16) -> u8 { self.tick(); self.mmu.read(a) }
#[inline] fn bus_write(&mut self, a: u16, v: u8) { self.tick(); self.mmu.write(a, v) }

OpCode::Increment { register: mHL } => {
    let addr = self.registers.hl();
    let v = self.bus_read(addr);              // M2
    let r = self.alu_increment(v);
    self.bus_write(addr, r);                  // M3
}
OpCode::Push { register } => {
    self.tick();                              // M2 internal
    let v = self.register16_stack(register);
    self.registers.sp = self.registers.sp.wrapping_sub(1);
    self.bus_write(self.registers.sp, (v >> 8) as u8);   // M3
    self.registers.sp = self.registers.sp.wrapping_sub(1);
    self.bus_write(self.registers.sp, v as u8);          // M4
}
```

- [ ] **Cheap interim option** if the full refactor is off the table: keep `execute` atomic but
      call `self.mmu.update(from_m(n - 1))` *before* the final bus access of the instruction. ~30
      lines; fixes `LDH A,(n)`, `LD A,(nn)`, `LD (nn),A`, `INC (HL)` and most of `mem_timing`.

---

## 2. STOP, illegal opcodes, and two live bugs

### 2a. STOP is wrong in three ways

```cpp
// cpu.cpp:613-621 + memory.cpp:390-392
case 0x10:
    PC_READ(opcode_);                                      // eats the second byte
    cycleCounter = mem_.stop(cycleCounter - 4, prefetched_);
// Memory::stop: intreq_.setEventTime<intevent_unhalt>(cc + 0x20000 + 4);
```

```rust
// src/opcode.rs:653 — no second byte fetched
0x10 => OpCode::Stop,
// src/core.rs:461-464, 486-493
OpCode::Stop => { self.mode = CoreMode::Stop; self.mmu.stop(); }
CoreMode::Stop => {
    if self.mmu.joypad().is_activation_pending() { self.mode = CoreMode::Normal; }
    MachineCycles::ZERO
}
```

1. **PC is left on the pad byte** — STOP is a 2-byte instruction.
2. **Immediate wake**, with no 131 072-cycle delay.
3. **`MMU::restart()` is never called from anywhere.** Verified:
   `grep -rn "restart" --include=*.rs src/` finds only the definition at `src/mmu.rs:210` and an
   unrelated RST *test* name at `src/core.rs:2032`.

Since `MMU::stop()` (`src/mmu.rs:205-208`) calls `divider.disable()` and `timer.disable()`, and the
APU frame sequencer is clocked from `div_clocks` (`src/mmu.rs:231-234`), **after any STOP+wake DIV
is pinned at 0, TIMA is frozen, and all APU length/envelope/sweep clocking dies — permanently.**

Also: `CoreMode::Stop` and `CoreMode::Crash` both return `MachineCycles::ZERO`, so
`GameBoy::run`'s `while cycles < min_cycles` loop **spins forever** if nothing wakes it.

- [ ] Fetch and discard the second STOP byte.
- [ ] Call `MMU::restart()` on wake.
- [ ] Return a non-zero cycle count from the `Stop` / `Crash` arms so `run()` cannot livelock.
- [ ] (Optional) Model the `0x20000 + 4` wake delay.

### 2b. Illegal opcodes freeze the whole machine

Gambatte freezes the **CPU only** — video, audio and DIV keep running:

```cpp
// memory.cpp:344-351
void Memory::freeze(unsigned long cc) {
    nontrivial_ff_write(0xFF, 0, cc);   // IE = 0, so we can never unhalt
    ackDmaReq(intreq_);
    intreq_.halt();
}
```

`gb` kills everything:

```rust
// src/core.rs:472-476
OpCode::Illegal { .. } => {
    println!("Illegal opcode encountered: {:?}", opcode);
    self.mode = CoreMode::Crash;
    self.mmu.stop();          // disables divider AND timer, permanently
}
```

and `CoreMode::Crash` (`src/core.rs:494-497`) never calls `mmu.update` again — PPU, APU and serial
stop dead.

- [ ] Replace with `self.mmu.write(0xFFFF, 0); self.mode = CoreMode::Halt;` and drop the
      `println!`.

---

## 3. Interrupt handling

### How gambatte does it

A 20-T sequence with the vector sampled **mid-push**:

```cpp
// interrupter.cpp:40-70
if (prefetched_) { pc_ = (pc_ - 1) & 0xFFFF; prefetched_ = false; }   // undo prefetch
cc += 12;
sp_ = (sp_ - 1) & 0xFFFF;  memory.write(sp_, pc_ >> 8, cc);   // push high @ +12
cc += 4;
unsigned const pendingIrqs = memory.pendingIrqs(cc);          // SAMPLED HERE
unsigned const n = pendingIrqs & -pendingIrqs;
if (n <= 4) { static unsigned char const lut[] = {0x00,0x40,0x48,0x48,0x50}; address = lut[n]; }
else address = 0x50 + n;                                      // 8→0x58, 16→0x60
sp_ = (sp_ - 1) & 0xFFFF;  memory.write(sp_, pc_ & 0xFF, cc); // push low @ +16
memory.ackIrq(n, cc);                                          // IF cleared @ +16
pc_ = address; cc += 4;                                        // total 20 T
```

`n == 0 → lut[0] == 0x00` **is** the cancelled-interrupt quirk: the high push landed on `0xFFFF`
(the IE register), clearing the pending bit, so the CPU jumps to `0x0000`.

EI's one-instruction delay:

```cpp
// interruptrequester.cpp:56-62
void InterruptRequester::ei(unsigned long cc) {
    intFlags_.setIme();
    minIntTime_ = cc + 1;    // not 4-aligned -> exactly one more instruction runs
    if (pendingIrqs()) eventTimes_.setValue<intevent_interrupts>(minIntTime_);
}
```

`RETI` (`cpu.cpp:1745-1753`) uses the same `ei` with `cc` already 16 T along, so the interrupt
fires *immediately* after `RETI`.

### How gb does it

```rust
// src/core.rs:505-524
fn interrupt(&mut self) -> MachineCycles {
    if let Some(interrupt) = self.mmu.interrupt_pending() {
        if self.mode == CoreMode::Halt { self.mode = CoreMode::Normal; }
        if !self.interrupts_enabled { return MachineCycles::ZERO; }
        self.mmu.clear_interrupt_request(interrupt);   // IF cleared FIRST
        self.interrupts_enabled = false;
        self.call(interrupt.address());                // both pushes, atomically
        MachineCycles::from_m(5)
    } else { MachineCycles::ZERO }
}
```

Priority order is **correct**. `EI; DI` correctly nets out. Note there is a second, **dead**
implementation `MMU::check_interrupts` (`src/mmu.rs:264-281`) which uniquely contains the
STOP/Joypad-only filter the live path lacks.

### Gap

1. **No cancelled-interrupt quirk** — the vector is committed and IF cleared before the push.
2. **Dispatch is atomic**, so the stack writes are 12–16 T early.
3. **`RETI` sets the EI latch, not IME** (`src/core.rs:451-454`), so `interrupt()` at the end of
   `RETI` sees IME = false and the interrupt is one whole instruction late.
4. **IF is OR'd in at the instruction boundary** from a one-shot `Activation` bool
   (`src/mmu.rs:236-248`) — two overflows in one window collapse to one interrupt, whereas
   gambatte's `updateTimaIrq` loops.

### Tasks

- [ ] Rewrite dispatch as a real 5-M-cycle sequence with the vector sampled after the high push.
- [ ] Make `RETI` set IME immediately.
- [ ] Let multiple same-source interrupts in one window each raise IF.
- [ ] Delete the dead `MMU::check_interrupts` (or promote its STOP/Joypad filter into the live
      path).

```rust
fn dispatch_interrupt(&mut self) {
    self.tick(); self.tick();                                  // M1, M2 internal
    let pc = self.registers.pc;
    self.registers.sp = self.registers.sp.wrapping_sub(1);
    self.bus_write(self.registers.sp, (pc >> 8) as u8);        // M3 — may clobber IE at 0xFFFF
    let pending = self.mmu.ie_bits() & self.mmu.if_bits() & 0x1F;
    let n = pending & pending.wrapping_neg();
    let vector: u16 = match n {
        0x01 => 0x40, 0x02 => 0x48, 0x04 => 0x50,
        0x08 => 0x58, 0x10 => 0x60, _ => 0x0000,               // cancelled interrupt
    };
    self.registers.sp = self.registers.sp.wrapping_sub(1);
    self.bus_write(self.registers.sp, pc as u8);               // M4
    if n != 0 { self.mmu.clear_if_bit(n); }
    self.registers.pc = vector;
    self.tick();                                               // M5
    self.interrupts_enabled = false;
}
```

---

## 4. HALT and the HALT bug

Gambatte implements the halt bug via the prefetch flag — it reads the next byte **without
advancing PC**, so it is executed twice:

```cpp
// cpu.cpp:1018-1032
case 0x76:
    opcode_ = mem_.read(pc, cycleCounter);          // pc already past the HALT
    if (mem_.pendingIrqs(cycleCounter)) {
        prefetched_ = true;                         // PC not advanced -> byte executed twice
    } else {
        prefetched_ = mem_.halt(cycleCounter);
        cycleCounter += 4 + 4 * !mem_.isCgb();      // DMG halt costs an extra 4 T
        if (cycleCounter < mem_.nextEventTime()) { /* skip straight to next event */ }
    }
```

`gb` enters `Halt` unconditionally (`src/core.rs:458`), `fetch()` returns a virtual `Nop`
(`src/core.rs:143-151`), and `interrupt()` un-halts *before* checking IME
(`src/core.rs:506-510`) — so `HALT` with IME=0 and a pending interrupt degenerates into a
1-M-cycle NOP with no double read. There is also no extra DMG halt cycle.

**Performance note:** halt *spins* — one virtual NOP plus one `MMU::update(1 M)` per M-cycle,
roughly 17 500 dispatch iterations per halted frame, versus gambatte's single jump.

### Tasks

- [ ] Implement the HALT bug.
- [ ] Add the DMG extra-4-T halt cost.
- [ ] Add a skip-to-next-event fast path while halted.

```rust
OpCode::Halt => {
    let pending = self.mmu.pending_irq_bits() != 0;   // IE & IF & 0x1F
    if !self.interrupts_enabled && pending { self.halt_bug = true; }
    else { self.mode = CoreMode::Halt; }
}
fn fetch_u8(&mut self) -> u8 {
    let b = self.mmu.read(self.registers.pc);
    if self.halt_bug { self.halt_bug = false }
    else { self.registers.pc = self.registers.pc.wrapping_add(1) }
    b
}
```

---

## 5. DIV / timer — highest accuracy win per line

### How gambatte does it

**There is no DIV register.** DIV is a *view* of the global counter
(`memory.cpp:569-570`): `return (cc - tima_.divLastUpdate()) >> 8 & 0xFF;`

TIMA is computed lazily with the period as a shift — `timaClock[] = { 10, 4, 6, 8 }`
(`tima.cpp:26`, i.e. 1024/16/64/256 T) — and the overflow window is explicit:

```cpp
// tima.cpp:78-105 (excerpt)
if (cc >= tmatime_) { if (cc >= tmatime_ + 4) tmatime_ = disabled_time; tima_ = tma_; }
unsigned long tmp = tima_ + ticks;
while (tmp > 0x100) tmp -= 0x100 - tma_;
if (tmp == 0x100) { tmp = 0; tmatime_ = lastUpdate_ + 3;
    if (cc >= tmatime_) { if (cc >= tmatime_ + 4) tmatime_ = disabled_time; tmp = tma_; } }
```

The four write quirks:
- `setTima` cancels a pending reload — `if (tmatime_ - cc < 4) tmatime_ = disabled_time;`
  (`tima.cpp:112`)
- `setTma` re-applies through `updateTima` so a new TMA lands inside the window
  (`tima.cpp:121-128`)
- `setTac` does the glitch increment and re-phases to DIV:
  `lastUpdate_ = cc - ((cc - divLastUpdate_) & ((1u << timaClock[data & 3]) - 1));`
  (`tima.cpp:130-155`)
- `divReset` rewinds half a period to produce the DIV-write glitch tick (`tima.cpp:157-171`)

### How gb does it

Two independent accumulators, neither related to the other:

```rust
// src/timer.rs:49-66
self.cycles += cycles;
let cycles_per_tick = self.mode.cycles_per_tick();
while self.cycles >= cycles_per_tick {
    self.cycles -= cycles_per_tick;
    if self.value == 0xFF { self.value = self.modulo; self.interrupt_pending = true; }
    else { self.value += 1; }
}
```

The periods themselves are right (`src/timer.rs:89-98` = 1024/16/64/256 T). Everything else is not.

| Behaviour | gambatte | gb |
|---|---|---|
| TIMA from DIV-bit falling edge | yes, phase-locked | **no** |
| DIV write → glitch increment | yes | **no** |
| DIV write resets DIV phase | yes | **no** (`Divider::reset` zeroes only the visible byte) |
| TAC write → glitch increment | yes | **no** |
| TAC frequency change re-phases | yes | **no** — `self.cycles` carries over, so a frequency change can emit **up to 63 TIMA increments in one call** |
| TIMA = 0 for 4 T before TMA load | `tmatime_` window | **no**, instantaneous |
| Write TIMA in the window cancels reload | yes | **no** |
| Write TMA in the window takes effect | yes | **no** |
| Interrupt at the exact overflow T | scheduled event | **no**, boundary + one-shot bool |
| `FF07` read-back | `0xF8 \| tac` | **no** — `src/timer.rs:24-26` returns `0x00`–`0x07` |

### Fix sketch — single system counter + edge detector

```rust
pub struct Timer { counter: u16, tima: u8, tma: u8, tac: u8, last_edge: bool, reload: u8 }
const TAC_BIT: [u8; 4] = [9, 3, 5, 7];

fn edge(&self) -> bool {
    self.tac & 4 != 0 && (self.counter >> TAC_BIT[(self.tac & 3) as usize]) & 1 != 0
}
fn detect(&mut self) {
    let now = self.edge();
    if self.last_edge && !now { self.inc_tima(); }
    self.last_edge = now;
}
fn inc_tima(&mut self) {
    let (v, ov) = self.tima.overflowing_add(1);
    self.tima = v;
    if ov { self.reload = 4; }
}

pub fn tick_t(&mut self, irq: &mut InterruptFlags) {
    self.counter = self.counter.wrapping_add(1);
    self.detect();
    if self.reload > 0 {
        self.reload -= 1;
        if self.reload == 0 { self.tima = self.tma; irq.set_interrupt(InterruptType::Timer); }
    }
}
pub fn write_div(&mut self)       { self.counter = 0; self.detect(); }   // glitch tick falls out
pub fn write_tac(&mut self, v: u8){ self.tac = v & 7; self.detect(); }
pub fn read_tac(&self) -> u8      { 0xF8 | self.tac }
pub fn write_tima(&mut self, v: u8){ if self.reload > 0 { self.reload = 0; } self.tima = v; }
pub fn write_tma(&mut self, v: u8) { self.tma = v; if self.reload == 1 { self.tima = v; } }
pub fn div(&self) -> u8           { (self.counter >> 8) as u8 }
```

The APU frame sequencer then hangs off `counter` bit 12 falling edges instead of
`DividerClocks::bit_fall_edge` — which also fixes the DIV-write frame-sequencer bug documented in
[`04-apu.md` §2](04-apu.md#2-frame-sequencer).

### Tasks

- [ ] Replace `Divider` + `Timer` with a single 16-bit system counter and a falling-edge detector.
- [ ] Implement the 4-cycle TIMA reload window and its three write quirks.
- [ ] Make `Divider::reset` clear the sub-tick phase.
- [ ] Return `0xF8 | tac` from `FF07`.

> ⚠️ **Fixture impact.** This changes the bincode layout of `Divider` and `Timer` and invalidates
> every `src/pokemon/data/*.bin` fixture. Plan the regen: run the affected legs **in chain order**
> with `--features slow-tests,regen-fixtures`. See [`01-architecture.md`](01-architecture.md) for a
> savestate-versioning scheme that would remove this class of pain permanently.

---

## 6. STOP / KEY1 / double speed

Gambatte's `Memory::stop` (`memory.cpp:390-420`) does `tima_.speedChange()`, a DIV reset via
`nontrivial_ff_write(0x04, 0, cc)`, `psg_.speedChange`, `lcd_.speedChange`,
`ioamhram_[0x14D] ^= 0x81`, re-arms `intevent_blit`, and schedules unhalt at `cc + 0x20000 + 4`.
`FF4D` accepts only bit 0, and only on CGB (`memory.cpp:991-995`).

`gb` has none of it — `0xFF4D` is not in `MMU::read`/`write` at all (falls to `_ => 0xFF` at
`src/mmu.rs:333`, ignored at `src/mmu.rs:388`), and `MachineCycles::CPU_FREQ` is a hard constant
(`src/cycles.rs:11`).

Low priority for a DMG-only agent. But note the shape of the constraint: **if double speed is ever
wanted, `MachineCycles` must stop being "the" clock** — everything moves to T-cycles with a speed
shift (`<< is_double_speed()`).

- [ ] (Deferred) KEY1 + double-speed clock scaling.

---

## 7. OAM DMA and bus conflicts

Gambatte **disconnects** the conflicting areas from the fast pointer table during DMA
(`disconnectOamDmaAreas`, `memptrs.cpp:47-56`) so accesses fall to the slow path, which returns the
in-flight byte and locks OAM:

```cpp
// memory.cpp:624-632, 659-660
if (cart_.isInOamDmaConflictArea(p) && oamDmaPos_ < oam_size) { /* ... */ return r; }
if (!lcd_.oamReadable(cc) || oamDmaPos_ < oam_size) return 0xFF;
```

The transfer is incremental, one byte per 4 T (`updateOamDma`, `memory.cpp:493-514`).

`gb`:

```rust
// src/lcd_dma.rs:14-31 — state cleared BEFORE the copy
if state.cycles >= DMA_TRANSFER_CYCLES {
    let transfer = /* ... */; self.state = None; Some(transfer)
}
// src/mmu.rs:221-227
for i in 0..0xA0 { let value = self.read(transfer.address + i); self.ppu.write_oam(i, value); }
// src/ppu.rs:109-121 — gates INVERTED
if self.lcd_status.mode().oam_accessible() || self.dma.is_active() { /* ... */ }
```

### Latent data-loss bug

By the time the copy loop runs, `dma.is_active()` is already `false`, so `write_oam` falls back to
`lcd_status.mode().oam_accessible()` (= `HBlank || VBlank`, `src/lcd_status.rs:107-109`) using the
mode from the *previous* step (`ppu.update` runs later, `src/mmu.rs:233`). **If that mode is 2 or
3, the entire 160-byte transfer is silently discarded.** Pokémon Red always DMAs in VBlank, which
is the only reason this has never shown up.

Also: `|| self.dma.is_active()` makes VRAM (`src/ppu.rs:90`, `:103`) and OAM *more* accessible
during DMA — the opposite of hardware; there is no CPU HRAM restriction; and the source mask
`((value & 0xDF) as u16) << 8` (`src/lcd_dma.rs:11`) clears bit 5 of the page, silently
redirecting `$20xx`→`$00xx`, `$A0xx`→`$80xx`, `$E0xx`→`$C0xx`. See
[`05-mmu-cartridge.md` §5](05-mmu-cartridge.md#5-oam-dma) for the full source-classification fix.

### Tasks

- [ ] Make the copy incremental with a `pos` field; keep `is_active()` true for the full 160
      M-cycles.
- [ ] Give the DMA its own `write_oam_dma` that bypasses the mode gate.
- [ ] Invert the CPU gates to `mode.oam_accessible() && !dma.is_active()`.

---

## 8. Dispatch loop and memory hot path

**gambatte:** three nested loops with a cycle budget, a flat `switch` on a raw `unsigned char`, and
hot registers as locals:

```cpp
// cpu.cpp:511-537 (excerpt)
while (mem_.isActive()) {
    unsigned short pc = pc_;
    if (mem_.halted()) { /* skip to nextEventTime */ }
    else while (cycleCounter < mem_.nextEventTime()) {
        unsigned char opcode;
        if (!prefetched_) { PC_READ(opcode); }
        else { opcode = opcode_; cycleCounter += 4; prefetched_ = false; }
        switch (opcode) { /* ... */ }
    }
    pc_ = pc;
    cycleCounter = mem_.event(cycleCounter);
}
```

No peripheral is touched until an event is due or an access needs a lazily-caught-up value. Flags
are **lazy** (`hf1/hf2/zf/cf`, materialised only by `toF()` at `cpu.cpp:75-78`). The bus is a
pre-biased pointer table (`memory.h:76-85`).

**gb** (`src/game_boy.rs:29-36`): two-stage dispatch — `OpCode::parse`
(`src/opcode.rs:636-758`, nested matches on `x()/y()/z()/p()/q()`) builds an enum, then `execute`
re-matches on it, **every single execution**. No cycle budget: `MMU::update` unconditionally
touches DMA, serial, divider, timer, PPU and audio, then loops all five `InterruptType`s calling
`consume_pending_activation` (`src/mmu.rs:216-249`) — per instruction. Flags are four eager `bool`s
(`src/registers.rs:4-9`). `MMU::read` is a 20+-arm range `match` with guards that re-read
`self.header.ram_banks()`.

### Tasks (performance, ranked)

- [ ] Skip-to-next-event while halted (biggest single win; see §4).
- [ ] A `[u8; 0x10]`-style page table for read/write, replacing the range `match`.
- [ ] Fuse decode and execute into one `match raw_opcode`, keeping `OpCode` purely for the
      disassembler and debug UI.

See [`01-architecture.md`](01-architecture.md) for the full performance analysis.

---

## Suggested implementation order

1. [ ] Thread a clock through the bus (§1) — everything depends on it. Migrate incrementally with
       `machine_cycles` as a `debug_assert!` oracle so `instr_timing` keeps passing.
2. [ ] Rewrite the timer as a DIV-derived edge detector (§5) — ~120 lines, biggest accuracy win
       per line. Budget a fixture regen.
3. [ ] Fix STOP (§2a): consume the pad byte, call the dead `MMU::restart()`, return non-zero
       cycles.
4. [ ] Fix illegal opcodes (§2b): halt the CPU, not the machine.
5. [ ] Interrupt dispatch as a real 5-M-cycle sequence + immediate IME on `RETI` (§3).
6. [ ] HALT bug + skip-to-next-event fast path (§4).
7. [ ] OAM DMA: incremental copy, un-invert the gates (§7) — the silent-drop path is a real latent
       bug independent of any accuracy work.
8. [ ] Add the missing test ROMs — see [`07-testing.md`](07-testing.md).

---

## References

- Pan Docs — <https://gbdev.io/pandocs/> (CPU Instruction Set, Interrupts, Timer chapters)
- gbops opcode table — <https://izik1.github.io/gbops/>
- The Cycle-Accurate Game Boy Docs (AntonioND) —
  <https://github.com/AntonioND/giibiiadvance/blob/master/docs/TCAGBD.pdf>
- mooneye-gb test suite — <https://github.com/Gekkio/mooneye-test-suite>
- blargg's test ROMs — <https://github.com/retrio/gb-test-roms>
