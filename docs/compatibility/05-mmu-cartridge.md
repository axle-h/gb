# Memory map / cartridge / I/O compatibility guide

`gb` (`/home/alex/projects/gb` — `src/mmu.rs`, `src/header.rs`, `src/lcd_dma.rs`, `src/joypad.rs`,
`src/serial.rs`) vs **gambatte** (`libgambatte/src/memory.cpp`, `mem/cartridge.cpp`,
`mem/memptrs.cpp`, `mem/rtc.cpp`, `mem/pakinfo.cpp`, `initstate.cpp`).

---

## Ranked gap summary

| # | Gap | Severity | Section |
|---|---|---|---|
| 1 | **No MBC abstraction at all.** One hardcoded pseudo-mapper (MBC1's register layout with MBC3's 7-bit width) for every cartridge | Critical | [§1](#1-mbc-support-matrix) |
| 2 | **`0x6000-0x7FFF` writes silently dropped** — MBC1 mode-select and MBC3 RTC latch are both no-ops | Critical | [§1](#1-mbc-support-matrix) |
| 3 | **Bank numbers clamped, not masked** — out-of-range banks alias to the top bank instead of wrapping; ROM is never padded, so a short file **panics** | High | [§1](#1-mbc-support-matrix) |
| 4 | **OAM DMA source mask `& 0xDF` mis-maps `0xA0`→`0x80`** — any game DMAing from SRAM reads VRAM | High | [§5](#5-oam-dma) |
| 5 | **The DMA access gate is inverted** — VRAM/OAM become *more* accessible to the CPU during DMA | High | [§5](#5-oam-dma) |
| 6 | **No RTC whatsoever** | Medium | [§3](#3-save-data-sram--rtc) |
| 7 | **Seven missing I/O read-back masks** (STAT.7, IF, TAC, SC, P1, IE, `FF46`) | Medium | [§4c](#4c-undefined-io-and-read-back-masks) |
| 8 | **`0xFEA0-0xFEFF` reads `0xFF`**; DMG hardware returns `0x00` | Medium | [§4b](#4b-the-unusable-region-0xfea0-0xfeff) |
| 9 | **Boot state diverges** — F, LCDC, BGP/OBP, DIV, and all of WRAM/HRAM/OAM/VRAM | Medium | [§7](#7-boot-state) |
| 10 | **Header parsing rejects valid cartridges** (non-UTF-8 titles, ROM-size bytes `0x52/53/54`) | Medium | [§2](#2-cartridge-header-parsing) |
| 11 | **Serial is send-only** — SB never shifts in `1`s progressively; no DIV alignment | Low-Med | [§8b](#8b-serial) |
| 12 | **Joypad**: P1 bits 6-7 read 0; IRQ is edge-on-press, not edge-on-register-nibble | Low | [§8a](#8a-joypad) |
| 13 | **No CGB memory features** (SVBK/VBK/KEY1/HDMA/FF72-77) | Low (scope) | [§6](#6-cgb-only-memory-features) |

### What `gb` does *better* than gambatte

- Savestates **exclude the ROM** and are lz4-compressed (`src/mmu.rs:397`,
  `src/game_boy.rs:60-65`); gambatte has neither.
- Savestates **validate the header matches** (`src/game_boy.rs:78-80`); gambatte loads a savestate
  for a completely different ROM without complaint.
- `restore_sram` **rejects a size mismatch** (`src/mmu.rs:157-166`); gambatte silently tolerates a
  short file.
- **Explicit** save/load rather than gambatte's destructor-time writes (`gambatte.cpp:55-60`) — much
  safer for a test harness.
- `Serial::enable_buffer` (`src/serial.rs:29-35`) is a genuinely useful test tap that gambatte has
  no equivalent of.
- The `.sav` layout is **byte-compatible** with gambatte's / VBA / BGB / mGBA.

---

## 1. MBC support matrix

### How gambatte does it

Selection happens once, at load, from header byte `0x147`
(`mem/cartridge.cpp:583-616` classifies, `:659-674` constructs):

```cpp
	case type_mbc1:
		if (multicartCompat && presumedMulti64Mbc1(memptrs_.romdata(), rombanks)) {
			mbc_.reset(new Mbc1Multi64(memptrs_));
		} else
			mbc_.reset(new Mbc1(memptrs_));
		break;
	case type_mbc2: mbc_.reset(new Mbc2(memptrs_)); break;
	case type_mbc3:
		mbc_.reset(new Mbc3(memptrs_, hasRtc(memptrs_.romdata()[0x147]) ? &rtc_ : 0));
		break;
	case type_mbc5: mbc_.reset(new Mbc5(memptrs_)); break;
	case type_huc1: mbc_.reset(new HuC1(memptrs_)); break;
```

Unsupported mappers are **rejected at load** rather than mis-emulated (`cartridge.cpp:592-615`):
MMM01, MBC4, MBC6, MBC7, Pocket Camera, TAMA5, and — note — **HuC3**. Only HuC1 (`0xFF`) is
supported. Error strings in `loadres.cpp:5-20`.

Address decode is uniform: `switch (p >> 13 & 3)`. RAM enable is `(data & 0xF) == 0xA` everywhere —
the low *nibble*, so `0x1A`/`0xCA` also enable.

Bank counts come from the allocated buffer (`cartridge.cpp:72-78`);
`rombanks = std::max(pow2ceil(filesize / rombank_size()), 2u)` (`:639`) with `0xFF` tail padding
(`:649-651`), so `& (rombanks - 1)` is always a valid AND-mask and **out-of-range banks wrap**.

**MBC1** (`cartridge.cpp:91-116`, `141-148`):

```cpp
		case 1:
			rombank_ = rambankMode_ ? data & 0x1F : (rombank_ & 0x60) | (data & 0x1F);
			setRombank();
			break;
		case 2:
			if (rambankMode_) { rambank_ = data & 3; setRambank(); }
			else { rombank_ = (data << 5 & 0x60) | (rombank_ & 0x1F); setRombank(); }
			break;
		case 3:
			rambankMode_ = data & 1;
			break;
```

```cpp
	static unsigned adjustedRombank(unsigned bank) { return bank & 0x1F ? bank : bank | 1; }
```

`adjustedRombank` *is* the `0x00/0x20/0x40/0x60` aliasing: the low 5 bits are tested, so
`0x20`→`0x21`, **not** `0x01`.

**MBC2** — the A8 register select (`:231-242`):

```cpp
		switch (p & 0x6100) {
		case 0x0000:
			enableRam_ = (data & 0xF) == 0xA;
			memptrs_.setRambank(enableRam_ ? MemPtrs::read_en | MemPtrs::write_en : 0, 0);
			break;
		case 0x2100:
			rombank_ = data & 0xF;
			memptrs_.setRombank(rombank_ & (rombanks(memptrs_) - 1));
			break;
		}
```

`p & 0x6100` masks A14/A13 **and A8**: RAM-enable needs A8 clear, ROM-bank needs A8 set;
wrong-A8 writes do nothing.

**MBC3 + RTC** (`:273-331`):

```cpp
		case 1: rombank_ = data & 0x7F; setRombank(); break;
		case 2: rambank_ = data;        setRambank(); break;   // unmasked: 0x08-0x0C = RTC
		case 3: if (rtc_) rtc_->latch(data); break;
```

Note MBC3's bank-0 fixup is `std::max(rombank_ & (rombanks - 1), 1u)` — a genuine 0→1, applied
*after* masking, unlike MBC1's low-5-bits rule.

**MBC5** (`:411-429`) — 9-bit bank split by sub-window:

```cpp
		case 1:
			rombank_ = p < 0x3000
			         ? (rombank_  & 0x100) |  data
			         : (data << 8 & 0x100) | (rombank_ & 0xFF);
			setRombank();
			break;
		case 2: rambank_ = data & 0xF; setRambank(); break;
```

Bank 0 is *not* remapped (correct for MBC5). The rumble bit is not modelled by gambatte either.

Application is via `MemPtrs`, a 16-entry pointer table indexed by `p >> 12`, so `Memory::read` is
one predicated indirection (`memory.h:76-78`). A null entry means "needs logic". Disabled SRAM
reads come from a page pre-filled with `0xFF` and writes go to a scratch page — branch-free.

### How gb does it

One `match` (`src/mmu.rs:342-355`):

```rust
            0x0000..=0x1FFF => {
                self.ram_enabled = value & 0xF == 0xA;
            }
            0x2000..=0x3FFF if self.header.rom_banks() > 2 => {
                self.set_rom_bank_register(value as usize);
            }
            0x4000..=0x5FFF if self.header.ram_banks() > 0 => {
                self.ram_bank_register = ((value & 0x03) as usize).min(self.header.ram_banks() - 1);
            }
```

```rust
// src/mmu.rs:80-85
    pub fn set_rom_bank_register(&mut self, value: usize) {
        // TODO MBC1 should mask to 0x1F
        self.rom_bank_register = (value & 0x7F)
            .min(self.header.rom_banks() - 1)
            .max(1);
    }
```

**`0x6000-0x7FFF` is absent from the match** → falls to `_ => {}` (`src/mmu.rs:388-390`).
Verified: `grep -n "0x6000" src/mmu.rs` returns nothing.

`CartHeader::cart_type` is parsed at `src/header.rs:67` and a repo-wide grep shows it is **never
read by anything that changes behaviour**.

### Matrix

| Feature | gambatte | gb | gb ref |
|---|---|---|---|
| Mapper selected from `0x147` | ✅ `cartridge.cpp:583-674` | ❌ never | `header.rs:67` parses only |
| ROM-only (`0x00/08/09`) | ✅ `Mbc0` | ⚠️ by accident (`rom_banks()>2` guard) | `mmu.rs:348` |
| MBC1 basic bank (5-bit) | ✅ `:98` | ⚠️ 7-bit mask | `mmu.rs:82` |
| MBC1 `0x20/40/60` → `+1` | ✅ `adjustedRombank` `:141` | ❌ `.max(1)` maps 0→1 only | `mmu.rs:84` |
| MBC1 bank-2 register | ✅ `:106` | ❌ arm is RAM-only, gated on RAM | `mmu.rs:352-355` |
| MBC1 mode select | ✅ `:111-114` | ❌ **dropped** | `mmu.rs:388` |
| MBC1 mode-1 RAM banking | ✅ `:102-104` | ❌ | `mmu.rs:354` |
| MBC1 multicart | ✅ `Mbc1Multi64` | ❌ | — |
| MBC2 A8 select | ✅ `p & 0x6100` `:232` | ❌ | — |
| MBC2 4-bit bank | ✅ | ❌ | — |
| MBC2 512×4 RAM | ⚠️ 1 full bank | ❌ 0 banks | `header.rs:85` |
| MBC3 7-bit bank | ✅ `:280` | ✅ *(the only mapper gb has)* | `mmu.rs:82` |
| MBC3 bank 0→1 | ✅ `max` after mask | ⚠️ `.max(1)` before clamp | `mmu.rs:84` |
| MBC3 RTC reg select `0x08-0x0C` | ✅ `rtc.h:45-52` | ❌ | — |
| MBC3 RTC latch `0→1` | ✅ `rtc.h:35-40` | ❌ | `mmu.rs:388` |
| MBC5 9-bit bank | ✅ `:418-420` | ❌ one arm | `mmu.rs:348` |
| MBC5 bank 0 allowed | ✅ | ❌ `.max(1)` | `mmu.rs:84` |
| MBC5 4-bit RAM bank | ✅ | ❌ `& 0x03` | `mmu.rs:354` |
| HuC1 | ✅ `:334-399` | ❌ | — |
| HuC3 | ❌ *rejected* | ❌ enum only | `header.rs:31` |
| MMM01/4/6/7/Camera/TAMA5 | ❌ *named error* | ❌ **silently mis-emulated** | `mmu.rs:342` |
| RAM enable `(v&0xF)==0xA` | ✅ | ✅ | `mmu.rs:346` |
| ROM bank masked (wraps) | ✅ | ❌ **clamped** | `mmu.rs:83` |
| RAM bank masked (wraps) | ✅ | ❌ clamped | `mmu.rs:354` |
| ROM padded to pow2 w/ `0xFF` | ✅ `:639-651` | ❌ **OOB panics** | `mmu.rs:297` |
| Disabled SRAM → `0xFF` | ✅ | ✅ | `mmu.rs:335` |

### Symptom / failing tests

mooneye `emulator-only/mbc1/{bits_bank1,bits_bank2,bits_mode,bits_ramg,rom_1Mb…rom_8Mb,ram_64kb,ram_256kb,multicart_rom_8Mb}.gb`;
`mbc2/{bits_ramg,bits_romb,bits_unused,ram,rom_512kb,rom_1Mb,rom_2Mb}.gb`;
`mbc5/rom_512kb.gb`…`rom_64Mb.gb`. gambatte `test/hwtests/sram.asm`.

Commercial: **Super Mario Land**, **Tetris**, **Kirby's Dream Land** (MBC1) mis-bank on
`0x20/40/60`; **Final Fantasy Legend / SaGa** (MBC2) won't boot past the save check; **Pokémon
G/S/C**, **Harvest Moon GB** lose the clock; **Pokémon Yellow** and most GBC titles (MBC5) break
above bank 255. Pokémon Red is safe today only because it is MBC3-no-RTC under 128 banks.

### Tasks

- [ ] Introduce a `trait Mbc`:
      `fn rom_write(&mut self, addr: u16, value: u8); fn rom_bank(&self) -> usize; fn ram_target(&self) -> RamTarget`
      with `RamTarget::{Bank(usize), Rtc(RtcReg), None}`. Store `Box<dyn Mbc>` selected by
      `CartType`.
- [ ] Pad the ROM to a power of two with `0xFF` at load; replace every `.min()` with `& (n - 1)`.
- [ ] Port the four `romWrite` bodies verbatim — `p >> 13 & 3` maps directly to
      `match addr >> 13 & 3`.
- [ ] Add a `LoadError::UnsupportedMbc` so unknown cartridges fail loudly rather than being
      mis-emulated.

---

## 2. Cartridge header parsing

**gambatte** reads only `0x150` bytes to classify (`cartridge.cpp:580-636`).

- **ROM size**: the `header[0x0148]` switch is **commented out** (`:618-632`). Size is derived from
  the file. A truncated or over-dumped ROM still loads.
- **RAM size** (`pakinfo.cpp:19-28`) — note the MBC2 case, that `0x01` (2 KiB) rounds **up** to a
  full bank, and that `default:` shares the `0x03` arm, so an unknown byte is 4 banks, **not** an
  error.
- **CGB flag**: `cgb = header[0x0143] >> 7 & (1 ^ forceDmg)` (`:635`) — bit 7 only, so `0x80` and
  `0xC0` are identical.
- **SGB flag** (`0x0146`): not parsed. Neither emulator supports SGB.
- **Header checksum**: computed but purely informational via `PakInfo` (`pakinfo.cpp:9-15`), never
  enforced.
- **Title**: raw pointer, truncated in `GB::romTitle()`. No UTF-8 validation.

**gb** (`src/header.rs:52-95`):

```rust
        let title = std::str::from_utf8(&title_bytes[0..title_length])
            .map_err(|_| "Invalid UTF-8 in title")?.to_string();
        let rom_banks = data.get(0x0148)
            .and_then(|&value| { if value < 0x09 { Some(1 << (value + 1)) } else { None } })
            .ok_or("Invalid ROM size")?;
```

| Aspect | gambatte | gb |
|---|---|---|
| ROM size source | file size, `pow2ceil`, min 2 | header `0x148` only |
| `0x148` = `0x52/53/54` (72/80/96 banks) | irrelevant | **hard error** |
| Truncated / non-pow2 ROM | `0xFF`-padded, loads | **OOB panic** at `mmu.rs:297` |
| Unknown `0x149` | 4 banks | hard error |
| `0x149 = 0x01` (2 KiB) | 1 full bank | **0 banks** → SRAM vanishes |
| MBC2 implied RAM | 1 bank | 0 banks |
| Unknown `0x147` | named `LoadRes` | `String` error |
| Header checksum | computed, exposed | not computed |
| Title | 15/16 raw bytes | **UTF-8-validated → rejects valid carts** |

Two latent bugs rather than accuracy nits:

1. `src/header.rs:57` errors on non-UTF-8 titles. Real headers put the manufacturer code and CGB
   flag *inside* `0x13F-0x143`, so bytes like `0x80`/`0xC0` land in the slice whenever there is no
   preceding NUL.
2. `src/header.rs:71-79` rejects `0x148 >= 0x09`.

> Concretely: gambatte's own test ROMs declare `.data@143 80 00 00 00 03 00 01`
> (`fexx_ffxx_dumper.asm:8`) — `0x147 = 0x03` (MBC1+RAM+battery) with `0x149 = 0x00`. **`gb`
> allocates zero RAM banks and drops every SRAM write**, so the dumper's results would be
> unreadable. This blocks reusing gambatte's test suite (see [`07-testing.md`](07-testing.md)).

### Tasks

- [ ] Derive `rom_banks` from `data.len()` (pow2, min 2) and pad the backing `Vec` with `0xFF`.
- [ ] Default unknown `0x149` to 4 banks; add the MBC2 → 1 case.
- [ ] Replace the UTF-8 decode with a byte filter over `0x134..0x143`, truncating at the first byte
      `< 0x20 || >= 0x80`.
- [ ] Map `CartType::from_repr` failure to a `LoadError` enum.

---

## 3. Save data (SRAM + RTC)

**gambatte** detects battery by header byte, not mapper (`cartridge.cpp:499-514`:
`0x03, 0x06, 0x09, 0x0F, 0x10, 0x13, 0x1B, 0x1E, 0xFF`) and RTC likewise (`:516-524`: only `0x0F`,
`0x10`). Two sidecars (`:679-721`):

- `.sav` — a raw contiguous bank dump, no header. Loading is tolerant; short reads leave the tail
  untouched, **no length check**.
- `.rtc` — 4 bytes big-endian: the Unix `time_t` **base time**, not register values.

`Rtc` stores only `baseTime_` and derives S/M/H/D on latch (`rtc.cpp:40-61`), so wall-clock time
passes while the emulator is closed — exactly like hardware. Day overflow folds into `baseTime_`
and sets the carry bit; the halt bit substitutes `haltTime_` for `std::time(0)`; register writes are
*inverse* adjustments of `baseTime_` (`:114-153`). Savestates carry the RTC separately (`:91-112`),
so a savestate is time-consistent while a `.rtc` is wall-clock-consistent.

> Note: gambatte's wall-clock RTC makes savestates **non-deterministic** across replays. For a
> replay-driven agent harness this is a reason to prefer an explicit, injectable time source.

**gb** (`src/mmu.rs:149-166`): layout is byte-compatible with gambatte's `.sav`. Differences: no
battery check; **strict length equality** on restore; **no RTC**; and saving is entirely
caller-driven with nothing on drop/reset, so a crash loses the save.

### Symptom

**Pokémon Gold/Silver/Crystal** — day/night, Bug-Catching Contest, Goldenrod Radio, berry regrowth
and Kurt's Apricorns are all RTC-driven; Crystal shows "the clock has stopped" on a hung RTC. Also
**Harvest Moon GB/GB2**, **Pokémon Pinball**. No public test ROM covers RTC; gambatte's suite has
none either.

### Tasks

- [ ] Port `Rtc` — 155 lines, only needs `<ctime>`. In Rust: `base_time: i64`, `halt_time: i64`,
      five data bytes, `last_latch`, using an **injectable** time source rather than
      `SystemTime::now()` directly, so replays stay deterministic.
- [ ] Add `dump_rtc()` / `restore_rtc()` in gambatte's 4-byte BE format for interop.
- [ ] Add `has_battery` / `has_rtc` derived from `CartType`.
- [ ] Relax `restore_sram` to copy `min(len)` rather than requiring exact equality.

---

## 4. Obscure corners of the memory map

### 4a. Echo RAM `0xE000-0xFDFF` — no gap

gambatte covers `0xE000-0xEFFF` by pointer alias (`memptrs.cpp:108`); `0xF000-0xFDFF` falls to
`return cart_.wramdata(p >> 12 & 1)[p & 0xFFF];` (`memory.cpp:652-653`) — CGB-correct, degenerating
to bank 1 on DMG.

`gb`: `0xE000..=0xFDFF => self.work_ram[(address - 0xE000) as usize]` (`src/mmu.rs:308`, `:363`),
exactly right for DMG, with a unit test at `src/mmu.rs:488-496`. ✅

### 4b. The unusable region `0xFEA0-0xFEFF`

gambatte has no special case — the region is plain `ioamhram_[0xA0..0xFF]`; the behaviour comes from
the init table plus the *write* guard (`memory.cpp:1138-1145`):

```cpp
	} else if (p - mm_hram_begin >= 0x7Fu) {
		long const ffp = static_cast<long>(p) - mm_io_begin;
		if (ffp < 0) {
			if (lcd_.oamWritable(cc) && oamDmaPos_ >= oam_size
					&& (p < mm_oam_begin + oam_size || isCgb())) {
				lcd_.oamChange(cc);
				ioamhram_[p - mm_oam_begin] = data;
			}
		} else
			nontrivial_ff_write(ffp, data, cc);
	}
```

The clause `(p < mm_oam_begin + oam_size || isCgb())` is the entire DMG/CGB split: **DMG is
write-protected** (keeping the boot-ROM zeros), **CGB is ordinary RAM**. Confirmed by
`initstate.cpp:1085-1090` — DMG does `memset(ioamhram + 0x0A0, 0x00, 0x060)` while CGB copies a
`feaxDump[0x60]` table.

The committed hardware dumps settle it:

```
# fexx_ffxx_dumper_dmg08.bin, offsets 0xA0..0xFF  (= 0xFEA0..0xFEFF)
0000a0 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
*
```
```
# fexx_ffxx_dumper_cgb.bin, offsets 0xA0..0xFF
0000a0 08 01 ef de 06 4a cd bd 08 01 ef de 06 4a cd bd
*
0000c0 00 90 f7 7f c0 b1 bc fb 00 90 f7 7f c0 b1 bc fb
*
0000e0 24 13 fd 3a 10 10 ad 45 24 13 fd 3a 10 10 ad 45
*
```

**DMG → `0x00`; CGB → three 8-byte patterns, each repeated 4×.**

`gb` matches no arm in either match, so reads hit `_ => 0xFF` (`src/mmu.rs:333-336`) and writes hit
`_ => {}` (`:388-390`). The write-drop is coincidentally correct; the read is wrong.

- [ ] Add `0xFEA0..=0xFEFF => 0x00` above the read catch-all.

### 4c. Undefined I/O and read-back masks

gambatte applies masks **on write**, so reads are a plain array fetch.

| Addr | Reg | DMG expected | gambatte | gb read | Match? |
|---|---|---|---|---|---|
| `FF00` | P1 | `CF` | init sets 6-7 | max `0x3F` | ❌ missing `0xC0` |
| `FF02` | SC | `7E` | `data \|= 0x7E - isCgb()*2` (`:691`) | bits 7,0 only | ❌ |
| `FF07` | TAC | `F8` | `data \|= 0xF8` (`:713`) | no `0xF8` | ❌ |
| `FF0F` | IF | `E1` | `setIfreg(0xE0 \| data)` (`:718`) | max `0x1F` | ❌ |
| `FF41` | STAT | `80` | bit 7 from init, preserved by `(old & 0x87) \| (data & 0x78)` (`:951`) | bit 7 always 0 | ❌ |
| `FF46` | DMA | last written | `ioamhram_[0x146] = data` (`:966`) | **hard-coded `0`** (`mmu.rs:325`) | ❌ |
| `FFFF` | IE | all 8 bits | `setIereg(data)` | top 3 dropped | ❌ |
| `FF01` SB, `FF42-45`, `FF47-49`, `FF4A-4B` | — | — | plain | plain | ✅ |
| `FF03`, `FF08-0E`, `FF4C`, `FF4D`, `FF4F`, `FF50-7F` | — | `FF` | init `FF` | `_ => 0xFF` | ✅ |

APU masks are covered separately in [`04-apu.md` §9](04-apu.md#9-register-read-back-masks--no-gaps)
— **all correct**.

### Tasks

- [ ] One-liners in the read arms:
      `0xFF41 => 0x80 | …stat()`, `0xFF0F => 0xE0 | …get()`, `0xFF07 => 0xF8 | …control()`,
      `0xFF02 => 0x7E | …control()`, `0xFF00 => 0xC0 | …get()`
- [ ] Store the last written DMA byte for `0xFF46`.
- [ ] Widen `InterruptFlags` for `0xFFFF` to a raw `u8`.

**Failing tests:** mooneye `acceptance/bits/unused_hwio-GS.gb` (canonical), `bits/reg_f.gb`,
`acceptance/boot_hwio-dmgABCmgb.gb`; gambatte `fexx_ffxx_dumper_dmg08.bin`,
`serial/start_wait_read_sc_1_dmg08_outFF.asm`.

---

## 5. OAM DMA

### How gambatte does it

**Trigger** (`memory.cpp:962-968`):

```cpp
	case 0x46:
		lastOamDmaUpdate_ = cc;
		oamDmaStartPos_ = (oamDmaPos_ + 2) & 0xFF;
		intreq_.setEventTime<intevent_oam>(std::min(intreq_.eventTime(intevent_oam), cc + 8));
		ioamhram_[0x146] = data;
		oamDmaInitSetup();
		return;
```

`+ 2` is the two-M-cycle startup delay; because `oamDmaStartPos_` derives from the *current*
`oamDmaPos_`, **restarting mid-transfer falls out for free**.

**Source classification** — this is where `>= 0xE000` and the wrap live (`memory.cpp:516-523`):

```cpp
void Memory::oamDmaInitSetup() {
	if (ioamhram_[0x146] < mm_sram_begin / 0x100) {
		cart_.setOamDmaSrc(ioamhram_[0x146] < mm_vram_begin / 0x100 ? oam_dma_src_rom : oam_dma_src_vram);
	} else if (ioamhram_[0x146] < 0x100 - isCgb() * 0x20) {
		cart_.setOamDmaSrc(ioamhram_[0x146] < mm_wram_begin / 0x100 ? oam_dma_src_sram : oam_dma_src_wram);
	} else
		cart_.setOamDmaSrc(oam_dma_src_invalid);
}
```

`0x100 - isCgb() * 0x20` is `0x100` on DMG, `0xE0` on CGB. So on **DMG, sources `0xE0-0xFF` are
WRAM** (echo), fetched with `& 0xFFF` wrap within the bank. On **CGB, `0xE0-0xFF` is invalid** →
`0xFF`.

**Progressive transfer, 4 T-cycles/byte** (`memory.cpp:492-514`), paused during HALT, entirely
lazy — advanced only when something observes memory.

**Bus conflicts** — `MemPtrs` disconnects whole regions from the pointer table
(`memptrs.cpp:27-45`). Each bit is one 4 KiB page; `0xFCFF` = every page except VRAM. A conflicted
read returns the byte the DMA is moving; a conflicted write corrupts the DMA byte instead of memory.

### How gb does it

```rust
// src/lcd_dma.rs:10-11
    pub fn set(&mut self, value: u8) {
        self.state = Some(LcdDmaState { address: ((value & 0xDF) as u16) << 8, cycles: MachineCycles::ZERO });
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

Duration is right (`DMA_TRANSFER_CYCLES = from_m(160)`, `src/lcd_dma.rs:43`). Everything else is
not.

### Gap — six distinct differences

1. **All-at-once at the end.** 160 bytes appear atomically instead of one per 4 T.
2. **Source is masked, not wrapped.** `(value & 0xDF) << 8` clears bit 5 of the page:
   `0xE0 → 0xC0` and `0xFF → 0xDF` approximate the echo wrap, but it also does `0x20 → 0x00`,
   `0x60 → 0x40`, and critically **`0xA0 → 0x80`, sending an SRAM-sourced DMA to VRAM**.
3. **No bus conflict.** Nothing consults DMA state on a CPU access.
4. **No restart semantics.** `set()` replaces state with `cycles: ZERO`, discarding an in-flight
   transfer.
5. **`0xFF46` reads back `0`.**
6. **The PPU is not told** — sprites render from real OAM mid-DMA.

Plus the **silent-drop bug** and the **inverted access gate** documented in
[`02-cpu.md` §7](02-cpu.md#7-oam-dma-and-bus-conflicts) and
[`03-ppu.md` §7](03-ppu.md#7-vramoam-access-blocking-and-oam-dma).

### Symptom / failing tests

gambatte `test/hwtests/oamdma/` (~60 ROMs whose filenames encode the expectations). The source-wrap
and conflict cases are decisive:

- `oamdma_src0000_busypopDFFF_dmg08_out65766576_cgb04c_out657655AA.asm`
- `oamdma_src0000_busypopFDFF_dmg08_out657665FF_…`
- `oamdma_src0000_busypopFE9F_dmg08_cgb04c_out6576FFFF.asm`
- `oamdma_src0000_busypop7FFF_dmg08_cgb04c_out657665AA.asm`
- `oamdma_src0000_busypop9FFF_dmg08_cgb04c_out65765576.asm`
- `oamdma_busydelay_1_dmg08_cgb04c_out5.asm` (the 2-cycle startup)
- ~30 `late_sp00x_*` / `late_sp39y_*` sprite-visibility ROMs

mooneye `acceptance/oam_dma/{basic,reg_read,sources-GS}.gb`,
`acceptance/{oam_dma_start,oam_dma_restart,oam_dma_timing}.gb`.

### Tasks

- [ ] `LcdDmaState { page: u8, pos: u8, cycles }`; `update` returns bytes-to-move so `MMU::update`
      copies incrementally.
- [ ] Replace `& 0xDF` with a classifier mirroring `oamDmaInitSetup`:
      `0x00-0x7F` ROM, `0x80-0x9F` VRAM, `0xA0-0xBF` SRAM, `0xC0-0xFF` WRAM with `addr & 0x1FFF`
      wrap.
- [ ] Privileged OAM writer; remove the inverted `|| dma.is_active()` gate from all four PPU
      accessors.
- [ ] On `set()` while active, keep the running transfer and set pending start `(pos + 2) & 0xFF`.
- [ ] Store the byte for `0xFF46` read-back.
- [ ] (Optional) The conflict mask — a 5-entry table and one `if`.

---

## 6. CGB-only memory features

**gambatte.** WRAM allocation is `cgb ? 8 : 2` banks (`cartridge.cpp:644`) and CGB-ness is
*inferred from that count* (`memptrs.h:103-107`) — it costs a pointer subtraction and survives
savestate round-trips for free.

- **SVBK** (`memory.cpp:1074-1079`): `cart_.setWrambank(data & 0x07 ? data & 0x07 : 1);` —
  bank 0 selects bank 1.
- **VBK** (`:996-1002`): `cart_.setVrambank(data & 1); ioamhram_[0x14F] = 0xFE | data;`
- **KEY1** (`:991-995`): only bit 0 writable; the switch happens in `Memory::stop` (`:390-441`).
- **HDMA/GDMA** registers at `:1003-1035`, engine at `Memory::dma` (`:278-342`): `0x10`-byte chunks
  for HDMA vs full length for GDMA, dest wrap sets the done bit, source reads return `0xFF` for
  VRAM and `>= 0xFE00`, and — the subtle one — **HDMA and OAM DMA interleave** (`:306-319`).
- `FF72/73/74` plain RW on CGB only; `FF75` is `data | 0x8F`; `FF76/77` read `0x00`.

**gb.** Nothing. A grep for `CGB|KEY1|SVBK|VBK|HDMA|double_speed` across `src/` finds only the
`CGBMode` enum in `src/header.rs:36-64` (read *only* by its own tests), a comment in
`src/lcd_control.rs:13`, and a TODO at `src/ppu.rs:202`. `MMU` has a single
`work_ram: [u8; 0x2000]` (`src/mmu.rs:29`); the PPU has one `[0; 0x2000]` VRAM.

> **This is a deliberate scope choice** — Pokémon Red is DMG and `pokered.gbc` runs in DMG mode.
> Rank it last. If ever wanted, the order that minimises churn is: SVBK (make `work_ram` a
> `Vec<[u8; 0x1000]>` + index) → VBK → KEY1 + a speed multiplier threaded through `MachineCycles`
> → HDMA last, since it entangles with PPU mode timing.

---

## 7. Boot state

**Neither emulator runs a boot ROM.** This gambatte tree has *no* BIOS support at all — a
case-insensitive grep for `bios|bootrom|boot_rom` across every `.cpp`/`.h` returns **zero matches**.
`0xFF50` is simply absent from the write switch.

Instead gambatte installs a full hardware-dumped table via
`setInitState(SaveState &, bool cgb, bool gbaCgbMode)` — **1332 lines**, ~1120 of them captured
dumps. The dumper ROMs ship in `test/hwtests/*_dumper.asm`.

```cpp
// initstate.cpp:1171-1184
state.cpu.cycleCounter = cgb ? 0x102A0 : 0x102A0 + 0x8D2C;
state.cpu.pc = 0x100;   state.cpu.sp = 0xFFFE;
state.cpu.a = cgb * 0x10 | 0x01;
state.cpu.b = cgb & gbaCgbMode;
state.cpu.c = 0x13;     state.cpu.d = 0x00;   state.cpu.e = 0xD8;
state.cpu.f = 0xB0;     state.cpu.h = 0x01;   state.cpu.l = 0x4D;
```

`cycleCounter` is deliberately non-zero because DIV/TIMA/frame-sequencer/PPU phases all derive from
it. DIV is not stored as a byte — it is derived on read, and with `divLastUpdate = -0x1C00`
(`:1205`) the effective boot DIV is **DMG `0xAB`, CGB `0x1E`**, pinned by
`test/hwtests/div/start_inc_1_dmg08_outAB.asm`.

### Concrete differences

| Item | gambatte (DMG) | gb | gb ref |
|---|---|---|---|
| A / C / D / E / H / L / SP / PC | 01 / 13 / 00 / D8 / 01 / 4D / FFFE / 0100 | **identical** ✅ | `registers.rs:53-71` |
| **F** | **0xB0** (Z,H,C) [^f-cond] | **0x80** (Z only) | `registers.rs:56-61` |
| F low nibble on `POP AF` | forced 0 | forced 0 ✅ | `registers.rs:21-35` |
| **DIV at handoff** | **0xAB** | **0x00** | `divider.rs:11-19` |
| **LCDC** | **0x91** | **0x80** | `lcd_control.rs:74-87` |
| **BGP / OBP0 / OBP1** | **FC / FF / FF** | **00 / 00 / 00** | `lcd_palette.rs:5-13` |
| STAT | 0x80 | 0x00 | `lcd_status.rs:60-67` |
| IF | 0xE1 | 0x00 | `interrupt.rs:14-24` |
| TAC | 0xF8 | 0x00 | `timer.rs:24-26` |
| SC | 0x7E | 0x00 | `serial.rs:45-50` |
| HRAM | 127-byte dump `2B 0B 64 2F …` | all zero | `mmu.rs:56` |
| WRAM | pattern + 1008 patches | all zero | `mmu.rs:55` |
| VRAM | Nintendo logo tiles + DMG tilemap | zero | `ppu.rs:66` |
| OAM | 160-byte garbage dump | zero | `ppu.rs` default |
| SRAM initial fill | **0xFF** (`initstate.cpp:1186`) | 0x00 | `ram.rs:23` |
| `0xFEA0-0xFEFF` | 0x00 | reads 0xFF | `mmu.rs:335` |
| IME / halted | false / false | false / Normal ✅ | `core.rs:31-43` |

Two consequences beyond test scores:

1. **DIV = 0 at boot** makes any RNG seeded from DIV identical every run. For *this* project that is
   arguably a **feature** (the agent harness wants determinism) — but it should be a documented
   choice with a constructor parameter, not an accident.
2. **Zeroed HRAM/WRAM/OAM** changes behaviour for games that read uninitialised RAM before writing
   it. `0xFF` is the correct fill for erased battery SRAM.

### Tasks

- [ ] Add an `initstate` module: CPU register table keyed by model, a 127-byte HRAM array, a
      160-byte OAM array, and the I/O defaults. **The concrete bytes can be transcribed straight
      out of `test/hwtests/fexx_ffxx_dumper_dmg08.bin`** — a committed, hardware-verified dump of
      exactly `0xFE00-0xFFFF`.
- [ ] Fix F, LCDC to `0x91`, BGP/OBP to `FC/FF/FF`.

[^f-cond]: ⚠️ **Corrected 2026-08-06 (ledger #11, refined in #12).** `F` is **not** unconditionally
      `0xB0`; gambatte's flat `0xB0` (`initstate.cpp:1179`) does not model it, and neither does Pan
      Docs' footnote, which claims `H` and `C` are always both clear or both set. The DMG boot ROM's
      last flag-affecting instruction is `add a, [hl]` against the stored header checksum, and it
      locks up unless the 8-bit result is zero — so `C` is set iff the checksum is non-zero and `H`
      is set iff its **low nibble** is non-zero. That gives `0x80` / `0x90` / `0xB0` for 1 / 15 /
      240 of the 256 possible checksums. **`pokered`'s checksum is `0x20`, so Pokémon Red is one of
      the fifteen: `F = 0x90`.** See plan task B11.
- [ ] Fill SRAM with `0xFF` at allocation.
- [ ] Apply the table in `Core::dmg` **and** implement `Core::reset()` from it (currently
      `todo!()` — see [`06-features-and-robustness.md`](06-features-and-robustness.md)).
- [ ] Make the boot DIV value a constructor parameter so determinism stays an explicit choice.

**Failing tests:** mooneye `acceptance/boot_regs-{dmgABC,dmg0,mgb,sgb,sgb2}.gb` (the F = `0xB0`
difference alone fails these), `acceptance/boot_div-{dmgABCmgb,dmg0}.gb`,
`acceptance/boot_hwio-{dmgABCmgb,dmg0}.gb`; gambatte `div/start_inc_1_dmg08_outAB.asm`,
`ioregs_reset_dumper_dmg08.bin`, `fexx_ffxx_dumper_dmg08.bin`.

---

## 8. Joypad and serial

### 8a. Joypad

```cpp
// memory.cpp:473-490 (abridged)
void Memory::updateInput() {
	unsigned state = 0xF;
	if ((ioamhram_[0x100] & 0x30) != 0x30 && getInput_) {
		unsigned input = (*getInput_)();
		if (!(ioamhram_[0x100] & 0x10)) state &= ~input >> 4;
		if (!(ioamhram_[0x100] & 0x20)) state &= ~input;
	}
	if (state != 0xF && (ioamhram_[0x100] & 0xF) == 0xF)
		intreq_.flagIrq(0x10);
	ioamhram_[0x100] = (ioamhram_[0x100] & -0x10u) | state;
}
```

Three points: with both select lines high the low nibble is forced `0xF` (open-drain); both low ANDs
the nibbles (the "ghost input" quirk); and **the interrupt fires on a high→low edge of the combined
register nibble**, not on a button press — so changing select lines can itself raise a joypad IRQ.

`gb` (`src/joypad.rs:22-44`): the nibble combination is actually **equivalent** to gambatte's (OR of
active-high, inverted = AND of active-low) ✅ and both-unselected → `0xF` ✅.

**Gap:** P1 bits 6-7 read 0 where hardware reads 1 (`gb`'s own test asserts `0x3F` at
`src/joypad.rs:126`; hardware gives `0xFF`); and the interrupt is **edge-on-press pushed from the
host**, so a press while neither line is selected wrongly raises an interrupt, while changing select
lines with a button held wrongly raises none.

- [ ] `0xFF00 => 0xC0 | self.joypad_register.get()`
- [ ] Move interrupt generation into a `recompute()` run on `0xFF00` read and on writes to bits 4-5,
      comparing old vs new low nibble.

**Tests:** gambatte `test/hwtests/jpadirq_1.asm`, `jpadirq_2.asm` (expectations in `jpadirq.txt`).

### 8b. Serial

gambatte models a **disconnected cable**, shifting in `1`s progressively:

```cpp
// memory.cpp:151-158 (abridged)
	ioamhram_[0x101] = (((ioamhram_[0x101] + 1) << serialCnt_) - 1) & 0xFF;
```

so a mid-transfer read sees a partially shifted value. Start (`memory.cpp:681-692`):

```cpp
	case 0x02:
		updateSerial(cc);
		serialCnt_ = 8;
		if ((data & 0x81) == 0x81) {
			intreq_.setEventTime<intevent_serial>(data & isCgb() * 2
				? cc - (cc - tima_.divLastUpdate()) % 8 + 0x10 * serialCnt_
				: cc - (cc - tima_.divLastUpdate()) % 0x100 + 0x200 * serialCnt_);
		} else
			intreq_.setEventTime<intevent_serial>(disabled_time);
		data |= 0x7E - isCgb() * 2;
		break;
```

Two crucial details: a transfer starts only as **internal clock** (`(data & 0x81) == 0x81`); and
completion is **aligned to DIV**, because the serial clock taps the same divider. Writing DIV
mid-transfer shifts completion.

`gb` (`src/serial.rs:52-77`) gets the shape right: master-only start ✅, timed completion, SB →
`0xFF`, interrupt raised ✅, external-clock transfers correctly never complete ✅. Plus the
`buffer: Option<Vec<u8>>` tap used to capture blargg text output (`src/game_boy.rs:352-378`).

**Gap:** SB does not shift progressively (it jumps from written value to `0xFF`); SC read-back lacks
the `0x7E` mask; no DIV alignment.

- [ ] Add the `0x7E` mask to `control()`.
- [ ] Shift SB left by elapsed bit-periods filling with `1`s:
      `((data as u16 + 1) << n).wrapping_sub(1) as u8`
- [ ] (Optional) Pass the divider phase into `set_control` and start the countdown at
      `PER_SERIAL_BYTE_TRANSFER - (div_phase % period)`.

**What needs it:** blargg's `cpu_instrs` and `instr_timing` use serial as a text console and `gb`
already passes them — that is the load-bearing use here. Beyond that: gambatte
`test/hwtests/serial/*` (47 ROMs); mooneye `acceptance/serial/boot_sclk_align-dmgABCmgb.gb`.

---

## Suggested implementation order

1. [ ] `trait Mbc` + dispatch + mask-not-clamp + ROM padding (§1, §2) — unblocks every
       non-Pokémon cartridge and removes the only guest-reachable hard panic in the memory hot path
2. [ ] DMA source classifier + progressive fill + un-invert the PPU gate (§5)
3. [ ] I/O read-back masks + `0xFEA0` = `0x00` (§4b, §4c) — a handful of one-liners
4. [ ] `initstate` table + `Core::reset()` (§7)
5. [ ] Header parsing robustness (§2)
6. [ ] RTC (§3) — only if G/S/C ever becomes a target
7. [ ] Serial/joypad polish (§8)
8. [ ] CGB (§6) — last, if ever

---

## References

- Pan Docs — <https://gbdev.io/pandocs/> (Memory Map, MBCs, OAM DMA chapters)
- gbdev MBC documentation — <https://gbdev.io/pandocs/MBCs.html>
- mooneye-gb `emulator-only/` MBC tests — <https://github.com/Gekkio/mooneye-test-suite>
- gambatte hardware tests — `/home/alex/projects/gambatte/test/hwtests/`
  (`oamdma/`, `serial/`, `oam_access/`, `div/`, `sram.asm`, `fexx_ffxx_dumper*`)
