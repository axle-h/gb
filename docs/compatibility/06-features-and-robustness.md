# Feature inventory & robustness

Whole-system feature comparison of `gb` against **gambatte**, plus an audit of the panic sites in
`gb`'s core that could kill a long-running agent session.

---

## 0. Which gambatte is this? — read first

The checkout at `/home/alex/projects/gambatte` is **upstream sinamas gambatte, ~0.5.0-era**
(`changelog:1` newest entry is `-- 0.4.1 -- 2009-01-10`; git head `efa674a9`, 2021-07-06, a community
merge of two cartridge/RTC fixes). It is **not** gambatte-speedrun and **not** the libretro core.

Several APIs commonly attributed to "gambatte" do not exist in this tree. Reported honestly rather
than assumed:

| API often assumed present | Present? | Evidence |
|---|---|---|
| `setRtcDivisorOffset` | **No** | repo-wide grep → 0 hits |
| `setTimeMode` | **No** | 0 hits |
| `loadGbcBios` / boot-ROM loading | **No** | `rg -ni "boot.?rom\|bios"` over `libgambatte/` → **0 hits** |
| `CGB_MODE` load flag | **No** | `gambatte.h:37-43` has only `FORCE_DMG=1`, `GBA_CGB=2`, `MULTICART_COMPAT=4` |
| `GBA_FLAG` | named **`GBA_CGB`** | `gambatte.h:40` |
| `getPakInfo` | named **`pakInfo()`** | `gambatte.h:168`, impl `gambatte.cpp:207` |
| `setCgbPalette` | **No** | only `setDmgPaletteColor(palNum, colorNum, rgb32)`, `gambatte.h:96` |
| OPRI (`FF6C`) *behaviour* | **register only** | `memory.cpp:1069-1073` stores `data\|0xFE`; never read back |
| PCM12/PCM34 (`FF76`/`FF77`) | **No** | no `case 0x76/0x77`; falls to `default: return;` at `memory.cpp:1096` |
| CGB DMG-compat palette table keyed on title checksum | **No** | DMG mode uses a flat greyscale ramp, `video.cpp:126-128` |

**Neither emulator supports the Super Game Boy.** `rg -ni "sgb\|super.?game.?boy"` over
`libgambatte/` returns **0 hits** (the only repo hit is a Qt file glob). `gb`: 0 hits. Neither
implements a **link cable** either — gambatte shifts in all-1 bits from a disconnected port
(`memory.cpp:151-165`), `gb` sets `data = 0xFF` on completion (`src/serial.rs:61-76`). Same
behaviour; gambatte's is cycle-shaped.

> If you want the features in the "No" rows above, the reference is a newer fork
> (`pokemon-speedrunning/gambatte-core`). Everything below is verified against real code in *this*
> tree.

---

## 1. Hardware models

| Feature | gambatte | gb | Evidence |
|---|---|---|---|
| DMG | ✅ | ✅ | `initstate.cpp:1085` / `game_boy.rs:11`, `core.rs:31` |
| CGB | ✅ | ❌ | `isCgb()` branches throughout / `rg -ni "cgb"` over `src/` minus pokemon+sdl → **12 hits, all `header.rs` parse-only + 2 TODO comments** |
| CGB-in-DMG-compat | ⚠️ modelled as "CGB flag off ⇒ DMG paths" (`cartridge.cpp:635`) | ❌ | — |
| `FORCE_DMG` flag | ✅ `gambatte.h:38` | ❌ | — |
| GBA-vs-CGB difference | ⚠️ **one bit** — `state.cpu.b = cgb & gbaCgbMode;` (`initstate.cpp:1175`) is the entire model | ❌ | — |
| **SGB** | ❌ | ❌ | grep-verified both trees |
| `isCgb()` public query | ✅ `gambatte.h:108` | ❌ | `gb` parses `CGBMode` (`header.rs:36-40`) but `rg -n "cgb_mode()"` outside `header.rs` → **0 hits — the parsed value is never consumed** |

**`gb` is DMG-only.** It parses the CGB header byte and throws the answer away.

> **How gambatte decides "am I CGB?"** — one line at load
> (`cartridge.cpp:635`: `cgb = header[0x0143] >> 7 & (1 ^ forceDmg);`), then the answer is *encoded
> in the WRAM allocation* rather than stored: `memptrs_.reset(rombanks, rambanks, cgb ? 8 : 2)` and
> `memptrs.h:100-105` derives `isCgb()` from `wramdataend - wramdata(0) == 8 * 0x1000`. It costs a
> pointer subtraction and survives savestate round-trips for free. `isCgb()` is then a **branch
> predicate in ~30 places**, not a separate code path — that is the shape any retrofit into `gb`
> would have to take.

---

## 2. CGB feature checklist — `gb` implements 0 of 20

| CGB feature | gambatte | gb |
|---|---|---|
| KEY1 (`FF4D`) prepare bit | ✅ `memory.cpp:991-995` | ❌ `mmu.rs:333` unmapped → `_ => 0xFF` |
| Actual speed switch on STOP | ✅ `Memory::stop()` `memory.cpp:390-441` | ❌ `ppu.rs:202` literally reads `// TODO the PPU is twice as slow in CGB double speed mode` |
| Double-speed propagated to PPU/APU/timer | ✅ `isDoubleSpeed()` ~40× in `memory.cpp` alone | ❌ |
| VRAM banking (`FF4F`) | ✅ `memory.cpp:996-1002`, `max_num_vrambanks=2` | ❌ `ppu.rs:15` `vram: [u8; 0x2000]` — one bank |
| WRAM banking (`FF70`) | ✅ `memory.cpp:1074-1080`; 8 banks | ❌ `mmu.rs:29` `work_ram: [u8; 0x2000] // DMG mode only` |
| CGB BG palette RAM (`FF68/69`) | ✅ `memory.cpp:1041-1054` | ❌ `lcd_palette.rs` is 2-bit `DMGColor` only |
| CGB OBJ palette RAM (`FF6A/6B`) | ✅ `memory.cpp:1055-1068` | ❌ |
| CGB palette access *timing* | ✅ `video.h:88-94 cgbpAccessible(cc)` | n/a |
| BG map attributes (bank-1) | ✅ `video/ppu.cpp:101`, used at `:228,241,275,284,297,605,617,623` | ❌ tile maps read as flat bytes, `ppu.rs:315` |
| CGB BG-vs-OBJ master priority | ✅ `ppu.cpp:667,684` | ❌ `lcd_control.rs:13` comments the bit "not for CGB" |
| HDMA (`FF51-55`, HBlank) | ✅ `memory.cpp:1003-1035`, `haltHdmaState_` | ❌ only DMG OAM DMA (`lcd_dma.rs`, 48 lines) |
| GDMA | ✅ `memory.cpp:1031` | ❌ |
| HDMA × STOP/speed-switch | ✅ `memory.cpp:399-403` | n/a |
| OPRI (`FF6C`) | ⚠️ register only | ❌ |
| `FF72/73/74` undocumented | ✅ `memory.cpp:1081-1087` | ❌ |
| `FF75` (bits 4-6) | ✅ `data \| 0x8F` `memory.cpp:1088-1092` | ❌ |
| PCM12/PCM34 | ❌ | ❌ *(parity)* |
| CGB serial 32× clock | ✅ `memory.cpp:685-691` | ❌ `serial.rs:64` one fixed constant |
| CGB OAM-DMA conflict map | ✅ templated `OamDmaConflictMap<src, cgb>` `memptrs.cpp:28-32` | ❌ atomic 0xA0-byte copy, `mmu.rs:221-227` |
| CGB STAT-write IRQ quirk | ✅ `memory.cpp:947-950` | ❌ |
| CGB vs DMG APU power-off writes | ✅ `memory.cpp:729-734,763-768` | ❌ |
| CGB vs DMG post-boot RAM/regs | ✅ two full dumps, `initstate.cpp:30-707` / `:709-979` | ❌ |

The whole `FF4D`–`FF77` range is absent from `mmu.rs`'s match arms and falls into the catch-alls at
`mmu.rs:333-336` and `:388-390`.

**Recommendation: do not do this.** 20 features, a colour-type rewrite of the pixel pipeline
(`DMGColor` is 2-bit throughout), and **zero value for a DMG-only Pokémon Red agent**. Revisit only
if the target ROM changes. See [`03-ppu.md` §8](03-ppu.md#8-cgb-video-inventory) and
[`05-mmu-cartridge.md` §6](05-mmu-cartridge.md#6-cgb-only-memory-features) for the sequencing if it
ever becomes real.

---

## 3. Product features in the public API

Verdict column = does it matter for **a headless core driven by an LLM agent reading RAM and
synthesising joypad input**?

| Feature | gambatte | gb | Verdict |
|---|---|---|---|
| ROM load with typed error | ✅ `load()` → `LoadRes` (`loadres.h:8-19`) | ⚠️ `MMU::from_rom` returns `Result<_,String>` but the only public ctor **`.expect()`s it** (`core.rs:34`) | **Matters** — a stringly error `.expect()`ed one layer up is a crash |
| Unsupported-mapper reporting | ✅ 8 `LOADRES_UNSUPPORTED_MBC_*` | ⚠️ `CartType` parsed but **never dispatched on** | Matters if anything but Pokémon Red is loaded |
| `romTitle()` | ✅ | ✅ `header.rs:97` | already have it |
| `pakInfo()` + **header-checksum OK** | ✅ `pakinfo.cpp:9-15` | ⚠️ bank counts yes; **no checksum validation** | **Worth having** as a load-time gate |
| `setInputGetter` (pull, sampled at the exact cycle P1 is read) | ✅ `memory.cpp:563` | ❌ push: caller mutates `JoypadRegister` (`joypad.rs:44`) | **Design difference, not a gap.** Push is arguably better for a scripted agent |
| `setSaveDir` + implicit battery saves | ✅ written from `~GB()` (`gambatte.cpp:55-60`) | ⚠️ explicit `dump_sram_to_file` / `restore_sram_from_file` | **`gb`'s explicit model is better** — destructor-time writes are a test-harness hazard |
| Battery save format | raw SRAM dump; short files silently tolerated | raw concatenated banks; **rejects size mismatch** (`mmu.rs:158-159`) | Compatible; **`gb` stricter = better** |
| RTC (MBC3 timer) | ✅ `mem/rtc.cpp` | ❌ `rg -ni "\brtc\b"` over core → **0 hits** | **Doesn't matter** for Pokémon Red (cart `0x13` = MBC3+RAM+**no** timer). Matters for G/S/C |
| RTC divisor/desync handling | ❌ (wall-clock `std::time(0)`) | ❌ | n/a — but note wall-clock RTC makes savestates **non-deterministic** across replays |
| **Savestate versioning / forward-compat** | ✅ **label-keyed TLV**, unknown labels skipped (`statesaver.cpp:417-445`) | ❌ positional bincode, **no version tag** | **Biggest product gap** — see [`01-architecture.md` §6](01-architecture.md#6-save-states-and-the-fixture-freeze) |
| Savestate excludes ROM | ❌ | ✅ (`mmu.rs:397`) | **`gb` better** |
| Savestate validates ROM identity | ❌ loads a state for a different ROM happily | ✅ `game_boy.rs:78-80` | **`gb` better** |
| Compressed savestates | ❌ | ✅ lz4 | **`gb` better** |
| **`reset()`** | ✅ deterministic, battery-preserving (`gambatte.cpp:79-89`) | ❌ **`Core::reset()` is `todo!()`** (`core.rs:41-43`); `GameBoy::reset()` **panics unconditionally** | **Matters a lot** — no in-process recovery for a wedged agent |
| Video buffer + pitch; **null buffer legal** | ✅ `videoBuf==0` is cheap (`ppu.h:41-42`) | n/a — always renders into an owned buffer | `gb` **pays per-pixel cost even in tests that never look at the screen** (measured: 26.2× → 38.5×) |
| `runFor` returns **on frame completion** | ✅ `gambatte.cpp:62-77` | ❌ `run(min_cycles)`, no frame signal | **Matters** — `AGENT_RESOLUTION` = 20 ms is tuned *because* of this gap |
| Resampling location | frontend | **core** (`src/audio/blip/`) | Inverted; deliberate and documented. Not a gap |
| Screenshot export from the core | ❌ | ✅ `game_boy.rs:94-97` | **`gb` ahead** |
| Serial byte tap for tests | ❌ | ✅ `Serial::enable_buffer` | **`gb` ahead** |
| OSD text overlay | ✅ `bitmap_font.{h,cpp}` | ❌ | **Pure frontend fluff** for a headless core |
| Game Genie / Game Shark | ✅ `cartridge.cpp:723-763` | ❌ | Redundant — `gb` pokes RAM from Rust directly |
| Multicart (MBC1-M) heuristic | ✅ `presumedMulti64Mbc1()` | ❌ | Fluff |
| ZIP ROM loading | ✅ vendored `unzip/` | ❌ | Fluff (`include_bytes!`) |
| Savestate slots + thumbnail | ✅ | ❌ | Fluff |
| Bundled HW test suite | ✅ **3524 `.asm` / 66 dirs** | ⚠️ blargg + dmg-acid2 | See [`07-testing.md`](07-testing.md) |

---

## 4. Robustness

### 4.1 gambatte's failure modes

| Situation | Behaviour | Evidence |
|---|---|---|
| ROM missing/unreadable | `LOADRES_IO_ERROR`, nothing mutated | `cartridge.cpp:564-566` |
| Short ROM (<0x150) | short-read; `rombanks = max(pow2ceil(size/0x4000), 2)` guarantees ≥2 banks even for 0 bytes | `cartridge.cpp:580-655` |
| **ROM shorter than the header claims** | **Non-issue by construction** — header byte `0x148` is *ignored*, bank count derived from actual file size, tail **zero-padded with `0xFF`** | `cartridge.cpp:638-652` |
| Unsupported mapper | Typed negative `LoadRes`, no partial state | `cartridge.cpp:594-613` |
| RAM size disagrees with mapper | Normalised by table; explicit `default:` → 4 banks | `pakinfo.cpp:19-28` |
| Mismatched/truncated savestate | `false` on bad version; unknown labels skipped; truncation exits with an internally-consistent state | `statesaver.cpp:417-452` |
| Savestate for a **different ROM** | **Not checked** — loads happily | `gambatte.cpp:138-152` |
| Wild guest bank number | Masked: `bank & (rombanks-1)` | `cartridge.cpp:148` |
| **In-emulation aborts** | **None.** `rg "assert\|abort\|throw"` over the four core `.cpp` files → 0 hits. OOB is UB in C++, but the 4-bit-area pointer tables make it unreachable by construction | — |

### 4.2 gb's failure modes

| Situation | Behaviour | Evidence |
|---|---|---|
| Short ROM (<0x14A) | Clean `Err(String)` — every field is `data.get(..)` + `ok_or` | `header.rs:53-81` — **best-behaved parsing in the crate** |
| ROM-size byte `0x52/53/54` (72/80/96 banks) | `Err("Invalid ROM size")` — **rejects three legal values** | `header.rs:71-79` |
| …but the caller discards all of it | `.expect("could not load ROM")` | **`core.rs:34`** |
| **ROM file shorter than `header.rom_banks()*16 KB`** | **Index panic** on the first banked read; `gb` never pads | **`mmu.rs:291,297`** |
| Savestate schema drift | `DecodeError` → `Err` (good), *or* silent garbage if lengths align (bad) | `game_boy.rs:72-86` |
| Savestate from a different ROM | Detected & rejected | `game_boy.rs:78-80` — **better than gambatte** |
| SRAM size mismatch | Detected & rejected | `mmu.rs:157-166` — **better than gambatte** |
| Illegal opcode | `CoreMode::Crash`, no panic | `core.rs:474` — **good design** (though it wrongly stops the whole machine, see [`02-cpu.md` §2b](02-cpu.md#2b-illegal-opcodes-freeze-the-whole-machine)) |
| `reset()` | **`todo!()` — unconditional panic** | **`core.rs:42`** |
| Every `MMU::from_rom` | **`println!("{:?}", header)` on stdout** | `mmu.rs:45` — noise in a headless harness |

### 4.3 The actual abort sites in `gb`'s core

Counts from a ripgrep sweep of `src/*.rs` + `src/audio/**` + `src/roms/**`, excluding
`#[cfg(test)]`. (Note `src/audio/reference.rs` and `src/audio/wav.rs` are *entire* modules gated
`#[cfg(test)]` at `audio/mod.rs:29-32`; `audio/blip/tests.rs` is wholly tests.)

**`.unwrap()` — 16 total, 0 in production.** All test-only. Correct non-panicking variants already
exist at `ppu.rs:189,501` and `lcd_palette.rs:47-50`. *(But `unwrap_or_else` at `mmu.rs:92,94`
**does** panic — see below.)*

**`.expect(` — 14 total, 6 in production:**

- **`core.rs:34`** — `MMU::from_rom(cart).expect("could not load ROM")`. **The single most reachable
  one.**
- `opcode.rs:22,37,56,75,94` — `from_repr(..).expect("Invalid …")`. Provably unreachable (callers
  mask `&0b111`/`&0b11`, `opcode.rs:786-808`) but the safety argument lives three frames away.
- Borderline: `roms/mod.rs:77` `parse_png` — compiled into non-test builds, only test callers.

**Hard panics that fire in release — 33 outside test modules:**

- **`core.rs:42`** — `todo!()` in `Core::reset()`, reached via `game_boy.rs:39`
- `mmu.rs:92,94` — `panic!("ROM slice out of bounds: …")`
- `mmu.rs:102` — `panic!("Pointer {} is not a ROM pointer")`
- `mmu.rs:108` — `assert!(pointer < ROM_BANK_SIZE …)`
- `mmu.rs:118` — `panic!("Pointer {:04X} is invalid for bank {}")`
- `opcode.rs:632` — `_ => unreachable!("Machine cycles not defined for opcode: {:?}")`. All 71
  variants are covered today, but it is a **catch-all guarding an exhaustive-by-convention match**:
  adding a variant converts a compile error into a runtime abort. Same shape at `opcode.rs:726,740,
  751,754,772,777`
- `audio/square_channel.rs:225`, `audio/master_volume.rs:65` — `unreachable!()` with **non-local**
  guards
- `audio/timer.rs:21` — `assert_eq!` const-sanity check evaluated on **every** `PhaseTimer::new`
- `audio/blip/buffer.rs:61,78`; `audio/blip/mod.rs:128` (`set_speed`, called per frame from the
  frontend); `audio/blip/synth.rs:48,172,221` (`:221` is deliberately hard — the index derives from
  emulated clock time, so a long frame without `end_frame` aborts)

**`debug_assert` — 11, release-invisible:** `core.rs:516` (tautological), `divider.rs:71`,
`ppu.rs:351,355,471,476,486,491,498,520`, `audio/blip/buffer.rs:174`. ⚠️ **The `ppu.rs` ones are the
*only* bounds guard on the indexing that follows.**

### 4.4 Three genuine, guest-reachable hazards

**HAZARD 1 (HIGH) — ROM bank index vs actual ROM length.** `mmu.rs:291,297`.

`rom_bank_register` is clamped against **`header.rom_banks()` (cart byte `0x0148`)**, not
`self.data.len()` (`mmu.rs:80-85`), and `MMU::from_rom` never cross-checks. A ROM claiming 64 banks
but 32 KB on disk → **hard index panic on the first high-bank read**. A 400-byte file with a valid
header passes `from_rom` then panics on the first fetch past `0x4000`.

Also: `MMU::decode` sets `data: vec![]` (`mmu.rs:420,442`); any read before `set_data` panics. The
window is narrow (`game_boy.rs:82-84`) but the invariant is unenforced.

- [ ] Do what gambatte does at `cartridge.cpp:638-652`: **ignore byte `0x148`, derive the bank count
      from `data.len()`, pad to a power of two with `0xFF`**, and clamp `rom_bank_register` against
      the padded buffer. This removes the only guest-reachable hard abort in the memory hot path.

**HAZARD 2 (HIGH) — sprite-height mismatch between OAM scan and draw.** `ppu.rs:409-410`.

`scanline_sprites` is filtered at OAM time using `object_size()` *then* (`ppu.rs:210-217`);
`sprite_pixel` re-reads it at draw time. A guest LCDC write (`mmu.rs:376`) between the two flips
8×16 → 8×8, so `sprite_y` reaches 8..15 with `object_size == Single`:

- non-flipped path → `Tile::pixel` → `self.0[y*2+1]` (`ppu.rs:493`), index up to **31 on a 16-byte
  slice → OOB panic in release**
- flipped path → `8-1-15` on `usize` → subtract-overflow in debug

The only guard is the **release-elided** `debug_assert!` at `ppu.rs:491`.

- [ ] Carry the height captured at OAM-scan time through to draw, or clamp.

**HAZARD 3 (MEDIUM) — unvalidated caller lengths.** `mmu.rs:127,140`
(`read_vram_slice`/`read_wram_slice`) validate the **base address** and return `Result`, but
`length` is unchecked — `offset+length` walks past the `0x2000` array and panics. Same shape at
`ram.rs:23` and `ppu.rs:358`.

- [ ] Validate `offset + length` in both slice accessors.

### 4.5 Overflow arithmetic

`core.rs` is in good shape — 16 `wrapping_*`/`saturating_*`/`checked_*` calls, and `core.rs:557,568`
widen to `u16` first.

Latent-but-currently-safe: `audio/timer.rs:36` `self.period = 2048 - value` (a raw `u16` subtraction
on a guest period; safe only by a **four-call-site** 11-bit-mask invariant with no local assertion);
`audio/timer.rs:54` `self.counter -= 1`; `ppu.rs:53` `y - window_position.y` (guarded by an
*equality* test in a different function); `mmu.rs:88,90`.

**`Cargo.toml`:** the only profile block is `[profile.release] lto="thin", codegen-units=1`. **No
`panic = "abort"`** (panics unwind; nothing uses `catch_unwind`). **No `overflow-checks` override**
→ debug/test builds panic on overflow, release wraps. Since the project documents `--release` as the
normal path for every Pokémon suite, **the overflow-checked configuration is the less-exercised
one** for long runs.

---

## 5. Recommendations, ranked for a headless agent-driven core

1. [ ] **Make `reset()` real** (`core.rs:42`). Copy gambatte's shape (`gambatte.cpp:79-89`):
       re-apply the init state, preserve SRAM, guarantee `reset()` ≡ fresh boot. Today it is a
       landmine, and a long-running agent has **no in-process recovery from a wedged game**.
2. [ ] **Reconcile ROM length with the header** (HAZARD 1). Removes the only guest-reachable hard
       abort in the memory hot path.
3. [ ] **Adopt a versioned savestate format** — see
       [`01-architecture.md` §6.1](01-architecture.md#61-migration-plan--five-steps-each-shippable).
       This directly retires the `CLAUDE.md` rule "Nothing may be added to `Audio`'s serialised
       fields" and unblocks every future core change without regenerating 91 fixtures.
4. [ ] **Fix the sprite-height OOB** (HAZARD 2) — a genuine release-build panic reachable by a
       mid-scanline LCDC write.
5. [ ] **Stop discarding the load error** (`core.rs:34`); add
       `GameBoy::try_dmg(&[u8]) -> Result<Self, LoadError>`. Add gambatte's header-checksum check
       (`pakinfo.cpp:9-15`) as a load-time warning, and accept ROM-size bytes `0x52/53/54`
       (`header.rs:71-79`).
6. [ ] **Add a `run_until_vblank()` / frame-boundary primitive** (mirroring `gambatte.cpp:62-77`).
       `AGENT_RESOLUTION` = 20 ms is tuned around this gap; `ppu.rs:458` already tracks the VBlank
       edge, so the plumbing is a few lines.
7. [ ] **Delete the `println!`s** from `MMU::from_rom` (`mmu.rs:45`) and the illegal-opcode path
       (`core.rs:473`) — a library should not write to stdout.
8. [ ] **Initialise SRAM to `0xFF`** (`initstate.cpp:1186`) — what erased battery SRAM actually
       reads.
9. [ ] **Optional headless perf:** allow skipping the pixel blit. gambatte's null-`videoBuf` path is
       free accuracy-preserving speed; `gb` writes all 23 040 pixels even in tests that never call
       `screenshot()`. **Measured 26.2× → 38.5×.**
10. [ ] **If a second cartridge is ever loaded, dispatch on `CartType`** — see
        [`05-mmu-cartridge.md` §1](05-mmu-cartridge.md#1-mbc-support-matrix). Not urgent while the
        target is Pokémon Red alone.

**Explicitly not recommended:** OSD overlay, savestate slots + thumbnails, ZIP loading, multicart
heuristics, Game Genie/Shark, RTC (Pokémon Red has none), `setInputGetter` (`gb`'s push model is
better here), and **CGB support** (§2).

---

## References

- gambatte public API — `libgambatte/include/gambatte.h`
- gambatte load/robustness — `libgambatte/src/mem/cartridge.cpp`, `loadres.cpp`, `statesaver.cpp`
- gambatte post-boot state — `libgambatte/src/initstate.cpp`
- Pan Docs, Power-Up Sequence — <https://gbdev.io/pandocs/Power_Up_Sequence.html>
