# gb emulator — phased implementation plan

**Status:** ACTIVE · **Created:** 2026-08-04 · **Owner:** Alex (`alex.haslehurst@gmail.com`)
**Repo under work:** `/home/alex/projects/gb` · **Reference (READ-ONLY):** `/home/alex/projects/gambatte`
**Location:** `docs/compatibility/` inside the repo. Updating this file *is* part of doing the work —
see §1.3.

---

# 0. READ THIS FIRST

You are one of several agents executing this plan **sequentially**, across separate sessions. You
will not have the previous agent's context. This document *is* the shared memory. Read §1 (protocol)
before doing anything, then find the first task in the [Status Board](#3-status-board) that is not
`DONE`.

## 0.1 What this plan covers, in priority order

| Phase | Goal |
|---|---|
| **A — Stabilise** | Fix crash-class bugs, latent data-loss bugs, and cheap accuracy defects. Put the safety rails (savestate versioning, core-only benchmark) in place that later phases depend on |
| **B — CGB** | Game Boy Color support, **including the boot-ROM-baked DMG-compatibility palette** so Pokémon Red renders in colour as it does on real CGB hardware |
| **C — Performance** | Close the gap to gambatte. **Emulator core measured alone — the Pokémon agent is excluded from every number in this phase** |
| **D — Missing hardware** | Real MBC support (MBC1/2/3/5/HuC1, multicart), RTC, header robustness, serial/joypad fidelity |

## 0.2 What this plan explicitly does NOT cover

- ❌ **Sub-instruction (M-cycle) timing refactor.** Alex has deferred this. Peripherals continue to
  be advanced once per instruction. Several accuracy items in the companion guides are therefore
  **out of scope** and are marked `DEFERRED — needs M-cycle timing` where they arise. Do not start
  it. Do not partially start it.
- ❌ **Any change to the Pokémon agent harness** (`src/pokemon/**`) — except the narrow, explicitly
  enumerated API adaptations in §2.3.
- ❌ **Committing, pushing, or branching** unless Alex asks in that session. Treat the repo as
  read-only for version-control purposes (per global `CLAUDE.md`).
- ❌ **Any modification to `/home/alex/projects/gambatte`.** Read it, compile *copies* of it
  elsewhere, never write to it.

## 0.3 Companion documents

Background research lives beside this file. Each task cites the relevant section.

| Doc | Use it for |
|---|---|
| [`00-README.md`](00-README.md) | Overview and cross-cutting roadmap |
| [`01-architecture.md`](01-architecture.md) | Scheduling, memory hot path, perf measurements, savestates |
| [`02-cpu.md`](02-cpu.md) | CPU, interrupts, DIV/TIMA, HALT, STOP |
| [`03-ppu.md`](03-ppu.md) | PPU, STAT, window, sprites, CGB video inventory |
| [`04-apu.md`](04-apu.md) | APU behaviour and quirks |
| [`05-mmu-cartridge.md`](05-mmu-cartridge.md) | MBCs, memory map, OAM DMA, boot state |
| [`06-features-and-robustness.md`](06-features-and-robustness.md) | Feature matrix, panic audit |
| [`07-testing.md`](07-testing.md) | Test suites and harness design |

---

# 1. AGENT PROTOCOL

This section defines how agents communicate through this document across sessions. **Follow it
exactly.** It is the only thing keeping a multi-session effort coherent.

## 1.1 Session lifecycle

**On session start — do all five, in order:**

1. Read §0, §1, §2 of this document.
2. Read the [Status Board](#3-status-board). Identify the lowest-numbered task not in state `DONE`
   or `SKIPPED`.
3. Read the **last three entries** of the [Ledger](#8-ledger). They carry warnings, gotchas and
   in-flight context that the task definitions do not.
4. Run the **baseline check** (§1.6). If it fails *before you have changed anything*, stop and log a
   `BLOCKED` ledger entry — do not build on a broken tree.
5. Set your chosen task's state to `IN_PROGRESS` in the Status Board, with your session date.

**On session end — always, even if you ran out of time or the work failed:**

1. Update the task's state in the Status Board.
2. Append a Ledger entry (§1.4). **This is mandatory.** A session that leaves no Ledger entry has
   destroyed its own context for no benefit.
3. If you left the tree dirty, say exactly which files and why.

## 1.2 Task states

| State | Meaning |
|---|---|
| `TODO` | Not started |
| `IN_PROGRESS` | Claimed by a session. Includes the date. If you find a stale `IN_PROGRESS` from an earlier date, read the Ledger — it was probably interrupted; you may take it over |
| `DONE` | Implemented **and** its Verification block passes |
| `BLOCKED` | Cannot proceed. **Must** cite the blocker and the Ledger entry explaining it |
| `SKIPPED` | Deliberately not done. **Must** cite who decided and why. Only Alex may authorise a `SKIPPED` |
| `DEFERRED` | Out of scope for this plan (e.g. needs M-cycle timing). Informational only |

## 1.3 Editing rules for this document

- ✅ You **may** edit: the Status Board state/date/notes columns, and append to the Ledger.
- ✅ You **may** add a new task (e.g. `A9`) if you discover necessary work — append it to the end of
  its phase, give it a fresh ID, and log why in the Ledger. Never renumber existing tasks; IDs are
  referenced from Ledger entries and must be stable.
- ⚠️ You **may** correct a factual error in a task definition — but you **must** log the correction
  in the Ledger with the old and new claim. The research behind this plan was thorough but not
  infallible; if the code disagrees with the document, **the code wins**.
- ❌ You **may not** silently reorder phases, delete tasks, or relax an exit criterion.

## 1.4 Ledger entry format

Append to §8. Newest at the bottom. Use exactly this shape:

```markdown
### YYYY-MM-DD — <task IDs touched> — <one-line outcome>

**State:** <what changed: task IDs and their new states>
**Did:** <what you actually implemented, 2-6 lines, with file:line where useful>
**Verified:** <the exact commands you ran and their results — real output, never assumed>
**Surprises:** <anything that contradicted this plan or the companion guides. Be specific>
**Tree:** <clean | dirty: list files and why>
**Next agent:** <the single most useful thing for them to know>
```

Rules for Ledger entries:

- **Report outcomes faithfully.** If tests failed, say so and paste the failure. If you skipped a
  verification step, say that. A Ledger that overstates completion is worse than no Ledger.
- Under **Surprises**, record things that cost you time. That is the highest-value field in the
  document.

## 1.5 The prime directives

1. **Never touch `src/pokemon/**` except as permitted by §2.3.** Another agent works in this repo on
   the Pokémon post-game. Colliding with it wastes both efforts.
2. **Never regenerate fixtures without following §2.4.**
3. **Never claim a task `DONE` without running its Verification block** and pasting the result into
   the Ledger.
4. **The reference emulator is read-only.** If you need to run gambatte, copy sources out (§2.5).
5. **Do not start the M-cycle refactor.** If a task seems to require it, mark it `BLOCKED`, log why,
   and move on — do not improvise a half-refactor.

## 1.6 Baseline check

This is the "is the tree healthy?" gate. Run it at session start and before claiming any task
`DONE`.

```bash
cd /home/alex/projects/gb
cargo test --release --bin gb 2>&1 | tail -5
```

Expected: **all tests pass** (966+ tests, ~22 s). Note the count — if it drops, something was
silently removed.

Then confirm no committed fixture was disturbed:

```bash
git status --porcelain src/pokemon/data/ | grep '^ M' || echo "fixtures clean"
```

Expected: `fixtures clean`. Untracked (`??`) files there belong to the other agent — leave them
alone.

## 1.7 Escalation

Stop and ask Alex when:

- A task requires touching `src/pokemon/**` beyond §2.3.
- A fix would require the M-cycle refactor to be correct.
- You must break savestate compatibility in a way §2.4 does not cover.
- A phase's exit criteria cannot be met and you believe the criteria are wrong.

Do **not** silently redefine scope. Log a `BLOCKED` entry and surface it.

---

# 2. GLOBAL CONSTRAINTS & INVARIANTS

## 2.1 Build and test commands

Always `--release` — debug builds are unusably slow. There is **no lib target**; it is `--bin gb`.

```bash
cargo test --release --bin gb                          # default tier, ~22 s
cargo test --release --bin gb -- game_boy::tests       # ROM-based tests (31)
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests
```

⚠️ **Known documentation bug** — the benchmark command in `CLAUDE.md` matches zero tests. The
working form is:

```bash
cargo test --release --bin gb -- \
  pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture
```

Fixing that line in `CLAUDE.md` is task **A10**.

## 2.2 Files you own vs files you must not touch

| Path | Status |
|---|---|
| `src/core.rs`, `opcode.rs`, `registers.rs`, `cycles.rs`, `interrupt.rs` | ✅ yours |
| `src/mmu.rs`, `ram.rs`, `header.rs`, `lcd_dma.rs` | ✅ yours |
| `src/ppu.rs`, `lcd_*.rs`, `geometry.rs` | ✅ yours |
| `src/audio/**` | ✅ yours |
| `src/timer.rs`, `divider.rs`, `serial.rs`, `joypad.rs`, `activation.rs` | ✅ yours |
| `src/game_boy.rs` | ✅ yours (public API — see §2.3) |
| `src/roms/**` | ✅ yours (test ROM plumbing) |
| `src/sdl/**` | ⚠️ touch only to keep it compiling |
| **`src/pokemon/**`** | ❌ **another agent's work.** See §2.3 |
| `Cargo.toml` | ⚠️ features/profile only; do not add dependencies without asking |
| `CLAUDE.md` | ⚠️ factual corrections only (A10) |
| `docs/compatibility/**` | ✅ yours — **you are expected to edit `10-implementation-plan.md`** (§1.3) |
| `/home/alex/projects/gambatte` | ❌ **read-only, always** |

## 2.3 Permitted API changes (the only sanctioned reason to touch `src/pokemon/**`)

The emulator's public surface is `GameBoy` / `Core` / `MMU` accessors. These changes are
**pre-authorised**, provided you keep the old entry points working wherever practical:

1. **Adding constructors** — `GameBoy::cgb(cart)`, `GameBoy::new(cart, Model)`. `GameBoy::dmg(cart)`
   has **89 call sites**; it must keep working with identical behaviour. Add, don't replace.
2. **Adding methods** — `run_frame()`, `reset()`, `try_dmg()`, `set_video_enabled()`.
3. **Changing the framebuffer pixel type** (needed for CGB, task **B4**). Verified blast radius
   outside the core is **one line**: `src/sdl/render.rs:273`. The Pokémon layer touches pixels only
   via `save_screenshot_to_file`, and `PokemonTextReader` reads **VRAM**, not the framebuffer.
   Keep `ppu.screenshot()` returning an `RgbImage` so test call sites are unaffected.

If a change you need is **not** on this list, escalate (§1.7). Mechanical fixes to keep
`src/pokemon/**` *compiling* (e.g. a renamed method) are acceptable — but log every such file in the
Ledger so the other agent can see it.

## 2.4 Fixture protocol — read before changing anything stateful

`src/pokemon/data/*.bin` holds **91 committed emulator snapshots (1.4 MB)** — not the 27 that
`CLAUDE.md` claims. They are `include_bytes!`'d and are the inputs to the tiered Pokémon tests.

**Any change to a serialised struct's field layout invalidates all of them.** That is the constraint
`CLAUDE.md` records as "nothing may be added to `Audio`'s serialised fields".

Rules:

1. **Task A0 (savestate format) must be `DONE` before any task that changes serialised state.**
   Phases B, C and D all do. A0 is the gate. Do not route around it. Within Phase A, A6 and A7 also
   depend on it.
2. Never run with `--features regen-fixtures` unless a task explicitly says to.
3. If you must regenerate, run the affected legs **in chain order**, then verify the diff is
   plausible and log it.
4. After A0 lands: adding a **field** means appending it within its section and bumping that
   section's version; adding a **section** is free. **No fixture churn** either way. A0 itself is the
   one sanctioned mass-regeneration, done offline with content-equality verification.

## 2.5 Building gambatte for reference/benchmarking

You will want to compare behaviour or speed. **Never build inside the gambatte tree.**

```bash
SCRATCH="${TMPDIR:-/tmp}/gbref"; mkdir -p "$SCRATCH" && cd "$SCRATCH"
g++ -O2 -fomit-frame-pointer -fno-exceptions -fno-rtti \
    -I/home/alex/projects/gambatte/libgambatte/include \
    -I/home/alex/projects/gambatte/libgambatte/src \
    /home/alex/projects/gambatte/libgambatte/src/*.cpp \
    /home/alex/projects/gambatte/libgambatte/src/mem/*.cpp \
    /home/alex/projects/gambatte/libgambatte/src/sound/*.cpp \
    /home/alex/projects/gambatte/libgambatte/src/video/*.cpp \
    /home/alex/projects/gambatte/libgambatte/src/file/file.cpp \
    harness.cpp -o bench
```

⚠️ **Corrected 2026-08-05 (ledger #4).** As originally written this recipe does not link. Two
things were missing, both verified by building it:
- `-I/home/alex/projects/gambatte/common` — `memptrs.h` includes `array.h`, `video.h` includes
  `scoped_ptr.h`, and both live in `gambatte/common`, not under `libgambatte/`.
- `libgambatte/src/file/file.cpp` — otherwise `gambatte::newFileInstance` is undefined. Use
  `file.cpp`, **not** `file_zip.cpp`, which needs zlib.

Also note `runFor` takes `gambatte::uint_least32_t*` (a `long unsigned int*` here), not
`uint32_t*`; declaring the video/audio buffers as `uint32_t` fails to compile.

Write `harness.cpp` yourself; it needs `GB::load(path, flags)` then a loop over
`GB::runFor(video, 160, audio, samples)` with `samples = 35112` (one frame; 1 stereo sample =
2 t-cycles). Do 60 warm-up frames before timing. ⚠️ gambatte writes battery saves from its
destructor — **copy the ROM and `.sav` into the scratch dir** so it cannot write into the project.

Reference numbers already measured on this machine (AMD Ryzen 9 7900X), for orientation:

| Core | Workload | Realtime |
|---|---|---|
| gambatte | Pokémon Red, in-game | **457×** |
| gambatte | `cpu_instrs.gb` (never HALTs) | **428×** |
| gambatte | `dmg-acid2.gb` | **622×** |
| gb | core + agent | **~24×** |

---

# 3. STATUS BOARD

**Update the State and Date columns as you work.** This is the first thing the next agent reads.

## Phase A — Stabilise

| ID | Task | State | Date | Notes |
|---|---|---|---|---|
| **A0** | **New sectioned savestate format + one-time fixture conversion** | DONE | 2026-08-04 | Container v1, 10 live + 3 reserved sections. All 91 fixtures converted, −22.4%. Rules in `src/savestate/mod.rs` |
| A1 | `Core::reset()` is `todo!()` — implement it | DONE | 2026-08-05 | `MMU::reset()` preserves SRAM; equals fresh construction |
| A2 | ~~`run()` livelocks on STOP/Crash~~ → no such bug | DONE | 2026-08-05 | ⚠️ **premise was wrong twice** — see A2 + ledger #4. Closed with a guard test; the real fixes were A3/A4 |
| A3 | STOP permanently kills DIV/TIMA/APU (`restart()` dead) | DONE | 2026-08-05 | `restart()` on wake + STOP consumes its pad byte |
| A4 | Illegal opcode freezes whole machine; `println!` in hot path | DONE | 2026-08-05 | now `IE=0` + `Halt`; peripherals keep running; `println!` gone |
| A5 | ROM bank index vs actual ROM length → panic | DONE | 2026-08-05 | ROM padded to a power of two with `0xFF`; clamp follows the data, not `0x148` |
| A6 | Sprite-height OOB panic on mid-scanline LCDC write | DONE | 2026-08-05 | height captured at OAM-scan time; slice lengths validated |
| A7 | OAM DMA: silent drop, inverted gate, `0xA0`→`0x80` source bug | DONE | 2026-08-05 | incremental transfer; `dma` section → **v2**, 91 fixtures converted on read, none regenerated |
| A8 | ~~Versioned savestate envelope~~ | SUPERSEDED | 2026-08-04 | **Replaced by A0.** Version-guarded appends alone cannot carry the plan — see ledger 2026-08-04 (#2) |
| A9 | Core-only benchmark harness + baseline capture | DONE | 2026-08-05 | `game_boy::tests::bench_core_throughput`; gambatte harness built too. Baselines in ledger #4 |
| A10 | Fix stale facts in `CLAUDE.md` | DONE | 2026-08-05 | 5 corrections; both bench commands verified to run |
| A11 | PPU quadratic x-advance | DONE | 2026-08-05 | linear advance + flush on leaving mode 3; every screenshot test unchanged |
| A12 | Cheap APU fixes (duty rotation, DAC click, LFSR, sweep, init flag) | DONE | 2026-08-05 | all 5; blargg dmg_sound 9/9 green after each |
| A13 | I/O read-back masks + `0xFEA0` region | DONE | 2026-08-05 | `irq` section → **v2** (IE upper bits appended, no fixture churn) |
| A14 | Wire 19 more blargg ROMs | DONE | 2026-08-05 | 0/19 pass, as expected. ⚠️ only 4 report over serial — see A15 |
| A15 | Screen-output test harness (15 of A14's ROMs emit no serial) | TODO | | measurement · added 2026-08-05 |

## Phase B — CGB

| ID | Task | State | Date | Notes |
|---|---|---|---|---|
| B1 | Machine model plumbing (`Model` enum, `GameBoy::cgb`) | TODO | | depends A0 |
| B2 | WRAM banking (SVBK) + VRAM banking (VBK) | TODO | | |
| B3 | CGB palette RAM (BCPS/BCPD/OCPS/OCPD) | TODO | | |
| B4 | Framebuffer pixel type → RGB555 | TODO | | API change, §2.3 |
| B5 | **DMG-compatibility palette (the Pokémon Red colour path)** | TODO | | ⭐ the headline deliverable |
| B6 | BG map attributes + CGB sprite priority | TODO | | |
| B7 | KEY1 double-speed | TODO | | |
| B8 | HDMA / GDMA | TODO | | |
| B9 | CGB post-boot state + remaining CGB registers | TODO | | |
| B10 | CGB test-ROM adoption (cgb-acid2) | TODO | | |

## Phase C — Performance (emulator core alone)

| ID | Task | State | Date | Notes |
|---|---|---|---|---|
| C1 | Event scheduler skeleton (`Schedule`, absolute clock) | TODO | | |
| C2 | HALT fast-path | TODO | | 65% of cycles |
| C3 | Closed-form APU timers | TODO | | must land with C2 |
| C4 | Mix-on-change in `Audio::update` | TODO | | |
| C5 | Whole-scanline rendering + hoist sprite search | TODO | | |
| C6 | Memory page table | TODO | | |
| C7 | Cheap decode + drop per-instruction IRQ poll | TODO | | |
| C8 | Optional headless mode | TODO | | |

## Phase D — Missing hardware

| ID | Task | State | Date | Notes |
|---|---|---|---|---|
| D1 | ROM padding + bank masking (prereq for all MBCs) | TODO | | |
| D2 | `trait Mbc` + dispatch on `CartType` | TODO | | |
| D3 | MBC1 (+ multicart) | TODO | | |
| D4 | MBC2 | TODO | | |
| D5 | MBC3 + RTC | TODO | | |
| D6 | MBC5 | TODO | | |
| D7 | HuC1 + unsupported-mapper errors | TODO | | |
| D8 | Header parsing robustness | TODO | | |
| D9 | Serial + joypad fidelity | TODO | | |
| D10 | MBC test-ROM adoption (mooneye `emulator-only/`) | TODO | | |

---

# 4. PHASE A — STABILISE

**Goal:** eliminate every crash-class and data-loss bug, land the safety rails later phases need,
and take the cheap accuracy wins.

**Exit criteria:**
- Baseline check passes.
- **A0 `DONE` first**, then A1–A7.
- No `todo!()`, and no `println!`, reachable from the emulator core's hot path.
- Sectioned savestate format in place; all 91 fixtures converted **once**, with content equality
  proven across the conversion, and the `slow-tests` tier still green.
- A core-only benchmark exists and a baseline number is recorded in the Ledger.

---

### A0 — New sectioned savestate format + one-time fixture conversion ⚠️ DO THIS FIRST

**State:** DONE (2026-08-04) · **Depends:** none · **Risk:** medium · **Blocks:** A6, A7, and all of B, C, D

> ✅ **Landed.** The authoritative reference is now the module doc at the top of
> `src/savestate/mod.rs`, not this task definition. See ledger `2026-08-04 (#3)`.

**Why the original plan (A8) was wrong.** Savestates are positional `bincode` — field order *is* the
schema — and **bincode has no schema-migration support whatsoever** (verified against the vendored
`bincode-2.0.1` source: the only "migration" it ships is a bincode-1→2 *API* guide; the format has
no field names and no tags, so `#[serde(default)]`-style tolerance is impossible).

A version-guarded append (`if version >= N { read D }`) does work — decode is sequential, so an old
file simply has no `D` bytes and no legacy struct is needed. **But appends are the only change it
survives.** The plan requires, in Phases B and C:

| Change | Phase | Is it an append? |
|---|---|---|
| VRAM `[u8; 0x2000]` → 16 KB (CGB bank 1) | B2 | ❌ size change |
| WRAM `[u8; 0x2000]` → 32 KB (SVBK) | B2 | ❌ size change |
| Framebuffer `DMGColor` → `Rgb555` | B4 | ❌ type change |
| `MachineCycles(usize)` → `u64` | C1 | ❌ type change, in **every** cycle field in the tree |
| CGB palette RAM | B3 | ✅ append |
| MBC fields → `Box<dyn Mbc>` | D2 | ❌ restructure |
| `Sprite` gains a height field | A6 | ❌ nested, inside a serialised `Vec` |
| `LcdDmaState` gains `pos` | A7 | ✅ append |

So: build the tolerant format **now**, convert once, and stop paying this tax.

**The key insight that makes this cheap: do it at HEAD, before any struct changes.** At this commit
the old and new *structs* are identical — only the *encoding* differs. You need **two codecs, not
two struct sets.** The "keep the old structs around" problem collapses from 41 types to **exactly
one** (see step 3).

#### Do

**1. Sectioned container.** Replace the bare `lz4(bincode(GameBoy))` with a labelled container
modelled on gambatte's `statesaver.cpp:417-445`:

```
"GBST" | u16 container_version | lz4 { repeat: [label\0][u32 len][payload] }
```

- payload = `bincode` of that section, prefixed with its own `u16` section version
- **unknown label → `skip(len)` and continue** (forward compatible)
- **missing label → section keeps `Default`** (backward compatible)
- adding / removing / reordering a *section* is free, forever

**2. Section taxonomy — declare these now**, including ones that are empty until later phases. Empty
sections cost nothing (they are simply absent) and mean later phases never restructure the
container:

| Label | Contents | Populated from |
|---|---|---|
| `cpu` | `RegisterSet`, `CoreMode`, IME + EI latch | now |
| `cart` | header, bank registers, `ram_enabled`, SRAM banks | now |
| `wram` | work RAM (+ SVBK bank index) | now / B2 |
| `hram` | high RAM | now |
| `ppu` | VRAM, OAM, LCDC/STAT/scroll/window/palettes, mode counters | now / B2–B6 |
| `apu` | the four channels, frame sequencer, panning, master volume | now |
| `timer` | divider + timer + serial | now |
| `dma` | OAM DMA state | now / A7 |
| `irq` | IE / IF | now |
| `cgb` | speed, VBK/SVBK, CGB palette RAM, HDMA | **B (empty for now)** |
| `sched` | absolute clock + event schedule | **C (empty for now)** |
| `mbc` | mapper-specific state, RTC | **D (empty for now)** |

⚠️ **Do not try to pre-declare future *fields*** — you will guess wrong and be stuck with dead
slots. Pre-declare *sections* only. Within a section, appends stay free via the section version;
a shape change bumps that one section's version and is localised.

**3. Drop derived/transient state from serialisation — do this in the same pass.** `PPU` currently
serialises:

```rust
// src/ppu.rs:13-30
lcd: [DMGColor; LCD_WIDTH * LCD_HEIGHT],   // 23,040 entries of *derived output*
scanline_sprites: Vec<Sprite>,             // transient, rebuilt every scanline
```

Neither belongs in a savestate. Removing them shrinks all 91 fixtures **and removes B4's framebuffer
type change from the save format entirely** — after this, B4 stops being a serialisation concern.

This is the *only* shape change at HEAD, so **`PPU` is the only type needing a legacy copy.** Put a
verbatim copy in `src/savestate/legacy_v0.rs` as `PpuV0` (keep its `derive(Decode)`), used solely by
the converter, and delete it once conversion is committed.

⚠️ **Consequence:** `save_and_load_state` (`src/game_boy.rs:106-126`) asserts round-trip
`PartialEq`. `PPU`'s `PartialEq` must therefore exclude `lcd` and `scanline_sprites` — exactly the
precedent `Audio` already sets by excluding `output` (`src/audio/mod.rs:309-320`). Implement
`PartialEq` by hand for `PPU`.

**4. The offline converter.** A `#[test] #[ignore]` maintenance tool (not a shipped binary):

```
for each src/pokemon/data/*.bin:
    decode with the v0 path  (derived impls + PpuV0)   ->  in-memory GameBoy
    encode with the new sectioned writer                ->  overwrite the file
```

Run it **once**, review the diff (all 91 files change; sizes should *drop*), and commit.
`pokemon-red.sav` is raw SRAM, not a savestate — leave it alone.

**5. Guard test in the *default* tier.** Cheap (1.4 MB, milliseconds) and turns a layout break from
a confusing `slow-tests` failure hours later into a 2-second failure with the fix in the message:

```rust
#[test]
fn every_committed_fixture_decodes() { /* ... */ }
```

**6. Optional, take it if it falls out cleanly:** split `AudioState` from `Audio` and `MmuState`
from `MMU` ([`01-architecture.md` §6.1](01-architecture.md#61-migration-plan--five-steps-each-shippable)
step 5). This retires the hand-written impls and the `CLAUDE.md` "never add a field to `Audio`" rule
permanently. If it balloons, defer it — the sectioned container already solves the immediate
problem.

#### Verify

1. **Content equality across the conversion** — the strongest available check. Before overwriting,
   assert `decode_v0(old_bytes) == decode_new(new_bytes)` for all 91, comparing on the *emulator
   state* (with `lcd`/`scanline_sprites` excluded by the new `PartialEq`).
2. Full default tier passes.
3. `cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests` passes —
   this is the real proof the converted fixtures still drive the agent correctly.
4. Prove tolerance both ways: add a throwaway section, confirm an old reader skips it; remove a
   section, confirm the reader defaults it.
5. Record the fixture size before/after.

#### Ledger note

⭐ Record: the container version shipped, the **exact section list**, the rule for adding a field
(append within a section + bump that section's version), the rule for adding a section (just add
it), and confirmation that `legacy_v0.rs` was deleted. Every later agent depends on this entry.

---

### A1 — Implement `Core::reset()`

**State:** TODO · **Depends:** none · **Risk:** low

**Why.** `src/core.rs:41-43` is `todo!()`, reached via `GameBoy::reset()` (`src/game_boy.rs:38`).
Any caller panics. A long-running agent has no in-process recovery from a wedged game.

**Do.** Model on gambatte's `GB::reset()` (`gambatte.cpp:79-89`): re-apply the initial state,
**preserve cartridge SRAM**, and guarantee `reset()` produces a machine identical to a fresh
construction. Factor the init out of `Core::dmg` so both paths share it.

**Verify.**
```rust
#[test]
fn reset_matches_fresh_construction() {
    let mut a = GameBoy::dmg(crate::roms::acid::ROM);
    a.run(MachineCycles::from_m(500_000));
    a.reset();
    let b = GameBoy::dmg(crate::roms::acid::ROM);
    assert_eq!(a, b);   // GameBoy is PartialEq + Eq
}
```
Plus: SRAM survives a reset.

**Fixture impact.** None.

---

### A2 — ~~`run()` livelocks on STOP / Crash~~ — NO SUCH BUG

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low

> ⚠️ **This task's premise was wrong, and the first correction to it was also wrong.** Both
> readings are recorded here because the next person to read `Core::execute` will have the same
> two thoughts. See ledger `2026-08-05 (#4)`.

**Claim 1 (original, wrong).** *"The `CoreMode::Stop` and `CoreMode::Crash` arms both return
`MachineCycles::ZERO`, so `GameBoy::run`'s `while cycles < min_cycles` loop never terminates."*

Those `ZERO`s are **`interrupt_cycles`**, only one *addend* of what `execute` returns:

```rust
let cycles = MachineCycles::from_m(opcode.machine_cycles(condition_met));  // always >= 1
let interrupt_cycles = match self.mode { /* ZERO for Stop and Crash */ };
cycles + interrupt_cycles
```

`OpCode::machine_cycles` has **no arm returning 0** — `Illegal`, `Stop` and `Halt` are all 1. So
`execute` always returns at least one M-cycle and `run` always terminates.

**Claim 2 (first correction, also wrong).** *"The CPU keeps fetching and executing instructions in
Stop and Crash."* It does not. `Core::fetch` (`src/core.rs:178-186`) already returns a **virtual
`Nop` without touching PC** whenever the mode is not `Normal`:

```rust
pub fn fetch(&mut self) -> OpCode {
    if self.mode == CoreMode::Normal { OpCode::parse(self) } else { OpCode::Nop }
}
```

**What was actually wrong** was only the peripheral half — `mmu.update` is skipped in `Stop` and
`Crash`, freezing PPU, APU, serial and DIV. That is exactly what **A3** and **A4** fix, and they
have. A2 has no separate content.

**Done:** closed with a regression guard, `game_boy::tests::run_terminates_after_stop`, which
executes `STOP` with no joypad input and asserts `run(from_m(10_000))` returns and the machine is
still in `CoreMode::Stop`. It would hang, rather than fail, if anyone ever did make `execute`
return zero.

**Lesson for later tasks.** Two successive readings of this code produced two confident and wrong
conclusions. The guides in this directory were written by reading, not by running. **Reproduce a
claimed bug before fixing it** — for A6 that meant deliberately reintroducing the bug to confirm
the new test caught it, which is what showed the first draft of that test was exercising nothing.

---

### A3 — STOP permanently kills DIV / TIMA / APU

**State:** DONE (2026-08-05) · **Depends:** A2 · **Risk:** low

**Why.** `MMU::stop()` (`src/mmu.rs:205-208`) disables the divider and timer. The wake path
(`src/core.rs:486-493`) only sets `CoreMode::Normal` — **`MMU::restart()` is never called from
anywhere.** Verified: `grep -rn "restart" --include=*.rs src/` finds only the definition at
`src/mmu.rs:210` and an unrelated RST *test* name at `src/core.rs:2032`.

Because the APU frame sequencer is clocked from `div_clocks` (`src/mmu.rs:231-234`), after any
STOP+wake **DIV is pinned at 0, TIMA is frozen, and all APU length/envelope/sweep clocking dies —
permanently.**

**Do.** Call `MMU::restart()` on wake. Also consume STOP's second byte — it is a 2-byte instruction
and `src/opcode.rs:653` fetches only one, leaving PC on the pad byte.

**Ref.** [`02-cpu.md` §2a](02-cpu.md#2a-stop-is-wrong-in-three-ways)

**Verify.** Execute `STOP`, wake via joypad, then assert DIV increments and TIMA advances. Assert PC
advanced by 2.

**Note.** The 131 072-cycle wake delay is **out of scope** — approximate wake is fine here.

---

### A4 — Illegal opcodes freeze the whole machine

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low

**Why.** `src/core.rs:472-476` sets `CoreMode::Crash` **and** calls `self.mmu.stop()`, and the
`Crash` arm never calls `mmu.update` again — so PPU, APU and serial stop dead. Gambatte freezes only
the **CPU** (`memory.cpp:344-351`: set `IE = 0`, halt) and keeps video/audio/DIV running. It also
`println!`s from the hot path.

**Do.** Replace with `self.mmu.write(0xFFFF, 0); self.mode = CoreMode::Halt;`. Delete the
`println!`.

**Verify.** gambatte's `test/hwtests/undef_ops/` has 10 DMG-reachable tests
(`undef_op_d3_dmg08_cgb04c_out01.asm` and friends). Per [`07-testing.md` §2.4](07-testing.md#24-worked-example-hwtestsdivstart_inc_1_dmg08_outabasm),
these **should already pass** with `CoreMode::Crash` — after this change confirm they still do. If
A14's harness isn't in yet, a unit test asserting the LCD keeps advancing after an illegal opcode is
sufficient.

---

### A5 — ROM bank index vs actual ROM length → panic

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low · **Blocks:** D1

**Why.** `set_rom_bank_register` (`src/mmu.rs:80-85`) clamps against `header.rom_banks()` — cart
byte `0x0148` — **not** `self.data.len()`, and `MMU::from_rom` never cross-checks. A ROM claiming 64
banks but 32 KB on disk hard-panics at `src/mmu.rs:297` on the first high-bank read.

**Do.** Adopt gambatte's approach (`cartridge.cpp:638-652`): **ignore byte `0x148`**, derive the
bank count from `data.len()` (`pow2ceil`, minimum 2), and **pad the backing buffer to a power of two
with `0xFF`**. Then clamp against the padded buffer.

Also fix `MMU::decode` setting `data: vec![]` (`src/mmu.rs:420,442`) — any read before `set_data`
panics.

**Ref.** [`05-mmu-cartridge.md` §2](05-mmu-cartridge.md#2-cartridge-header-parsing)

**Verify.** Load a deliberately truncated ROM and assert reads return `0xFF` rather than panicking.
Confirm `pokered.gbc` (32 banks, `0x148 = 0x05`) still boots and the full default tier passes.

**Note.** Keep the *clamp*-vs-*mask* distinction for D1; this task only removes the panic.

---

### A6 — Sprite-height OOB panic

**State:** DONE (2026-08-05) · **Depends:** A0 · **Risk:** low

**Why.** `scanline_sprites` is filtered at OAM-scan time using `object_size()` *then*
(`src/ppu.rs:210-217`), but `sprite_pixel` re-reads it at draw time (`src/ppu.rs:409-410`). A guest
LCDC write (`src/mmu.rs:376`) between the two flips 8×16 → 8×8, so `sprite_y` reaches 8..15 with
`object_size == Single`:

- non-flipped → `Tile::pixel` → `self.0[y*2+1]` (`src/ppu.rs:493`), index up to **31 on a 16-byte
  slice → OOB panic in release**
- flipped → `8-1-15` on `usize` → subtract-overflow in debug

The only guard is the **release-elided** `debug_assert!` at `src/ppu.rs:491`.

**Do.** Capture the sprite height at OAM-scan time and carry it in the sprite record through to
draw. (Clamping also works but is less faithful.)

**Verify.** A unit test that enables 8×16, scans, flips LCDC to 8×8 mid-scanline, and renders — must
not panic in release. Also validate `offset + length` in `read_vram_slice`/`read_wram_slice`
(`src/mmu.rs:127,140`), which check the base address but not the length.

---

### A7 — OAM DMA: silent drop, inverted gate, source mask

**State:** DONE (2026-08-05) · **Depends:** A0 · **Risk:** medium

**Why.** Three separate defects in one small subsystem:

1. **Silent data loss.** `src/lcd_dma.rs:14-31` clears `self.state = None` *before*
   `src/mmu.rs:221-227` performs the copy. So `write_oam` falls back to
   `lcd_status.mode().oam_accessible()` using the **previous** step's mode (`ppu.update` runs later).
   **If that mode is 2 or 3, all 160 bytes are silently discarded.** Pokémon Red always DMAs during
   VBlank, which is the only reason this has never surfaced.
2. **Inverted access gate.** `|| self.dma.is_active()` in all four PPU accessors
   (`src/ppu.rs:90,103,109,118`) makes VRAM/OAM *more* accessible during DMA — the opposite of
   hardware.
3. **Source mask.** `((value & 0xDF) as u16) << 8` (`src/lcd_dma.rs:11`) clears bit 5 of the page:
   `0x20`→`0x00`, `0x60`→`0x40`, and critically **`0xA0`→`0x80`, sending an SRAM-sourced DMA to
   VRAM**.

**Do.**
- Give `LcdDma` a `pos: u8` and copy **incrementally**, one byte per 4 T over 160 M-cycles. Keep
  `is_active()` true for the whole transfer.
- Add a privileged `write_oam_dma` that bypasses the mode gate; remove `|| dma.is_active()` from the
  CPU-facing accessors and add `&& !dma.is_active()` to the OAM read path.
- Replace `& 0xDF` with a source classifier mirroring `oamDmaInitSetup` (`memory.cpp:516-523`):
  `0x00-0x7F` ROM · `0x80-0x9F` VRAM · `0xA0-0xBF` SRAM · `0xC0-0xFF` WRAM with `addr & 0x1FFF`
  wrap.
- Store the written byte so `0xFF46` reads back (currently hard-coded `0`, `src/mmu.rs:325`).

**Ref.** [`05-mmu-cartridge.md` §5](05-mmu-cartridge.md#5-oam-dma)

**Verify.** Full default tier — this is the highest-regression-risk task in Phase A because every
Pokémon fixture exercises OAM DMA every frame. Then the `slow-tests` tier. Add a unit test that DMAs
while the PPU is in mode 3 and asserts OAM actually receives the data.

**Bus conflicts are OUT OF SCOPE** — they need M-cycle timing.

---

### A8 — ~~Versioned savestate envelope~~ SUPERSEDED BY A0

Version-guarded appends handle only *appended* fields. Phases B/C/D require type changes, size
changes and restructuring, which appends cannot express. See **A0** and ledger entry
`2026-08-04 (#2)`. Row retained so existing references resolve.

---

### A9 — Core-only benchmark harness

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low · **Blocks:** all of Phase C

**Why.** Alex wants Phase C measured on **the emulator alone, with the Pokémon agent excluded**. The
existing `bench_emulation_throughput` reports a full-`agent.step()` number (~24× realtime) and lives
inside `src/pokemon/integration_tests/` — the wrong module and the wrong measurement.

**Do.** Add a benchmark in the **emulator core** (e.g. `src/game_boy.rs` behind `#[ignore]`) that:

- Loads a committed savestate, then runs **only** `GameBoy::run(...)` in a loop — no agent, no
  policy, no observation.
- Reports **realtime multiplier** and **t-cycles/sec**.
- Covers at least three workloads so a single number can't mislead:
  1. Pokémon Red in-game (from a fixture) — the representative case
  2. `cpu_instrs.gb` — **never HALTs**, so it isolates raw dispatch from idle-skipping
  3. `dmg-acid2.gb` — PPU-heavy
- Does at least 60 frames of warm-up.

Then **build the gambatte comparison harness** per §2.5 and record both sets of numbers.

**Verify.** Run it three times; report the spread (variance was ±6% during earlier measurement).

**Ledger note.** ⭐ **Paste the full baseline table into the Ledger.** Every Phase C task is scored
against it. Without this number, Phase C cannot be evaluated.

---

### A10 — Fix stale facts in `CLAUDE.md`

**State:** DONE (2026-08-05) · **Depends:** A9 (so you can quote the right command) · **Risk:** none

**Do.** Three corrections:
1. The `bench_emulation_throughput` command matches **zero** tests — `--exact` needs the full module
   path (§2.1). Also document the new core-only benchmark from A9.
2. "27 committed fixtures" → **91**.
3. The accuracy claim "passes Blargg's cpu_instrs, dmg_sound, instr_timing" — `dmg_sound` is
   **9 of 12**; tests 09/10/12 are commented out at `src/game_boy.rs:236-246`, `:251-256` with
   placeholder expectations at `src/roms/mod.rs:41-46`. Say "dmg_sound 1–8 and 11".
4. *(added by A0)* The "Important notes" rule **"Nothing may be added to `Audio`'s serialised
   fields"** is obsolete and now actively misleading — it forbids something that is safe. A0
   deleted `Audio`'s hand-written `Encode`/`Decode` entirely; adding a field means appending it to
   `ApuSection` and bumping `APU_SECTION_VERSION`, with **no fixture churn**. Replace the note with
   a pointer to the rules at the top of `src/savestate/mod.rs`. The save-state-format paragraph
   above it ("serialised with `bincode` + lz4 compression … `GameBoy` implements `Encode`/`Decode`")
   is also stale for the same reason.

**Verify.** Run each documented command and confirm it does what the doc says.

---

### A11 — PPU quadratic x-advance

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** medium (visual regression surface)

**Why.** `src/ppu.rs:230` mixes a *relative* base with an *absolute* offset:

```rust
let end_x = start_x + self.current_ticks - INITIAL_FIFO_LOAD_TICKS + 1;
```

`current_ticks` is not reset within mode 3, so the offset is re-added every call. Traced with 4-T
NOPs, `current_x` goes **1, 6, 15, 28, 45, 66, 91, 120, 153, 190** — all 160 pixels are emitted
~36 T into mode 3 instead of 160. The `if x < LCD_WIDTH` guard at `:239` swallows the overshoot,
which is why nothing visibly broke. **Consequence: any register write landing >~36 cycles into
mode 3 is a no-op for that scanline.**

**Do.**
```rust
let target_x = self.current_ticks
    .saturating_sub(INITIAL_FIFO_LOAD_TICKS)
    .min(LCD_WIDTH);
self.draw_pixels(self.current_x..target_x);
self.current_x = target_x;
```
Also make the `>= drawing_ticks` branch flush any remaining pixels before switching to HBlank — it
currently never draws (masked only by the bug).

**Ref.** [`03-ppu.md` §1](03-ppu.md#1-rendering-model)

**Verify.** dmg-acid2 and the 8 `button_test` screenshot tests must still pass — the final frame
should be *identical*, since all 160 pixels are drawn either way; only *when* changes. Then the full
default tier, then `slow-tests`.

⚠️ **If any screenshot assertion moves, stop and investigate before regenerating anything.** A
changed frame means something real changed, not that the fixture is stale.

---

### A12 — Cheap APU fixes

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low

**Why.** ~13 lines total, each independently verifiable against the blargg suite already wired up.

**Do.** All five from [`04-apu.md`](04-apu.md):

1. **Duty patterns 1–3 are rotated one step** (`src/audio/square_channel.rs:218-227`). Verified by
   unpacking gambatte's `0x7EE18180` (`duty_unit.cpp:28-30`): duty 1 should be high at pos {7,0},
   gb has {6,7}; duty 2 {0,5,6,7} vs {4,5,6,7}; duty 3 {1..6} vs {0..5}. Replace with the packed
   table.
2. **Channel-off DAC click** (`src/audio/square_channel.rs:140`, `noise_channel.rs:105`). Hardware
   holds the digital-0 DAC level when a channel is disabled but its DAC is on; gb snaps to `0.0` — a
   full-scale step on every note-off. **The wave channel already does this correctly**
   (`wave_channel.rs:204-213`) — copy that.
3. **Noise LFSR must stop at clock-shift ≥ 14** (`src/audio/noise_channel.rs:137`).
4. **Remove the `sweep_timer == 0` reload** in `Sweep::set_nr10` (`src/audio/sweep.rs:42-44`) — no
   gambatte counterpart; it can re-arm an idle sweep.
5. **`initialised: true`** (`src/audio/square_channel.rs:53`) makes the first-duty-step quirk at
   `:199-203` dead code. Set it `false`.

**Verify.** `cargo test --release --bin gb -- blargg_dmg_sound` — **all 9 must stay green** after
each change. Add a unit test asserting the four duty rows.

**Do these one at a time**, re-running blargg between each, so a regression is unambiguous.

---

### A13 — I/O read-back masks and the `0xFEA0` region

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low

**Do.** In `src/mmu.rs:310-336`:
- `0xFF41` → `0x80 | stat()` (STAT bit 7 always reads 1)
- `0xFF0F` → `0xE0 | get()`
- `0xFF07` → `0xF8 | control()`
- `0xFF02` → `0x7E | control()`
- `0xFF00` → `0xC0 | get()`
- `0xFFFF` → widen `InterruptFlags` to a raw `u8` (top 3 bits are writable/readable)
- `0xFEA0..=0xFEFF` → **`0x00`** on DMG (currently `0xFF`)

The `0xFEA0` value is settled by gambatte's committed hardware dump
`test/hwtests/fexx_ffxx_dumper_dmg08.bin`, which is all zeros at offsets `0xA0..0xFF`. (CGB differs
— three 8-byte patterns each repeated 4×. Revisit in B9.)

**Ref.** [`05-mmu-cartridge.md` §4b–4c](05-mmu-cartridge.md#4b-the-unusable-region-0xfea0-0xfeff)

⚠️ `gb`'s own joypad test asserts `0x3F` for all-released (`src/joypad.rs:126`); hardware gives
`0xFF`. **Update that test** — it currently encodes the bug.

**Verify.** Default tier. Note APU masks are already correct — don't touch them.

---

### A14 — Wire 19 more blargg ROMs

**State:** DONE (2026-08-05) · **Depends:** A1–A7 (so failures reflect real gaps, not known crashes) · **Risk:** none

**Why.** `serial_console_test` (`src/game_boy.rs:352-378`) already implements blargg's convention
correctly. These ROMs need **zero new harness code** and will produce a named, measurable list of
defects.

**Do.** Add `mem_timing` (4), `mem_timing-2` (4), `halt_bug` (1), `oam_bug` (9), `interrupt_time`
(1). Source from `c-sp/game-boy-test-roms` v7.0.

**Expect most to fail.** That is the point — they quantify the instruction-granularity gap. **Do not
"fix" them**: `mem_timing`, `halt_bug` and `interrupt_time` need M-cycle timing (out of scope), and
`oam_bug` needs the DMG OAM corruption quirk (also out of scope, and gambatte doesn't model it
either).

**Do** mark them `#[ignore]` with a comment naming the blocking phase, so they are documentation
rather than noise.

⚠️ **Correction to a common belief:** blargg's *Game Boy* ROMs have **no** `$A000`/`$DE $B0 $61`
magic signature — that is his *NES* suite. Serial text is the correct mechanism.

**Ledger note.** Record the pass/fail split. It is the honest baseline for the accuracy claim.

---

### A15 — Screen-output harness for the remaining blargg ROMs

**State:** TODO · **Depends:** A14 · **Risk:** low · **Added:** 2026-08-05 (ledger #4)

**Why.** A14 assumed all 19 ROMs report over the link port and so would need *no* new harness code.
Measured: **only the four `mem_timing` ROMs do.** The other 15 — `mem_timing-2` (4), `halt_bug`,
`interrupt_time` and all nine `oam_bug` — emit **nothing** over serial and write their results to
the screen instead. They currently fail through `serial_console_test` for a harness reason, not a
fidelity one, which is exactly the kind of misleading signal A14 set out to avoid.

**Do.** Score them from the frame buffer. Two options, cheapest first:

1. **`LD B,B` breakpoint + register check** — the mooneye convention D10 also needs, so building it
   here pays for both. Run until opcode `0x40` executes, then read the registers.
2. **OCR-free screenshot compare** — blargg's screen output is a fixed font; the existing
   `ppu_test` screenshot comparison already exists but needs a reference PNG per ROM, which this
   repo does not have.

Prefer (1). Note `serial_console_test` stays correct for `cpu_instrs`, `dmg_sound` and
`mem_timing` — do not replace it.

**Verify.** The four `mem_timing` ROMs must keep reporting the same failures through the serial
path. The 15 others must produce a real pass/fail rather than a timeout.

**Expect them to still fail** for the reasons A14 gives — the point is an honest number.

---

# 5. PHASE B — CGB

**Goal:** full Game Boy Color support, with the **DMG-compatibility boot-ROM palette** as the
headline deliverable so Pokémon Red renders in colour.

**Exit criteria:**
- `GameBoy::cgb(cart)` exists; `GameBoy::dmg(cart)` is unchanged in behaviour.
- Pokémon Red runs in CGB compatibility mode with the **correct boot-ROM palette**, verified against
  a reference screenshot.
- `cgb-acid2` passes, or its failures are documented and understood.
- The full DMG test suite **still passes** — no DMG regressions.

## ⚠️ Sequencing note for whoever starts Phase B

Alex chose CGB before performance. Understood and respected — but be aware CGB code written now will
be touched again in Phase C when the event scheduler lands. **Mitigation: write every new peripheral
method in `catch_up(now)` shape** — take an absolute cycle stamp and derive elapsed time internally
— even though the driver still calls it once per instruction. Phase C then becomes a driver change
rather than a peripheral rewrite. This costs nothing now and saves real rework later.

**⚠️ A0 must be `DONE` before starting B1.** CGB changes the *shape* of serialised state (VRAM/WRAM sizes, framebuffer type, new palette RAM), not merely appending to it.

---

### B1 — Machine model plumbing

**State:** TODO · **Depends:** A0

**Do.** Introduce `Model { Dmg, Cgb }` (leave room for `Mgb`/`Sgb`). Add `GameBoy::cgb(cart)` and
`GameBoy::new(cart, Model)`; **keep `GameBoy::dmg(cart)` byte-identical in behaviour** — 89 call
sites depend on it.

Follow gambatte's shape: `isCgb()` is a **branch predicate in ~30 places, not a parallel code path**
(`cartridge.cpp:635`, `memptrs.h:100-105`). Resist forking the PPU.

Also derive **CGB-compat mode**: cart byte `0x143` bit 7. For pokered, `0x143 = 0x00` → Pokémon Red
is a **DMG-only game** that runs on CGB hardware in *compatibility* mode. That distinction drives B5.

**Verify.** Default tier unchanged. `GameBoy::cgb(pokered)` constructs and runs without panicking.

---

### B2 — WRAM and VRAM banking

**State:** TODO · **Depends:** B1

**Do.**
- **SVBK (`FF70`)**: 8 WRAM banks on CGB, 2 on DMG. **Bank 0 selects bank 1** — gambatte does this
  fixup twice, in `memory.cpp:1074-1079` and `memptrs.cpp:146-150`. Read-back is `data | 0xF8`.
- **VBK (`FF4F`)**: 2 VRAM banks. Read-back is `0xFE | data`. Allocate both banks even on DMG
  (gambatte does) and zero bank 1 on DMG state load.
- Echo RAM `0xF000-0xFDFF` must mirror the **SVBK-selected** bank.

**Verify.** Default tier (DMG must be unaffected). Unit tests for the bank-0→1 fixup and read-back
masks.

---

### B3 — CGB palette RAM

**State:** TODO · **Depends:** B2

**Do.** `BCPS/BCPD` (`FF68/69`) and `OCPS/OCPD` (`FF6A/6B`): 64 bytes each, auto-increment on write
when bit 7 of the index is set. Read-back masks: BCPS `data | 0x40`, OCPS `data | 0x40`.

Store raw **and** keep a pre-expanded RGB mirror (gambatte: `video.h:205-206`) so the pixel path
doesn't unpack per pixel.

**Mode-3 palette access blocking is DEFERRED** — it needs precise mode-3 timing. Note it in the
Ledger.

**Verify.** Write/read-back round-trip including auto-increment.

---

### B4 — Framebuffer pixel type → RGB555

**State:** TODO · **Depends:** B3 · **API change — see §2.3**

**Why.** `DMGColor` is a 2-bit shade. CGB needs 15-bit colour.

**Blast radius is small** (verified): outside the core, only `src/sdl/render.rs:273` reads
`ppu().lcd()`. The Pokémon layer touches pixels only through `save_screenshot_to_file`.

**Do.** Change the framebuffer to `Rgb555` (or `u32`). **Keep `ppu.screenshot()` returning an
`RgbImage`** so every existing screenshot test is unaffected. On DMG, map shades through the
existing palette so output is **bit-identical to today** — verify, don't assume.

⚠️ **`gb`'s DMG shades are `FF/AA/55/00`** (`src/lcd_palette.rs:17-22`), which happens to match the
palette `c-sp/game-boy-test-roms` documents for reference screenshots. Preserve that.

**Verify.** dmg-acid2 + all 8 button tests must pass **unchanged**. If any screenshot differs, the
DMG mapping is wrong.

---

### B5 — DMG-compatibility palette ⭐ THE HEADLINE DELIVERABLE

**State:** TODO · **Depends:** B4

**Why.** Pokémon Red has `0x143 = 0x00`, so on real CGB hardware the **boot ROM** picks a palette
from the cartridge title and writes it into CGB palette RAM before handing over. That is why
Pokémon Red is red-tinted on a Game Boy Color. Reproducing it is the visible payoff of Phase B.

**Inputs already derived from `pokered/pokered.gbc` — verified, use these:**

```
title 0x134..0x143  = "POKEMON RED"  (50 4f 4b 45 4d 4f 4e 20 52 45 44 00 00 00 00 00)
CGB flag 0x143      = 0x00           -> DMG game, CGB compatibility mode
title checksum      = 0x14           (sum of 0x134..0x143, & 0xFF) -> the lookup key
4th title char 0x137= 'E'            -> the disambiguator for ambiguous checksums
```

**Algorithm** (verify against a primary source before trusting this summary):
1. Compute the 8-bit sum of header bytes `0x134..0x143`. For pokered this is **`0x14`**.
2. Look it up in the boot ROM's checksum table.
3. If that checksum is one of the ambiguous entries, disambiguate using byte `0x137` (**`'E'`** for
   pokered).
4. The result indexes a palette set of three 4-colour palettes (BG, OBJ0, OBJ1), plus a flags byte.
5. Write them into CGB BG palette 0 and OBJ palettes 0 and 1.

**⚠️ Source the actual tables from a primary reference — do not trust any recollection, including
this document's.** This gambatte checkout **does not implement this feature** (verified: no title-
checksum table anywhere in `libgambatte/`; DMG mode uses a flat greyscale ramp at
`video.cpp:126-128`), so it cannot be your reference here. Use:
- **SameBoy's `BootROMs/cgb_boot.asm`** — contains the tables as data. Most authoritative.
- **Pan Docs**, "CGB Boot ROM" / DMG-compatibility palettes.

**Rendering path.** In compatibility mode the PPU still reads DMG registers, but indirects through
CGB palettes: BG pixel → `BGP` → CGB BG palette 0; OBJ pixel → `OBP0`/`OBP1` per OAM attribute
bit 4 → CGB OBJ palette 0/1. **BG map attributes are ignored** in this mode (that's B6's job for
real CGB games).

**Optional, nice-to-have:** the boot-ROM button-combo palette overrides (holding direction+button
combinations selects one of ~12 alternates). Implement only if cheap; log the decision.

**Verify.** ⭐ This one needs a **visual** check, not just an assertion:
1. Boot Pokémon Red via `GameBoy::cgb(...)`, run to the title screen, save a screenshot to
   `target/test-artifacts/`.
2. **Compare against a known-good reference** (SameBoy or BGB running the same ROM). Colours must
   match, not merely "look colourful".
3. Assert the DMG path is **unchanged** — `GameBoy::dmg(...)` must still produce the original
   greyscale output.

**Ledger note.** Record the palette index you resolved for checksum `0x14`/`'E'`, your source, and
paste the screenshot path. The next agent should not have to re-derive this.

---

### B6 — BG map attributes and CGB sprite priority

**State:** TODO · **Depends:** B5

**Do.** For true CGB games (not needed by Pokémon Red, but required for `cgb-acid2`):
- BG tile-map attributes from **VRAM bank 1** at the same tile-map offset (gambatte:
  `video/ppu.cpp:617`): palette 0-7, tile bank, X/Y flip, BG-over-OBJ priority.
- **Sprite priority by OAM index** on CGB (vs X-coordinate on DMG) — gambatte
  `video/ppu.cpp:853-884`.
- The CGB priority rule uses `(obj_attrib | bg_attrib) & bgpriority`, with LCDC bit 0 acting as a
  **master priority override** rather than "BG off".
- `OPRI` (`FF6C`) — read-back `data | 0xFE`.

**Verify.** `cgb-acid2` (task B10). DMG sprite behaviour must be **unchanged** — the existing
sprite handling is correct and is *not* to be "fixed".

---

### B7 — KEY1 double-speed

**State:** TODO · **Depends:** B6 · **Risk:** high

**Why this is the risky one.** `MachineCycles` is currently *the* clock
(`src/cycles.rs:11`, a hard `CPU_FREQ` constant). In double-speed the CPU runs at 2× while the PPU
and APU do **not**. There is no speed concept anywhere in `gb`.

**Do.**
- `FF4D`: bit 0 writable (prepare), bit 7 = current speed. CGB only.
- On `STOP` with the prepare bit set: toggle speed, **reset DIV**, and notify timer/PPU/APU.
- Thread a speed shift through the peripherals — everything the PPU and APU consume must be divided
  by the speed factor. Gambatte does this as `<< isDoubleSpeed()` in ~40 places in `memory.cpp`
  alone.

**Coordinate with Phase C.** If C1 (absolute clock + scheduler) is already done, do this on top of
it — it will be far cleaner. **If C1 is not done, consider deferring B7 until after C1 and say so in
the Ledger.** This is a legitimate reordering; log it rather than forcing it.

**Verify.** DMG unaffected. A CGB ROM that switches speed runs at the right rate (a timer-based
assertion, not a visual one).

---

### B8 — HDMA / GDMA

**State:** TODO · **Depends:** B7

**Do.** `FF51-FF55`. GDMA transfers the full length at once; HDMA transfers `0x10` bytes per HBlank.
Source reads return `0xFF` for VRAM and `>= 0xFE00`. Destination wrap sets the done bit. `FF55`
read-back: bit 7 = done.

**Accuracy caveat.** Precise HBlank-timed HDMA needs mode-0 timing that `gb` does not model
(mode 3 is a fixed 172 T). Implement HDMA **triggered at the mode-3→mode-0 transition** — correct in
ordering, approximate in cycle placement. **Document this limitation in the Ledger.** HDMA/OAM-DMA
interleaving is out of scope.

---

### B9 — CGB post-boot state and remaining registers

**State:** TODO · **Depends:** B8

**Do.**
- CGB post-boot register values (differs from DMG: A=`0x11`, and the whole I/O block).
- `FF72/73/74` (plain RW, CGB only), `FF75` (`data | 0x8F`), `FF76/77` (PCM12/34 — **gambatte does
  not implement these**; optional).
- `0xFEA0-0xFEFF` on CGB: three 8-byte patterns each repeated 4×, per
  `test/hwtests/fexx_ffxx_dumper_cgb.bin`. Unlike DMG it is ordinary RAM, not write-protected.
- CGB serial clock is 32× faster.

While here, consider the **DMG** post-boot table too (F=`0xB0` not `0x80`, LCDC=`0x91` not `0x80`,
BGP/OBP=`FC/FF/FF` not `00/00/00`, SRAM filled `0xFF`) — see
[`05-mmu-cartridge.md` §7](05-mmu-cartridge.md#7-boot-state). ⚠️ **This changes DMG boot behaviour
and may shift fixtures.** If it does, treat it as its own task, log it, and get Alex's call before
regenerating.

---

### B10 — CGB test-ROM adoption

**State:** TODO · **Depends:** B9

**Do.** Wire up `cgb-acid2` (PNG-compared, same harness shape as dmg-acid2). Optionally
`cgb_sound`. Record the pass/fail split honestly — full CGB APU differences are not in scope.

---

# 6. PHASE C — PERFORMANCE (emulator core alone)

**Goal:** close the gap to gambatte, measured on **the emulator core with the Pokémon agent
entirely excluded**.

**Baseline:** whatever A9 recorded. For orientation, gambatte measures 428× (`cpu_instrs`, never
HALTs) to 622× (`dmg-acid2`) on this machine; `gb` measured ~24× *with* the agent.

**Targets** (hypotheses — revise after C1/C2 with real data, and log the revision):
- **Primary: ≥ 120× realtime** core-only on the Pokémon in-game workload.
- **Stretch: ≥ 200×.**
- Gambatte's ~450× likely needs the deferred M-cycle/lazy-evaluation architecture. Do not
  over-promise.

**Exit criteria:**
- Primary target met, **or** a Ledger entry explaining with measurements why it is not reachable
  without the deferred refactor.
- **Zero behaviour change**: full default tier + `slow-tests` pass, and all 91 fixtures still load.
- Every task's before/after numbers recorded.

**Method rule.** Measure before and after **every** task with A9's harness, three runs each.
Variance is ±6%; a claimed 3% win is noise.

---

### C1 — Event scheduler skeleton

**State:** TODO · **Depends:** A0, A9

**Do.** Per [`01-architecture.md` §2](01-architecture.md#2-the-scheduling-model):
- Add `now: u64` (m-cycles) to `MMU` — **the absolute clock `gb` currently lacks entirely.**
- Add a dependency-free `src/schedule.rs` with `Ev` and `Schedule` (a flat `[u64; 8]` min — **do not
  port `MinKeeper`**; at N=8 a linear scan auto-vectorises and is simpler).
- Use `u64::MAX` as the disabled sentinel. **Resist `Option<u64>`** — it doubles the array and adds
  a branch per comparison.
- Convert `Timer`, `Divider`, `Serial` to `catch_up(now)`/`next_event()`. These are already
  `while cycles >= period` loops, so the conversion is behaviour-preserving.

**This task alone need not be faster.** It is the structure C2–C4 need. Verify no regression and no
significant slowdown.

**Also:** change `MachineCycles` to wrap `u64` rather than `usize`, and replace the **saturating**
`Sub`/`SubAssign` (`src/cycles.rs:76-86`) with `debug_assert!` plus explicit `saturating_sub` at the
only two sites that want it (`src/sdl/render.rs:236,239`). Saturating subtraction silently converts
cycle-ordering bugs into timing skew.

---

### C2 — HALT fast-path

**State:** TODO · **Depends:** C1 · **Expected: the single biggest win**

**Why.** Measured: **81.2% of CPU dispatches and 65.0% of emulated m-cycles are HALT.** Each
executes a virtual `Nop` *and* a full `MMU::update` over every peripheral. Gambatte skips the entire
idle span with one addition (`cpu.cpp:521-525`).

**Do.** When `mode == Halt`, jump `now` to `min(schedule.next(), end_of_slice)` instead of stepping.

⚠️ **Not free money on its own.** With today's peripherals `catch_up(delta)` is still O(delta) in
`PhaseTimer` and the PPU mode loop, so the win collapses to ~15% saved dispatch unless **C3 and C5
land with or before it.** Plan them together.

**Verify.** Behaviour identical (fixtures are the strong test here). Record before/after.

---

### C3 — Closed-form APU timers

**State:** TODO · **Depends:** C2 · **Measured share: 7.7%, growing with C2**

**Do.** Replace `PhaseTimer::update`'s `for _ in 0..ticks` (`src/audio/timer.rs:52-64`) with the
closed form, exactly as gambatte's `DutyUnit::updatePos` (`sound/duty_unit.cpp:51-58`):
`inc = (cc - next_pos_update) / period + 1`, then advance phase modularly. Same for
`NoiseChannel::update`, `Divider::update`, `Timer::update`.

**Principle:** *store when the next thing happens, not how long since the last thing happened.*

**Verify.** blargg `dmg_sound` (9) guards this well. Run after each channel.

---

### C4 — Mix on change

**State:** TODO · **Depends:** C3 · **Measured share: 10.5%**

**Why.** `Audio::update` (`src/audio/mod.rs:110-137`) recomputes 4 × `output_f32()`, 4 pans, a
volume multiply and a `/4.0` **every instruction**, whether or not anything changed.

**Do.** Only recompute and push a sample when a channel's output actually transitions. The blip
layer already accepts arbitrary sub-sample positions (16.16 cursor, `src/audio/mod.rs:142-146`), so
no resampler change is needed. **The blip layer is fine; the driving is wrong.**

**Verify.** blargg `dmg_sound`; plus render a reference WAV
(`audio::reference::tests::render_reference_wav`) and listen / compare.

---

### C5 — Whole-scanline rendering

**State:** TODO · **Depends:** C2 · **Measured share: 32.2% is pixel work**

**Do.**
1. Move the pixel loop out of `PPU::update`'s `Drawing` arm to the mode-3→HBlank transition
   (`src/ppu.rs:221-273`).
2. **Hoist the sprite search out of the pixel loop.** `src/ppu.rs:257-266` runs
   `.filter().map().filter().sorted_by_key().next()` **per pixel** — and `sorted_by_key` **allocates
   a `Vec` in the innermost loop.** Pre-sort by X once per scanline (gambatte uses a stable
   `insertionSort`, `video/sprite_mapper.cpp:180`) then scan linearly. This is likely the largest
   single line-item in the whole phase.
3. Replace the per-scanline `scanline_sprites` `Vec` (`src/ppu.rs:210-219`, ~8640 allocations/sec)
   with `[Sprite; 10]` + `len` — the DMG limit is a hard 10.

⚠️ **Preserve the sprite semantics exactly.** The 10-per-line limit taken in OAM order after a
Y-only filter, and stable X-sorting for DMG priority, are **already correct**. A naive rewrite will
break them. Re-read [`03-ppu.md` "Sprites — what is already correct"](03-ppu.md#sprites--what-is-already-correct) first.

**Verify.** dmg-acid2 + button tests + full fixture chain. This is the highest visual-regression
risk in Phase C.

---

### C6 — Memory page table

**State:** TODO · **Depends:** C5 · **Expected: 5–10%**

**Do.** Replace the 25-arm range `match` (`src/mmu.rs:284-338`) with a pre-biased offset table per
[`01-architecture.md` §3](01-architecture.md#3-memory-access-hot-path): one `Box<[u8]>` chunk plus
`[u32; 16]` read/write base tables, `SLOW = u32::MAX` as the fall-through marker.

**Use offsets, not pointers** — the struct must stay movable, `Clone` and `Encode`/`Decode`-able. No
`unsafe`.

Bonus: VRAM/SRAM blocking becomes "set the region to `SLOW`", so the fast path never tests for it —
the same trick gambatte uses for OAM-DMA conflicts.

Give HRAM its own branch (Pokémon Red's hot loops live there).

---

### C7 — Cheap decode + drop the per-instruction IRQ poll

**State:** TODO · **Depends:** C6 · **Expected: 3–8% combined**

**Do.**
1. `OpCode::parse` builds a fat enum, then `machine_cycles(condition_met)` re-matches the whole enum
   afterwards (`src/opcode.rs:577-635`) — **two full dispatches per instruction.** Replace the
   second with a `[u8; 256]` cycle table plus a conditional-taken table.
2. Replace the 5-way `Activation` poll (`src/mmu.rs:237-248`, plus a second iteration at `:251-258`)
   with `(ie & iflag).trailing_zeros()`. With C1's `catch_up` setting `interrupt_request` directly,
   `src/activation.rs` can likely be **deleted entirely**.

---

### C8 — Optional headless mode

**State:** TODO · **Depends:** C7 · **Measured: 26.2× → 38.5× in ablation**

**Do.** `set_video_enabled(false)` to skip the pixel blit, mirroring gambatte's null-`videoBuf` path
(`ppu.h:41-42`). The PPU state machine must keep running — only the pixel writes are skipped.

**Safe for most tests:** `PokemonTextReader` reads **VRAM**, not the framebuffer. dmg-acid2, the
button tests, and any `screenshot()` path must keep it enabled.

**Do not enable it by default.** Make it opt-in, and leave the Pokémon harness alone (§2.2) — Alex
can decide separately whether the test tiers use it.

---

# 7. PHASE D — MISSING HARDWARE

**Goal:** real cartridge support, so `gb` is a general Game Boy emulator rather than a
Pokémon-Red-shaped one.

**Exit criteria:** mooneye `emulator-only/mbc1|mbc2|mbc5` pass (28 ROMs); Pokémon Red still works
identically; unsupported mappers fail with a **typed error** rather than silent mis-emulation.

**Context.** `gb` today has **no MBC abstraction at all**. `CartType` is parsed
(`src/header.rs:67`) and **never dispatched on**. One hardcoded pseudo-mapper — MBC1's register
layout with MBC3's 7-bit width — serves every cartridge. `0x6000-0x7FFF` writes are **silently
dropped**, so MBC1 mode-select and the MBC3 RTC latch are both no-ops. It works only because
Pokémon Red is MBC3-no-RTC under 128 banks.

---

### D1 — ROM padding and bank masking

**State:** TODO · **Depends:** A5 · **Blocks:** D2–D7

**Do.** Complete what A5 started: derive bank count from file size, pad with `0xFF`, and **replace
every `.min()` clamp with `& (n - 1)` masking**. Hardware **wraps**; `gb` saturates
(`src/mmu.rs:82-84`, `:354`), so an out-of-range bank silently aliases to the top bank.

---

### D2 — `trait Mbc` + dispatch

**State:** TODO · **Depends:** D1, A0

**Do.**
```rust
pub enum RamTarget { Bank(usize), Rtc(RtcReg), None }
pub trait Mbc {
    fn rom_write(&mut self, addr: u16, value: u8);
    fn rom_bank(&self) -> usize;
    fn ram_target(&self) -> RamTarget;
}
```
Store `Box<dyn Mbc>` selected from `CartType`. Gambatte decodes uniformly as `switch (p >> 13 & 3)`
→ `match addr >> 13 & 3` in Rust.

⚠️ **Serialisation.** A boxed trait object needs an explicit `Encode`/`Decode`. Put it in A0's
`mbc` section, which was reserved for exactly this. **Verify all 91 fixtures still load** — Pokémon Red's MBC3 state must round-trip.

---

### D3–D7 — The mappers

**Depends:** D2. Port each `romWrite` body from `mem/cartridge.cpp`; the details are tabulated in
[`05-mmu-cartridge.md` §1](05-mmu-cartridge.md#1-mbc-support-matrix).

- **D3 — MBC1** (`cartridge.cpp:91-148`). The `0x20/0x40/0x60` aliasing is
  `bank & 0x1F ? bank : bank | 1` — note it tests the **low 5 bits**, so `0x20`→`0x21`, *not*
  `0x01`. Plus mode-select at `0x6000`, mode-1 RAM banking, and the bank-2 register wired to ROM
  (not RAM) in mode 0. Multicart (`Mbc1Multi64`) is optional — log if skipped.
- **D4 — MBC2** (`:231-242`). The register select is `p & 0x6100` — masking A14/A13 **and A8**.
  Built-in 512×4-bit RAM; header says 0 banks, so allocate 1.
- **D5 — MBC3 + RTC** (`:273-331`). ⭐ **Verify Pokémon Red is unaffected** — this is the live path.
  RTC: latch on a `0→1` edge, registers at bank `0x08-0x0C`. Model as a **`base_time` offset**
  (gambatte's `rtc.cpp`) with an **injectable time source**, not `SystemTime::now()` directly —
  wall-clock RTC makes replays non-deterministic, which matters for a fixture-driven harness.
  Persist in gambatte's 4-byte big-endian `.rtc` format for interop.
- **D6 — MBC5** (`:411-429`). 9-bit ROM bank split across `0x2000-0x2FFF` / `0x3000-0x3FFF`; 4-bit
  RAM bank; **bank 0 is NOT remapped to 1**.
- **D7 — HuC1** + typed `LoadError::UnsupportedMbc` for MMM01/MBC4/6/7/Camera/TAMA5/HuC3 (gambatte
  rejects these at load rather than mis-emulating — `cartridge.cpp:592-615`).

---

### D8 — Header parsing robustness

**State:** TODO · **Depends:** D1

**Do.** Two real bugs that reject valid cartridges:
1. **Non-UTF-8 titles are rejected** (`src/header.rs:57`). Real headers place the manufacturer code
   and CGB flag inside `0x13F-0x143`, so bytes like `0x80`/`0xC0` land in the slice. Replace the
   UTF-8 decode with a byte filter over `0x134..0x143`, truncating at the first byte
   `< 0x20 || >= 0x80`.
2. **ROM-size bytes `0x52/0x53/0x54` are rejected** (`src/header.rs:71-79`) — three legal values.
   Moot once D1 derives size from the file, but remove the error path.

Also: default unknown `0x149` to 4 banks; MBC2 → 1 bank; add the header checksum as a load-time
warning. Replace `Result<_, String>` with a `LoadError` enum, and add
`GameBoy::try_dmg(&[u8]) -> Result<Self, LoadError>` so `Core::dmg`'s `.expect()`
(`src/core.rs:34`) stops being the only path. **Delete the `println!` at `src/mmu.rs:45`.**

⚠️ **This blocks reusing gambatte's test ROMs**: they declare `0x147 = 0x03` with `0x149 = 0x00`, so
`gb` currently allocates zero RAM banks and drops every SRAM write.

---

### D9 — Serial and joypad fidelity

**State:** TODO · **Depends:** none

**Do.**
- **Serial:** add the `0x7E` read-back mask to `control()`; shift SB left by elapsed bit-periods
  filling with `1`s (`((data as u16 + 1) << n).wrapping_sub(1) as u8`) instead of jumping straight
  to `0xFF`. DIV alignment is **optional** (approximate without M-cycle timing).
  ⚠️ **`serial_console_test` depends on this path** — blargg output capture must keep working.
- **Joypad:** move interrupt generation to a `recompute()` on `FF00` read and on writes to bits 4-5,
  comparing the old and new low nibble (hardware fires on a **register-nibble** edge, not a button
  press). A13 already fixed the `0xC0` read-back.

---

### D10 — MBC test-ROM adoption

**State:** TODO · **Depends:** D3–D7

**Do.** Wire mooneye `emulator-only/mbc1` (13), `mbc2` (7), `mbc5` (8) — 28 ROMs, MIT, prebuilt in
`c-sp/game-boy-test-roms` v7.0.

Needs the **Fibonacci + `LD B,B`** harness: run until opcode `0x40` executes with
`B=3 C=5 D=8 E=13 H=21 L=34`. ⚠️ **Shortcut:** mooneye also pushes the same six bytes over the link
port, and `gb` already captures serial (`src/serial.rs:29`) — so you may need **no CPU hook at
all**. Try that first.

Gate behind a `hwtests` Cargo feature, consistent with the existing tiering. See
[`07-testing.md` §4.1](07-testing.md#41-infrastructure-needed-in-build-order).

---

# 8. LEDGER

Append-only. Newest at the bottom. Format defined in §1.4. **Every session must add an entry.**

---

### 2026-08-04 — (plan authored) — Plan created from the compatibility research

**State:** All tasks `TODO`. Nothing implemented.
**Did:** Authored this plan from the seven companion guides in this directory. Derived the CGB
palette inputs for B5 directly from `pokered/pokered.gbc` (title checksum `0x14`, 4th char `'E'`,
CGB flag `0x00` → DMG game in CGB compatibility mode). Verified the framebuffer blast radius for B4
is one line outside the core (`src/sdl/render.rs:273`), and that `GameBoy::dmg` has 89 call sites so
must be kept.
**Verified:** `cargo test --release --bin gb` → all pass. `ls src/pokemon/data/*.bin | wc -l` → **91**
files, 1.4 MB (CLAUDE.md's "27" is stale). `git status` in `/home/alex/projects/gambatte` → clean.
**Surprises:**
- This gambatte checkout is upstream sinamas ~0.5.0-era and **has no CGB DMG-compat palette table**,
  no boot-ROM support, no `setTimeMode`/`setRtcDivisorOffset`. It **cannot** be the reference for
  B5 — use SameBoy's `cgb_boot.asm`.
- Neither emulator supports SGB or a link cable (grep-verified both trees).
- The repo working tree already has uncommitted Pokémon post-game work from **another agent**. Do
  not touch `src/pokemon/**` (§2.2).
**Tree:** clean at the time of writing (these documents were authored outside the repo and moved into
`docs/compatibility/` on 2026-08-04, after the concurrent Pokemon post-game work had landed).
**Next agent:** superseded by entry #2 below — read that instead.

---

### 2026-08-04 (#2) — A0 / A8 — Savestate design reworked after review; A8 superseded by A0

**State:** **A8 → `SUPERSEDED`.** New **A0** added at the head of Phase A and made a hard dependency
of A6, A7, B1, C1, D2. Alex approved reordering.

**Did:** Alex challenged whether version-guarded appends could actually load older states without
keeping old struct copies. He was right that they can't carry this plan. Reworked the design:

- Verified **bincode 2.0.1 has no schema migration** — vendored source at
  `~/.cargo/registry/src/*/bincode-2.0.1/` contains only a bincode-1→2 *API* guide
  (`src/lib.rs:229`). The format has no field names or tags, so `#[serde(default)]`-style tolerance
  is impossible.
- Confirmed `if version >= N` *does* work for pure appends with no legacy struct (decode is
  sequential) — but the plan needs **type changes, size changes and restructuring**, which appends
  cannot express. Table of the specific offenders is in A0.
- New design: **gambatte-style sectioned/labelled container**, converted **once, offline, at HEAD**.
  Because the conversion happens before any struct change, old and new *structs* are identical —
  so it needs **two codecs, not two struct sets**.
- Found that `PPU` serialises `lcd: [DMGColor; 160*144]` (23,040 entries of derived output) **and**
  `scanline_sprites: Vec<Sprite>` (transient) — `src/ppu.rs:13-30`. Dropping both shrinks every
  fixture and **removes B4's framebuffer type change from the save format entirely**.
- That drop is the *only* shape change at HEAD, so **`PPU` is the single type needing a legacy
  copy** (`legacy_v0::PpuV0`), not all 41.

**Verified:** `grep` of the vendored bincode source (no migration API). Serialisation surface
counted: **41 derived `Encode`/`Decode` types across 26 files**, plus 3 hand-written impls (`MMU`
`src/mmu.rs:395-439`, `Audio` `src/audio/mod.rs:324-370`). `PPU` field list read directly.

**Surprises:**
- The framebuffer being in the savestate was not in any companion guide — found only by reading the
  struct. Assume other "obviously excluded" state is also included until you check.
- `Audio` already excludes `output` from `PartialEq` (`src/audio/mod.rs:309-320`). **`PPU` will need
  the same hand-written `PartialEq`** once `lcd` is dropped, or `save_and_load_state`
  (`src/game_boy.rs:106-126`) will fail its round-trip assertion.

**Tree:** clean — no repo changes; plan document only.

**Next agent:** Start at **A0**, not A1. It is the gate for A6, A7 and all of B/C/D. Do it at HEAD
**before** any behavioural change, so the converter needs two codecs rather than two struct sets.
Note that A0 deliberately regenerates all 91 fixtures **once** — that is sanctioned and is the only
mass regeneration in this plan; verify content equality across the conversion rather than trusting
it.

---

### 2026-08-05 (#3) — A0 (+ A2 premise correction) — Sectioned savestate landed; 91 fixtures converted

**State:** **A0 → `DONE`.** A6, A7, B1, C1, D2 are unblocked. **A2** kept `TODO` but its *premise*
was corrected in place (see below). A1, A3–A14 untouched — I stopped after A0 rather than rushing
the crash-class fixes.

**Did:**

- New module `src/savestate/mod.rs` (~430 lines incl. tests). Container:
  `"GBST" | u16 container_version | lz4_prepend_size { [label\0][u32 len][payload] }`, payload =
  `u16 section_version | bincode(value)*`. **Container version shipped: `1`.**
- **Section list (10 written, 3 reserved):** `cpu`, `cart`, `wram`, `hram`, `ppu`, `apu`, `timer`,
  `dma`, `irq`, **`joyp`** — plus `cgb`, `sched`, `mbc` declared as labels but not written.
  ⚠️ `joyp` is **not** in A0's original taxonomy table; `JoypadRegister` is real machine state and
  had nowhere else to live, so I added a section rather than smuggling it into `cpu` or `irq`.
- Section structs live next to their owners (`CpuSection` `src/core.rs:30`, `CartSection` /
  `IrqSection` / `TimerSection` `src/mmu.rs:43-66`, `PpuSection` `src/ppu.rs:91`, `ApuSection`
  `src/audio/mod.rs:310`) with `write_sections`/`read_sections` methods, so no field had to be made
  public. `GameBoy::save_state`/`load_state` just orchestrate.
- **Dropped from serialisation** as planned: `PPU::lcd` (23,040 `DMGColor`) and
  `PPU::scanline_sprites`. Hand-wrote `PartialEq`/`Eq` for `PPU` excluding both (`src/ppu.rs:69`),
  mirroring `Audio`'s exclusion of `output`. `read_sections` resets `lcd` to white and clears
  `scanline_sprites` on load.
- **Deleted the three retired codecs**: the derived `Encode`/`Decode` on `GameBoy`, `Core` and
  `PPU`, and the hand-written `Encode`/`Decode`/`BorrowDecode` on `MMU` and `Audio`. The emulator
  no longer implements bincode at the aggregate level at all.
- `load_state` now applies sections to a **clone of the target machine**, then swaps, so a failure
  cannot leave a half-loaded machine. It also no longer copies the ROM twice.

**⭐ THE RULES (this is the entry later agents need):**

- **Adding a section is free, forever.** Old readers skip the label; old files just lack it and the
  component keeps its current value. No fixture churn. Prefer this.
- **Adding a field to an existing section:** do **not** append to the section struct. Emit it as an
  additional *value* via `writer.write_fields(label, version, |f| { f.field(&old)?; f.field(&new) })`
  and bump that section's `*_SECTION_VERSION` constant. Read it back with
  `reader.section(label)?` then `fields.field::<T>()?`, which yields `None` when the payload
  predates the field. **No fixture churn.** Both directions are covered by
  `savestate::tests::appended_fields_are_compatible_both_ways`.
- **Changing a shipped value's type/size or field order:** bump the section version and branch on
  `FieldReader::version()`, or retire the label and add a new one. bincode is positional — never
  change a shipped shape in place.
- **`legacy_v0.rs` was never created, and the converter has been deleted.** See Surprises.

**Verified:** (real output, in the order I ran it)

- Baseline before any change: `cargo test --release --bin gb` →
  `test result: ok. 866 passed; 0 failed; 121 ignored`; `git status --porcelain src/pokemon/data/`
  → `fixtures clean`. (Plan §1.6 predicts "966+ tests" — the true figure at HEAD is **866 passing /
  121 ignored = 987 total**. §1.6 is off; treat 866/121 as the baseline.)
- Conversion, with per-file content equality asserted **before** each write:
  `converted 91/91 fixtures: 1164099 -> 903368 bytes (-22.4%)`. Directory 1.4 MB → 1.2 MB. Four
  small, mostly-blank-screen fixtures grew by 8–24 bytes (section framing exceeds the `lcd` saving
  when `lcd` was uniform enough to compress to almost nothing); the rest shrank 8–34%.
- Default tier, final code: `test result: ok. 874 passed; 0 failed; 121 ignored` (866 + 8 new
  savestate tests).
- **`cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests` →
  `test result: ok. 111 passed; 0 failed; 28 ignored; 0 measured; 856 filtered out; finished in
  453.25s`.** This is the real proof the converted fixtures still drive the agent. Run twice (505s
  before a late refactor, 453s after); green both times.
- Tolerance proven both ways by unit test, not assumed: `unknown_sections_are_skipped`,
  `missing_sections_are_absent_not_errors`, `game_boy_tolerates_extra_and_missing_sections` (splices
  a bogus section into a real machine's state, and strips `hram` from another).
- Fixture bytes were **unchanged** by every test run after the conversion (`md5sum` of all 91
  → `51c00f7f8cd299815977e84026d77c05` before and after both slow-tier runs), confirming nothing
  silently rewrote them.

**Surprises:**

1. **The legacy-struct problem evaporated — no `legacy_v0.rs` was needed.** A0 assumed the converter
   would need a verbatim `PpuV0`. It doesn't, because *the new writer is hand-written per section*:
   at HEAD the old derived codec (which still reads `lcd`) and the new section writer (which chooses
   not to write it) coexist with **no struct change at all**. So the converter was just
   `decode_old(bytes) -> GameBoy -> save_state()`. It lived at `src/savestate/convert_v0.rs` as an
   `#[ignore]`d test and **was deleted after use**, together with the derives it was the last user
   of. Had A0's containment chain been needed it would have been *four* legacy types
   (`GameBoyV0`/`CoreV0`/`MmuV0`/`PpuV0`), not one, because `MMU`'s hand-written decode calls
   `PPU::decode` — worth knowing if this ever has to be redone.
2. **"Append a field within a section" does not work for free with plain bincode structs** — a
   struct's derived `Decode` reads fields positionally and errors with `UnexpectedEnd` on a short
   buffer. §2.4 rule 4 promised this would be free; it only became free once I made a section
   payload a *sequence of values* with a cursor (`write_fields` / `section()` / `field()`). If you
   add a field by editing a section struct, you **will** invalidate all 91 fixtures. Use the cursor.
3. **A2's premise is wrong — there is no livelock.** The `MachineCycles::ZERO` arms in
   `src/core.rs` produce `interrupt_cycles`, an *addend*; `Core::execute` returns
   `opcode_cycles + interrupt_cycles`, and no `OpCode::machine_cycles` arm returns 0 (checked all of
   `src/opcode.rs:577-700`), so `run()` always terminates. The real defect is that **the CPU keeps
   fetching and executing in both `Stop` and `Crash` while `mmu.update` is skipped**. I rewrote A2's
   Why/Do/Verify in place and flagged the Status Board row; the task ID and its position are
   unchanged. Implementing the *original* A2 would have been a no-op.
4. I removed `MMU`'s hand-written codec with a blunt "cut from `impl Encode for MMU` to EOF", which
   also took the `#[cfg(test)] mod tests` that sat after it — 6 tests, restored from `git show`.
   Caught only because the test count dropped 873 → 867. **Watch the test count after every
   deletion**; it is the cheapest tripwire in this repo.
5. `dmg-acid2` never writes high RAM, so the first draft of the "missing section" test was vacuous
   (comparing zeros to zeros). It now pokes HRAM explicitly and asserts the comparison is
   non-trivial before relying on it.

**Tree:** dirty, uncommitted, as instructed. Changed: `src/savestate/mod.rs` (**new**),
`src/main.rs` (`mod savestate;`), `src/game_boy.rs`, `src/core.rs`, `src/mmu.rs`, `src/ppu.rs`,
`src/audio/mod.rs`, `docs/compatibility/10-implementation-plan.md`, and **all 91
`src/pokemon/data/*.bin`** (the sanctioned one-time conversion). **No `src/pokemon/**` source file
was touched** — verified with `git diff --name-only src/pokemon/ | grep -v data/` → empty. A backup
of the pre-conversion fixtures is in this session's scratchpad only; `git checkout
src/pokemon/data/` is the real undo.

**Next agent:** A0 is the gate and it is open — but **the `CLAUDE.md` note "Nothing may be added to
`Audio`'s serialised fields" is now false and dangerous in reverse**: it forbids something that is
now safe, and its neighbouring save-format paragraph is stale too. I did not edit `CLAUDE.md`
because §2.2 reserves it for A10; I have added this as a fourth correction under A10, so **do A10
early** rather than in task order. Beyond that: A1 (`Core::reset`) is the cleanest next task, and
read A2's corrected premise before touching A2/A3/A4 — those three are all facets of the same
`Stop`/`Crash` wiring and are probably best done together.

---

### 2026-08-05 (#4) — A1–A7, A9–A14 — Phase A complete except A15 (new)

**State:** **A1, A2, A3, A4, A5, A6, A7, A9, A10, A11, A12, A13, A14 → `DONE`.** New **A15** added
(`TODO`). Phase A's exit criteria are met: no `todo!()` or `println!` in the core hot path, every
crash-class and data-loss bug fixed, sectioned savestates in place, core-only benchmark recorded.

**Did:**

- **A1** `Core::reset()` (was `todo!()`). New `MMU::reset()` (`src/mmu.rs:177`) restores power-on
  state but keeps `data`, `header` and `ram_banks` — battery-backed SRAM survives, per gambatte.
- **A2** No bug. See the rewritten A2 — its premise was wrong *twice*. Closed with a guard test.
- **A3** `mmu.restart()` on STOP wake (`src/core.rs`), and STOP now consumes its pad byte
  (`src/opcode.rs:653`) so PC no longer lands on it.
- **A4** Illegal opcode → `IE = 0` + `CoreMode::Halt` instead of `CoreMode::Crash` + `mmu.stop()`.
  The CPU locks; PPU, APU, serial and DIV keep running. `println!` deleted from the hot path.
- **A5** `pad_rom` (`src/mmu.rs:24`) rounds every ROM up to a power-of-two bank count filling with
  `0xFF`; `rom_bank_count()` derives from the data, so the clamp no longer trusts header `0x148`.
- **A6** `Sprite` carries the `height` it was **selected** under, so a mid-scanline LCDC flip can no
  longer index past a 16-byte tile. Also range-checked the *length* in `read_vram_slice` /
  `read_wram_slice`, which only checked the base address.
- **A7** `LcdDma` rewritten: incremental (1 byte per M-cycle over 160), `is_active()` true
  throughout, privileged `PPU::write_oam_dma` bypassing the mode gate, `|| dma.is_active()` removed
  from the four CPU-facing accessors and `&& !dma.is_active()` added to OAM, source page classified
  rather than `& 0xDF`-masked, and `FF46` reads back.
- **A9** `game_boy::tests::bench_core_throughput` — core only, three workloads, 60 warm-up frames.
- **A10** Five corrections to `CLAUDE.md` (see below).
- **A11** Linear pixel clock via a new `PPU::draw_pixels_to`, and leaving mode 3 now flushes the
  rest of the scanline — that branch previously drew nothing.
- **A12** All five APU fixes, one at a time with blargg re-run between each.
- **A13** Read-back masks for `FF00/FF02/FF07/FF0F/FF41`, IE's upper three bits, and `FEA0-FEFF`
  → `0x00`. Updated five tests that encoded the old values (four `ldh` tests plus the joypad one
  A13 names).
- **A14** 19 ROMs from `c-sp/game-boy-test-roms` v7.0 wired and `#[ignore]`d.

**⭐ Two section-version bumps, and neither regenerated a fixture** — the A0 mechanism working as
designed, on one shape change and one pure append:

| Section | v | Change | How |
|---|---|---|---|
| `dma` | 1 → 2 | **Shape change** (A7): `address: u16` → `page: u8`, plus `pos` and `register` | `LcdDmaV1` + `From` conversion, selected on `FieldReader::version()` (`src/ppu.rs:130`) |
| `irq` | 1 → 2 | **Append** (A13): IE's upper 3 bits | `write_fields` / `fields.field()`; absent in v1 reads as `None` |

**Verified:**

- Default tier: `test result: ok. 897 passed; 0 failed; 141 ignored` (866 at session start, +31
  new tests; ignored 121 → 141 = 19 blargg + the core benchmark).
- Slow tier, run twice — after A7 (the highest-risk change) and again on the final tree:
  `test result: ok. 111 passed; 0 failed; 28 ignored; finished in 499.89s` and
  `test result: ok. 111 passed; 0 failed; 28 ignored; finished in 388.23s`.
- blargg `dmg_sound` **9/9 after every single A12 change**, run five times.
- **A6 and A11 were verified by reintroducing the bug**, not just by passing. The first draft of the
  A6 test passed *with the bug present* — it never reached the draw path, because a single large
  `update` takes the `>= drawing_ticks` branch. Rewritten to step the PPU one M-cycle at a time, it
  fails with `index out of bounds: the len is 16 but the index is 24`, which is the A6 panic exactly.
- dmg-acid2 and all 8 `button_test` screenshots are **byte-identical** after A11, as that task
  predicted.

**⭐ A9 BASELINE — Phase C is scored against this table.** Ryzen 9 7900X, `--release`, 600 measured
frames after 60 warm-up, three runs each. `gb` spread <1%; gambatte <3%.

| Workload | `gb` realtime | `gb` t-cycles/s | gambatte realtime | gambatte t-cycles/s | Ratio |
|---|---|---|---|---|---|
| Pokémon Red (mid-game fixture) | **33.6x** | 141.0M | 602x † | 2.53G † | — † |
| `cpu_instrs.gb` (never HALTs) | **51.8x** | 217.3M | 333x | 1.40G | **6.4x** |
| `dmg-acid2.gb` (PPU-heavy) | **48.6x** | 204.0M | 605x | 2.54G | **12.4x** |

† **Not comparable.** `gb` runs from a mid-game save state; the gambatte harness has no save-state
loading and so boots from scratch, sitting in the title/intro where it HALTs heavily. Use the
`cpu_instrs` and `dmg-acid2` rows for the real gap. The plan's §2.5 table (457x / 428x / 622x) was
measured differently again — prefer these numbers, which come from one harness on one machine.

Agent-inclusive, for the record: 33.6x raw / **28.3x** through `agent.step()`, so the agent costs
~16%, not the ~11% `CLAUDE.md` claimed.

**⭐ A14 RESULT — 0 of 19 pass**, which is the expected and useful answer. But the split matters:

- **4 report over serial** (`mem_timing`) and fail with per-instruction detail, e.g. `01-read_timing`
  → `F0:2-3 FA:2-4 CB 46:2-3 CB 4E:2-3 ...` — those reads take 3 M-cycles where hardware takes 2.
  That is the instruction-granularity gap named instruction by instruction, and it is exactly what
  A14 was for.
- **15 emit nothing over serial** (`mem_timing-2` x4, `halt_bug`, `interrupt_time`, `oam_bug` x9).
  They write to the screen. So they currently fail for a *harness* reason. **A14's claim that all 19
  need "zero new harness code" is wrong for 15 of them** — hence new task **A15**.

**Surprises:**

1. **A2's premise was wrong twice, and I published the first wrong correction.** Reading #1 (the
   guides'): "returns 0 cycles, so `run()` livelocks" — no; that `ZERO` is one addend. Reading #2
   (mine, in ledger #3): "the CPU keeps executing instructions in Stop/Crash" — also no; `fetch`
   already returns a virtual `Nop` without touching PC when the mode is not `Normal`
   (`src/core.rs:178`). The only real defect was the peripheral half, which A3 and A4 fix. **Both
   readings came from reading the code rather than running it.** Corrected in A2 in full.
2. **A6's first test passed with the bug reintroduced** — it drove the PPU with one large `update`,
   which takes the `>= drawing_ticks` branch and draws nothing at all. That branch not drawing was
   itself A11's second defect, so one bug hid the test for another. **Reintroduce the bug to prove a
   regression test works**; two of mine were vacuous until I did.
3. **§2.5's gambatte build recipe does not link** — missing `-I gambatte/common` (for `array.h`,
   `scoped_ptr.h`) and `libgambatte/src/file/file.cpp` (for `newFileInstance`), and `runFor` needs
   `gambatte::uint_least32_t*`, not `uint32_t*`. Corrected in §2.5 with a working command.
4. `MMU::reset` has to mirror `MMU::from_rom` field for field. There is no compiler check on that;
   if someone adds a field to `MMU` and forgets `reset`, only
   `game_boy::tests::reset_matches_fresh_construction` will catch it. Keep that test.
5. Four `ldh` unit tests asserted `0x3F` from `FF00`, encoding the same bug A13 called out in
   `src/joypad.rs`. Grep for a *value*, not just the file the task names.

**A10 — what changed in `CLAUDE.md`:** blargg claim narrowed to "dmg_sound 1-8 and 11"; the
`bench_emulation_throughput` command given its full module path (it matched zero tests) plus the new
core benchmark; the save-state section rewritten around the sectioned container with the
add-a-field/add-a-section rules; **the "nothing may be added to `Audio`'s serialised fields" rule
deleted** (it forbade something now safe) and replaced with the derived-state-excluded-from-`PartialEq`
note; throughput figures re-measured (~34x core, ~28x with agent). "27 fixtures" is gone; it now says
91.

**Tree:** dirty, uncommitted. Modified: `src/core.rs`, `src/mmu.rs`, `src/ppu.rs`, `src/lcd_dma.rs`,
`src/opcode.rs`, `src/game_boy.rs`, `src/joypad.rs`, `src/audio/{square_channel,noise_channel,sweep}.rs`,
`src/roms/mod.rs`, `CLAUDE.md`, this document. Added: `src/roms/{mem_timing,mem_timing_2,oam_bug}/`,
`src/roms/halt_bug.gb`, `src/roms/interrupt_time.gb` (19 ROMs, ~1 MB, from `c-sp/game-boy-test-roms`
v7.0). **No `src/pokemon/**` source touched and no fixture regenerated** — `src/pokemon/data/` is
byte-identical to the A0 commit. gambatte tree verified clean (`git status --porcelain` → empty).

**Next agent:** Phase A is done bar **A15**, which is small and worth doing before Phase B so the
accuracy baseline is honest. Then **B1**. Before you touch serialised state, read the rules at the
top of `src/savestate/mod.rs` and the two worked examples in this entry — B2's VRAM/WRAM resize is a
**shape change** like `dma` was, not an append, so it needs the `FieldReader::version()` branch, not
`write_fields`. And the lesson that cost me the most time this session: **reproduce a bug before you
fix it, and reintroduce it to prove your test catches it.** Three of this plan's stated premises
turned out to be wrong when actually run.

---
