# APU / sound compatibility guide

`gb` (`/home/alex/projects/gb`, `src/audio/`) vs **gambatte**
(`/home/alex/projects/gambatte`, `libgambatte/src/sound.cpp` + `libgambatte/src/sound/*`).

> `src/audio/blip/` (the Blip_Buffer port) is out of scope here — it is separately tested and
> documented in `CLAUDE.md`. This guide is about **APU behaviour**.

---

## Ground truth first

`gb` already embeds blargg `dmg_sound` and wires 9 of the 12 sub-ROMs
(`src/game_boy.rs:193-257`). **All 9 pass:** `registers`, `length_counter`, `trigger`, `sweep`,
`sweep_details`, `overflow_on_trigger`, `length_sweep_period_sync`, `length_counter_during_power`,
`registers_after_power`.

The three commented out at `src/game_boy.rs:236-247` and `:252-257` are:
`09-wave read while on`, `10-wave trigger while on`, `12-wave write while on` — with placeholder
`EXPECTED_*` aliases at `src/roms/mod.rs:41-46`. **Those are exactly the wave-RAM quirks in §6.**

So the register-level, frame-sequencer-step-level, and length/sweep quirks are **already correct**.
This is the strongest subsystem in the emulator. The real gaps are sub-instruction timing, wave
RAM, envelope zombie mode, duty phase, and DIV coupling.

gambatte also ships **164** hand-written tests in `test/hwtests/sound/`, with expected results
encoded in the filename (e.g. `..._dmg08_cgb04c_outF1.asm`). They are cited by name below.

---

## Ranked gap summary

| # | Gap | Severity | Effort | Fails |
|---|---|---|---|---|
| 1 | **Instruction-granularity mixing; no resync on register write** | Critical (root cause of 3–10) | Large | gambatte `ch1_duty0_pos6_to_pos7_timing_1/2`, `ch2_init_reset_env_counter_timing_1..16`; SameSuite `apu/channel_1_align.gb`, `channel_1_delay.gb` |
| 2 | **Duty waveforms 1/2/3 rotated one step vs duty 0** | High | **Trivial** | gambatte `ch1_duty{1,2,3}_pattern_pos0..8` (27 tests); SameSuite `apu/channel_1_duty.gb` |
| 3 | **Frame sequencer not clocked by a DIV write** | High | Small | gambatte `ch{1..4}_div_write_reset_length_counter_timing_nr52_*`; SameSuite `apu/div_write_trigger.gb` |
| 4 | **Envelope "zombie mode" entirely missing** | High | Small | SameSuite `apu/channel_{1,2,4}_nrx2_glitch.gb`, `channel_1_restart_nrx2_glitch.gb` |
| 5 | **Wave RAM access while enabled is CGB-style unconditionally**; no DMG `0xFF` rule, no window, no retrigger corruption | High | Medium | blargg `dmg_sound/09`, `/10`, `/12` (already disabled); SameSuite `apu/channel_3_wave_ram_*` |
| 6 | **Channel-off-but-DAC-on outputs digital silence instead of the DAC zero level** (square + noise) | Medium-High | **Trivial** | audible click on every note-off |
| 7 | **Trigger resets duty position to 0** (gambatte preserves it) | Medium-High | Small | gambatte `ch1_init_pos_1..8`; SameSuite `apu/channel_1_restart.gb` |
| 8 | **Noise LFSR does not stop at clock-shift ≥ 14** | Medium | **Trivial** | SameSuite `apu/channel_4_lfsr_7_15.gb`, `channel_4_freq_change.gb` |
| 9 | **Wave channel: no +3 trigger delay, no "first read is position 1"** | Medium | Medium | SameSuite `apu/channel_3_first_sample.gb`, `channel_3_restart_delay.gb` |
| 10 | **No CGB/DMG flag anywhere in the APU**; no double-speed FS source | Medium | Medium | blargg `cgb_sound/09..12` |
| 11 | **NR12/NR17 write defers the DAC-off disable** while NR42/NR30 do it inline | Low-Med | Trivial | gambatte `ch*_late_reset_nr52_*` |
| 12 | **"First duty step plays as 0" quirk is coded but dead** (`initialised: true`) | Low-Med | **Trivial** | gambatte `ch1_init_pos_*` |
| 13 | **`Sweep::set_nr10` reloads the sweep timer when it reads 0** — no gambatte counterpart | Low | Trivial | gambatte `ch1_init_reset_sweep_counter_timing_nr52_1..4` |
| 14 | PCM12/PCM34 (`FF76`/`FF77`) unimplemented — **also absent from gambatte** | Low | Small | parity, not a regression |
| 15 | **Register read-back masks: no gaps found** | — | — | `01-registers` passes |

---

## 1. Architecture: event-driven vs per-instruction

### How gambatte does it

Every clockable thing is a `SoundUnit` with one absolute-cycle deadline (`sound_unit.h:24-41`).
Each channel picks the soonest via a hand-rolled 3-way min-keeper:

```cpp
// channel1.cpp:122-128
void Channel1::setEvent() {
    nextEventUnit_ = &sweepUnit_;
    if (envelopeUnit_.counter() < nextEventUnit_->counter()) nextEventUnit_ = &envelopeUnit_;
    if (lengthCounter_.counter() < nextEventUnit_->counter()) nextEventUnit_ = &lengthCounter_;
}
```

`setEvent()` is called at the end of **every** `setNrX` (`channel1.cpp:130-174`) — that is the
resync-on-write. The render loop then jumps edge to edge writing *deltas*:

```cpp
// channel1.cpp:220-227
while (dutyUnit_.counter() <= nextMajorEvent) {
    *buf = out - prevOut_;
    prevOut_ = out;
    buf += dutyUnit_.counter() - cc;   // skip the flat run
    cc = dutyUnit_.counter();
    dutyUnit_.event();
    out = dutyUnit_.isHighState() ? outHigh : outLow;
}
```

`DutyUnit` even skips duty positions that don't change the level, via `nextStateDistance[][8]`
(`duty_unit.cpp:60-78`). And `memory.cpp:719-905` calls `psg_.generateSamples(cc, isDoubleSpeed())`
before every NR write and before the NR52 read (`:579-583`).

### How gb does it

One update per instruction — `src/core.rs:479-483` → `src/mmu.rs:230-234`:

```rust
let div_clocks = self.divider.update(delta_machine_cycles);
self.timer.update(delta_machine_cycles);
self.ppu.update(delta_machine_cycles);
self.audio.update(delta_machine_cycles, div_clocks);
```

`src/audio/mod.rs:110-136` mixes one value for the whole window and `:147-150` pushes it as a
single step. Inside a channel, only the *last* state survives:

```rust
// src/audio/timer.rs:49-63
let ticks = machine_cycles.m_cycles() * SPEED_MULTIPLIER;
for _ in 0..ticks {
    self.counter -= 1;
    if self.counter == 0 {
        self.counter = self.period;
        self.phase = (self.phase + 1) & MAX_PHASE;
        clocked = true;
    }
}
```

`src/audio/square_channel.rs:206-215` then reads `waveform_bit()` once, *after* the loop.

### Gap

**(a) Lost transitions.** A square at `freq = 2047` advances one duty step per M-cycle; a 4 M-cycle
instruction passes four steps and `gb` emits one. The rate arithmetic is right (`src/cycles.rs:13`,
`src/audio/timer.rs:66-67` — `PulseTimer` 1 tick/M-cycle, `WavetableTimer` 2) but the *output* is
decimated.

**(b) No resync.** `mmu.update()` runs **after** the instruction that wrote, so every NR write is
misplaced by up to one instruction.

Gambatte is also *cheaper* — it skips flat runs, whereas `gb` runs a per-M-cycle `for` loop
regardless.

### Tasks

- [ ] **Tier 1** (keeps the current architecture): give each channel
      `next_edge_in() -> Option<u32>`; have `Audio::update` sub-divide the window at the minimum,
      calling `push_sample(step, mix)` per sub-window. `blip` already handles arbitrary sub-sample
      positions (16.16 cursor, `src/audio/mod.rs:142-146`), so **no resampler change is needed**.
- [ ] **Tier 2**: have the MMU call `audio.catch_up(offset_within_instruction)` before each
      `0xFF10..=0xFF3F` access, as gambatte does. Requires `core.rs`/`opcode.rs` to report a
      within-instruction cycle offset — i.e. it depends on
      [`02-cpu.md` §1](02-cpu.md#1-timing-granularity--the-root-cause). Scope separately.

---

## 2. Frame sequencer

### How gambatte does it

There is no step variable — the FS position *is* bits 12–14 of the PSG cycle counter
(`sound.cpp:68`: `// cycleCounter >> 12 & 7 represents the frame sequencer position.`). Units
schedule in multiples: length `<< 13` (`length_counter.cpp:45`), envelope `<< 15`
(`envelope_unit.cpp:65`), sweep `<< 14` (`channel1.cpp:61`).

A DIV write re-phases the whole PSG:

```cpp
// sound.cpp:77-85
void PSG::divReset(bool ds) {
    unsigned long const cc = cycleCounter_ + divOffset;
    cycleCounter_ = (cc & -0x1000) + 2 * (cc & 0x800) - divOffset;
    ch1_.resetCc(cc - divOffset, cycleCounter_); /* …2,3,4… */
}
```

wired at `memory.cpp:693-704` (`case 0x04:` → `psg_.divReset(isDoubleSpeed())`).

### How gb does it

`gb` *does* use DIV, correctly:

```rust
// src/audio/frame_sequencer.rs:20-30
// TODO bit 4 in normal speed mode, bit 5 in CBG (double) speed mode
let delta = div_clocks.bit_fall_edge(4);
for _ in 0..delta { self.value += 1; self.value %= 8; events |= self.current_events(); }
```

DIV register bit 4 == internal bit 12 == 512 Hz ✅. The step table (`:35-41`) matches gambatte
exactly. Power-on `reset_to_max()` (`src/audio/mod.rs:179-182`) sets `value = 7` so the next edge
yields step 0 — correct.

### Gap

**The DIV *write* path is broken.**

```rust
// src/mmu.rs:368
0xFF04 => self.divider.reset(), // DIV register (reset on write)
```

`Divider::reset()` sets `self.value = 0` only — no falling edge is emitted, and
`cycles_since_tick` is not cleared. `DividerClocks::bit_fall_edge` only sees edges produced *inside*
`Divider::update`, and on the next call `initial_value` is already 0. So a DIV write that drops
bit 4 from 1→0 **silently loses the frame-sequencer clock**.

### Tasks

- [ ] Emit the falling edge on a DIV write and clock the frame sequencer once.
- [ ] Clear `cycles_since_tick` in `Divider::reset` (it currently leaks sub-tick phase — a
      timer-domain bug too, see [`02-cpu.md` §5](02-cpu.md#5-div--timer--highest-accuracy-win-per-line)).

```rust
0xFF04 => {
    let edge = self.divider.reset_with_edge(4);   // did bit 4 fall?
    if edge { self.audio.clock_frame_sequencer_once(); }
}
```

Gate on `self.audio.enabled` — while the APU is off the FS does not run
(`src/audio/mod.rs:111-114` early-returns, which is correct).

> If you adopt the unified system counter from [`02-cpu.md` §5](02-cpu.md#5-div--timer--highest-accuracy-win-per-line),
> hang the FS off `counter` bit 12 falling edges and this bug disappears for free.

---

## 3. Length counters — already correct

Gambatte's `nr4Change` (`length_counter.cpp:48-67`) uses `dec = ~cc >> 12 & 1`, which is 1 when the
FS step ∈ {0,2,4,6}. `gb`'s `current_events().is_length_counter()`
(`src/audio/frame_sequencer.rs:36-37`) is true for exactly those steps, and `value` is the current
step — **identical semantics**.

```rust
// src/audio/length.rs:28-51
pub fn trigger(&mut self, frame_sequencer: &FrameSequencer) {
    if self.value == 0 {
        self.reset(0x00);
        if self.enabled && frame_sequencer.current_events().is_length_counter() {
            self.value = self.value.saturating_sub(1);
        }
    }
}
```

Writes while off are allowed for the right registers (`src/audio/mod.rs:244`) with the duty mask at
`src/audio/square_channel.rs:83-92`. blargg `02`/`03`/`07`/`08` all pass.

### Remaining minor gaps

- [ ] **No CGB gate** on the four length registers. Gambatte returns early on CGB
      (`memory.cpp:806-811`): `if (!psg_.isEnabled() && isCgb()) return;`. Fails blargg
      `cgb_sound/08`.
- [ ] `src/audio/square_channel.rs:120-124` calls `set_enabled` then `trigger` as two steps, so the
      `dec` predicate is evaluated twice rather than fused as gambatte does. Fragile rather than
      currently wrong — consider fusing.

```rust
let write_allowed = self.enabled
    || address == 0xFF26
    || matches!(address, 0xFF30..=0xFF3F)
    || (is_length_register(address) && !self.cgb);
```

---

## 4. Envelope unit — zombie mode is missing

### How gambatte does it

```cpp
// envelope_unit.cpp:65-80
bool EnvelopeUnit::nr2Change(unsigned const newNr2) {          // ZOMBIE MODE
    if (!(nr2_ & psg_nr2_step) && counter_ != counter_disabled) ++volume_;
    else if (!(nr2_ & psg_nr2_inc)) volume_ += 2;
    if ((nr2_ ^ newNr2) & psg_nr2_inc) volume_ = 0x10 - volume_;
    volume_ &= 0xF; nr2_ = newNr2;
    return !(newNr2 & (psg_nr2_initvol | psg_nr2_inc));         // true == DAC now off
}
```

DAC rule (`envelope_unit.h:36`): `bool dacIsOn() const { return nr2_ & 0xF8; }`.

### How gb does it

`src/audio/volume.rs:70-105`. `sweep_pace()` (`:43-45`) returns 8 for a stored 0, and `clock()`
bails at `:80-82` so period 0 produces no change — equivalent to gambatte. It saturates at 0/15
instead of latching `counter_disabled`; behaviourally equivalent. `dac_enabled()` (`:70-72`)
matches `nr2_ & 0xF8` exactly ✅.

But the NRx2 write path for ch1/ch2 is a bare store (`src/audio/mod.rs:249`, `:253`):

```rust
0xFF12 => self.channel1.volume_envelope_register_mut().set(value),
0xFF17 => self.channel2.volume_envelope_register_mut().set(value),
```

while ch4 applies the DAC rule inline (`src/audio/noise_channel.rs:63-68`) and ch1/ch2 defer to the
next `update` (`src/audio/square_channel.rs:177-179`).

### Tasks

- [ ] Implement zombie mode. It must run **before** the register is overwritten.
- [ ] Add a `saturated` flag (gambatte's `counter_ == counter_disabled`), set when volume hits 0/15
      with a non-zero pace, cleared on `trigger()`.
- [ ] Apply the DAC-off disable inline for ch1/ch2 as ch4 already does.
- [ ] (Optional) Envelope-period near-edge nudge on trigger — `envelope_unit.cpp:88`:
      `if (((cc + 2) & 0x7000) == 0x0000) ++period;`

```rust
/// Returns true if the write turned the DAC off (caller clears `active`).
pub fn nr2_change(&mut self, new_value: u8) -> bool {
    let old = self.register.get();
    let mut vol = self.current_volume as u16;
    if old & 0x07 == 0 && !self.saturated { vol += 1; }
    else if old & 0x08 == 0 { vol += 2; }
    if (old ^ new_value) & 0x08 != 0 { vol = 0x10u16.wrapping_sub(vol); }
    self.current_volume = (vol & 0xF) as u8;
    self.register.set(new_value);
    !self.dac_enabled()
}
```

---

## 5. Sweep — correct, with one non-standard extra

`src/audio/sweep.rs:57-111` has the shadow register, the immediate calc when step > 0, the second
overflow check after writeback, the negate quirk (`:46-50`) and period 0 → 8 (`:53-55`).
Behaviourally **correct** — blargg `04`/`05`/`06`/`07` pass.

### Gap

```rust
// src/audio/sweep.rs:42-44
if self.sweep_timer == 0 { self.reset_sweep_timer(); }
```

Gambatte's `nr0Change` (`channel1.cpp:70-74`) does nothing but the negate check, so `gb` can
**re-arm a sweep that had reached 0**. Diverges on
`ch1_init_reset_sweep_counter_timing_nr52_1..4`.

- [ ] Delete the `sweep_timer == 0` reload; re-run blargg `04`/`05`/`06`/`07` (they should stay
      green — the reload only matters when the sweep is idle).
- [ ] (Deferred) The CGB `+2` scheduling offset.

---

## 6. Wave channel — the biggest concrete gap

### How gambatte does it

The whole DMG/CGB split is these 14 lines:

```cpp
// channel3.h:49-69
unsigned waveRamRead(unsigned index, unsigned long cc) const {
    if (master_) {
        if (!cgb_ && cc != lastReadTime_) return 0xFF;   // DMG: 0xFF outside the 1-cycle window
        index = wavePos_ / 2;                            // CGB (and DMG in-window)
    }
    return waveRam_[index];
}
void waveRamWrite(unsigned index, unsigned data, unsigned long cc) {
    if (master_) {
        if (!cgb_ && cc != lastReadTime_) return;        // DMG: write dropped
        index = wavePos_ / 2;
    }
    waveRam_[index] = data;
}
```

DMG retrigger corruption plus the `+3` delay:

```cpp
// channel3.cpp:64-82
if (data & nr0_) {
    if (!cgb_ && waveCounter_ == cc + 1) {
        int const pos = (wavePos_ + 1) / 2 % sizeof waveRam_;
        if (pos < 4) waveRam_[0] = waveRam_[pos];
        else std::memcpy(waveRam_, waveRam_ + (pos & ~3), 4);
    }
    master_ = true; wavePos_ = 0;
    lastReadTime_ = waveCounter_ = cc + toPeriod(nr3_, data) + 3;
}
```

`updateWaveCounter` advances `wavePos_` by `periods + 1` (`:140`) — **the first sample played is
position 1**, not 0.

### How gb does it

```rust
// src/audio/wave_channel.rs:130-142
pub fn wave_ram(&self, index: usize) -> u8 {
    if self.active { self.current_sample_byte() } else { self.wave_ram[index] }
}
pub fn set_wave_ram(&mut self, index: usize, value: u8) { self.wave_ram[index] = value; }
```

`trigger()` (`:167-173`) sets `frequency_timer.trigger()` → `phase = 0; counter = period`.

### Gap

1. Read while enabled is **unconditionally CGB**; no `lastReadTime_` equivalent exists.
2. Write while enabled is **neither DMG nor CGB** — it writes the requested index.
3. No retrigger corruption.
4. No `+3` delay; position starts at 0 instead of 1.

Volume shift (`:152-165`) ✅ and `reset()` preserving wave RAM (`:64-67`) ✅.

### Tasks

- [ ] **Cheap intermediate that re-enables blargg `09`/`12` today**: return `0xFF` / drop the write
      whenever `self.active`, ignoring the exact window. The window is 1 cycle out of
      `(2048 - freq) * 2`, so "always locked" is overwhelmingly the common case and is strictly
      closer to hardware than what is there now. **Two lines.**
- [ ] Add `last_read_cycle: u64` + a `cgb: bool` and thread the current APU cycle into the
      accessors, mirroring gambatte 1:1. Set `last_read_cycle` where `sample_buffer` is refreshed
      (`:194-197`). *(Needs §1 Tier 2 for the exact 1-cycle window.)*
- [ ] Retrigger corruption in `trigger()`, gated on "the wave counter was about to fire".
- [ ] `+3` trigger delay and first-sample-is-position-1.
- [ ] Re-enable the three commented-out blargg tests at `src/game_boy.rs:236-247`, `:252-257` and
      replace the placeholder `EXPECTED_*` aliases at `src/roms/mod.rs:41-46`.

---

## 7. Noise channel

Width and polarity are **correct** (feedback into bit 14, additionally bit 6 for 7-bit, high when
bit 0 == 0) ✅. The divisor table is **correct**: `divider == 0 → 8`, else `16 * r`, `<< shift`,
`/4` for T→M cycles — equivalent to gambatte's `r << (s + 3)` ✅.

### Gap

**Missing the shift ≥ 14 stop.** Gambatte:

```cpp
// channel4.cpp:84-96
inline void Channel4::Lfsr::event() {
    if (nr3_ < 0xE * (1u * psg_nr43_s & -psg_nr43_s)) {   // shift < 14
        unsigned const shifted = reg_ >> 1;
        unsigned const xored = (reg_ ^ shifted) & 1;
        reg_ = shifted | xored << 14;
        if (nr3_ & psg_nr43_7biten) reg_ = (reg_ & ~0x40u) | xored << 6;
    }
    counter_ += toPeriod(nr3_); backupCounter_ = counter_;
}
```

`gb` (`src/audio/noise_channel.rs:137-163`) happily computes `(8 << 15) / 4` and keeps shifting.

Also: no `+4` trigger delay; `trigger()` (`:118`) reloads the counter from scratch instead of
continuing gambatte's free-running `backupCounter_` phase.

### Tasks

- [ ] Gate the LFSR shift on `clock_shift < 14`.
- [ ] Guard `self.counter -= 1` against underflow (minimum is 2 today, but it is unchecked and
      would panic in a debug build).
- [ ] (Optional) `+4` trigger delay and free-running phase.

```rust
if self.counter == 0 {
    self.counter = compute_clock_period(self.clock_divider, self.clock_shift);
    if self.clock_shift < 14 {
        let new_bit = (self.lfsr ^ (self.lfsr >> 1)) & 0x01;
        self.lfsr = (self.lfsr >> 1) | (new_bit << 14);
        if self.lfsr_width { self.lfsr = (self.lfsr & !(1 << 6)) | (new_bit << 6); }
    }
}
```

---

## 8. Power off / on — in good shape

Read-back while off is `0x70` ✅ (all `active` cleared by `reset()`); the FS is frozen while off ✅
(`src/audio/mod.rs:111-114`). blargg `08` and `11` pass.

### One item

`reset()` recreates channels via `channel1()`/`channel2()`, and `SquareWaveChannel::new` sets
**`initialised: true`** (`src/audio/square_channel.rs:53`) — directly contradicting the comment two
lines below it about the first duty step playing as 0. The guard at `:199-203` is therefore **dead
code**. Gambatte's equivalent is `DutyUnit::reset()` (`duty_unit.cpp:118-123`) setting
`nextPosUpdate_ = counter_disabled`.

- [ ] Set `initialised: false`; re-run blargg; then check gambatte `ch1_init_pos_1..8`.

---

## 9. Register read-back masks — no gaps

Gambatte has no mask table: it ORs the read-as-1 bits into `data` at **write** time and stores the
masked byte, so reads are a plain array fetch (`memory.h:72-73`). `gb` computes the mask in each
getter. Both arrive at the same answer.

| Addr | Reg | Reads-as-1 | gambatte | gb | Verdict |
|---|---|---|---|---|---|
| FF10 | NR10 | `0x80` | `memory.cpp:727` | `sweep.rs:28` | ✅ |
| FF11 | NR11 | `0x3F` | `memory.cpp:738` | `square_channel.rs:80` | ✅ |
| FF12 | NR12 | `0x00` | no OR `:740-745` | `volume.rs:21-27` | ✅ |
| FF13 | NR13 | `0xFF` | `return` `:753` | `square_channel.rs:103` | ✅ |
| FF14 | NR14 | `0xBF` | `memory.cpp:760` | `square_channel.rs:112` | ✅ |
| FF15 | — | `0xFF` | not a case | `mod.rs:232-235` | ✅ |
| FF16 | NR21 | `0x3F` | `memory.cpp:772` | `square_channel.rs:80` | ✅ |
| FF17 | NR22 | `0x00` | no OR | `volume.rs:21-27` | ✅ |
| FF18 | NR23 | `0xFF` | `return` `:786` | `square_channel.rs:103` | ✅ |
| FF19 | NR24 | `0xBF` | `memory.cpp:794` | `square_channel.rs:112` | ✅ |
| FF1A | NR30 | `0x7F` | `memory.cpp:803` | `wave_channel.rs:69-77` | ✅ |
| FF1B | NR31 | `0xFF` | `return` `:811` | `wave_channel.rs:87` | ✅ |
| FF1C | NR32 | `0x9F` | `memory.cpp:819` | `wave_channel.rs:101` | ✅ |
| FF1D | NR33 | `0xFF` | `return` `:826` | `wave_channel.rs:109` | ✅ |
| FF1E | NR34 | `0xBF` | `memory.cpp:832` | `wave_channel.rs:118` | ✅ |
| FF1F | — | `0xFF` | not a case | `mod.rs:232-235` | ✅ |
| FF20 | NR41 | `0xFF` | `return` `:838` | `noise_channel.rs:49` | ✅ |
| FF21 | NR42 | `0x00` | no OR | `noise_channel.rs:60` | ✅ |
| FF22 | NR43 | `0x00` | no OR | `noise_channel.rs:71` | ✅ |
| FF23 | NR44 | `0xBF` | `memory.cpp:861` | `noise_channel.rs:82` | ✅ |
| FF24 | NR50 | `0x00` | no OR | `master_volume.rs:19-26` | ✅ |
| FF25 | NR51 | `0x00` | no OR | `panning.rs:27-38` | ✅ |
| FF26 | NR52 | `0x70` | `memory.cpp:581,583` | `mod.rs:152-171` | ✅ |
| FF27–FF2F | — | `0xFF` | not cases | `mod.rs:232-235` | ✅ |
| FF30–FF3F | Wave | conditional | `channel3.h:49-58` | `wave_channel.rs:130-138` | ❌ see §6 |
| FF76/FF77 | PCM12/34 | CGB only | **not implemented** | **not implemented** | ⚠️ parity |

**`gb`'s read-back masks are correct for every register.** Only wave RAM is defective.

---

## 10. Output path and mixing

### The DAC-off click — highest value per line in this document

Gambatte models **three** states (`channel1.cpp:210-218`):

```cpp
unsigned long const outBase = envelopeUnit_.dacIsOn() ? soBaseVol & soMask_ : 0;
unsigned long const outLow  = outBase * -15;
unsigned long const outHigh = master_ ? outBase * (envelopeUnit_.getVolume() * 2l - 15) : outLow;
```

- DAC off → contributes nothing
- **DAC on + channel off → `outLow`, the DC level of digital 0**
- DAC on + running → `v * 2 - 15`

`gb` collapses the middle case to silence:

```rust
// src/audio/square_channel.rs:140-146  (and identically noise_channel.rs:105-111)
pub fn output_f32(&self) -> f32 {
    if self.envelope_function.dac_enabled() && self.active { dac_sample(self.output) }
    else { 0.0 }
}
```

When length disables a channel whose DAC is still on, hardware holds the digital-0 DAC level; `gb`
snaps to `0.0` — a full-scale step, i.e. **a click on every note-off**.

Notably the **wave channel already gets this right** — `src/audio/wave_channel.rs:204-213`
deliberately clears `sample_buffer` so `output_f32` returns `dac_sample(0)`, with a comment
explaining exactly this. Square and noise never got the same treatment.

- [ ] Fix square and noise:

```rust
pub fn output_f32(&self) -> f32 {
    if !self.envelope_function.dac_enabled() { 0.0 }
    else if self.active { dac_sample(self.output) }
    else { dac_sample(0) }        // DAC on, channel off: hold the digital-0 level
}
```

### Other output notes

Panning matches NR51 exactly ✅ (`src/audio/panning.rs:11-16`, `:27-49`). The all-DACs-off shortcut
(`src/audio/mod.rs:122-126`) is correct ✅. The DAC curve (`src/audio/dac.rs:1-22`) maps 0 → `+1.0`
and 15 → `−1.0`, sign-inverted relative to gambatte — pure convention, not a bug. Master volume
applies `(v+1)/8` then a global `/7.0` (headroom choice, not accuracy). VIN bits are read and
ignored — same as gambatte, and correct.

### CGB

- [ ] (Deferred) No CGB flag anywhere in the APU. Gambatte threads it via `PSG::init(bool cgb)` →
      sweep `+2` (`channel1.cpp:81`) and wave RAM (`channel3.h:49-69`).

> ⚠️ **Serialisation constraint.** `CLAUDE.md` records that nothing may be added to `Audio`'s
> serialised fields — the hand-written `Encode`/`Decode` at `src/audio/mod.rs:324-370` would change
> the bincode layout and invalidate all 27 fixtures in `src/pokemon/data/*.bin`. So a `cgb` flag
> must be applied **from outside**, like `set_output_sample_rate` (`src/audio/mod.rs:72-74`) —
> a `set_cgb(bool)` called by the loader after `decode`, **not** a `Decode`d field.
> See [`01-architecture.md`](01-architecture.md) for a scheme that lifts this restriction.

---

## 11. Duty waveform phase — the cheapest real bug

### How gambatte does it

```cpp
// duty_unit.cpp:28-30
bool toOutState(unsigned duty, unsigned pos) {
    return 0x7EE18180 >> (duty * duty_pattern_len + pos) & 1;
}
```

Unpacking `0x7EE18180` (LSB = duty 0): duty 0 → `0x80` → pos {7}; duty 1 → `0x81` → {0,7};
duty 2 → `0xE1` → {0,5,6,7}; duty 3 → `0x7E` → {1,2,3,4,5,6}. That is the canonical
`00000001 / 10000001 / 10000111 / 01111110`.

### How gb does it

```rust
// src/audio/square_channel.rs:218-227
fn waveform_bit(&self) -> bool {
    let bit = 7 - self.frequency_timer.phase();
    match self.wave_duty_cycle {
        0 => bit == 0,  // 12.5%
        1 => bit < 2,   // 25%
        2 => bit < 4,   // 50%
        3 => bit > 1,   // 75%
        _ => unreachable!(),
    }
}
```

| duty | gb positions high | gambatte | match |
|---|---|---|---|
| 0 | 7 | 7 | ✅ |
| 1 | 6, 7 | 7, 0 | ❌ rotated −1 |
| 2 | 4, 5, 6, 7 | 5, 6, 7, 0 | ❌ rotated −1 |
| 3 | 0, 1, 2, 3, 4, 5 | 1, 2, 3, 4, 5, 6 | ❌ rotated −1 |

### Gap

The *ratios* are all correct (1/8, 2/8, 4/8, 6/8) and each pattern is cyclically contiguous, so a
steady note sounds right. But duties 1–3 are rotated one position relative to duty 0, so:

- a mid-note duty change produces the wrong instantaneous level and a spurious edge
- the phase relative to a trigger is wrong for duties 1–3
- two channels at the same frequency with different duties have the wrong relative phase

### Tasks

- [ ] Replace with the packed table:

```rust
fn waveform_bit(&self) -> bool {
    const DUTY_PATTERNS: u32 = 0x7EE1_8180;
    let pos = self.frequency_timer.phase() & 7;
    (DUTY_PATTERNS >> (self.wave_duty_cycle as u32 * 8 + pos as u32)) & 1 != 0
}
```

- [ ] Add a unit test asserting the four rows of the table above.
- [ ] **Also fix the trigger-position reset** — `trigger()` (`:148-174`) calls
      `frequency_timer.trigger()` which sets `phase = 0`, but gambatte's `DutyUnit::nr4Change`
      (`duty_unit.cpp:109-116`) deliberately does **not** touch `pos_`/`high_` on trigger; only
      power-off `reset()` does. The `TODO` already at `src/audio/timer.rs:40` ("the low two bits of
      the frequency timer are NOT modified") is gambatte's
      `cc - (cc - ref) % 2 + period_ + 4` at `duty_unit.cpp:113`. Fixing the pattern table without
      also fixing this leaves `ch1_init_pos_1..8` failing.

This change is a strict improvement even without §1, and is verifiable against the existing blargg
suite (nothing there depends on duty phase, so all 9 should stay green).

---

## Suggested implementation order

1. [ ] §11 duty table (3 lines, no dependencies, immediately verifiable)
2. [ ] §10 square/noise DC level on channel-off (4 lines, audibly better)
3. [ ] §7 shift ≥ 14 LFSR stop (2 lines)
4. [ ] §5 remove the NR10 sweep-timer reload (3 lines)
5. [ ] §8 `initialised: false` (1 line) — re-run blargg after
6. [ ] §2 DIV-write frame-sequencer clock (small, self-contained, unlocks ~20 gambatte tests)
7. [ ] §4 envelope zombie mode (unlocks the SameSuite `nrx2_glitch` family)
8. [ ] §6 wave RAM — "always locked while active" DMG approximation first, to re-enable blargg
       `09`/`12`; exact window once §1 Tier 2 exists
9. [ ] §11 trigger-preserves-duty-position + §6 wave `+3` delay (care around the fixture chain)
10. [ ] §1 Tier 1 sub-instruction stepping, then Tier 2 write resync
11. [ ] §10 / §3 CGB support (respecting the `Audio` serialisation constraint)

Items 1–5 total roughly **13 lines** and are independently verifiable against the blargg suite
already wired up.

---

## References

- Pan Docs, Audio chapter — <https://gbdev.io/pandocs/Audio.html>
- Game Boy Sound Hardware (gbdev wiki) — <https://gbdev.gg8.se/wiki/articles/Gameboy_sound_hardware>
- blargg `dmg_sound` / `cgb_sound` — <https://github.com/retrio/gb-test-roms>
- SameSuite `apu/*` — <https://github.com/LIJI32/SameSuite>
- gambatte's own suite — `/home/alex/projects/gambatte/test/hwtests/sound/` (164 tests)
