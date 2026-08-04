# PPU / LCD compatibility guide

Comparison of `gb` (`/home/alex/projects/gb`, Rust) against **gambatte**
(`/home/alex/projects/gambatte`, C++) — `libgambatte/src/video.cpp`, `video.h` and
`libgambatte/src/video/*`.

Gambatte's PPU is generally regarded as the most accurate open-source DMG/CGB video
implementation; it is the reference throughout. All `gb` line numbers were read on
2026-08-04 on branch `roms`.

---

## Ranked gap summary

| # | Gap | Severity | Effort | Section |
|---|---|---|---|---|
| 1 | **No pixel pipeline**, and the x-advance is **quadratic** — a genuine bug at `src/ppu.rs:230` | Critical | Low to fix the bug / High to replace | [§1](#1-rendering-model) |
| 2 | Mode 3 fixed at 172 T, mode 0 fixed at 204 T — no SCX&7, window or sprite penalties | Critical | High | [§2](#2-mode-timing) |
| 3 | STAT is a sticky latch, not an OR-ed level with edge detection — no blocking, no DMG STAT-write bug, no mode-2 IRQ on line 144, no LY=153 quirk | Critical | Medium | [§3](#3-stat-interrupts) |
| 4 | LCD enable/disable is an early `return` — LY/mode freeze instead of resetting | High | Medium | [§4](#4-lcd-enabledisable) |
| 5 | Register writes are instruction-granular and un-offset | High | Medium | [§5](#5-mid-scanline-register-writes) |
| 6 | Window: no WX<7 shift, no WX=166 next-line latch, WY sampled at the wrong time | Medium | Medium | [§6](#6-window) |
| 7 | VRAM/OAM blocking is mode-derived; **OAM-DMA blocking is inverted**; DMA is one bulk copy | Medium | Low–Med | [§7](#7-vramoam-access-blocking-and-oam-dma) |
| 8 | Zero CGB video (no VRAM bank 1, BG attributes, colour palette RAM, HDMA, OPRI) | Medium | High | [§8](#8-cgb-video-inventory) |
| 9 | No frame-done signal — the host samples the LCD on a wall-clock timer | Low | Low | [§9](#9-frame-output) |
| 10 | OAM scan is atomic at end of mode 2 | Low | Medium | [§10](#10-small-cheap-fidelity-items) |
| 11 | STAT bit 7 not forced to 1; `FF46` reads 0; no post-boot state | Low | Trivial | [§10](#10-small-cheap-fidelity-items) |

**Headline:** the single highest-value change in this document is a **one-line fix** at
`src/ppu.rs:230`, worth roughly 4.5× in raster fidelity.

`dmg-acid2` currently passes (asserted at `src/game_boy.rs:336`) only because it is a static
single-frame test that exercises none of the above. Sprites are the best-modelled area — the
10-per-line limit, OAM-order selection and DMG X-priority are all **already correct**.

---

## 1. Rendering model

### How gambatte does it

`ppu.cpp` is a cycle-driven continuation state machine. `PPUPriv` holds a `long cycles` budget and
a `PPUState const *nextCallPtr`; each state is a struct of a function plus a cycle predictor
(`video/ppu.h:55-59`).

```cpp
// video/ppu.cpp:1815-1825
void PPU::update(unsigned long const cc) {
    long const cycles = (cc - p_.now) >> p_.lyCounter.isDoubleSpeed();
    p_.now   += cycles << p_.lyCounter.isDoubleSpeed();
    p_.cycles += cycles;
    if (p_.cycles >= 0) {
        p_.framebuf.setFbline(p_.lyCounter.ly());
        p_.nextCallPtr->f(p_);
    }
}
```

```cpp
// video/ppu.cpp:130-139
inline void nextCall(int const cycles, PPUState const &state, PPUPriv &p) {
    int const c = p.cycles - cycles;
    if (c >= 0) { p.cycles = c; return state.f(p); }
    p.cycles = c;
    p.nextCallPtr = &state;
}
```

The line decomposes into `M2_Ly0` / `M2_LyNon0` (WY-latch checks), `M3Start::f0/f1` (the SCX&7
discard), then a `M3Loop` of `Tile::f0..f5` (the 6-cycle fetcher: map / data-lo / data-hi),
`LoadSprites::f0..f5`, and `StartWindowDraw::f0..f5`. `Tile::f5` shifts out one pixel per cycle:

```cpp
// video/ppu.cpp:887-891
    if (xpos - tile_len >= 0)
        fbline[xpos - tile_len] = pixel;
    p.xpos = xpos + 1;
    p.tileword = tileword >> tile_bpp;
```

`xpos` runs 0..168 (`xpos_end = 168`, `ppu.cpp:115`) — the first 8 discarded pixels *are* the FIFO
prefill. `doFullTilesUnrolledDmg` / `doFullTilesUnrolledCgb` (`ppu.cpp:349`, `ppu.cpp:547`) are a
fast path used only when no sprite or window event lands in the next whole tile; each consumes
exactly `tile_len` cycles so it stays cycle-identical to the slow path.

Crucially, every state also carries a **predictor** — `predictCyclesUntilXpos_f*`
(`ppu.cpp:1240-1530`) — answering "how many cycles until `xpos` reaches N?" *without* running the
pipeline. That is what makes cycle accuracy affordable; `next_m0_time.cpp:4-6` uses it to schedule
the mode-0 interrupt.

### How gb does it

`PPU::update` (`src/ppu.rs:196-306`) is a single `match` on the current mode, called once per CPU
instruction with that whole instruction's cycle count:

```rust
// src/ppu.rs:196-207
pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
    if !self.lcd_control.is_enabled() { return }
    self.current_ticks += delta_machine_cycles.t_cycles();
    match self.lcd_status.mode() {
        LcdMode::OAM => { if self.current_ticks >= OAM_TICKS { /* ... */ } }
```

```rust
// src/ppu.rs:222-232
LcdMode::Drawing => {
    let drawing_ticks = INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH;   // 12 + 160 = 172, constant
    if self.current_ticks >= drawing_ticks {
        self.lcd_status.set_mode(LcdMode::HBlank);
        self.current_ticks -= drawing_ticks;
    } else if self.current_ticks >= INITIAL_FIFO_LOAD_TICKS {
        let start_x = self.current_x;
        let end_x = start_x + self.current_ticks - INITIAL_FIFO_LOAD_TICKS + 1;
```

then `for x in start_x..end_x { ... self.lcd[y * LCD_WIDTH + x] = color; }`
(`src/ppu.rs:238-271`) and `self.current_x = end_x`. It is driven from `Core::execute` →
`self.mmu.update(cycles)` (`src/core.rs:479-483`) → `self.ppu.update(delta)` (`src/mmu.rs:233`).

### Gap

**(a) Quadratic x-advance — a real, confirmed bug.** `end_x` mixes a *relative* base (`start_x`,
which is the previous `end_x`) with an *absolute* offset (`current_ticks - 12`). `current_ticks`
is not reset within mode 3, so the offset is re-added on every call. Traced with 4-T `NOP`s
entering mode 3 at `current_ticks = 0`:

| `current_ticks` | 12 | 16 | 20 | 24 | 28 | 32 | 36 | 40 | 44 | 48 |
|---|---|---|---|---|---|---|---|---|---|---|
| `current_x` after | 1 | 6 | 15 | 28 | 45 | 66 | 91 | 120 | 153 | 190 |

All 160 pixels are emitted by roughly T+36 of mode 3 — about **4.5× too fast**. The
`if x < LCD_WIDTH` guard at `src/ppu.rs:239` silently swallows the overshoot, which is why nothing
visibly broke. The practical consequence: **any register write landing more than ~36 cycles into
mode 3 is a no-op for that scanline.**

**(b) The `>= drawing_ticks` branch never draws.** Crossing 172 flips straight to HBlank without
emitting the remaining pixels. This is masked only by (a) having already drawn them.

**(c) Granularity is a whole instruction (4–24 T).** Even with (a) fixed, a `LD (HL),A` that writes
`SCX` cannot take effect part-way through its own 8 cycles, and the PPU sees the write applied
*before* the cycles it belongs to.

Gambatte resolves to 1 T-cycle. `gb` resolves to one instruction, and inside mode 3 the pixel
position is off by up to 124 pixels.

### Symptom / failing tests

- gambatte `test/hwtests/scx_during_m3/*` — `scx_during_m3_spx0/1/2`, `scx1_scx0_during_m3_1`,
  `scx_attrib_during_m3_*`
- gambatte `test/hwtests/dmgpalette_during_m3/*`
- mooneye `acceptance/ppu/hblank_ly_scx_timing-GS`
- Commercial: **Prehistorik Man**, **Road Rash**, **Pinball Deluxe/Fantasies**,
  **Star Trek: 25th Anniversary**, **Toy Story**

### Tasks

- [ ] **Fix `end_x`** at `src/ppu.rs:230` to be absolute:
      `let end_x = self.current_ticks - INITIAL_FIFO_LOAD_TICKS;`
      ⚠️ This changes every rendered frame — see [Fixture warning](#fixture-warning) below.
- [ ] Make the `>= drawing_ticks` branch flush any pixels `current_x..160` before switching to
      HBlank.
- [ ] Step the PPU in 4-T units from `MMU::update` rather than one whole-instruction lump.
- [ ] Introduce an `xpos: u8` (0..168) with explicit fetcher sub-states mirroring `Tile::f0..f5`
      and `LoadSprites::f0..f5`.
- [ ] Keep an unrolled whole-tile fast path (as gambatte does) so the ~23× realtime budget the
      Pokémon integration tests depend on survives.
- [ ] Add a `predict_cycles_until_xpos` equivalent only if mode-0 IRQ scheduling shows up hot in a
      profile.

#### Fix sketch

```rust
// src/ppu.rs — minimal correction, no pipeline rewrite
LcdMode::Drawing => {
    let drawing_ticks = INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH;
    // Pixels 0..n have been shifted out once `current_ticks` reaches 12 + n.
    let target_x = self.current_ticks
        .saturating_sub(INITIAL_FIFO_LOAD_TICKS)
        .min(LCD_WIDTH);
    self.draw_pixels(self.current_x..target_x);   // was the `for x in start_x..end_x` body
    self.current_x = target_x;

    if self.current_ticks >= drawing_ticks {
        self.lcd_status.set_mode(LcdMode::HBlank);
        self.current_ticks -= drawing_ticks;
    }
}
```

#### Fixture warning

`gb` commits 27 emulator snapshots under `src/pokemon/data/*.bin` and asserts screenshots. Any
change in this section alters rendered output and *when* pixels appear. Before landing:

1. Run the default tier (`cargo test --release`) and record which screenshot assertions move.
2. Regenerate affected fixtures **in chain order** with
   `cargo test --release --features slow-tests,regen-fixtures --bin gb -- <leg> --exact`
   (see `CLAUDE.md`; the `regen-fixtures` gate exists precisely for this).
3. Re-check the `dmg-acid2` expectation at `src/game_boy.rs:336`.

---

## 2. Mode timing

### How gambatte does it

Constants live in `video/lcddef.h:23-30`. Mode 3 starts at `m3StartLineCycle = 83 + cgb`
(`ppu.cpp:128`) and **has no stored length** — it ends when `xpos` reaches 168, so every stall
lengthens it implicitly. Mode 0 is simply whatever remains of the 456-cycle line.

**SCX & 7 fine-scroll discard:**

```cpp
// video/ppu.cpp:329-343 (abridged)
    void f1(PPUPriv &p) {
        while (p.xpos < max_m3start_cycles) {
            if (p.xpos % tile_len == p.scx % tile_len) break;
            /* ... */
        }
        p.xpos = 0;
        p.endx = tile_len - p.scx % tile_len;
        static PPUState const *const flut[] = { &Tile::f0_, /* ... */ &Tile::f5_ };
        nextCall(1 - p.cgb, *flut[p.scx % tile_len], p);
    }
```

The predictor mirrors it exactly (`ppu.cpp:1464`):
`cycles += std::min((p.scx - xpos) % tile_len, max_m3start_cycles - xpos) + 1 - p.cgb;`

**Window activation costs 6 cycles:**

```cpp
// video/ppu.cpp:1302-1308
    if (p.wx - 1u * xpos < targetx - 1u * xpos
            && lcdcWinEn(p) && (weMaster || p.wy2 == ly)
            && !(winDrawState & win_draw_started)
            && (p.cgb || p.wx != lcd_hres + 6)) {
        nwx = p.wx;
        cycles += 6;
    }
```

**Sprites cost 6 cycles each, up to 11 for the first in a tile:**

```cpp
// video/ppu.cpp:1261-1282 (abridged)
    for (; nextSprite < spriteEnd && spxOf[*nextSprite] <= maxSpx; ++nextSprite) {
        int cycles = 6;
        int const distanceFromTileStart = (spxOf[*nextSprite] - firstTileXpos) % tile_len;
        unsigned const tileNo = (spxOf[*nextSprite] - firstTileXpos) & -tile_len;
        if (distanceFromTileStart < 5 && tileNo != prevSpriteTileNo)
            cycles = 11 - distanceFromTileStart;
        prevSpriteTileNo = tileNo;
        sum += cycles;
    }
```

`lastM0Time` is recorded when the pipeline finishes (`ppu.cpp:915-922`). Mode 2 is not a live scan
loop — `namespace M2` (`ppu.cpp:163-220`) is commented out; the sprite list comes from
`SpriteMapper` and mode 2's duration is baked into `m3StartLineCycle`. The *observed* mode is
reconstructed on demand in `LCD::getStat`:

```cpp
// video.cpp:756-762
    } else if (lineCycles < 77 || lineCycles >= lcd_cycles_per_line - 3) {
        if (!ppu_.inactivePeriodAfterDisplayEnable(cc + 1)) stat = 2;
    } else if (cc + 2 < m0TimeOfCurrentLine(cc)) {
        if (!ppu_.inactivePeriodAfterDisplayEnable(cc + 1)) stat = 3;
    }
```

### How gb does it

```rust
// src/ppu.rs:452-454
const OAM_TICKS: usize = 80;
const INITIAL_FIFO_LOAD_TICKS: usize = 12;
const SCANLINE_TICKS: usize = 456;
```

```rust
// src/ppu.rs:275-278
LcdMode::HBlank => {
    // TODO vary the length of the HBlank period based on the length of the Drawing phase
    let hblank_ticks = SCANLINE_TICKS - OAM_TICKS - INITIAL_FIFO_LOAD_TICKS - LCD_WIDTH; // 204
```

### Gap

`gb` models exactly the **minimum** mode 3 (172) and the **maximum** mode 0 (204), always. Real
mode 3 ranges ~172–289 T. Specifically:

- SCX&7 is never charged (`scroll.x` is only read in `bg_pixel`, `src/ppu.rs:345`)
- Window activation is never charged
- Sprites cost nothing — `scanline_sprites` is built once at the end of mode 2
  (`src/ppu.rs:212-219`)

So the mode-0 STAT interrupt always fires at a constant line cycle 252, instead of somewhere in
252–369.

### Symptom / failing tests

- mooneye `acceptance/ppu/intr_2_mode0_timing`, `intr_2_mode0_timing_sprites`,
  `intr_2_mode3_timing`, `hblank_ly_scx_timing-GS`
- gambatte `scx_during_m3/scx_m3_extend_{1,2}_dmg08_cgb04c_out{3,0}.asm`
- gambatte `sprites/10spritesPrLine_10xposA6_m0irq_*`, `10spritesPrLine_1xpos0_m3stat_*`
- gambatte `m0enable/disable_scx1_*` … `disable_scx4_*`, `window/*`

### Tasks

- [ ] Charge `scx & 7` cycles in the mode-3 start discard.
- [ ] Charge 6 cycles on window activation.
- [ ] Charge `max(11 - (spx - xpos), 6)` for the first sprite in a tile and 6 for each subsequent
      sprite.
- [ ] Derive `hblank_ticks = SCANLINE_TICKS - OAM_TICKS - actual_mode3_ticks` instead of the
      constant at `src/ppu.rs:277`.
- [ ] Store a `last_m0_time` so the mode-0 STAT source can be scheduled from it.

These all fall out naturally once §1's `xpos` pipeline exists; attempting them before that is
wasted effort.

---

## 3. STAT interrupts

### How gambatte does it

STAT is five scheduled events on a `MinKeeper` (`video.h:151-158`), dispatched in `LCD::event()`
(`video.cpp:793-855`). **Blocking** lives in `MStatIrqEvent`, which remembers the *previous* STAT
value so it can detect a rising edge of the OR-ed condition:

```cpp
// video/mstat_irq.h:28-51
    bool doM0Event(unsigned ly, unsigned statReg, unsigned lycReg) {
        bool const flagIrq = ((statReg | statReg_) & lcdstat_m0irqen)
            && (!(statReg_ & lcdstat_lycirqen) || ly != lycReg_);
        lycReg_ = lycReg; statReg_ = statReg; return flagIrq;
    }
    bool doM1Event(unsigned statReg) {
        bool const flagIrq = (statReg & lcdstat_m1irqen)
            && !(statReg_ & (lcdstat_m2irqen | lcdstat_m0irqen));
        statReg_ = statReg; return flagIrq;
    }
```

Symmetrically for LYC (`video/lyc_irq.cpp:40-44`):

```cpp
bool lycIrqBlockedByM2OrM1StatIrq(unsigned ly, unsigned statreg) {
    return ly <= lcd_vres && ly > 0 ? statreg & lcdstat_m2irqen
                                    : statreg & lcdstat_m1irqen;
}
```

The LYC interrupt is scheduled **2 cycles before** the line boundary, and for LYC=0 at line 153+6
— the LY=153 quirk is encoded directly in the schedule:

```cpp
// video/lyc_irq.cpp:31-38
unsigned long schedule(unsigned statReg, unsigned lycReg,
                       LyCounter const &lyCounter, unsigned long cc) {
    return (statReg & lcdstat_lycirqen) && lycReg < lcd_lines_per_frame
    ? lyCounter.nextFrameCycle(lycReg ? 1l * lycReg * lcd_cycles_per_line - 2
                                      : (lcd_lines_per_frame - 1l) * lcd_cycles_per_line + 6, cc)
    : 1 * disabled_time;
}
```

**The DMG STAT-write bug.** Note the signature of `statChangeTriggersStatIrqDmg(unsigned old,
unsigned long cc)` — `data` is *not* a parameter. Any write to `FF41` on DMG fires a STAT interrupt
unless a source is already asserting:

```cpp
// video.cpp:583-602 (abridged)
inline bool LCD::statChangeTriggersStatIrqDmg(unsigned const old, unsigned long const cc) {
    LyCnt const lycCmp = getLycCmpLy(ppu_.lyCounter(), cc);
    if (ppu_.lyCounter().ly() < lcd_vres) {
        if (m0IrqTime == disabled_time || m0IrqTime < ppu_.lyCounter().time())
            return lycCmp.ly == lycIrq_.lycReg() && !(old & lcdstat_lycirqen);
        return !(old & lcdstat_m0irqen)
            && !(lycCmp.ly == lycIrq_.lycReg() && (old & lcdstat_lycirqen));
    }
    return !(old & lcdstat_m1irqen)
        && !(lycCmp.ly == lycIrq_.lycReg() && (old & lcdstat_lycirqen));
}
```

A second half of the same bug is handled at `memory.cpp:945-951` for STAT writes while the LCD is
off.

**LY=153 / early-LY=0** appears in three places — the LY register read (`video.h:113-133`), the
LYC comparison value (`video.cpp:550-562`), and the flag's 2-cycle guard:

```cpp
// video.cpp:550-562
LyCnt const getLycCmpLy(LyCounter const &lyCounter, unsigned long cc) {
    unsigned ly = lyCounter.ly();
    int timeToNextLy = lyCounter.time() - cc;
    if (ly == lcd_lines_per_frame - 1) {
        int const lineTime = lyCounter.lineTime();
        if ((timeToNextLy -= (lineTime - 6 - 6 * lyCounter.isDoubleSpeed())) <= 0)
            ly = 0, timeToNextLy += lineTime;
    } else if ((timeToNextLy -= (2 + 2 * lyCounter.isDoubleSpeed())) <= 0)
        ++ly, timeToNextLy += lyCounter.lineTime();
    return LyCnt(ly, timeToNextLy);
}
```

```cpp
// video.cpp:764-766
    if (lycReg == lycCmp.ly && lycCmp.timeToNextLy > 2) stat |= lcdstat_lycflag;
```

**Mode-2 IRQ on line 144.** `mode2IrqSchedule` (`video.cpp:78-88`) uses `456-4` normally and
`456-2` for LY 0; `doMode2IrqEvent` (`video.cpp:772-791`) explicitly handles `ly == lcd_vres` — the
mode-2 STAT interrupt *does* occur on the VBlank line. LYC writes get their own edge analysis in
`lycRegChangeTriggersStatIrq` (`video.cpp:698-714`), including the "simultaneous LY/LYC increment →
flag never goes low → no trigger" case.

### How gb does it

```rust
// src/lcd_status.rs:44-58
pub fn set_mode(&mut self, mode: LcdMode) {
    if self.mode == mode { return; }
    self.mode = mode;
    // check interrupt
    // TODO emulate STAT blocking
    self.interrupt_pending |= match mode {
        LcdMode::HBlank  => self.hblank_interrupt,
        LcdMode::VBlank  => self.vblank_interrupt,
        LcdMode::OAM     => self.oam_interrupt,
        LcdMode::Drawing => false
    };
}
```

```rust
// src/lcd_status.rs:21-29, 77-79
pub fn increment_ly(&mut self) -> u8 {
    self.ly += 1;
    if self.ly > 153 { self.ly = 0; }
    self.check_lyc_interrupt();
    self.ly
}
fn check_lyc_interrupt(&mut self) {
    self.interrupt_pending |= self.lyc_interrupt && self.lyc == self.ly;
}
```

`interrupt_pending` is drained once per `MMU::update` into `IF` (`src/mmu.rs:237-248`).

### Gap

1. **No blocking, no OR-ed level.** Each source independently sets a latch, so two sources on one
   line produce two interrupts. Hardware ORs the enabled conditions and fires only on the rising
   edge.
2. **No DMG STAT-write bug.** `set_stat` (`src/lcd_status.rs:69-75`) writes four bits and nothing
   else.
3. **No mode-2 STAT IRQ on line 144** — `src/ppu.rs:284-291` goes `HBlank → VBlank` directly.
4. **No LY=153 quirk.** `increment_ly` holds 153 for a full line; both the LY read
   (`src/mmu.rs:323`) and the LYC compare are wrong there.
5. **LYC flag computed live** in `stat()` (`src/lcd_status.rs:60-67`) with no ±2-cycle guard.
6. **LYC compared only at LY-increment and LYC-write** — no "2 cycles before the boundary"
   schedule.
7. **No interrupt ordering** — one latch drained after the instruction; ordering is whatever the
   `match` arm order happens to be.
8. **STAT bit 7 not forced to 1** (`src/lcd_status.rs:60`, `src/mmu.rs:320`).

### Symptom / failing tests

- mooneye `acceptance/ppu/stat_irq_blocking`, `stat_lyc_onoff`, `intr_1_2_timing-GS`,
  `intr_2_0_timing`, `vblank_stat_intr-GS`
- gambatte `miscmstatirq/lycflag_statwirq_{1..4}_dmg08_out*.asm`
- gambatte's 16 `lycstatwirq_trigger_XX_YY_dmg08_outN_cgb04c_outM.asm` — the DMG-write-bug matrix;
  DMG and CGB have *different* expectations, which is the whole point of the test
- gambatte `lyc0int_m0irq/*`, `lyc153int_m2irq/*` (incl. `late_retrigger`), `lycint_lycflag/*`,
  `lycint_ly/*`, `lycint_lycirq/*`, `lycEnable/{early_ff41_response_*, early_ff45_response_*}`,
  `m0int_m0irq/*`, `m2int_m0irq/*`, `m2int_m2irq/*`, `m0enable/*`, `m2enable/*`,
  `lcdirq_precedence/*`, `irq_precedence/*`, `ly0/*`, `lywrite/*`

### Tasks

- [ ] Split `LcdStatus` into the register value plus an explicit `stat_line: bool` holding the OR
      of the four enabled conditions.
- [ ] Raise `IF.1` only on a rising edge: `if !prev_line && line { request_stat_irq() }`.
- [ ] Add the mode-2 STAT source on line 144.
- [ ] Implement the LY=153 early-0 rule in **both** the LY register read and the LYC comparison.
- [ ] Add a DMG-only side effect in `set_stat` that pulses the STAT line — with the edge model in
      place this reproduces the DMG STAT-write bug for free.
- [ ] Force STAT bit 7 to 1 on read.

#### Fix sketch

```rust
// src/lcd_status.rs
fn stat_line(&self) -> bool {
    (self.lyc_interrupt    && self.lyc == self.ly)
        || (self.hblank_interrupt && self.mode == LcdMode::HBlank)
        || (self.vblank_interrupt && self.mode == LcdMode::VBlank)
        || (self.oam_interrupt    && (self.mode == LcdMode::OAM
                                      || self.mode == LcdMode::VBlank)) // mode 2 fires on line 144
}

fn refresh_stat_line(&mut self) {
    let line = self.stat_line();
    if line && !self.prev_stat_line {
        self.interrupt_pending = true;   // rising edge only — this *is* STAT blocking
    }
    self.prev_stat_line = line;
}
```

Call `refresh_stat_line()` after every mode change, every LY increment, and every write to
`FF41`/`FF45`.

> **Do not port `MinKeeper` for this.** A per-T-cycle recompute of the OR is far simpler and
> entirely affordable at `gb`'s 23× realtime budget.

---

## 4. LCD enable/disable

### How gambatte does it

```cpp
// video/ppu.cpp:1785-1794
void PPU::setLcdc(unsigned const lcdc, unsigned long const cc) {
    if ((p_.lcdc ^ lcdc) & lcdc & lcdc_en) {
        p_.now = cc;
        p_.lastM0Time = 0;
        p_.lyCounter.reset(0, p_.now);
        p_.spriteMapper.enableDisplay(cc);
        p_.weMaster = (lcdc & lcdc_we) && 0 == p_.wy;
        p_.winDrawState = 0;
        p_.nextCallPtr = &M3Start::f0_;
        p_.cycles = -(m3StartLineCycle(p_.cgb) + 2);
    }
```

Note it restarts **inside mode 3**, not at the top of mode 2. The "OAM is garbage for the first
line" window:

```cpp
// video/sprite_mapper.cpp:132-137
void SpriteMapper::OamReader::enableDisplay(unsigned long cc) {
    std::fill_n(buf_, sizeof buf_ / sizeof *buf_, 0);
    std::fill_n(lsbuf_, sizeof lsbuf_ / sizeof *lsbuf_, false);
    lu_ = cc + (2 * lcd_num_oam_entries << lyCounter_.isDoubleSpeed()) + 1;
    lastChange_ = 2 * lcd_num_oam_entries;
}
```

`inactivePeriodAfterDisplayEnable(cc) { return cc < lu_; }` is consulted by `vramReadable`,
`vramWritable`, `cgbpAccessible`, `oamReadable`, `oamWritable` and `getStat`
(`video.cpp:344-416`, `757-761`).

On the memory side:

```cpp
// memory.cpp:923-937 (abridged)
    if (data & lcdc_en) {
        if (ioamhram_[0x141] & lcdstat_lycirqen && ioamhram_[0x145] == 0
                && !(stat & lcdstat_lycflag))
            intreq_.flagIrq(2);
        intreq_.setEventTime<intevent_blit>(blanklcd_
            ? lcd_.nextMode1IrqTime()
            : lcd_.nextMode1IrqTime() + (lcd_cycles_per_frame << isDoubleSpeed()));
    } else {
        ioamhram_[0x141] |= stat & lcdstat_lycflag;
        intreq_.setEventTime<intevent_blit>(cc + (lcd_cycles_per_line * 4 << isDoubleSpeed()));
        if (hdmaEnabled) flagHdmaReq(intreq_);
    }
```

plus `ioamhram_[0x144] = 0; ioamhram_[0x141] &= 0xF8;` (`memory.cpp:920-921`) — LY forced to 0,
mode and LYC-flag bits cleared. `blanklcd_` makes the next blit clear the framebuffer
(`memory.cpp:203-220`, `video.cpp:239-245`), and re-enabling deliberately skips one frame.

`LCD::lcdcChange` (`video.cpp:485-541`) rebuilds or tears down all event times. In the *non*-enable
branch, CGB applies `lcdc_tdsel` at `cc+1` and the rest at `cc+2`, and DMG applies `lcdc_obj2x`
2 cycles later than the other bits.

### How gb does it

```rust
// src/ppu.rs:196-201
pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
    if !self.lcd_control.is_enabled() {
        // TODO should the screen be blanked?
        return
    }
```

`LcdControl::set` (`src/lcd_control.rs:17-26`) is a pure bit-decode with no side effects;
`src/mmu.rs:374` wires `0xFF40` straight into it.

### Gap

- **LY, mode and `current_ticks` freeze.** Hardware forces LY=0 and mode 0. Software polling
  `FF44` while the LCD is off sees a constant value and can hang.
- **The frozen mode leaks into memory access.** Disabling while `mode == Drawing` locks VRAM and
  OAM **forever**, because the predicates at `src/lcd_status.rs:103-109` are mode-derived. Games
  get away with it today only because they conventionally disable during VBlank.
- **On re-enable, rendering resumes mid-frame** from the frozen state — no short first line, no
  `inactivePeriodAfterDisplayEnable` dead window, no blank frame, no skipped first frame.
- **No LCDC bit-application skew.**
- `LcdControl::default()` has `enabled: true` (`src/lcd_control.rs:78`) with LY=0 / mode=HBlank.
  `gb` has no boot ROM, so power-on state does not match DMG post-boot state.

### Symptom / failing tests

- mooneye `acceptance/ppu/lcdon_timing-GS`, `lcdon_write_timing-GS`
- gambatte `enable_display/*` (14+ tests: `enable_display_ly0_m0irq_trigger`,
  `enable_display_ly0_oambusy_read_{1,2}`, `enable_display_ly0_sprites_m0stat_{1,2}`,
  `enable_display_ly0_wemaster_{1,2}`, `frame0_ly_count_1_dmg08_cgb04c_out99`,
  `disable_display_regs_{1,2,3}`)
- gambatte `display_startstate/*` (`stat_1_dmg08_out85`, `stat_scx2_*`, `irq_*`, `ly_*`)
- Commercial: anything that blanks the LCD to bulk-load VRAM — very common on a title→gameplay
  transition

### Tasks

- [ ] Detect the LCDC.7 edge in `LcdControl::set` and hand it to the PPU.
- [ ] On 1→0: set `ly = 0`, `mode = HBlank`, `current_ticks = 0`, clear the STAT line **without**
      firing an interrupt, and optionally white-fill `self.lcd`.
- [ ] On 0→1: set `ly = 0`, `current_ticks = 0`, start in mode 3 with a shortened first line.
- [ ] Add an `enable_dead_until` cycle stamp for ~80 cycles during which VRAM/OAM read freely and
      STAT reports mode 0.
- [ ] Fix `LcdControl::default()` to match DMG post-boot state (LCDC = `0x91`, STAT = `0x85`).

---

## 5. Mid-scanline register writes

### How gambatte does it

Each register write first advances the PPU to a *register-specific* cycle offset:

```cpp
// video.cpp:434-467
void LCD::wxChange(unsigned newValue, unsigned long cycleCounter) {
    update(cycleCounter + 1 + ppu_.cgb());
    ppu_.setWx(newValue);
    mode3CyclesChange();
}
void LCD::wyChange(unsigned const newValue, unsigned long const cc) {
    update(cc + 1 + ppu_.cgb());
    ppu_.setWy(newValue);
    if (ppu_.cgb() && (ppu_.lcdc() & lcdc_en)) {
        eventTimes_.setm<memevent_oneshot_updatewy2>(cc + 6 - isDoubleSpeed());
    } else { update(cc + 2); ppu_.updateWy2(); mode3CyclesChange(); }
}
void LCD::scxChange(unsigned newScx, unsigned long cycleCounter) {
    update(cycleCounter + 2 * ppu_.cgb());
    ppu_.setScx(newScx);
    mode3CyclesChange();
}
void LCD::scyChange(unsigned newValue, unsigned long cycleCounter) {
    update(cycleCounter + 2 * ppu_.cgb());
    ppu_.setScy(newValue);
}
```

`mode3CyclesChange()` (`video.cpp:418-432`) invalidates the mode-0 prediction, because SCX / WX /
LCDC.5 change *when mode 3 ends*. BGP and OBP follow the same pattern (`video.h:61-77`).

### How gb does it

```rust
// src/mmu.rs:374-385
0xFF40 => self.ppu.lcd_control_mut().set(value),
0xFF41 => self.ppu.lcd_status_mut().set_stat(value),
0xFF42 => self.ppu.scroll_mut().y = value,
0xFF43 => self.ppu.scroll_mut().x = value,
0xFF45 => self.ppu.lcd_status_mut().set_lyc(value),
0xFF47 => self.ppu.palette_mut().background_mut().set_from_byte(value),
0xFF4A => self.ppu.window_position_mut().y = value,
0xFF4B => self.ppu.window_position_mut().x = value,
```

`MMU::update` runs *after* the opcode (`src/core.rs:479-483`), so a write effectively lands at the
start of that instruction's cycle block.

### Gap

- Because of §1(a), a write more than ~36 cycles into mode 3 changes nothing at all. `gb` is
  effectively **per-scanline** for SCX/SCY/BGP, and roughly **per-frame** for WX/WY.
- No per-register sub-cycle offset.
- `LcdControl` changes are instantaneous, including LCDC.4 (tile data select) and LCDC.2 (sprite
  size).

### Tasks

- [ ] Route `FF40..FF4B` through a `PPU::sync_then_write(reg, value, cycle_offset)` mirroring
      gambatte's `scxChange` / `wxChange` / `wyChange`.
- [ ] Give each register its correct offset (SCX/SCY `+2*cgb`, WX/WY `+1+cgb`, then `+2` for the
      WY2 latch on DMG).
- [ ] Invalidate any cached mode-3 length on SCX/WX/LCDC.5 writes.

---

## 6. Window

### How gambatte does it

`wy2` is a **delayed copy** of WY — the WY latch. `weMaster` is sampled at three specific line
cycles:

```cpp
// video/ppu.cpp:125-127, 141-160
inline int weMasterCheckLy0LineCycle(bool cgb)        { return 1 + cgb; }
inline int weMasterCheckPriorToLyIncLineCycle(bool)   { return 450; }
inline int weMasterCheckAfterLyIncLineCycle(bool)     { return 454; }
    void f0(PPUPriv &p) { p.weMaster |= lcdcWinEn(p) && p.lyCounter.ly()     == p.wy; /* ... */ }
    void f1(PPUPriv &p) { p.weMaster |= lcdcWinEn(p) && p.lyCounter.ly() + 1 == p.wy; /* ... */ }
```

**WX=0 and WX=166.** `plotPixel` compares `p.wx == xpos` in the 0..168 space (framebuffer
x = `xpos - 8`), so WX=7 is x=−1 and WX=0 is x=−8; the leading pixels are naturally consumed:

```cpp
// video/ppu.cpp:835-843
    if (p.wx == xpos && (p.weMaster || (p.wy2 == p.lyCounter.ly() && lcdcWinEn(p)))
            && xpos < lcd_hres + 7) {
        if (p.winDrawState == 0 && lcdcWinEn(p)) {
            p.winDrawState = win_draw_start | win_draw_started;
            ++p.winYPos;
        } else if (!p.cgb && (p.winDrawState == 0 || xpos == lcd_hres + 6))
            p.winDrawState |= win_draw_start;
    }
```

The `xpos == lcd_hres + 6` (166) branch is the **WX=166 quirk**: on DMG it sets `win_draw_start`
so the *next* line begins with the window already active.
`predictCyclesUntilXposNextLine` (`ppu.cpp:1240-1252`) carries the same rule.

**Mid-frame window disable:**

```cpp
// video/ppu.cpp:1795-1803
    } else if ((p_.lcdc ^ lcdc) & lcdc_we) {
        if (!(lcdc & lcdc_we)) {
            if (p_.winDrawState == win_draw_started || p_.xpos == xpos_end)
                p_.winDrawState &= ~(1u * win_draw_started);
        } else if (p_.winDrawState == win_draw_start) {
            p_.winDrawState |= win_draw_started;
            ++p_.winYPos;
        }
    }
```

`winYPos` increments on *activation*, not per scanline — so disabling and re-enabling mid-frame
does not reset it.

### How gb does it

```rust
// src/ppu.rs:233-235
if self.lcd_status.ly() == self.window_position.y && !self.window_state.is_active {
    self.window_state.activate(y, self.window_position);
}
```

```rust
// src/ppu.rs:322-338
fn in_window(&self, x: usize, y: usize) -> bool {
    self.lcd_control.window_enabled()
        && self.window_state.is_active
        && x >= self.window_position.x.saturating_sub(7) as usize
}
fn window_pixel(&self, x: usize) -> u8 {
    /* ... */
    x + 7 - self.window_position.x as usize,
    self.window_state.window_y
```

### Gap

- **The WY latch is wrong.** `ly == wy` is evaluated *during* mode 3, so a mid-line WY write can
  activate the window on that same line. Hardware samples at line cycles ~450/454 of the
  *previous* line (and cycle 1 for LY 0). The check also does not require LCDC.5, unlike gambatte's
  `lcdcWinEn(p) &&`.
- **WX < 7 is collapsed.** `saturating_sub(7)` maps WX ∈ {0..7} all to x ≥ 0, and `window_pixel`
  computes `x + 7 - wx`, so WX=0 draws the window offset by 7 instead of shifted left by 7.
- **No WX=166 quirk.** `in_window` gives exactly one window pixel and no next-line latch.
- **Mid-frame window disable** only makes `in_window` false; `is_active` and `window_y` keep
  running. `WindowRenderState::update_if_active` (`src/ppu.rs:41-48`) approximates the internal
  line counter but is driven by the broken x-advance.

### Symptom / failing tests

- gambatte `window/late_disable_{0,1,2}_*`, `late_disable_early_scx03_wx{0f,10,11,12}_*`,
  `late_disable_late_scx03_wx{0f,10,11}_*` and the `_ds_` variants — roughly 40 tests, all
  exercising exactly this
- gambatte `window/arg/*`, `scx_during_m3/*`, `dmgpalette_during_m3/*`
- Commercial: **Prehistorik Man**, **Pinball Deluxe**, **Road Rash**, **Wave Race** intro,
  **Star Trek: 25th Anniversary**

### Tasks

- [ ] Add a `wy2` field and sample `we_master` at line cycles 450 and 454 (and cycle 1 for LY 0).
- [ ] Require `lcd_control.window_enabled()` in the WY comparison.
- [ ] Move the window-start test to `wx == xpos` in 0..168 space, so WX<7 and WX=166 fall out
      naturally.
- [ ] Add the DMG `xpos == 166` next-line latch.
- [ ] Track a two-bit `{win_draw_start, win_draw_started}` state and increment `win_y_pos` on
      activation rather than per scanline.

---

## 7. VRAM/OAM access blocking and OAM DMA

### How gambatte does it

Access is a **cycle predicate**, and read and write differ:

```cpp
// video.cpp:344-364
bool LCD::vramReadable(unsigned long const cc) {
    if (cc >= eventTimes_.nextEventTime()) update(cc);
    return !(ppu_.lcdc() & lcdc_en)
    || ppu_.lyCounter().ly() >= lcd_vres
    || ppu_.inactivePeriodAfterDisplayEnable(cc + 1 - ppu_.cgb() + isDoubleSpeed())
    || ppu_.lyCounter().lineCycles(cc) + isDoubleSpeed() < 76u + 3 * ppu_.cgb()
    || cc + 2 >= m0TimeOfCurrentLine(cc);
}
bool LCD::vramWritable(unsigned long const cc) {
    /* ... */ || ppu_.lyCounter().lineCycles(cc) + isDoubleSpeed() < 79
              || cc + 2 >= m0TimeOfCurrentLine(cc);
}
```

```cpp
// video.cpp:404-416
bool LCD::oamWritable(unsigned long const cc) {
    if (!(ppu_.lcdc() & lcdc_en)
            || ppu_.inactivePeriodAfterDisplayEnable(cc + 4 + isDoubleSpeed()))
        return true;
    if (cc >= eventTimes_.nextEventTime()) update(cc);
    if (ppu_.lyCounter().lineCycles(cc) + 3 + ppu_.cgb() >= lcd_cycles_per_line)
        return ppu_.lyCounter().ly() >= lcd_vres - 1
            && ppu_.lyCounter().ly() <  lcd_lines_per_frame - 1;
    return ppu_.lyCounter().ly() >= lcd_vres || cc + 2 >= m0TimeOfCurrentLine(cc)
        || (ppu_.lyCounter().lineCycles(cc) == 76 && !ppu_.cgb());
}
```

That last clause — OAM writable at exactly line cycle 76, DMG only — is a hardware quirk. OAM DMA
is progressive (`memory.cpp:225-233`, `519-532`, `625-628`).

> Gambatte does **not** model the DMG OAM corruption bug, so that is not a gap relative to this
> reference. If you want it, the reference is SameBoy plus blargg's `oam_bug` suite.

### How gb does it

```rust
// src/ppu.rs:89-121
pub fn read_vram(&self, address: u16) -> u8 {
    if self.lcd_status.mode().vram_accessible() || self.dma.is_active() {
        self.vram[address as usize]
    } else { 0xff }
}
pub fn read_oam(&self, address: u16) -> u8 {
    if self.lcd_status.mode().oam_accessible() || self.dma.is_active() {
        self.oam[address as usize]
    } else { 0xff }
}
```

```rust
// src/lcd_status.rs:102-109
pub fn vram_accessible(self) -> bool { self != LcdMode::Drawing }
pub fn oam_accessible(self) -> bool {
    self == LcdMode::HBlank || self == LcdMode::VBlank
}
```

```rust
// src/mmu.rs:221-227
if let Some(transfer) = self.ppu.dma_mut().update(delta_machine_cycles) {
    for i in 0..0xA0 {
        let value = self.read(transfer.address + i);
        self.ppu.write_oam(i, value);
    }
}
```

### Gap

1. **Blocking is only as accurate as the mode** — per §2, off by up to ~117 cycles.
2. **`|| self.dma.is_active()` is inverted.** During OAM DMA the CPU must see OAM as *blocked*;
   `gb` makes it *more* accessible. The clause exists only so the bulk copy at `src/mmu.rs:225` can
   write through `write_oam` — a self-inflicted hack.
3. **DMA is not progressive.** All 160 source reads happen on a single cycle, and they go through
   `self.read`, which would re-enter VRAM blocking for a `0x8000`-range source.
4. **Read and write share one predicate.** Hardware differs by ~3 cycles.
5. No `lineCycles == 76` DMG OAM-write quirk; no post-enable dead window.

### Symptom / failing tests

- gambatte `oam_access/{midread_1..3, midwrite_1..3, postread_*, postread_scx2_*, postread_ds_*}`
- gambatte `vram_m3/*`, `vramw_m3end/*`, `oamdma/*`
- mooneye `acceptance/ppu/intr_2_oam_ok_timing`, `acceptance/oam_dma_timing`,
  `acceptance/oam_dma/basic`, `oam_dma_restart`

### Tasks — cheap wins that need no pipeline

- [ ] Replace `|| self.dma.is_active()` with a dedicated unchecked OAM write used only by the DMA
      copy, and make `read_oam` return `0xFF` **during** DMA.
- [ ] Split `vram_accessible` into separate read and write predicates with distinct thresholds.
- [ ] Make the DMA copy progressive (one byte per 4 T over 640 T).
- [ ] Later: replace the mode lookup entirely with
      `line_cycles < 79 || cycle >= m0_start_of_line`.

---

## 8. CGB video inventory

| Feature | gambatte | gb |
|---|---|---|
| VRAM bank 1 (`FF4F`) | `memory.cpp:996-1002`; PPU indexes `+ vram_bank_size` (`ppu.cpp:228`, `275-278`, `1136-1139`) | **Absent.** `vram: [u8; 0x2000]` (`src/ppu.rs:15`); `0xFF4F` falls to `_ => 0xFF` (`src/mmu.rs:333`) |
| BG map attributes | `attr_cgbpalno/tdbank/dmgpalno/xflip/yflip/bgpriority` (`ppu.cpp:100-101`); `nattrib` fetched with every tile number | **Absent.** `TileMap::tile_index` returns a bare `u8` (`src/ppu.rs:475-478`) |
| BCPS/BCPD/OCPS/OCPD (`FF68`–`FF6B`) | `video.h:79-95`, `doCgbColorChange` (`video.cpp:104-109`) | **Absent.** `LcdPalette` is 3 × `[DMGColor; 4]` (`src/lcd_palette.rs:62-67`) |
| Palette blocking during mode 3 | `LCD::cgbpAccessible` (`video.cpp:366-375`), own `< 80` threshold, gates reads (0xFF) and writes | **Absent** |
| DMG-compat palette on CGB | `dmgColorsRgb32_[3][4]`, `setDmgPaletteColor` (`video.cpp:873-878`), `refreshPalettes` (`video.cpp:193-204`) | N/A (DMG-only) |
| HDMA/GDMA (`FF51`–`FF55`) | `memevent_hdma`, `enableHdma`/`disableHdma`/`isHdmaPeriod` (`video.cpp:320-342`), `NextM0Time`, `memory.cpp:283-334`, `haltHdmaState_` | **Absent** |
| OPRI (`FF6C`) | `memory.cpp:1069` | **Absent** |
| CGB `lcdc_bgen` as master-priority override | `ppu.cpp:845`, `868` | N/A |
| Double-speed PPU scaling | `LyCounter::setDoubleSpeed`, `>> isDoubleSpeed()` throughout | **Absent** — TODO at `src/ppu.rs:202` |
| CGB LCDC write skew (tdsel +1, rest +2) | `video.cpp:513-524` | **Absent** |

Failing: `cgb-acid2` entirely; gambatte `cgbpal_m3/*`, `cgb_bgp_dumper.asm`, `cgb_objp_dumper.asm`,
`bgtiledata/*`, `bgtilemap/*`, and every `*_cgb04c_out*` variant.

### Tasks (only if project scope changes)

- [ ] VRAM bank 1 + `VBK` register
- [ ] BG tile-map attributes (palette, bank, x/y flip, priority)
- [ ] CGB palette RAM + mode-3 access blocking
- [ ] OAM-index sprite priority + OBJ attributes + OPRI
- [ ] HDMA/GDMA
- [ ] Double-speed PPU scaling

> **Judgement:** `gb`'s actual purpose is a DMG Pokémon **Red** agent. Despite its size, this is
> almost certainly the lowest-value section in this document. Do not start here.

---

## 9. Frame output

**gambatte** schedules the frame as an `intevent_blit` tied to mode-1 interrupt time:

```cpp
// memory.cpp:203-221 (abridged)
    case intevent_blit: {
        bool const lcden = ioamhram_[0x140] & lcdc_en;
        unsigned long blitTime = intreq_.eventTime(intevent_blit);
        if (lcden | blanklcd_) {
            lcd_.updateScreen(blanklcd_, cc);
            intreq_.setEventTime<intevent_blit>(disabled_time);
            /* ... */
        } else
            blitTime += lcd_cycles_per_frame << isDoubleSpeed();
        blanklcd_ = lcden ^ 1;
        intreq_.setEventTime<intevent_blit>(blitTime);
    }
```

`GB::runFor` (`gambatte.cpp:62`) returns the exact sample offset of the blit, and `updateScreen`
first calls `update(cc)` so no partial line is ever visible.

**gb** has no frame event. The SDL loop reads the array on its own wall clock
(`src/sdl/render.rs:267-273`), and `GameBoy::run` (`src/game_boy.rs:29-35`) runs to a cycle budget
with no notion of a frame. Tests poll `ppu().screenshot()` at arbitrary offsets
(`src/game_boy.rs:380-396`).

**Gap:** no cycle-exact frame boundary (the top of the image can be frame N while the bottom is
N−1); pacing is host wall-clock; no blank-while-off and no skip-first-frame; screenshot tests can
spuriously mismatch, which is currently hidden by a retry loop.

### Tasks

- [ ] Set a `frame_complete` flag in `PPU::update` on the 153→0 LY roll.
- [ ] Expose `GameBoy::run_frame()` returning at that boundary.
- [ ] Double-buffer `lcd` so the host never observes a half-drawn frame.

This is cheap and also gives the SDL loop a natural vsync source.

---

## 10. Small, cheap fidelity items

| Item | `gb` location | Correct behaviour | Reference |
|---|---|---|---|
| STAT bit 7 reads 0 | `src/lcd_status.rs:60-67` | Always reads 1 | gambatte `display_startstate/stat_1_dmg08_out85` expects `0x85` |
| `FF46` reads 0 | `src/mmu.rs:325` | Reads back the last written DMA source | `memory.cpp:966` |
| `FF44` writes ignored | `src/mmu.rs:378` | Correct on DMG | gambatte `lywrite/*` |
| `LcdControl::default()` = enabled, LY=0, mode=HBlank | `src/lcd_control.rs:74-87` | DMG post-boot: LCDC=`0x91`, LY=0, STAT=`0x85` | gambatte `display_startstate/*` |
| STAT reads not cycle-offset | — | Reads are offset by +2 | `video.cpp:759`, `cc + 2 < m0TimeOfCurrentLine(cc)` |
| OAM scan is atomic | `src/ppu.rs:212-219` | Progressive, 1 entry per 2 T | `sprite_mapper.cpp:73-113` |

### Tasks

- [ ] Force STAT bit 7 to 1.
- [ ] Make `FF46` read back the last written value.
- [ ] Fix `LcdControl::default()` to DMG post-boot state.
- [ ] (Optional) Progressive OAM scan with a shadow `pos_buf[80]` — only if the `oam_access` tests
      become a goal.

---

## Sprites — what is already correct

Worth recording so nobody "fixes" it. `gb`'s sprite handling is the strongest part of its PPU:

```rust
// src/ppu.rs:210-219
self.scanline_sprites = if self.lcd_control.objects_enabled() {
    self.sprites().into_iter()
        .filter(|sprite| y >= sprite.y && y < sprite.y + sprite_height)
        .take(MAX_SPRITES_PER_SCANLINE)
        .collect()
} else { vec![] }
```

Already matching gambatte's `mapSprites` (`sprite_mapper.cpp:157-176`):

- ✅ 10-per-line limit taken in **OAM order** after a **Y-only** filter — so an off-screen or X=0
  sprite still consumes a slot
- ✅ DMG priority: itertools' `sorted_by_key` is stable, so equal-X ties break by OAM order, and
  the `filter(color != 0)` before the sort makes it behaviourally equivalent to gambatte's DMG loop
  (`ppu.cpp:876-884`)
- ✅ BG-priority bit handling
- ✅ 8×16 addressing (`tile_index & 0xFE` / `| 0x01`) and Y-flip over 16 rows
  (`src/ppu.rs:402-421`)

Remaining sprite gaps: no progressive OAM scan (§10); no mode-3 cost (§2); `objects_enabled()` is
sampled once at end-of-mode-2 whereas gambatte re-checks `lcdcObjEn(p)` at every pixel
(`ppu.cpp:867`, `882`); no CGB path (§8); and `sprite_pixel` re-reads VRAM per pixel with no
per-sprite tile-word latch, so a mid-line VRAM change retroactively alters an already-fetched
sprite row.

- [ ] Re-check `objects_enabled()` per pixel instead of once per scanline.
- [ ] Latch each sprite's tile word at fetch time.

---

## Suggested implementation order

1. [ ] **One-line fix**: `end_x` at `src/ppu.rs:230` (§1). Immediate ~4.5× raster-fidelity win,
       near-zero risk. **Read the [fixture warning](#fixture-warning) first.**
2. [ ] **STAT OR-edge line** (§3) — self-contained in `lcd_status.rs`; buys `stat_irq_blocking` and
       most `intr_*` tests.
3. [ ] **LCD enable/disable state reset** (§4) — small, and removes the latent "VRAM locked
       forever" hazard.
4. [ ] **OAM DMA blocking inversion** (§7, item 2) — trivial, currently backwards.
5. [ ] **Sub-instruction PPU stepping** (§1c) plus the sync-then-write register path (§5).
6. [ ] **Real `xpos` pipeline with variable mode 3** (§1, §2) — the big one; unlocks the window
       quirks (§6) and accurate access windows (§7).
7. [ ] CGB (§8) — only if project scope changes.

Items 1–4 are each under ~50 lines and independently testable. Item 5 gates everything after it.
Item 6 is a rewrite of `PPU::update`.

---

## References

- Pan Docs — <https://gbdev.io/pandocs/> (Rendering, STAT, Pixel FIFO chapters)
- The Cycle-Accurate Game Boy Docs (AntonioND) —
  <https://github.com/AntonioND/giibiiadvance/blob/master/docs/TCAGBD.pdf>
- gambatte hardware tests — `/home/alex/projects/gambatte/test/hwtests/`
- mooneye-gb test suite — <https://github.com/Gekkio/mooneye-test-suite>
- dmg-acid2 / cgb-acid2 — <https://github.com/mattcurrie/dmg-acid2>,
  <https://github.com/mattcurrie/cgb-acid2>
- MealyBug Tearoom tests (mid-scanline PPU writes) —
  <https://github.com/mattcurrie/mealybug-tearoom-tests>
