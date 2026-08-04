# Test harness & compatibility-suite guide

How `gb` is tested today, how **gambatte's** 3524-ROM hardware suite works and could be reused, what
other suites exist, and in what order to adopt them.

---

## 1. Current state of `gb`

### 1.1 What actually runs

`cargo test --release --bin gb -- game_boy::tests` → **31 passed; 0 failed; 0.56s**
(verified 2026-08-04). Total default-tier tests: **966**.

| Path | Declared in `src/roms/mod.rs` | Asserted by a test |
|---|---|---|
| `cpu_instrs/01-special.gb` … `11-op a,(hl).gb` | yes | ✅ **all 11 pass** |
| `cpu_instrs/cpu_instrs.gb` (combined) | yes | ❌ used only as a convenient cartridge for unrelated unit tests (`src/header.rs:124`, `src/mmu.rs:464`, `src/pokemon/options.rs:80`, `src/pokemon/encoding.rs:309`) |
| `instr_timing.gb` | yes | ✅ **passes** |
| `dmg_sound/01…12` | yes | ⚠️ **9 of 12** |
| `dmg_sound/dmg_sound.gb` (combined) | yes | ❌ never referenced |
| `dmg-acid2/dmg-acid2.gb` + `reference-dmg.png` | yes | ✅ **passes** |
| `button_test/rom.gb` + 8 PNGs | yes | ✅ 8 tests |
| `Jayro's Test Cart v2.3.0.gb` | yes (`homebrew::TEST_CART`) | ❌ zero references |
| `tetris.gb` | **not declared** | ❌ orphan file on disk |

### 1.2 The CLAUDE.md claim is overstated on one point

> *"passes Blargg's cpu_instrs, dmg_sound, instr_timing; dmg-acid2 PPU test"*

`cpu_instrs` (11/11), `instr_timing` and `dmg-acid2` are genuinely asserted and genuinely pass.
**`dmg_sound` is 9 of 12.** Tests `09-wave read while on`, `10-wave trigger while on` and
`12-wave write while on` are commented out at `src/game_boy.rs:236-246` and `:251-256`, and their
"expected" PNGs are placeholders:

```rust
// src/roms/mod.rs:41-46
pub const EXPECTED_WAVE_READ_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
pub const EXPECTED_WAVE_TRIGGER_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
pub const EXPECTED_WAVE_WRITE_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
```

The honest claim is *"Blargg dmg_sound 1–8 and 11"*. The three failures are exactly the wave-RAM
quirks in [`04-apu.md` §6](04-apu.md#6-wave-channel--the-biggest-concrete-gap), where a **two-line**
approximation would likely re-enable `09` and `12`.

- [ ] Correct the CLAUDE.md accuracy claim.
- [ ] Delete or wire up `tetris.gb`, `Jayro's Test Cart v2.3.0.gb`, and the two unused combined
      ROMs.
- [ ] Unify failure artifacts: `src/game_boy.rs:404-408` writes to `target/test_failure_<name>.png`
      while the Pokémon tests use `target/test-artifacts/`. Two conventions coexist.
- [ ] Rename `src/header.rs:134 fn parse_cpu_tetris` — it parses POKERED, not Tetris.

### 1.3 The existing harness (reuse this)

All in `src/game_boy.rs:100-409`.

**Serial capture** (`:352-378`) — runs in 1000-M-cycle slices, polls `Serial::buffered_bytes`
(`src/serial.rs:33`), passes on substring `"Passed"`. This is the correct blargg convention and is
**directly reusable for 19 more blargg ROMs with zero new code**.

**Framebuffer equality** (`:380-397`) — exact `RgbImage ==` against a decoded PNG, polled every 1000
M-cycles up to a 20M budget. dmg-acid2 (`:335-349`) is a one-shot.

> ⚠️ **Correction to a widespread belief:** blargg's *Game Boy* ROMs have **no** `$A000` /
> `$DE $B0 $61` magic signature — that is his *NES* suite. `cpu_instrs/readme.txt` says outright
> that there is no well-defined programmatic result location and recommends a screenshot. The
> serial-substring approach `gb` already uses is the de-facto standard and is correct.

### 1.4 Architectural limits — these predict which suites fail wholesale

1. **No CGB.** `Core::dmg` is the only constructor (`src/core.rs:31`). `cgb_sound`, `cgb-acid2`,
   HDMA, KEY1, CGB palettes — all out of scope.
2. **Instruction-granular timing.** `Core::execute` → `self.mmu.update(cycles)` once per instruction
   (`src/core.rs:483`, `:500`). **The single biggest blocker** — nothing sub-instruction can pass.
   See [`02-cpu.md` §1](02-cpu.md#1-timing-granularity--the-root-cause).
3. **Coarse PPU** — fixed mode-3 length with an explicit
   `// TODO vary the length of the HBlank period` (`src/ppu.rs:275`).
4. **No VRAM/OAM access blocking** at the right cycles.
5. **STAT IRQ is level-OR, not edge** (`src/lcd_status.rs:52`, `:78`) — exactly what
   `m0enable`/`m2enable`/`lycEnable`/`miscmstatirq` (~480 gambatte tests) probe.
6. **DIV is 8-bit and starts at 0** (`src/divider.rs`), not the 16-bit internal counter whose
   post-boot visible value is `0xAB`.
7. **No halt bug, no OAM bug** (`grep -rn "halt_bug\|oam bug" src/` → nothing).

---

## 2. gambatte's hardware test suite

### 2.1 Inventory (verified in this checkout)

```
find test/hwtests -name '*.asm' | wc -l   ->  3524
find test/hwtests -name '*.gb*' | wc -l   ->     0     (nothing prebuilt)
find test/hwtests -name '*.png' | wc -l   ->   413
```

1000 of the 3524 contain `_ds_` (double-speed) → CGB-only. **DMG-reachable total: 1873 runs.**

| Area | `.asm` | DMG-reachable | Targets |
|---|---:|---:|---|
| `oamdma` | 442 | 393 | Bus conflicts during DMA, `late_sp00x_*`, sources C000/FE00/FF00 |
| `sprites` | 420 | 96 | 10-per-line limit, X-priority, sprite effect on mode-3 length |
| `window` | 276 | 203 | `late_disable_*`, `late_enable_afterVblank_*`, WX=A7 edge |
| `speedchange` | 242 | 2 | CGB KEY1. **CGB-only** |
| `dma` | 229 | 0 | CGB GDMA/HDMA. **CGB-only** |
| `sound` | 164 | 139 | Duty pattern position, length-ctr vs DIV write, NR52 readback |
| `miscmstatirq` | 159 | 139 | `lycflag_statwirq` — the DMG STAT-write bug |
| `lycEnable` | 147 | 78 | LYC IRQ enable/disable at every frame phase |
| `scx_during_m3` | 134 | 44 | SCX writes mid-mode-3. PNG-judged |
| `tima` | 121 | 113 | TIMA/TMA/TAC, TMA-reload delay |
| `enable_display` | 116 | 68 | LCDC bit 7 off/on, first-frame LY/STAT/IRQ |
| `m1` | 109 | 61 | VBlank vs STAT ordering |
| `m0enable` | 99 | 69 | Mode-0 STAT IRQ enable timing |
| `halt` | 89 | 75 | `ifandie_ei_halt_*` (IF+IE matrix) |
| `m2enable` | 78 | 42 | Mode-2 equivalent |
| `ly0` | 63 | 33 | LY=153→0 boundary |
| `serial` | 46 | 36 | Transfer start vs DIV write |
| `oam_access` | 45 | 24 | OAM reads during mode 2/3 |
| `irq_precedence` | 44 | 20 | Simultaneous IRQ resolution |
| `vram_m3` | 35 | 15 | VRAM reads during mode 3 |
| `lcdirq_precedence` | 31 | 31 | LYC-vs-mode STAT precedence |
| `dmgpalette_during_m3` | 17 | 17 | BGP writes mid-scanline. PNG-judged |
| `undef_ops` | 10 | 10 | The 10 undefined opcodes — CPU must lock |
| `div` | 8 | 2 | DIV rate + STOP-resets-DIV |
| root | 10 | — | `sram.asm`, `jpadirq_1/2.asm`, 6 **dumper** ROMs (hardware documentation, not judged) |

There are also **80 `.txt` files of plain-English behaviour descriptions** — genuinely the best
documentation in the ecosystem. Example, `lycint_ly/lycint_ly.txt`:

```
waits until ly=3 / sets lyc to 5 / enables lyc int / waits for int
on int: jumps to 0x1000 / lots of nops / reads ly / outputs value of ly&7
DMG-08 / CGB: lycint_ly_1.gb should output 5, lycint_ly_2.gb should output 6
```

### 2.2 The filename *is* the expected result — there is no checksum

`testrunner.cpp:293-315` is the entire dispatch:

```cpp
std::string const s = extensionStripped(argv[i]);
char const *dmgout = 0;
char const *cgbout = 0;

if (s.find("dmg08_cgb04c_out") != std::string::npos) {
        dmgout = cgbout = "dmg08_cgb04c_out";
} else {
        if (s.find("dmg08_out") != std::string::npos) {
                dmgout = "dmg08_out";
                if (s.find("cgb04c_out") != std::string::npos)
                        cgbout = "cgb04c_out";
        } else if (s.find("_out") != std::string::npos)
                cgbout = "_out";
}
if (cgbout) { numTestsSucceeded += runStrTest(argv[i], false, cgbout); ++numTestsRun; }
if (dmgout) { numTestsSucceeded += runStrTest(argv[i],  true, dmgout); ++numTestsRun; }
```

| Token | Meaning |
|---|---|
| `_dmg08_cgb04c_out<HEX>` | run twice (CGB, then forced-DMG); both must print `<HEX>` |
| `_dmg08_out<X>_cgb04c_out<Y>` | run twice; DMG prints `<X>`, CGB prints `<Y>` |
| `_dmg08_out<HEX>` alone | DMG run only |
| `_cgb04c_out<HEX>` / bare `_out<HEX>` | CGB-only |
| `outaudio0` / `outaudio1` | audio-silence assertion instead of a screen check |
| `_xout…`, `_blank` | contains no `_out` substring → **deliberately not run** |
| sibling `<base>_dmg08.png` etc. | extra full-framebuffer comparison |

`dmg08` / `cgb04c` name the hardware revision the expectation was captured on.

Census over all 3524: 1470 CGB-only · 1272 both-same · 412 both-different · 314 PNG-or-helper ·
56 DMG-only. **Total runs: CGB 3154 + DMG 1740 + PNG 331 = 5225.**

### 2.3 How pass/fail is decided — three mechanisms

**(a) Screen text.** `evaluateStrTestResults` (`:236`) → `frameBufferMatchesOut` (`:225`) →
`tilesAreEqual` (`:214`) → `tileFromChar` (`:71`). A **16-glyph 8×8 hex font is hard-coded in the
C++ source** (`#define _ 0xF8F8F8`, `#define O 0x000000`). For each expected char it compares the
framebuffer tile at `(i*8, 0)`:

```cpp
if ((lhs[y * gb_width + x] & 0xF8F8F8) != rhs[y * 8 + x]) return false;
```

> **Porting subtlety:** `runStrTest` receives `argv[i]` **with** the extension, so `substr` yields
> e.g. `"AB.gb"`. The loop terminates on the first char `tileFromChar` rejects (anything outside
> `0-9A-Fa-f`, including `.`, `_`, and NUL). The correct rule is therefore **"take the leading
> hex-digit run"** — which also handles `_dmg08_outX_cgb04c_outY` correctly.

**(b) Audio.** `testrunner.cpp:242-248`: `audio0` = the final frame must be perfectly constant
(silence); `audio1` = must not be. Blunt but toolchain-free. 134 sources.

**(c) PNG.** `runPngTest` (`:277`) → full 160×144 compare masked `& 0xFCFCFC`.

**Run length** (`:262-275`) is a fixed `samples_per_frame * 15` ≈ **16 frames ≈ 0.27 s emulated per
run**. No polling, no timeouts — nothing waits. The whole 5225-run suite is ~24 emulated minutes,
i.e. **under a minute of wall clock at `gb`'s 23× throughput.**

`main` prints `c`/`d` per run then `Ran N tests. / M failures.` — **exit code is always 0**, so
output must be parsed.

### 2.4 Worked example: `hwtests/div/start_inc_1_dmg08_outAB.asm`

```asm
.text@1000
ltest:
	nop
	nop
	ldff a, (04)          ; <-- THE TEST: read DIV (FF04)
	jp lprint_a

.text@7000
lprint_a:                 ; shared boilerplate, byte-identical in hundreds of tests
	push af
	ld b, 91
	call lwaitly_b        ; wait for LY == 0x91 (VBlank)
	xor a, a
	ldff(40), a           ; LCDC = 0 -> LCD off, VRAM free
	ld bc, 7a00
	ld hl, 8000
	ld d, 00
lprint_copytiles:         ; copy 256 bytes = 16 hex glyphs to VRAM 0x8000
	ld a, (bc)
	inc bc
	ld(hl++), a
	dec d
	jrnz lprint_copytiles
	pop af
	ld b, a
	srl a / srl a / srl a / srl a
	ld(9800), a           ; tilemap[0][0] = high nibble
	ld a, b
	and a, 0f
	ld(9801), a           ; tilemap[0][1] = low nibble
	ld a, c0
	ldff(47), a           ; BGP = 0xC0 -> colour 3 black, 0/1/2 white
	ld a, 91
	ldff(40), a           ; LCDC = 0x91 -> LCD on
lprint_limbo:
	jr lprint_limbo
```

**Judgement, step by step:**

1. Stem = `hwtests/div/start_inc_1_dmg08_outAB`
2. `:296` — no `dmg08_cgb04c_out`
3. `:298` — `dmg08_out` found → `dmgout = "dmg08_out"`
4. `:301` — no `cgb04c_out` → `cgbout` stays null. **One run, forced-DMG.**
5. `runStrTest(file, forceDmg=true, "dmg08_out")` runs 16 frames
6. `evaluateStrTestResults` takes `file.substr(p + 9)` → `"AB.gb"`
7. Compares tile 0 vs glyph `A`, tile 1 vs glyph `B`, masked `& 0xF8F8F8`, stopping at `.`
8. **Pass iff the screen literally reads `AB` in the top-left corner**

**What `gb` does today:** `Divider::default()` starts at 0 with no post-boot seed, so it prints
`00`. Fixing DIV (see [`02-cpu.md` §5](02-cpu.md#5-div--timer--highest-accuracy-win-per-line))
clears `div` and unblocks much of `tima`.

> **A cheap first win:** `undef_ops/undef_op_d3_dmg08_cgb04c_out01.asm` prints `01`, enables the
> VBlank IRQ, then executes a raw `D3`. If the CPU locks (correct) the screen still reads `01`.
> `gb`'s `CoreMode::Crash` means **these 10 should already pass**.

### 2.5 Building: qdgbas.py

`qdgbas.py` (304 lines) imports only `re`, `sys`, `collections` — fully self-contained. It supports
~110 `addop(...)` entries (exactly the SM83 subset the tests use), stamps the Nintendo logo, header
checksum and global checksum, and emits `.gbc` if header byte `0x143` bit 7 is set, else `.gb`.

**It is Python 2**, but the port is **four mechanical edits** (verified working on Python 3.14,
assembling three real tests to correct 32 KB ROMs with a valid `0xE7` header checksum):

1. `xrange` → `range`
2. two `print` statements → functions
3. `re.sub(r'\s+', r'\s*', …)` → `re.sub(r'\s+', r'\\s*', …)` — Python ≥3.7 rejects the unknown
   escape `\s` in a *replacement template*
4. (cosmetic) raw-string three regex literals

**But you almost certainly don't need to port it** — see §3.

### 2.6 Licence — use a submodule, do not vendor

**gambatte is GPL-2.0 only, whole tree.** `COPYING` is GPLv2 verbatim; `README:4-6` says
"version 2". There is **no separate licence under `test/`**, and the `.asm` files carry no headers,
so they inherit the project licence. The 413 PNGs are likewise GPLv2 artifacts.

`gb` ships **no LICENSE** and already contains an LGPL-2.1+ blip port. Vendoring GPLv2 sources into
it is an avoidable entanglement.

- [ ] **Use a git submodule, exactly as `pokered/` is handled.** No GPL source enters `gb`'s
      history, and the corpus is never linked into the binary.
- [ ] If the Python-3 port is needed, keep it as `tools/qdgbas3.py` with the licence noted (it is a
      derivative work and is itself GPLv2), or patch the submodule copy at build time.

### 2.7 Concrete plan for wiring hwtests into `cargo test`

**Step 0 — scope to DMG.** Keep stems containing `dmg08_out` or `dmg08_cgb04c_out`, plus those with
`_dmg08.png` / `_dmg08_cgb04c.png` siblings → 1873 runs. Report the rest as `skipped: CGB`.

**Step 1 — get the binaries.** Preferred: **`c-sp/game-boy-test-roms` v7.0 ships all 3524 gambatte
ROMs prebuilt** plus the 413 PNGs. No Python at all. Alternative: submodule + a
`#[cfg(feature = "hwtests")]` branch in the existing `build.rs` that shells out to
`tools/qdgbas3.py` once when `$OUT_DIR/hwtests/` is missing or stale.

> **Do not `include_bytes!`** — 3524 × 32 KB ≈ 110 MB. Read from disk via `env!("OUT_DIR")`.

**Step 2 — expectation extraction** (a faithful port of `:293-315`):

```rust
enum Expect { Screen(String), AudioConstant, AudioVarying, Png(PathBuf) }

fn dmg_expectation(stem: &str) -> Option<Expect> {
    let tail = if let Some(p) = stem.find("dmg08_cgb04c_out") { &stem[p + 16..] }
               else if let Some(p) = stem.find("dmg08_out")    { &stem[p +  9..] }
               else { return None };                      // cgb-only, or _xout / _blank
    Some(match tail {
        t if t.starts_with("audio0") => Expect::AudioConstant,
        t if t.starts_with("audio1") => Expect::AudioVarying,
        t => Expect::Screen(t.chars().take_while(|c| c.is_ascii_hexdigit()).collect()),
    })
}
```

The `take_while(is_ascii_hexdigit)` is the crucial bit — it reproduces `tileFromChar` returning
null, which is what makes `_dmg08_outX_cgb04c_outY` and the trailing `.gb` work.

**Step 3 — the screen check.** Port the 16-glyph font from `testrunner.cpp:73-200` into a Rust
`const [[u8; 8]; 16]` bitmask, then compare **semantically** on `DMGColor`, sidestepping the
`& 0xF8F8F8` palette tolerance entirely. The tests set `BGP = 0xC0`, so glyph pixels are colour 3
and background is colour 0 — a two-level check is *exact*:

```rust
fn screen_reads(ppu: &PPU, expect: &str) -> bool {
    for (i, ch) in expect.chars().enumerate() {
        let Some(glyph) = glyph_for(ch) else { return true };
        for y in 0..8 { for x in 0..8 {
            let dark = ppu.lcd()[y * LCD_WIDTH + i * 8 + x] == DMGColor::Black;
            if dark != ((glyph[y] >> (7 - x)) & 1 == 1) { return false; }
        }}
    }
    true
}
```

**Step 4 — the run.** No polling, no timeout:

```rust
let mut gb = GameBoy::dmg(&rom);
gb.run(MachineCycles::from_m(70_224 / 4 * 16));   // 16 frames, matching samples_per_frame*15
```

For `audio0`/`audio1`, drain one frame from `Audio::read_samples_f32` (`src/audio/mod.rs:93`) and
test `all_equal()`. `src/audio/reference.rs` already has capture plumbing to reuse.

**Step 5 — the 133 PNG tests need palette remapping.** gambatte's references use *its* DMG shades
(`0xF8F8F8` white); `gb`'s `DMGColor::to_rgb` uses `FF/AA/55/00` (`src/lcd_palette.rs:17-22`).
Quantise the reference to 4 levels → indices 0–3 and compare against `ppu.lcd()` indices. **Do not
compare RGB.**

**Step 6 — reporting.** With 1873 cases, prefer an **aggregate + committed known-failure baseline**:
one test walks the tree, collects `Vec<(name, pass)>`, writes a sorted
`target/test-artifacts/hwtests-results.txt`, and asserts against a committed list — so it fails only
on *regression*, and progress shows as "N newly passing". This matches `gb`'s existing
committed-fixture-baseline habit. (A `build.rs`-generated one-`#[test]`-per-ROM variant gives better
signal but much heavier compiles; keep it for focused work on a subdirectory.)

**Step 7 — the gate**, matching CLAUDE.md's tiering:

```toml
# gambatte's hardware test suite: ~1873 DMG runs, 16 emulated frames each.
#   cargo test --release --features hwtests --bin gb -- hwtests
hwtests = []
```

Cost: 1873 × 16 frames ≈ 30 000 frames ≈ 8.3 emulated minutes ≈ **~20 s single-threaded** at 23×.
Cheap enough for the default tier *once* the pass rate is respectable; keep it gated while the
baseline is red.

> ⚠️ **Expect a low initial pass rate** (likely a few hundred of 1873 — `undef_ops`, some
> `div`/`tima`, coarse `m1`/VBlank). The value of this corpus is as a **ratchet during an
> M-cycle-accuracy refactor**, not as a quick win.

---

## 3. External suites you should be running and are not

### The headline: one download gets almost everything

**`c-sp/game-boy-test-roms` v7.0** — <https://github.com/c-sp/game-boy-test-roms> — one 3.7 MB zip
containing **4510 prebuilt ROMs + 542 expected PNGs + 44 markdown how-tos**, including all 3524
gambatte ROMs. Each suite ships a `game-boy-test-roms-howto.md` documenting the compatible hardware
revisions, exit condition, and success/failure identification — the single most useful
harness-building document in the ecosystem. Screenshot palettes are pinned there: DMG
`#000000/#555555/#AAAAAA/#FFFFFF`, which **matches `gb`'s `DMGColor::to_rgb` exactly**.

⚠️ The repo is MIT (covering c-sp's scripts only) and **the zip contains no upstream LICENSE
files** — vendor those separately. Note blargg's ROMs have **no declared licence at all**.

### 3.1 The four pass/fail conventions

| # | Convention | Used by |
|---|---|---|
| 1 | **Fibonacci registers + `LD B,B`** — `B=3 C=5 D=8 E=13 H=21 L=34`, then opcode `0x40` as a software breakpoint; fail writes `0x42` to all six. The same six bytes are **also pushed over the link port**, so detection via serial works too — no CPU hook strictly required | mooneye, SameSuite, AGE, both acid2s (exit only) |
| 2 | **Serial text** (`"Passed"` / `"Failed"` / `"Failed #n"`) | blargg only |
| 3 | **Screenshot / reference PNG** | mealybug, gambatte PNG tests, bully, strikethrough, scribbltests, TurtleTests, little-things-gb, rtc3test, mbc3-tester, acid2 |
| 4 | **Memory-mapped result byte** — `0xFF82` = `0x01` pass / `0xFF` fail | gbmicrotest only |

### 3.2 Suite-by-suite

| Suite | Covers | Signalling | Count | Licence | Prebuilt |
|---|---|---|---|---|---|
| Blargg `cpu_instrs` | SM83 correctness | serial | 12 | ⚠️ none | yes |
| Blargg `instr_timing` | per-instruction cycle counts | serial | 1 | ⚠️ none | yes |
| **Blargg `mem_timing`** | **read/write cycle *within* each instruction** | serial | 4 | ⚠️ none | yes |
| **Blargg `mem_timing-2`** | same, different method | serial | 4 | ⚠️ none | yes |
| Blargg `dmg_sound` | APU registers, length ctr, sweep, wave RAM | serial | 13 | ⚠️ none | yes |
| Blargg `cgb_sound` | CGB APU | serial | 13 | ⚠️ none | yes |
| **Blargg `oam_bug`** | DMG OAM corruption on 16-bit inc/dec at `$FE00-$FEFF` | serial | 9 | ⚠️ none | yes |
| **Blargg `halt_bug`** | HALT with IME=0 + pending IRQ → PC fails to increment | serial | 1 | ⚠️ none | yes |
| **Blargg `interrupt_time`** | IRQ dispatch cycle cost | serial | 1 | ⚠️ none | yes |
| **Mooneye Test Suite** | `acceptance/` **75** (incl. **ppu 12**, **timer 13**); `emulator-only/` **28** (mbc1 13, mbc2 7, mbc5 8 — **no mbc3**); `misc/` 8 | Fibonacci + `LD B,B`, ≤120 s | **115** | MIT | ⚠️ no GitHub releases — use gekkio.fi or the c-sp bundle |
| wilbertpol legacy | older/extended snapshot, overlapping | Fibonacci, but terminates on opcode `0xED` | 121 | GPL-3.0 | via bundle |
| **dmg-acid2** | BG+window, OBJ priority, signed/unsigned tile addressing, palettes, flips, 10-sprite limit. Explicitly **does not** need T-cycle accuracy | PNG; `LD B,B` exit | 1 | MIT | ✅ already used |
| cgb-acid2 | CGB equivalent, 21 annotated failure images | PNG | 1 | MIT | yes |
| **SameSuite** | `apu/` **70** (ch1 21, ch2 15, ch3 16, ch4 13), dma 4, interrupt 1, ppu 1, sgb 2 — **overwhelmingly APU** | Fibonacci + `LD B,B` | **78** | X11 | via bundle |
| **AGE** (`c-sp/age-test-roms`) | deliberate gap-filler: halt, `lcd-align-ly`, ly, `m3-bg-bgp/lcdc/scx`, oam, `stat-mode*`, vram | both Fibonacci **and** PNG | 47 | MIT | via bundle |
| **MealyBug Tearoom** | **register writes during STAT mode 3** — BG tile & sprite fetch timing per scanline (`m3_bgp_change`, `m3_lcdc_*_change`, `m3_scx_high_5_bits`, `m3_wx_4/5/6_change`, `m3_window_timing`, …) | PNG at the `LD B,B` breakpoint, **zero differing pixels** | 35 (73 refs) | MIT | yes, `.zip` in repo |
| **gbmicrotest** | one register/address at one exact cycle after boot. Author: *"solely for tracking down cycle-accuracy issues in emulators that are already functional"* | `0xFF82` only; 2 frames | **513** | MIT | yes, `bin/` in-repo |
| rtc3test | MBC3 RTC | PNG; ⚠️ **one ROM, 3 menu-selected subtests** — needs button synthesis | 1 (3 sub) | Unlicense | yes |
| BullyGB | wide range incl. edge cases; docs in the Wiki | PNG, ~0.5 s. ⚠️ fails on real DMG-C with `Bad Echo RAM Reads` — expected | 1 | MIT | via bundle |
| strikethrough.gb | *"some weird OAM DMA behavior"* — ⚠️ that is literally all the documentation | PNG | 1 | MIT | via bundle |
| scribbltests | `fairylake`, `lycscx`, `lycscy`, `palettely`, `scxly`, `statcount`, `winpos` | PNG. ⚠️ no references for `fairylake`/`winpos` | 8 | MIT | yes |
| TurtleTests | `window_y_trigger`, `..._wx_offscreen` | PNG | 2 | ⚠️ none | via bundle |
| MBC3 Tester | MBC3 **bank** switching | PNG at 40 frames | 1 | ⚠️ none | via bundle |

> ⚠️ **Two easy mistakes:** `c-sp/AGE` is the *emulator*; `c-sp/age-test-roms` is the suite.
> And "which-boot-rom-is-this" could not be verified to exist — do not cite it.

---

## 4. Prioritised adoption roadmap

| # | Action | Effort | Impact | Why this order |
|---|---|---|---|---|
| **0** | Fix the CLAUDE.md `dmg_sound` claim; delete/wire the 4 unused ROMs; unify failure artifacts on `target/test-artifacts/` | XS (1 h) | truth-in-advertising | Costs nothing |
| **1** | **Blargg `mem_timing` + `mem_timing-2` + `halt_bug` + `oam_bug` + `interrupt_time`** — 19 more ROMs, **zero new harness** | **XS (2 h)** | **very high** | `serial_console_test` runs them as-is. They will fail — and `mem_timing` failing is the precise, named measurement of the instruction-granularity gap. **Highest accuracy-per-effort in this entire document.** |
| **2** | **Mooneye acceptance/** — 115 MIT prebuilt ROMs | **S (~1 day)** | **very high** | ~30 lines: run until opcode `0x40` with the Fibonacci registers set, *or* watch for the same 6 bytes on serial (already captured — you may need no CPU hook at all). Yields ~75 *named* failures (`div_timing`, `ei_timing`, `rapid_di_ei`, `oam_dma_timing`, `intr_timing`, `push_timing`, `boot_div-dmgABCmgb`…), each a concrete work item |
| **3** | **Fix DIV** — 16-bit counter, post-boot seed, TIMA from a falling edge | S (~1 day) | high | Unblocks mooneye `timer/` (13), blargg `interrupt_time`, gambatte `div` (2) + much of `tima` (113). Self-contained: `src/divider.rs` + `src/timer.rs` |
| **4** | **M-cycle-granular `mmu.update`** | **L (weeks)** | **the highest of all** | Everything after depends on it. Do it *after* 1–2 so there is a measurable baseline to ratchet against |
| **5** | Mooneye `emulator-only/mbc1\|mbc2\|mbc5` — 28 ROMs | S | medium | Independent of timing work; hardens `src/mmu.rs`. See [`05-mmu-cartridge.md`](05-mmu-cartridge.md) |
| **6** | **MealyBug Tearoom** — 35 ROMs | M | high (PPU) | Needs the PNG infra you already have plus per-revision reference selection. Directly targets the fixed-mode-3 and mid-scanline gaps in [`03-ppu.md`](03-ppu.md). Pick **one** revision's reference set and document it |
| **7** | **gambatte hwtests, DMG subset** — 1873 runs, per §2.7 | M | high, **only after (4)** | The ~500-test STAT/IRQ corpus is the most detailed PPU-interrupt spec in existence, and the `.txt` files explain each. Adopt as a known-failure ratchet. GPL → submodule |
| **8** | **gbmicrotest** — 513 ROMs | S harness / L to pass | medium | Simplest harness of any suite. But the author is explicit it's for emulators that are *already* cycle-accurate — adopt **after** step 4 |
| **9** | BullyGB, strikethrough, scribbltests, TurtleTests, little-things-gb | S each | low–medium | Cheap once PNG infra exists. ⚠️ `strikethrough` and `cgb-acid-hell` give **no diagnostics** on failure |
| **10** | rtc3test / MBC3-Tester | M | medium | Pokémon Red is MBC3 (`CartType::MBC3RamBattery`, `src/header.rs:141`). Only matters if the RTC is ever emulated — see [`05-mmu-cartridge.md` §3](05-mmu-cartridge.md#3-save-data-sram--rtc) |
| — | cgb_sound, cgb-acid2, gambatte CGB (3352 runs), SameSuite `apu/` | — | **N/A** | Blocked on CGB, which does not exist. Explicitly out of scope. (SameSuite `apu/` only passes on CPU CGB E anyway) |

### 4.1 Infrastructure needed, in build order

- [ ] **`hwtests` Cargo feature**, documented alongside `slow-tests` / `full-playthrough` /
      `regen-fixtures`
- [ ] **ROM sourcing**: the `c-sp/game-boy-test-roms` v7.0 zip, or per-suite submodules. Vendor
      upstream LICENSE files separately
- [x] **Serial-out capture** — already exists and is already correct (`src/serial.rs:29`)
- [ ] **Index-space framebuffer comparison** — `DMGColor` 0–3 vs a 4-level-quantised reference, so
      references from any emulator's palette work
- [ ] **`LD B,B` breakpoint hook** — a `Core` callback or an "executed opcode 0x40" flag plus a
      register-signature check (~30 lines). Serves mooneye + SameSuite + AGE + both acid2s
- [ ] **Memory-probe hook** — read `0xFF82` after N frames. Trivial. Serves gbmicrotest
- [ ] **Per-test frame budget**, not a wall-clock timeout (gambatte 16 frames; gbmicrotest 2;
      mealybug at the breakpoint; blargg 1–55 s; mooneye ≤120 s)
- [ ] **Committed known-failure baseline** with a `regen`-style gate mirroring `regen-fixtures`
- [x] **Button synthesis** — already exists (`Joypad::press_button`, used by the `button_test` tests)

### 4.2 The one-line version

Spend an afternoon on step 1 (19 blargg ROMs, **zero new code**) and a day on step 2 (mooneye,
~30 lines). That turns *"we pass the easy suites"* into a **named, prioritised list of ~90 real
timing defects** — exactly the input the M-cycle refactor needs. gambatte's 3524-test corpus is the
endgame ratchet, not the opening move.

---

## References

- `c-sp/game-boy-test-roms` — <https://github.com/c-sp/game-boy-test-roms> (**start here**)
- mooneye-test-suite — <https://github.com/Gekkio/mooneye-test-suite>
- blargg's ROMs — <https://github.com/retrio/gb-test-roms>
- MealyBug Tearoom — <https://github.com/mattcurrie/mealybug-tearoom-tests>
- gbmicrotest — <https://github.com/aappleby/gbmicrotest>
- SameSuite — <https://github.com/LIJI32/SameSuite>
- AGE test ROMs — <https://github.com/c-sp/age-test-roms>
- gambatte's suite — `/home/alex/projects/gambatte/test/` (`testrunner.cpp`, `qdgbas.py`,
  `hwtests/**`)
