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

### Using gambatte as a reference-screenshot source

⭐ **Added 2026-08-05 (ledger #7).** The same harness, with the loop body replaced by a dump of the
video buffer, is the cheapest way to obtain a **reference image for a test ROM that ships none** —
which is what A16 needed and what **B10** (`cgb-acid2`) will need:

```cpp
FILE *out = std::fopen(argv[2], "wb");                 // binary PGM: greys, no dependencies
std::fprintf(out, "P5\n160 144\n255\n");
for (int i = 0; i < 160 * 144; ++i)
    std::fputc(static_cast<int>((video[i] >> 8) & 0xFF), out);   // 0x00RRGGBB -> one channel
```

Gambatte's DMG greys come out equal to gb's `FF/AA/55/00`, so the two frames compare byte for byte
after `convert('L')` — A16 confirmed that on three ROMs. **Check the frame reads as a pass before
promoting it**, and say in the constant's doc comment where it came from.

Patching a *copy* of the gambatte sources (`cp -r libgambatte common` into scratch, then edit) is
also the fastest way to answer "what does the reference actually do here?" — A16 found its real bug
by printing `cc`/`lastReadTime_`/`wavePos_` from `Channel3::waveRamRead` and diffing that trace
against gb's.

Reference numbers already measured on this machine (AMD Ryzen 9 7900X), for orientation:

| Core | Workload | Realtime |
|---|---|---|
| gambatte | Pokémon Red, in-game | **457×** |
| gambatte | `cpu_instrs.gb` (never HALTs) | **428×** |
| gambatte | `dmg-acid2.gb` | **622×** |
| gb | core + agent | **~24×** |

⚠️ **Benchmarking methodology — read this before quoting any number in this document**
(added 2026-08-05, ledger #9). This machine has fast and slow states that differ by **~15%**: the
same unmodified binary measured `cpu_instrs` at **43.5×** and **53.2×** twenty minutes apart. A
figure taken on its own therefore means nothing, and neither does one taken minutes after its
control.

- Compare only **adjacent paired runs** of the two builds you are comparing.
- **Alternate which build runs first** and report both orders. A block that always runs the
  baseline first will manufacture a regression out of ordinary drift — that nearly happened in
  ledger #9, and it took an accidental control to catch.
- Keep a `git`-clean copy of the comparison point built with the same toolchain (`rsync` the tree
  excluding `target`, `git checkout -- .`) rather than trusting a number from an earlier session.
- **Also diff hot-function sizes**: `nm -S --size-sort -C target/release/deps/gb-*`. Growth in
  `MMU::update` (called once per instruction) cost more in Phase B than any algorithmic change,
  and it is invisible to bisection.

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
| A14 | Wire 19 more blargg ROMs | DONE | 2026-08-05 | **1/19 passes** (`interrupt_time`). Harnesses corrected in ledger #5 |
| A15 | ~~Screen-output test harness~~ | SKIPPED | 2026-08-05 | **Alex decided.** The harness already existed (`ppu_test`). Row kept so references resolve |
| A16 | `dmg_sound` 09/10/12 — wave-channel read/trigger/write while on | DONE | 2026-08-05 | all 12 green; screens byte-identical to gambatte. Needed a bus-access *placement* hint, not M-cycle timing |
| A17 | ~~Combined suite ROMs never terminate~~ | SUPERSEDED | 2026-08-05 | root-caused to **D1**'s bank *mask* vs gb's *clamp*. `cpu_instrs` was never broken — it needed a bigger budget, and is now wired and green. See ledger #8 |

## Phase B — CGB

| ID | Task | State | Date | Notes |
|---|---|---|---|---|
| B1 | Machine model plumbing (`Model` enum, `GameBoy::cgb`) | DONE | 2026-08-05 | `Model` + `ColorMode` in `src/model.rs`; `dmg()` unchanged |
| B2 | WRAM banking (SVBK) + VRAM banking (VBK) | DONE | 2026-08-05 | appended to the `wram`/`ppu` sections → **v2**, no fixture churn |
| B3 | CGB palette RAM (BCPS/BCPD/OCPS/OCPD) | DONE | 2026-08-05 | `src/cgb_palette.rs`, raw + expanded mirror. Mode-3 blocking deferred |
| B4 | Framebuffer pixel type → RGB555 | DONE | 2026-08-05 | **24-bit, not RGB555** — `0xAA` has no 5-bit form. See ledger #9 |
| B5 | **DMG-compatibility palette (the Pokémon Red colour path)** | DONE | 2026-08-05 | ⭐ combination **13** for checksum `0x14`. Tables in `src/boot_palette/` |
| B6 | BG map attributes + CGB sprite priority | DONE | 2026-08-05 | attributes from VRAM bank 1; OAM-index priority gated on `OPRI` |
| B7 | KEY1 double-speed | DONE | 2026-08-05 | done **before** C1: halved video clock + carry bit, DIV undivided |
| B8 | HDMA / GDMA | DONE | 2026-08-05 | `src/hdma.rs`; HDMA at the mode-3→0 edge, GDMA has no CPU stall |
| B9 | CGB post-boot state + remaining CGB registers | DONE | 2026-08-06 | `FF72-75`, `FEA0` CGB pattern, 32× serial. **Two** boot register files (ledger #10). DMG boot state **untouched** |
| B10 | CGB test-ROM adoption (cgb-acid2) | DONE | 2026-08-05 | **passes byte-for-byte** against the ROM's own reference image |
| B11 | DMG post-boot register/IO table | TODO | | split out of B9 — **needs Alex's call**, it can move fixtures |

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
| D1 | ROM padding + bank masking (prereq for all MBCs) | TODO | | ⭐ has a **failing acceptance test already**: `blargg_dmg_sound::all`. Mask width is per-mapper — see A17 |
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

### A15 — ~~Screen-output harness~~ SKIPPED

**Skipped by Alex, 2026-08-05.** The premise was wrong: `ppu_test` (`src/game_boy.rs`) has always
been a screenshot-comparison harness, and `gb_test_failed_with_screenshot` already dumps the actual
frame on failure. Nothing needed building.

What A14 actually lacked was **reference images**, and `c-sp/game-boy-test-roms` v7.0 ships them for
the five *combined* suite ROMs — now committed and wired. The individual sub-ROMs still have none;
they use `screenshot_pending`, which dumps their output for later promotion (see that function).

Row retained so existing references resolve.

---

### A16 — `dmg_sound` 09/10/12: wave channel read/trigger/write while on

**State:** DONE (2026-08-05) · **Depends:** none · **Risk:** low · **Added:** 2026-08-05 (ledger #5)

> ✅ **Landed.** All twelve `dmg_sound` sub-tests pass, and gb's frame for each of the three is
> **byte-identical to gambatte's**. References promoted, tests un-ignored. See ledger #7.

**Why.** These three were the only ignored emulator tests fixable today — everything else in the
ignored list is blocked on the deferred M-cycle refactor, is an unscheduled hardware quirk, or is a
tool rather than a test.

- `09-wave read while on` — reading wave RAM while channel 3 plays.
- `10-wave trigger while on` — retriggering while playing corrupts the first bytes of wave RAM.
- `12-wave write while on` — same aperture as 09, for writes.

⚠️ **Correction to this task's premise (§1.3).** It claimed "none of the three needs sub-instruction
timing", and that the read "returns the byte the channel is currently fetching". Both are wrong as
written:

- On **DMG** the read returns **`0xFF`** unless it lands on the *exact tick* the channel fetches its
  next sample — a window one tick (2 T-cycles) wide. Returning the current byte unconditionally, as
  gb did, is the **CGB** behaviour (gambatte `channel3.h:47-56`).
- A one-tick window cannot be resolved by "somewhere in this instruction". These tests **do** need
  the bus access placed inside the instruction — but *only its placement*, not a scheduler.
  Peripherals are still advanced once per instruction, so §0.2 holds and no part of the M-cycle
  refactor was started.

**What it actually took** — four defects, of which only the first two were in the task:

1. **No aperture.** `wave_ram`/`set_wave_ram` ignored timing entirely.
2. **No trigger corruption**, and the first fetch after a trigger was `period` ticks out rather than
   `period + 3`.
3. **The bus-access placement hint.** `Core::execute` now tells the APU the instruction's length
   (`Audio::set_instruction_length`); hardware puts a load's or store's memory access in the final
   M-cycle, so the access sits `(len - 1)` M-cycles in. Verified against gambatte's `cpu.cpp`
   macros, where `LDH A,(n)` reads at `cc + 8` T of a 12 T instruction.
4. **⭐ The one that actually blocked test 09, and was in nobody's list: gb latched the wave period
   at trigger and never looked again.** Hardware reloads the frequency timer from NR33/NR34 at every
   overflow. Test 09 triggers at a long period and then writes `NR33 = 0xFE` (period 2) *before*
   reading — so gb kept fetching at the old rate and every read after the first missed the window.

**Verify.** All 12 `dmg_sound` sub-tests green, and the other 9 must not regress. Done, plus a
gambatte cross-check: gb's frame for 09/10/12 is pixel-identical to gambatte's.

⚠️ **Wiring the combined ROM as an end check does not work** — it never terminates. That is
**A17**, not a sound bug.

---

### A17 — ~~The combined blargg suite ROMs never terminate~~ SUPERSEDED BY D1

**State:** SUPERSEDED (2026-08-05) · **Added:** 2026-08-05 (ledger #7) · **Root-caused:** ledger #8

⚠️ **This task was raised on a wrong reading, and the correction is the useful part.** Both of its
claims were investigated properly and neither survived:

**`cpu_instrs.gb` was never broken.** It does not stall at test 11 — it needs about **2.5x** the
default cycle budget, because the eleven sub-tests run back to back and `11-op a,(hl)` is most of
the total. At 40M M-cycles it stops after `10:ok  11`; at 60M it prints `11:ok` and `Passed all
tests`. It is now wired as `game_boy::tests::blargg_cpu::all` and **passes**. A budget shortfall and
a hang look identical from outside — check by *raising the budget* before concluding "stall".

**`dmg_sound.gb` is a real bug, and it is exactly the bank `clamp` vs `mask` that A5 deferred to
D1.** Traced on both emulators. Blargg's runner walks its sub-tests by writing the test index to
the bank register, and the ROM is 64 KB / 4 banks:

| Write to `0x2000` | gambatte | gb |
|---|---|---|
| 1, 2, 3 | banks 1, 2, 3 | banks 1, 2, 3 ✅ |
| **4** | **bank 0** — `adjustedRombank(4) & (4-1)` | **bank 3** — `min(rom_bank_count()-1)` ❌ |

Bank 0 in the switchable slot is where the runner's terminator lives. gb clamps to 3, re-runs bank
3's test, and loops forever printing `NN:ok` past 12, past 99, into ASCII. Applying gambatte's mask
to gb reproduces its bank trace exactly — `1, 2, 3, 0`, one write, done — and
`blargg_dmg_sound::all` **passes against the committed reference**.

**Why the earlier "not the bank clamp — tested" note was wrong.** That experiment was run against
`cpu_instrs`, which never had the bug, so it could not have shown a difference. Right experiment,
wrong ROM.

**⚠️ Do not "just apply the mask" — it is mapper-specific, which is why it is D1 and not a
one-liner.** MBC1 masks the bank register to **5 bits**; `pokered.gbc` is **MBC3** (`0x147 = 0x13`)
with **64 banks**, needing 6. A universal `& 0x1F` fails 8 tests in the default tier immediately.
Verified, then reverted. The mask width has to come from the mapper, which is precisely what
**D1**/**D2** are for.

**Where the work now lives:** D1. Its acceptance test already exists —
`game_boy::tests::blargg_dmg_sound::all`, `#[ignore]`d with a correct committed reference, green the
moment D1 lands.

---

# 5. PHASE B — CGB

**Goal:** full Game Boy Color support, with the **DMG-compatibility boot-ROM palette** as the
headline deliverable so Pokémon Red renders in colour.

**Exit criteria — all met, 2026-08-05 (ledger #9):**
- ✅ `GameBoy::cgb(cart)` exists; `GameBoy::dmg(cart)` is unchanged in behaviour.
- ✅ Pokémon Red runs in CGB compatibility mode with the **correct boot-ROM palette**, asserted
  pixel by pixel against the DMG frame at three points through the intro.
- ✅ `cgb-acid2` passes — **byte-for-byte** against the reference image the ROM ships.
- ✅ The full DMG test suite still passes: default tier, `slow-tests`, and `full_playthrough`.

## ⚠️ Sequencing note for whoever starts Phase B

Alex chose CGB before performance. Understood and respected — but be aware CGB code written now will
be touched again in Phase C when the event scheduler lands. **Mitigation: write every new peripheral
method in `catch_up(now)` shape** — take an absolute cycle stamp and derive elapsed time internally
— even though the driver still calls it once per instruction. Phase C then becomes a driver change
rather than a peripheral rewrite. This costs nothing now and saves real rework later.

**⚠️ A0 must be `DONE` before starting B1.** CGB changes the *shape* of serialised state (VRAM/WRAM sizes, framebuffer type, new palette RAM), not merely appending to it.

> ⚠️ **Corrected 2026-08-05 (ledger #9).** The premise of that last sentence turned out to be
> wrong, in a way worth knowing before Phase C repeats the reasoning. A0 removed the framebuffer
> from serialisation entirely, so B4 was not a save-format concern at all; and the VRAM/WRAM size
> changes were expressible as **appends** — keep the shipped array as field 1 (bank 0, or banks 0
> and 1) and append the new banks as field 2. Phase B therefore needed **no legacy struct and no
> fixture regeneration**: two section-version bumps and one new section. A0 was still the enabling
> work — none of that is possible without the sectioned container — but "a size change forces a
> shape change" is not true when you can re-cut where the boundary falls.

---

### B1 — Machine model plumbing

**State:** DONE (2026-08-05) · **Depends:** A0

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

**State:** DONE (2026-08-05) · **Depends:** B1

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

**State:** DONE (2026-08-05) · **Depends:** B2

**Do.** `BCPS/BCPD` (`FF68/69`) and `OCPS/OCPD` (`FF6A/6B`): 64 bytes each, auto-increment on write
when bit 7 of the index is set. Read-back masks: BCPS `data | 0x40`, OCPS `data | 0x40`.

Store raw **and** keep a pre-expanded RGB mirror (gambatte: `video.h:205-206`) so the pixel path
doesn't unpack per pixel.

**Mode-3 palette access blocking is DEFERRED** — it needs precise mode-3 timing. Note it in the
Ledger.

**Verify.** Write/read-back round-trip including auto-increment.

---

### B4 — Framebuffer pixel type → RGB555

**State:** DONE (2026-08-05) · **Depends:** B3 · **API change — see §2.3**

> ⚠️ **Shipped as 24-bit colour, not RGB555, and the difference is not cosmetic.** `gb`'s DMG
> shades are `FF/AA/55/00`, and **`0xAA` is not expressible as a 5-bit channel widened back to 8**
> — `(21 << 3) | (21 >> 2)` is `0xAD`. Storing RGB555 would therefore have shifted every committed
> reference screenshot by three units. The task text allows this ("`Rgb555` (or `u32`)"); taking
> the option is what made "verify, don't assume" come out clean. `LcdColor` is `0x00RRGGBB`, DMG
> shades are written into it exactly, and CGB colours go through `from_rgb555`. Blast radius
> outside the core was the one line §2.3 predicted, `src/sdl/render.rs:273`.

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

**State:** DONE (2026-08-05) · **Depends:** B4

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

> ✅ **Resolved 2026-08-05.** Checksum `0x14` is at index **22** of `TitleChecksums`, which is
> *before* `FirstChecksumWithDuplicate` (65) — so the 4th letter `'E'` is **not** consulted, and
> this document's step 3 does not apply to Pokémon Red. `PalettePerChecksum[22]` is combination
> **13** = `palette_comb 3, 4, 4`: OBJ0 from pool palette 3 (white / yellow-green / dark / black),
> OBJ1 and BG from palette 4 (`7FFF 421F 1CF2 0000` — white / salmon / dark red / black). Source:
> SameBoy `BootROMs/cgb_boot.asm` on `master`, fetched 2026-08-05; the tables are generated from
> it mechanically rather than transcribed, see `src/boot_palette/tables.rs`.
>
> One more precondition the task text omits: the boot ROM only colours **first-party** cartridges.
> `GetPaletteIndex` requires old licensee `0x01`, or `0x33` with new licensee `"01"`. Pokémon Red
> is the second case. A third-party cartridge falls through to combination 0 whatever its title.
>
> **Button-combination overrides: not implemented.** `gb` starts the cartridge directly, so there
> is no boot window in which a combination could be held. It is a user convenience, not accuracy.
>
> Screenshots: `target/pokered-cgb-*.png` from `game_boy::tests::ppu` (regenerate by running that
> module); the assertion itself is `pokemon_red_boots_in_colour_on_a_cgb`, which compares the CGB
> frame against the DMG frame shade by shade rather than eyeballing a colour.

---

### B6 — BG map attributes and CGB sprite priority

**State:** DONE (2026-08-05) · **Depends:** B5

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

**State:** DONE (2026-08-05) · **Depends:** B6 · **Risk:** high

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

> ✅ **Done before C1, deliberately.** The deferral was considered and turned down: without the
> scheduler the change is *smaller*, not larger, because there is exactly one place where CPU time
> becomes peripheral time — `MMU::update`. Double speed halves the M-cycles handed to the PPU and
> APU there, with a one-bit carry so an odd cycle is not rounded away, and leaves DIV, the timer
> and serial on the CPU clock. That is the hardware relationship: DIV is CPU-clocked, so the APU
> frame sequencer that hangs off it stays at 512 Hz in real time without any extra work.
> `MachineCycles` did **not** need to change. The ~2050-M-cycle CPU stall during the switch is not
> modelled. When C1 lands, `video_cycles` is the one line that moves.

**Verify.** DMG unaffected. A CGB ROM that switches speed runs at the right rate (a timer-based
assertion, not a visual one).

---

### B8 — HDMA / GDMA

**State:** DONE (2026-08-05) · **Depends:** B7

**Do.** `FF51-FF55`. GDMA transfers the full length at once; HDMA transfers `0x10` bytes per HBlank.
Source reads return `0xFF` for VRAM and `>= 0xFE00`. Destination wrap sets the done bit. `FF55`
read-back: bit 7 = done.

**Accuracy caveat.** Precise HBlank-timed HDMA needs mode-0 timing that `gb` does not model
(mode 3 is a fixed 172 T). Implement HDMA **triggered at the mode-3→mode-0 transition** — correct in
ordering, approximate in cycle placement. **Document this limitation in the Ledger.** HDMA/OAM-DMA
interleaving is out of scope.

> ✅ **Implemented exactly as described**, at the mode-3→mode-0 edge, via a latch the PPU sets and
> `MMU::update` consumes in the same call. Two limitations beyond the one above, both recorded in
> `src/hdma.rs`: **GDMA does not stall the CPU** (the copy is instantaneous, so a guest that times
> its own code against a GDMA sees it finish for free), and OAM-DMA interleaving is not modelled.
> Both need the M-cycle work §0.2 defers.

---

### B9 — CGB post-boot state and remaining registers

**State:** DONE (2026-08-05) · **Depends:** B8

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

> ⚠️ **The DMG post-boot table was deliberately NOT touched** (2026-08-05). It is the one change
> in Phase B that could move the 91 committed fixtures and re-roll Pokémon Red's RNG stream, and
> the task itself says to make it a separate task with Alex's call. It is now **B11**, `TODO`.
> `RegisterSet::boot(Model::Dmg)` returns `gb`'s original values unchanged and says so in its doc
> comment; only the CGB arm is new.
>
> ⚠️ **Corrected 2026-08-06 (ledger #10): CGB hardware has *two* boot register files.** The first
> version of B9 shipped one — the CGB-mode file — for both CGB and compatibility mode. A CGB
> running a DMG-only cartridge takes the boot ROM's `EmulateDMG` path, which ends `ld de, 8` /
> `ld l, $7C` and then loads `hTitleChecksum` into `B`, so four registers differ:
>
> | | CGB (CGB mode) | CGB (DMG-compat mode) |
> |---|---|---|
> | `A` / `F` | `0x11` / `0x80` | `0x11` / `0x80` |
> | `B` | `0x00` | **title checksum** (`0x14` for Pokémon Red), or `0x00` if not first-party |
> | `C` | `0x00` | `0x00` |
> | `DE` | `0xFF56` | **`0x0008`** |
> | `HL` | `0x000D` | **`0x007C`** — or `0x991A` when `B` is `0x43`/`0x58` |
>
> `RegisterSet::boot` now takes the `ColorMode` **and the cartridge**, and all three files are
> pinned by tests. ⚠️ **gambatte is not the reference for this** — `initstate.cpp:1174-1181` uses
> the DMG values for CGB with only `A` and `B` changed, which contradicts both Pan Docs and the
> boot ROM. Two independent sources were used instead and agree exactly: SameBoy's `cgb_boot.asm`
> traced by hand through `Preboot`/`EmulateDMG`, and Pan Docs' "Power-Up Sequence" table (itself
> confirmed against mooneye's `misc/boot_regs-cgb`).
>
> ✅ Done here: `A = 0x11` and the rest of the CGB register file (the byte every CGB game
> branches on); `FF72`-`FF74` as plain RW and `FF75` masked to `0x8F | data`, CGB only;
> `0xFEA0..=0xFEFF` as three 8-byte blocks of ordinary RAM mirrored 4x, taken byte for byte from
> `test/hwtests/fexx_ffxx_dumper_cgb.bin`; and `SC` bit 1 as the 32x serial clock, with `SC`'s
> read mask widened from `0x7E` to `0x7C` on CGB because bit 1 is real there. `FF76`/`FF77`
> (PCM12/34) are **not** implemented — gambatte does not either, and the task marks them
> optional.

---

### B11 — DMG post-boot register and I/O table

**State:** TODO · **Depends:** none · **Risk:** medium · **Added 2026-08-05, split out of B9**

**Why it is its own task.** B9 says to "consider the DMG post-boot table too" and then warns that
doing so may shift the committed fixtures and needs Alex's call. It does not belong inside a CGB
task, because it is the only part of Phase B that can change what a **DMG** does — and therefore
the only part that can re-roll Pokémon Red's RNG stream and break the leg-fixture chain.

**Do.** Apply hardware's DMG post-boot state (see
[`05-mmu-cartridge.md` §7](05-mmu-cartridge.md#7-boot-state)):

| | `gb` today | hardware |
|---|---|---|
| `F` | `0x80` (Z only) | **`0xB0` or `0x80` — conditional, see below** |
| `LCDC` | `0x80` | `0x91` |
| `BGP` / `OBP0` / `OBP1` | `0x00` / `0x00` / `0x00` | `0xFC` / `0xFF` / `0xFF` |
| SRAM | zero-filled | `0xFF`-filled |

⚠️ **`F` is a function of the cartridge header, and everything written about it here before
2026-08-06 was wrong — including Pan Docs' own footnote.** See ledger #12 for the derivation. The
rule is:

```rust
// The DMG boot ROM's last flag-affecting instruction is `add a, [hl]` against the stored
// header checksum, and it locks up unless the 8-bit result is zero.
let checksum = rom[0x14D];
FlagsRegister { z: true, n: false, h: checksum & 0x0F != 0, c: checksum != 0 }
```

| `rom[0x14D]` | `F` | how many of 256 |
|---|---|---|
| `0x00` | `0x80` | 1 |
| non-zero multiple of `0x10` | **`0x90`** | 15 |
| anything else | `0xB0` | 240 |

⚠️ **Pan Docs says `H` and `C` are either both clear or both set.** That is wrong for the middle
row: `C` is set iff the checksum is non-zero, but `H` is set iff its *low nibble* is non-zero, and
those are different questions. ⚠️ **And `pokered`'s header checksum is `0x20`** — so Pokémon Red,
the cartridge this whole project runs, is one of the fifteen: it boots with **`F = 0x90`**, not
`0xB0`. `cpu_instrs` (`0x3B`), `dmg-acid2` (`0x9F`), `cgb-acid2` (`0xEB`), `button_test` (`0x1D`)
and `tetris` (`0x0A`) all want `0xB0`.

⚠️ **gambatte hardcodes `0xB0`** (`initstate.cpp:1179`), so it models none of this; it is not the
reference here (ledger #10).

**The other models, while you are in there.** `Model` deliberately has room for `Mgb`/`Sgb` and the
full table is one row each — from Pan Docs, same source:

| | DMG0 | DMG | MGB | SGB | SGB2 |
|---|---|---|---|---|---|
| `A` | `$01` | `$01` | `$FF` | `$01` | `$FF` |
| `F` | `$00` | conditional | conditional | `$00` | `$00` |
| `B` | `$FF` | `$00` | `$00` | `$00` | `$00` |
| `C` | `$13` | `$13` | `$13` | `$14` | `$14` |
| `D` | `$00` | `$00` | `$00` | `$00` | `$00` |
| `E` | `$C1` | `$D8` | `$D8` | `$00` | `$00` |
| `HL` | `$8403` | `$014D` | `$014D` | `$C060` | `$C060` |

Adding a model is only worth it if something will run against it — none of these have test ROMs in
the repo today. The CGB and CGB-compat files are already implemented and pinned; see
`registers::tests::the_boot_register_file_matches_the_boot_rom`.

**⚠️ Get Alex's call before running with `--features regen-fixtures`.** Run the default tier and
the `slow-tests` tier first and report exactly what moves. `full_playthrough` is the real check —
anything that changes frame timing re-rolls the RNG stream (see §2.4 and `CLAUDE.md`).

`F = 0xB0` is the cheap half and probably moves nothing; the `LCDC`/`BGP` half is what changes the
first frames a game sees.

---

### B10 — CGB test-ROM adoption

**State:** DONE (2026-08-05) · **Depends:** B9

**Do.** Wire up `cgb-acid2` (PNG-compared, same harness shape as dmg-acid2). Optionally
`cgb_sound`. Record the pass/fail split honestly — full CGB APU differences are not in scope.

> ✅ **`cgb-acid2` v1.1 passes, byte for byte.** No pass/fail split to report: 0 differing pixels
> against the reference image, with all 8 of its distinct colours present. Unlike the audio suites
> this ROM **ships its own reference**, so nothing had to be promoted from `gb`'s own output — the
> gambatte screenshot-dump recipe in §2.5 was not needed after all. Committed at
> `src/roms/cgb-acid2/`; the test is `game_boy::tests::ppu::cgb_ppu`.
>
> Its README pins the 5-bit to 8-bit expansion as `(c << 3) | (c >> 2)` — the plain widening, not
> a colour-correction curve. Confirmed independently against the PNG before trusting it: its
> palette resolves to `0x6B/0xBD/0xFF/0x9C/0x73/0xAD`, which is exactly that formula on 13, 23,
> 31, 19, 14 and 21. **Do not adopt gambatte's `gbcToRgb32` correction** — it would break this.
>
> `cgb_sound` is **not** wired up. It is a CGB-APU suite and the plan puts CGB APU differences out
> of scope; adding it would only add ignored tests, which §A14's hygiene rules argue against.

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

### 2026-08-05 (#5) — A14, A15, A16 — Ignored-test audit; A14's harnesses corrected

**State:** **A15 → `SKIPPED`** (Alex; its premise was wrong). **A16 → `TODO`** (new). A14 stays
`DONE` but its harness wiring and every `#[ignore]` reason were rewritten.

**Did:** Alex asked for the ignored list to be audited and categorised. Of the 144 ignored tests,
**91 are merely tier-gated** by `slow-tests` and all pass; the real backlog was **53**.

- **A15 was wrong and is skipped.** `ppu_test` has always been a screenshot-comparison harness and
  `gb_test_failed_with_screenshot` already dumps the actual frame. Nothing needed building. What
  was missing was *reference images*, and `c-sp/game-boy-test-roms` v7.0 ships them for the five
  combined suite ROMs. Committed and wired.
- **A14 rewired onto three harnesses**, chosen by how each ROM reports:
  `serial_console_test` for the four `mem_timing` ROMs (the only ones that use the link port);
  `ppu_test` against the c-sp reference for the five combined ROMs; and a new
  `screenshot_pending` for the ten sub-ROMs that have no reference — it dumps their output and
  fails with instructions for promoting it later.
- **⭐ `interrupt_time` PASSES and is now un-ignored.** It never needed a fix, only the right
  harness. Note before being confused by it: the DMG reference image shows **`Failed`** and
  checksum `7F8F4AAF` — the ROM targets CGB double-speed and legitimately fails on DMG hardware, so
  reproducing that exactly, checksum included, is a pass.
- **Every `#[ignore]` reason now names its blocker**: a plan §, a task ID, or an explicit won't-fix.
- **`render_reference_wav` deleted** — a WAV ear check, not an assertion. Took `src/audio/wav.rs`,
  `render()`, `artifact_dir()` and `CAPTURE_SECONDS` with it. `capture_golden_input` **stays**: it
  is a documented fixture generator, not an ear check.
- **Benchmarks moved behind a new `bench` Cargo feature** rather than `#[ignore]`, so they are
  compiled out entirely and no longer pad the ignored count.
- **A16 added** for `dmg_sound` 09/10/12 — the only ignored *emulator* test that is fixable today.

**Not touched, deliberately:** the 22 `probe_*` / diagnostic tests under `src/pokemon/**`. Alex is
having the Pokémon agent work through them first and will ask for a cleanup pass afterwards.

**Verified:**
- Default tier: `test result: ok. 898 passed; 0 failed; 141 ignored`. Passing count up one —
  that is `interrupt_time`.
- Ignored list diffed before/after by name, not by count: 144 → 142 removed exactly
  `render_reference_wav` and `bench_core_throughput`, added nothing; then 142 → 141 for
  `interrupt_time`.
- The 18 still-ignored blargg ROMs were run: each now fails with a *meaningful* reason —
  `screenshot does not match` for the referenced ones, per-opcode serial detail for `mem_timing`
  (`01-read_timing` → `F0:2-3 FA:2-4 …`), and the promote-me message for the unreferenced ones.
  None fails for a harness reason any more.
- Benchmark re-run through the new feature gate: 33.3x / 50.3x / 46.3x, consistent with ledger #4.

**Surprises:**
1. **A gap I invented turned out to be a test we were passing.** `interrupt_time` had been failing
   purely because it was wired to the serial harness, which it does not use. Wiring the right
   harness turned a "known gap" into a green test. Check the harness before believing a failure.
2. **A reference image that says `Failed` can be the correct expectation.** c-sp's screenshots are
   what hardware prints, not what a passing run prints. `interrupt_time-dmg.png` and the DMG
   `oam_bug` reference differ on exactly this point — the latter says `Passed`, so our failure there
   is real.
3. I repeated A10's own bug in my new doc comment: `--exact` with a bare test name matches nothing.
   Caught only by running the command I had just written. **Run every command you document.**
4. Deleting the ear check orphaned four helpers and a whole module; a first attempt at removing
   `render()` by line range also swallowed `encode_runs`/`rle_decode`. Same class of mistake as
   ledger #4's surprise 4 — **cut by symbol, not by line range**, and check the test count after.

**Tree:** dirty, uncommitted. Modified `src/game_boy.rs`, `src/roms/mod.rs`, `src/audio/reference.rs`,
`src/audio/mod.rs`, `Cargo.toml`, `CLAUDE.md`, this document. Added five reference PNGs under
`src/roms/`. Deleted `src/audio/wav.rs`. `src/pokemon/**` untouched; no fixture regenerated.

**Next agent:** **A16** is the one genuinely fixable emulator test left — everything else ignored is
blocked on the deferred M-cycle refactor, an unscheduled hardware quirk, or belongs to the Pokémon
agent. Then **B1**. When you add a test, put tiering behind a Cargo feature and keep `#[ignore]` for
things that are genuinely blocked, with the blocker named — the ignored list is meant to read as a
backlog.

---

### 2026-08-05 (#6) — test hygiene — Diagnostics gated; the ignored list is now a pure backlog

**State:** No task states changed. Housekeeping, at Alex's request, after the Pokémon agent
finished its own cleanup (`996dbf3`, `6876feb` — it fixed the four blocked leg tests and added a
`very-slow-tests` tier).

**Did:** Alex asked me to re-confirm why the remaining non-feature-gated `#[ignore]`s under
`src/pokemon/**` exist and delete the useless ones. **I deleted none, because none was useless** —
that is the finding, not an omission. All 24 are documented diagnostics, several explicitly written
to be re-run ("kept runnable so the next agent can re-derive it", "run this before adding a ground,
not after the leg times out"). I ran every one: **22 pass**; the two that fail
(`aides::probe_sweep_grounds`, `items::probe_town_hidden_item_reachability`) do so by exhausting
their cycle budget *after* printing their report, which is a rough edge, not deadness.

So the fix was categorisation, applying the rule Alex set for benchmarks:

- New **`diagnostics`** Cargo feature. All 23 `probe_*`, `dump_fixture_states` and
  `capture_golden_input` now sit behind it. They keep `#[ignore]` on top of the gate, deliberately:
  their pass/fail is not a signal, so they should not run as assertions.
- `bench_emulation_throughput` moved behind the existing **`bench`** feature, matching
  `bench_core_throughput`. Its doc quoted 23x/20x and "~11% agent overhead"; re-measured to
  **33.6x / 28.3x, ~16%**, and it now cross-references the core-only benchmark.

**Verified:**
- Default tier `test result: ok. 899 passed; 0 failed; 117 ignored` (was 141 ignored — the 24 tools
  are compiled out).
- **With every tier feature on and the tool features off, the ignored list is exactly 21 tests**:
  9 `oam_bug`, 9 `mem_timing`/`halt_bug`, 3 `dmg_sound` wave. Nothing from `src/pokemon/**` remains
  in it. That list is now a genuine backlog, and every entry names its blocker.
- Both gates exercised end to end: `dump_fixture_states` under `--features diagnostics` still
  prints (`at-celadon: CeladonCity @ (42, 10) | badges Badge(7) | ¥24781`), and
  `bench_emulation_throughput` under `--features bench` still reports (33.1x raw / 28.0x agent).

**Surprises:**
1. **"Still ignored" did not mean "still useless".** The probes had been left ignored because
   that is the only way Rust marks a non-assertion test, not because they were spent. The listing
   flags them identically to genuinely blocked tests, which is what made them look like debt.
   A feature gate distinguishes the two; `#[ignore]` alone cannot.
2. Two probes fail by design-adjacent accident — they finish their work then hit the fixture
   harness's cycle cap. Worth someone from the Pokémon side tidying, but they still deliver output.
3. I hit the `--exact`-needs-the-full-module-path trap **again** while diagnosing those two, having
   fixed exactly that bug in `CLAUDE.md` as task A10. It is a genuinely easy mistake; the fix is to
   copy the path from `--list` rather than typing a suffix.

**Tree:** dirty, uncommitted: `Cargo.toml`, `CLAUDE.md`, this document,
`src/audio/reference.rs`, `src/pokemon/integration_tests/**` (attribute-only changes plus one doc
comment). No emulator behaviour changed; no fixture regenerated.

**Next agent:** **A16** is still the only fixable ignored emulator test. When you add a test, decide
which of the three buckets it is in — blocked (`#[ignore]` with the blocker named), tiered
(`slow-tests` and friends), or a tool (`diagnostics`/`bench`) — and gate it accordingly.

---

### 2026-08-05 (#7) — A16, A17 — All 12 `dmg_sound` tests pass; Phase A closed

**State:** **A16 → `DONE`.** New **A17** added (`TODO`). Phase A now has no `TODO` left except A17,
which A16 discovered and which is not on Phase A's exit criteria — **Phase B (B1) is next.**

**Did:** four defects in the wave channel, of which the task named two.

1. **The DMG wave-RAM aperture** (`src/audio/wave_channel.rs`). While channel 3 plays, a CPU read
   returns `0xFF` unless it lands on the **exact tick** the channel fetches its next sample; a write
   outside that tick is dropped, and inside it lands on the byte being fetched rather than the
   addressed one. gb previously returned the current byte unconditionally and always accepted the
   write — which is the **CGB** behaviour, not DMG.
2. **Trigger** (`trigger`): the DMG wave-RAM corruption quirk, plus the first fetch after a trigger
   is `period + 3` ticks out, not `period` (`PhaseTimer::trigger_after`).
3. **A bus-access placement hint.** `Core::execute` now hands the APU the instruction's length
   (`MMU::set_instruction_length` → `Audio::set_instruction_length`); the access is placed in the
   final M-cycle, so it sits `(len - 1)` M-cycles in. **Peripherals are still advanced once per
   instruction — no part of the M-cycle refactor was started** (§0.2 intact).
4. **⭐ The one that actually blocked test 09, and was in nobody's list.** gb latched the wave period
   into the frequency timer at trigger and never looked at NR33/NR34 again. Hardware reloads from
   them at every overflow. Test 09 triggers at period 103..0 and then writes `NR33 = 0xFE` (period 2)
   **before** reading — so gb kept fetching at the old rate and every read after the first missed.
   Fixed by `WaveChannel::reload_period`, which updates the period without disturbing the interval
   in flight.

Then: promoted the three reference images (`src/roms/dmg_sound/*.png`), un-ignored the three tests,
added 7 unit tests in `src/audio/wave_channel.rs`, and wired the combined ROM as `blargg_dmg_sound::all`.

**Verified:**

- `cargo test --release --bin gb` → `909 passed; 0 failed; 115 ignored` (was 899/117: +7 unit tests,
  +3 un-ignored wave ROMs, −3 from ignored, +1 new ignored `all`).
- **All 12 `dmg_sound` sub-tests green**, and — the check that actually settles it — **gb's frame for
  09, 10 and 12 is byte-identical to gambatte's**, compared pixel by pixel, not eyeballed. That is
  what made the dumps safe to promote as references.
- Slow tier: `115 passed; 0 failed; 2 ignored; finished in 120.20s`.
- `full_playthrough`: `1 passed; 0 failed; finished in 602.21s`. Run because this changes emulator
  behaviour the game exercises — pokered writes wave RAM, and those writes are now droppable. It
  does **not** change any cycle count, so the RNG stream is untouched, which is why the leg
  fixtures still line up.
- Ignored list re-listed by name with every tier feature on: **19**, exactly the expected
  9 `oam_bug` + 9 `mem_timing`/`halt_bug` + the new A17 entry. The 3 wave tests are gone.
- **Each of the four fixes was reintroduced as a bug and the tests re-run** (ledger #4's lesson).
  All four are caught: dropping `reload_period` fails 4 tests, dropping the aperture 3, dropping the
  trigger corruption 2, dropping the `+3` delay 9. My first attempt at the third check had a Python
  quoting error and silently patched nothing — it "passed", which is exactly the vacuous result the
  exercise exists to catch. **Read the patch output, not just the test result.**

**Benchmark.** The per-instruction store costs ~2-3% on the CPU-bound workloads. Measured against
the same tree with only that one line removed, three runs each, so this is the change and not
machine drift:

| Workload | with the hint | without it |
|---|---|---|
| Pokémon Red (fixture) | 31.9 / 32.0 / 31.9x | 31.1 / 31.4 / 31.7x |
| `cpu_instrs.gb` | 51.0 / 51.0 / 50.4x | 52.5 / 52.2 / 51.9x |
| `dmg-acid2.gb` | 46.9 / 46.1 / 47.0x | 47.9 / 48.4 / 48.2x |

Both columns sit ~4% below ledger #4's baseline table, on the same machine, so **compare within this
table, not across sessions**. A u8-store variant (store the length, derive the offset at the use
site) measured identically to a u16-store one, so the cost is hoisting `machine_cycles` rather than
the store; it is C7's territory.

**Surprises:**

1. **A16's own premise was wrong, and this is the fourth plan task where that has happened.** It
   said "none of the three needs sub-instruction timing" and that a read returns the byte currently
   being fetched. The window is *one tick* wide, and unconditional-return is CGB behaviour. Both
   corrected in A16.
2. **The bug that mattered was not in the task at all.** Three of the four defects I fixed are the
   ones A16 describes; the fourth — the latched period — is what actually kept test 09 red, and I
   only found it by printing gambatte's `NR33`/`NR34` writes beside gb's. Two emulators disagreeing
   about a *register value* was the tell.
3. **Instrumenting a copy of gambatte was worth far more than reading it.** Every wrong theory I had
   (offset parity, mask-vs-clamp, "the test sweeps the period") died in one run of a patched
   `Channel3::waveRamRead`. §2.5 now documents both that and the screenshot-dump harness, because
   **B10 will need reference images for `cgb-acid2` and this is how to get them**.
4. **A17: the combined suite ROMs do not terminate.** ⚠️ **Half of this bullet is wrong — see
   ledger #8, which root-caused it.** `cpu_instrs.gb` was never broken; it needed a bigger cycle
   budget, and is now wired and green. `dmg_sound.gb` is real, and it **is** the bank clamp after
   all: my "swapping the clamp for a mask changes nothing" experiment was run against `cpu_instrs`,
   the ROM that did not have the bug.

**Tree:** dirty, uncommitted. Modified `src/audio/{mod,timer,wave_channel}.rs`, `src/core.rs`,
`src/mmu.rs`, `src/game_boy.rs`, `src/roms/mod.rs`, `CLAUDE.md`, this document. Added four PNGs
under `src/roms/dmg_sound/` (three promoted from gb's own output, `dmg_sound.png` captured from
gambatte). **No `src/pokemon/**` source touched and no fixture regenerated** — `git status
--porcelain src/pokemon/data/` is empty. gambatte tree verified clean; all reference work was done
on a copy in scratch.

**Next agent:** **B1.** Phase A's exit criteria are met and A17 is not one of them — treat it as a
standalone bug, and note it may be masking a second failure mode in the other combined ROMs
(`mem_timing`, `oam_bug`), which are ignored for unrelated reasons. Two things from this session
will save you time in Phase B: §2.5 now tells you how to get a reference screenshot out of gambatte
(you need one for `cgb-acid2`), and `Audio::set_instruction_length` is the precedent for "the
peripheral needs to know where in the instruction the access was" — B3's mode-3 palette blocking is
the same shape of question, and the plan already defers it.

---

### 2026-08-05 (#8) — A17 → `SUPERSEDED`, D1 — Root-caused the combined-ROM failure; half of #7's A17 note was wrong

**State:** **A17 → `SUPERSEDED`** by **D1**, which now carries the work and has a ready-made failing
acceptance test. `blargg_cpu::all` added and **passing**. Ledger #7's surprise 4 corrected in place.

**Did:** Alex asked why the combined `cpu_instrs` ROM fails when the individual ones pass. It does
not fail. Two separate findings:

**1. `cpu_instrs.gb` was never broken — it needed 2.5x the cycle budget.** The sub-tests run back to
back and `11-op a,(hl)` dominates. Stepping the budget in 20M increments:

| Budget | Serial output |
|---|---|
| 40M M-cycles | `… 09:ok  10:ok  11` ← what #7 called a stall |
| 60M M-cycles | `… 10:ok  11:ok`, `Passed all tests` |

`serial_console_test` defaults to 25M, so it never had a chance. Now wired as
`game_boy::tests::blargg_cpu::all` via a new `serial_console_test_within`, budget 60M. **A budget
shortfall and a hang are indistinguishable from outside** — raise the budget before saying "stall".

**2. `dmg_sound.gb` is real, and it is exactly the `clamp` vs `mask` A5 deferred to D1.** Blargg's
runner selects each sub-test by writing its index to `0x2000`. The ROM is 64 KB / 4 banks. Traced on
both emulators:

| Write | gambatte (`cartridge.cpp:148`) | gb (`mmu.rs:228`) |
|---|---|---|
| 1, 2, 3 | banks 1, 2, 3 | banks 1, 2, 3 ✅ |
| **4** | `adjustedRombank(4) & 3` = **bank 0**, then the ROM stops | `min(count-1)` = **bank 3** ❌ |

Bank 0 in the switchable slot holds the runner's terminator. gb re-runs bank 3's test forever,
printing `NN:ok` past 12, past 99, into ASCII. Applying gambatte's mask to gb reproduces its trace
exactly (`1, 2, 3, 0`, one write, done) and `blargg_dmg_sound::all` **passes against the committed
reference**.

**⭐ But the mask cannot just be applied, and that is the finding that matters for D1.** MBC1 masks
the bank register to **5 bits**. `pokered.gbc` is **MBC3** (`0x147 = 0x13`, 1 MB, **64 banks**) and
needs 6. A universal `& 0x1F` fails **8 tests in the default tier** immediately. Verified, then
reverted — the mask width has to come from the mapper, which is what D1/D2 exist to build.

**Verified:**
- `cargo test --release --bin gb` → `910 passed; 0 failed; 115 ignored` (+1: `blargg_cpu::all`).
- `blargg_cpu` module: `13 passed; 0 failed`, combined ROM included.
- Bank traces captured from **both** emulators and compared line by line, not reasoned about.
- The mask fix was applied, shown to make `blargg_dmg_sound::all` pass, shown to break Pokémon,
  and reverted. `mmu.rs` is byte-identical to before the experiment.
- Fixtures clean; no `src/pokemon/**` source touched; gambatte tree clean (work done on a copy).

**Surprises:**
1. **I ruled out the right cause with the wrong ROM.** #7 recorded "not the bank clamp — tested",
   which sounded rigorous and was worthless: the experiment ran against `cpu_instrs`, which had no
   bug to fix. **A negative result is only as good as the case you ran it on** — state which one.
2. **The "obvious" fix is a trap that the default tier catches in 70 seconds.** Masking to MBC1's
   5 bits silently truncates pokered's 64 banks. Anyone reading only A17's summary would have
   written that one-liner; A17 now says so explicitly, with the mapper byte.
3. Two ROMs failing the same way at a glance had two unrelated explanations. Worth remembering
   the next time a class of tests "all fail for the same reason".

**Tree:** dirty, uncommitted. Since #7: `src/game_boy.rs` (combined `cpu_instrs` test +
`serial_console_test_within`, and `blargg_dmg_sound::all`'s ignore reason now names D1),
`CLAUDE.md`, this document. `src/mmu.rs` restored to its pre-experiment state.

**Next agent:** still **B1**. When you reach **D1**, `blargg_dmg_sound::all` is your acceptance
test and A17 has the whole trace — including the mapper-width trap, which will otherwise cost you
the default tier.

---

### 2026-08-05 (#9) — B1–B10 — **Phase B complete.** CGB support; Pokémon Red boots in colour; `cgb-acid2` passes byte-for-byte

**State:** **B1–B10 all → `DONE`.** New **B11** added (`TODO`): the DMG post-boot register/IO
table, split out of B9 because it is the one change in this phase that could move the committed
fixtures, and B9 itself says to get Alex's call first. **Phase C (C1) is next.**

**Did:** the whole phase, in the plan's order. New files: `src/model.rs` (`Model`, `ColorMode`),
`src/cgb_palette.rs`, `src/boot_palette/` (`mod.rs` + a generated `tables.rs`), `src/hdma.rs`.
Six things worth knowing beyond the task text:

1. **`ColorMode` is the type that made this tractable** (`src/model.rs`). `Model` alone is not
   enough, because a CGB running a DMG cartridge is a third thing: colour hardware drives the
   screen, but the *cartridge* sees the DMG register set. `ColorMode::{Dmg, CgbCompat, Cgb}`, with
   `cgb_features()` meaning "the cartridge can see CGB hardware", is the predicate every branch
   keys off. The PPU is not forked (§B1's warning respected) — `map_pixel` reads an all-zero
   attribute byte on DMG, which is exactly "palette 0, bank 0, no flips, no priority", so the two
   paths converge without a per-pixel branch.
2. **The frame buffer is 24-bit, not RGB555** — see the corrected B4. `0xAA` has no 5-bit form.
3. **B5's palette tables are generated, not transcribed** (`src/boot_palette/tables.rs`), from
   SameBoy's `BootROMs/cgb_boot.asm` on `master`. The generator asserted the invariants as it ran
   (94 checksums = 94 combination indexes, 29 duplicates = 29 disambiguating letters, every
   combination in range). **Pokémon Red resolves to combination 13** via checksum `0x14` at table
   index 22 — *before* the ambiguous tail, so the 4th letter `'E'` is never consulted. SameBoy's
   four "exclusive" combinations and two extra palettes are dropped: they are SameBoy additions,
   not boot-ROM data.
4. **B7 was done before C1 rather than deferred**, and it is 15 lines. See the note on B7.
5. **B9 grew `SC`'s CGB read mask** from `0x7E` to `0x7C`, because bit 1 is a real register there
   (the 32x serial clock). The `0xFEA0` region on CGB is three 8-byte RAM blocks mirrored 4x, from
   gambatte's `fexx_ffxx_dumper_cgb.bin`.
6. **`src/sdl/**` was not touched at all** (§2.2). The UI still constructs a DMG; `GameBoy::cgb`
   is one line away in `render.rs` if Alex wants colour on screen, but that is a UI decision, not
   a compatibility one.

**Verified:**

- Default tier: **`954 passed; 0 failed; 115 ignored`** (was 910/115 — +44 tests, no new ignores).
- `slow-tests`: `115 passed; 0 failed; 2 ignored; finished in 125.96s`.
- **`cgb-acid2` passes byte for byte** — not "looks right": the frame was dumped and diffed
  against the committed reference pixel by pixel, **0 differing pixels**, with all 8 of the
  reference's distinct colours present in the output. It passed on the first run, which is the
  kind of result that deserves suspicion, so it was checked this way before being believed.
- **Pokémon Red in colour**, asserted rather than eyeballed: at three points through the intro,
  every pixel of the CGB frame equals the DMG frame's shade put through the boot palette's BG
  ramp or its OBJ0 ramp. Screenshots: `target/pokered-cgb-*.png`. It is the real thing — black on
  white copyright screen, salmon stars and a green spark under the GAME FREAK logo, dark-red
  Nidorino against green Gengar.
- Ignored list with every tier feature on: **19**, name for name identical to ledger #7's. Phase B
  added no ignored tests.
- `full_playthrough`: **passes**, `finished in 654.85s` on the final tree (and again at 631.82s
  before the hot-path inlining work). Run because Phase B changes the pixel path
  and the memory map that Pokémon Red exercises every frame; it changes no cycle count, which is
  why the RNG stream and the leg fixtures still line up.
- Fixtures: `git status --porcelain src/pokemon/data/` is empty. **None regenerated, none needed
  to be** — see the correction under the Phase B sequencing note.
- ⭐ **Mutation-tested, 27 mutants, all caught.** Each Phase B mechanism was deliberately broken
  and the suite re-run (ledger #4's lesson, applied up front rather than after a scare). This is
  the part of the session that paid for itself — see Surprises 2.

**⚠️ Phase B costs core throughput. Measured, not guessed:**

| Workload | HEAD (`28a237c`) | Phase B | Δ |
|---|---|---|---|
| Pokémon Red (fixture) | 31.6x | 29.5x | **−6.6%** |
| `cpu_instrs.gb` | 51.3x | 45.3x | **−11.7%** |
| `dmg-acid2.gb` | 47.1x | 42.6x | **−9.6%** |

Six paired runs, **alternating which build goes first**, both trees built from the same toolchain
on the same machine minutes apart. Read the methodology note below before comparing these to any
other session's numbers.

Roughly a third of the original regression was recovered before landing, by pushing cold paths out
of `MMU::update` — see Surprise 3. What remains is real and unexplained at the line level; it is
**Phase C's to reclaim**, and C5 (whole-scanline rendering) rewrites the exact code involved.

**Surprises:**

1. ⭐⭐ **My benchmarking methodology was wrong, and it nearly produced a fabricated bisection.**
   This machine has fast and slow states that differ by **~15%** — the *same unmodified binary*
   measured 43.5x and 53.2x on `cpu_instrs` twenty minutes apart. My first "interleaved" runs put
   the baseline first and the candidate second every time, so a within-block drift would have been
   indistinguishable from a real regression. It took an accidental control — a build measuring
   *faster than the baseline's typical figure* — to notice. **Alternate the order (ABBA) and
   report both directions**; a single ordering is not a controlled experiment. Ledger #7's "compare
   within this table, not across sessions" was right and did not go far enough: you cannot compare
   across *minutes* either, only across adjacent paired runs.
2. **Six plausible causes were measured and eliminated at ≤2% each**, which is worth recording so
   nobody re-runs them: the 24-bit frame buffer (**~0%** — B4 is not the cost), the array growth to
   32 KB WRAM / 16 KB VRAM (**0%**), the `SVBK` indexing arithmetic (0%), missing bounds-check
   elision (~1.4%), the ~15 guarded match arms added to `MMU::read`/`write` (~0%, though they are
   now out of line anyway), the per-instruction HDMA hook (~1%), and the per-pixel `ColorMode`
   branches (**~0%** — const-folding them away changed nothing). Each was a fair adjacent pair.
3. ⭐ **The cause is code growth in the hot path, and `nm` found it when bisection could not.**
   `MMU::update` is called once per instruction, and Phase B grew it **60%**, from 3052 to 4893
   bytes, by inlining the CGB pixel path and three DMA loops into it — while `Core::execute` barely
   moved. Marking `draw_pixels_to`, `run_oam_dma`, `run_hblank_dma` and `run_general_dma`
   `#[inline(never)]`/`#[cold]` brought it back to 3268 bytes and recovered **+3% to +4.5%** across
   all three workloads. That is why nothing bisected: the cost is emergent, threshold-like, and
   spread over every added line rather than sitting in one of them. **`nm -S --size-sort` on the
   two binaries is the tool for this class of regression** — it took two minutes and half a dozen
   failed hypotheses had not.
4. **`cgb-acid2` passed first try, and that is the only reason to distrust it.** Ledger #7's
   Python-quoting incident is the precedent: a green run proves nothing until you have seen the
   thing you changed actually matter. Mutation testing is what settled it — six separate
   Phase B mechanisms each make `cgb_ppu` fail when broken, so the pass is load-bearing.
5. ⭐ **The mutation sweep found a real hole that every other check missed.** Dropping the `BGP`
   indirection in compatibility mode — rendering the raw colour index straight into CGB palette 0
   — left **the entire suite green**, `cgb-acid2` and the Pokémon Red colour test included.
   `cgb-acid2` never uses a non-identity `BGP` (it is a CGB-mode ROM; `BGP` is dead there), and
   Pokémon Red's intro happens to use `BGP = 0xE4`, which *is* the identity. A game that inverts
   `BGP` — which Pokémon Red does on every screen fade — would have rendered inside out. Closed by
   `ppu::tests::cgb`, which drives a reversed `BGP`/`OBP` through a known tile. **A reference ROM
   only tests what it happens to exercise.**
6. **The plan's own premise about savestates was wrong, and in the useful direction** — a size
   change did not force a shape change. Corrected in place under the Phase B sequencing note,
   because Phase C's `MachineCycles` widening will face the same question and the answer is
   probably the same.
7. **A CGB register block is mostly about what stays *off*.** More of B1-B9's bugs were "this
   register answered in compatibility mode when it should not have" than anything to do with
   colour. `cgb_registers_are_unmapped_without_cgb_features` covers all twelve at once.

**Tree:** dirty, uncommitted, and now also carries Phase B. New: `src/model.rs`,
`src/cgb_palette.rs`, `src/hdma.rs`, `src/boot_palette/{mod,tables}.rs`,
`src/roms/cgb-acid2/{cgb-acid2.gbc,reference.png}`. Modified: `src/{core,game_boy,mmu,ppu,
registers,serial,lcd_control,lcd_palette,main}.rs`, `src/roms/mod.rs`, `src/savestate/mod.rs`,
`CLAUDE.md`, this document. `CLAUDE.md`'s throughput figure was corrected from ~34x to ~30x core /
~25x end-to-end and now carries the benchmarking warning. **No `src/pokemon/**` source touched, no `src/sdl/**` touched, no
fixture regenerated.** gambatte tree untouched (it has no CGB-compat palette to reference anyway,
so the only thing read from it was the `fexx_ffxx_dumper_cgb.bin` hardware dump).

**Next agent:** **C1**, and you inherit a **6-12% deficit from Phase B** on top of the gap you
were already chasing — the table above is the new starting line, so re-measure it yourself before
attributing anything. Four things from this session bear on Phase C directly:

1. ⭐ **Fix your benchmarking before you trust a single number.** Surprise 1 is not a footnote —
   this machine's fast/slow states are as large as most of the wins you will be measuring. Run
   ABBA, report both orders, and treat any unpaired comparison as worthless.
2. **`nm -S --size-sort -C` on the two binaries, every time.** Hot-path code growth is invisible to
   bisection and cost more here than every algorithmic change put together. `MMU::update` is the
   function to watch; keep it under ~3 KB.
3. `MMU::update` now has exactly one line where CPU time becomes peripheral time (`video_cycles`,
   the double-speed divisor) — that is the seam C1 replaces, and B7 was written to put it there.
   The `catch_up(now)` shape §5 asks for was **not** adopted: nothing added in Phase B accumulates
   its own clock, they all still take a delta, so C1 changes signatures rather than semantics.
4. Before you widen `MachineCycles`, read the savestate correction above — the append trick that
   saved Phase B may save you too.

---

### 2026-08-06 (#10) — B9 — CGB hardware has **two** boot register files; gb shipped one

**State:** **B9 stays `DONE`**, with a correction folded into its task body. No task changed state.

**Did:** Alex asked whether the CGB post-boot register values were right, given that the DMG ones
in this codebase are known to be wrong. They were right for a CGB-aware cartridge and **wrong for
compatibility mode** — which is the mode Pokémon Red, the whole point of Phase B, runs in.

A CGB running a DMG-only cartridge takes the boot ROM's `EmulateDMG` path. That routine ends
`ld de, 8` / `ld l, $7C`, and the shared final block then does `ldh a, [hTitleChecksum]` / `ld b, a`
— so `B`, `D`, `E` and `L` all differ from the CGB-mode file that B9 originally used for both:

| | CGB (CGB mode) | CGB (DMG-compat mode) |
|---|---|---|
| `B` | `0x00` | **title checksum** — `0x14` for Pokémon Red; `0x00` if not first-party |
| `DE` | `0xFF56` | **`0x0008`** |
| `HL` | `0x000D` | **`0x007C`**, or `0x991A` when `B` is `0x43`/`0x58` |

`RegisterSet::boot` now takes `(ColorMode, cart)` rather than `Model`, and the licensee rule lives
with the palette code it shares (`boot_palette::compatibility_b_register`) rather than being
duplicated. The `0x991A` case is the two cartridges whose palette entry carries SameBoy's `$80`
flag: loading the DMG boot tilemap leaves `HL` pointing into VRAM. Those are exactly the entries
with title checksum `0x43` and `0x58`, both unambiguous, which is why Pan Docs can state the rule
on `B`.

**Verified:** `cargo test --release --bin gb` → `956 passed; 0 failed; 115 ignored` (+2). Four new
tests in `registers::tests` pin all three register files, the licensee rule, the `0x991A` case, and
that the file survives a reset. Fixtures clean.

**Surprises:**

1. ⭐ **gambatte is wrong here, and it is this plan's designated reference.** `initstate.cpp:1174-1181`
   uses the DMG values for CGB with only `A` and `B` changed — `C = 0x13`, `E = 0xD8`, `F = 0xB0`,
   `HL = 0x014D` — which matches neither Pan Docs nor the boot ROM. Had I checked B9 against
   gambatte, as §2.5 encourages for everything else, I would have "confirmed" a *different* wrong
   answer. **§1.5's prime directive 4 says the reference emulator is read-only; it does not say it
   is right.** For boot state, the boot ROM disassembly and Pan Docs are the sources, and B5's task
   text already said as much for palettes — the same caveat applies to registers.
2. **The bug was invisible to every test I had, including the mutation sweep.** Ledger #9's sweep
   mutated `A` (caught) but nothing else in the file, because nothing observable depends on `B`/`DE`/
   `HL`: Pokémon Red's entry point is `nop; jp Start` and it never reads them. A mutation sweep only
   probes the behaviours you already assert. **"No test caught it" and "it does not matter" are
   different claims** — this one was simply wrong, cheaply, and would have mattered to some other
   cartridge.
3. The correction cost nothing in risk precisely because it is unobservable to Pokémon Red — but
   that is luck, not design. It is also why it survived a full playthrough and 956 tests.

**Tree:** dirty, uncommitted, unchanged in scope from #9 plus `src/registers.rs`, `src/core.rs`
(`Core::new`/`reset` now pass the colour mode and cartridge) and `src/boot_palette/mod.rs` (the new
`compatibility_b_register`). No `src/pokemon/**`, no `src/sdl/**`, no fixture regenerated.

**Next agent:** still **C1**. If you touch boot state at all — **B11** is where that lives — treat
gambatte's `initstate.cpp` as a *hardware dump* for RAM contents (which it is, and a good one) but
**not** as authority for the CPU register file.

---

### 2026-08-06 (#11) — B11 (definition), `05-mmu-cartridge.md` §7 — DMG boot `F` is conditional, not `0xB0`

**State:** No task changed state. **B11's definition corrected**, and a factual error corrected in
`05-mmu-cartridge.md` §7 — logged here as §1.3 requires.

**Did:** Alex supplied Pan Docs' Power-Up Sequence register tables. My three implemented files —
DMG, CGB, CGB-compat — match them exactly, and the CGB pair is pinned by
`registers::tests::the_boot_register_file_matches_the_boot_rom` (ledger #10). But the table carries
a footnote on DMG `F` that this plan and the research doc had both flattened:

**Old claim** (`05-mmu-cartridge.md` §7 table and task list, and B11 as I wrote it yesterday):
"real DMG leaves `F` at `0xB0` (Z, H and C set)".

**New claim:** `F` is `Z=1 N=0 H=? C=?`, where *"if the header checksum is `$00`, then the carry and
half-carry flags are clear; otherwise, they are both set."* The boot ROM's last flag-affecting
operation is the header-checksum verification, so `H` and `C` carry its result:
`F = if rom[0x14D] == 0 { 0x80 } else { 0xB0 }`.

`gb`'s current flat `0x80` is therefore **correct** for a cartridge whose header sums to zero, and
wrong for every other. Checked every ROM committed here — `pokered` `0x20`, `cpu_instrs` `0x3B`,
`dmg-acid2` `0x9F`, `cgb-acid2` `0xEB`, `button_test` `0x1D`, `tetris` `0x0A` — all non-zero, so all
of them want `0xB0`. The zero case is reachable in principle (a real cartridge must have a *correct*
checksum to boot, and a correct checksum can legitimately be `0x00`), which is why mooneye ships
`boot_regs-dmg0` alongside `boot_regs-dmgABC`.

Also folded the rest of the table into B11 — DMG0 / MGB / SGB / SGB2 rows — so whoever takes it has
the data rather than a pointer.

**Verified:** documentation only; no code changed in this entry. Header-checksum bytes read directly
from the committed ROMs rather than assumed.

**Surprises:**

1. ⭐ **gambatte hardcodes `0xB0` too** (`initstate.cpp:1179`), so it does not model the condition
   either. That is the *second* boot-state detail in two days where gambatte is not the authority —
   see ledger #10. Both times the answer came from Pan Docs plus a boot-ROM disassembly. **For boot
   state specifically, stop reaching for gambatte first.**
2. **A one-line summary in a research doc lost the condition, and I then propagated it into a task
   definition.** The §7 table said "gambatte `0xB0` / gb `0x80`", which is a true statement about
   *gambatte* and reads as a statement about *hardware*. B11 would have been implemented as a
   constant. The guides in this directory were written by reading, not running (ledger #4's lesson)
   — this is the documentation-shaped version of the same failure mode.

**Tree:** dirty, uncommitted. This entry touched `docs/compatibility/10-implementation-plan.md` and
`docs/compatibility/05-mmu-cartridge.md` only.

**Next agent:** still **C1**. If you take **B11**, implement `F` as a function of `rom[0x14D]`, not
as a constant — and note that changing DMG `F` at all is the low-risk half of that task; the `DIV`
and RAM-fill half is what re-rolls Pokémon Red's RNG.

---

### 2026-08-06 (#12) — B11 (definition) — DMG boot `F`: derived from the boot ROM, and Pan Docs is wrong too

**State:** No task changed state. **B11's `F` rule replaced** with one derived from the boot ROM
source; ledger #11's version (which repeated Pan Docs) superseded.

**Did:** Alex pushed back on #11 with the right question — *"won't the header checksum always be the
result it needs to be to pass?"* — and then found the Pan Docs sentence that confirms it: the boot
ROM **locks up** if the checksum does not match, so control only ever reaches a cartridge whose
check passed.

That is exactly the point, and it is what makes the flags well-defined rather than what makes them
constant. Fetched the DMG boot ROM disassembly (`ISSOtm/gb-bootroms`, `src/dmg.asm`) rather than
reasoning about it further:

```asm
    ld b, HeaderChecksum - HeaderTitle   ; $19
    ld a, b
.computeChecksum
    add a, [hl]                          ; 0x134..0x14C
    inc hl
    dec b
    jr nz, .computeChecksum
    add a, [hl]                          ; <- the stored checksum at 0x14D
.checksumFailure
    jr nz, .checksumFailure               ; lock up unless A == 0
    ld a, BOOTUP_A_DMG                    ; `ld`/`ldh` do not touch flags
    ldh [rBANK], a                        ; ...so F at handoff is that `add`'s
```

The check always passes — but the *arithmetic that reaches zero* differs. Let `c = rom[0x14D]`; the
running sum must be `(-c) & 0xFF`. Then:

- `C` is set iff the 9-bit sum reached `0x100`, i.e. iff **`c != 0`**.
- `H` is set iff the low nibbles carried, i.e. iff **`c & 0x0F != 0`**.

Those are *different questions*, so:

| `rom[0x14D]` | `F` | count |
|---|---|---|
| `0x00` | `0x80` | 1 |
| non-zero multiple of `0x10` | **`0x90`** | 15 |
| anything else | `0xB0` | 240 |

**Verified:** derivation brute-forced over all 256 checksum values, and the boot ROM's two variant
blocks (`dmg.asm:57-65` and `:277-286`) confirmed to have identical structure, so this holds for
DMG, DMG0 and MGB. Checksum bytes read from the committed ROMs.

**Surprises:**

1. ⭐⭐ **Pan Docs is wrong here, and `pokered` is one of the cases it gets wrong.** The footnote
   says `H` and `C` are either both clear or both set; that fails for the 15 non-zero multiples of
   `0x10`, where `C=1` and `H=0`. **`pokered`'s header checksum is `0x20`** — so the single most
   important cartridge in this repo boots with `F = 0x90` on real DMG hardware, a value neither
   Pan Docs' table, gambatte, nor either of my two previous ledger entries would have produced.
2. **Three sources, three different wrong answers, and the code was right there.** gambatte
   hardcodes `0xB0`; Pan Docs gives a two-case rule; ledger #11 copied Pan Docs. The boot ROM
   disassembly is 322 lines and took one `curl`. **When a value is *derived* rather than *dumped*,
   go to the code that derives it** — dumps (like `fexx_ffxx_dumper_cgb.bin`) are authoritative for
   RAM contents, but a register left over from a computation is only as good as your model of the
   computation.
3. **The user's "wrong" intuition was the key.** "The checksum always passes" is true, and I had
   treated the footnote's condition as being about pass/fail. It is not — it is about *how* the sum
   reaches zero. Following the objection instead of defending the citation is what produced the
   right rule.

**Tree:** dirty, uncommitted. Documentation only in this entry:
`docs/compatibility/{10-implementation-plan,05-mmu-cartridge}.md`.

**Next agent:** still **C1**. **B11** now has a precise, tested-in-principle rule for `F`; implement
it as a function of `rom[0x14D]`, and remember it is the *low-risk* half of B11 — `DIV` and the RAM
fills are what re-roll Pokémon Red's RNG.

---

### 2026-08-06 (#13) — housekeeping — Phase B committed to `main`; Pan Docs report declined

**State:** No task changed state. Recorded here because two things every earlier entry says are no
longer true.

**Did:**

1. **Alex authorised committing and pushing to `main`**, so §0.2's "no committing, pushing or
   branching unless Alex asks in that session" was satisfied for this session. **The tree is no
   longer dirty** — ledger entries #7 through #12 all end with "dirty, uncommitted", and that is
   now historical. Phases A and B are both in `main`.
2. **Alex declined raising the Pan Docs correction upstream.** Ledger #12 found a genuine defect in
   Pan Docs' Power-Up Sequence footnote (DMG boot `F`; see that entry). Offered to file it against
   `gbdev/pandocs`; **Alex said no.** Do not re-raise it — the finding is recorded here and in
   `05-mmu-cartridge.md` §7, which is where this project needs it.

**Verified:** `cargo test --release --bin gb` → `956 passed; 0 failed; 115 ignored`; `slow-tests`
`115 passed`; `full_playthrough` `1 passed ... 667.34s`; `git status --porcelain src/pokemon/data/`
empty. Nothing under `src/pokemon/**` or `src/sdl/**` was modified.

**Tree:** clean, committed and pushed to `origin/main`.

**Next agent:** **C1**, and you now start from a committed baseline rather than someone else's
uncommitted work — `git log` is a usable history again. Read ledger #9's benchmarking warning
before you measure anything.

---
