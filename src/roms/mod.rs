

pub mod blargg_cpu {
    pub const ROM: &[u8] = include_bytes!("cpu_instrs/cpu_instrs.gb");
    pub const SPECIAL_01: &[u8] = include_bytes!("cpu_instrs/01-special.gb");
    pub const INTERRUPTS_02: &[u8] = include_bytes!("cpu_instrs/02-interrupts.gb");
    pub const OP_SP_HL_03: &[u8] = include_bytes!("cpu_instrs/03-op sp,hl.gb");
    pub const OP_R_IMM_04: &[u8] = include_bytes!("cpu_instrs/04-op r,imm.gb");
    pub const OP_RP_05: &[u8] = include_bytes!("cpu_instrs/05-op rp.gb");
    pub const LD_R_R_06: &[u8] = include_bytes!("cpu_instrs/06-ld r,r.gb");
    pub const JR_JP_CALL_RET_RST_07: &[u8] = include_bytes!("cpu_instrs/07-jr,jp,call,ret,rst.gb");
    pub const MISC_INSTRUCTIONS_08: &[u8] = include_bytes!("cpu_instrs/08-misc instrs.gb");
    pub const OP_R_R_09: &[u8] = include_bytes!("cpu_instrs/09-op r,r.gb");
    pub const BIT_OPS_10: &[u8] = include_bytes!("cpu_instrs/10-bit ops.gb");
    pub const OP_A_HL_11: &[u8] = include_bytes!("cpu_instrs/11-op a,(hl).gb");

    pub const INSTRUCTION_TIMING: &[u8] = include_bytes!("instr_timing.gb");
}

pub mod blargg_dmg_sound {
    pub const ROM: &[u8] = include_bytes!("dmg_sound/dmg_sound.gb");
    /// The combined suite's own screen: `01:ok` through `12:ok`, then `Passed`. All twelve
    /// sub-tests in one assertion. Captured from gambatte (see `10-implementation-plan.md` §2.5);
    /// gb reproduces it pixel for pixel.
    pub const EXPECTED_ALL: &[u8] = include_bytes!("dmg_sound/dmg_sound.png");

    pub const REGISTERS: &[u8] = include_bytes!("dmg_sound/01-registers.gb");
    pub const EXPECTED_REGISTERS: &[u8] = include_bytes!("dmg_sound/01-registers.png");
    pub const LENGTH_COUNTER: &[u8] = include_bytes!("dmg_sound/02-len ctr.gb");
    pub const EXPECTED_LENGTH_COUNTER: &[u8] = include_bytes!("dmg_sound/02-len ctr.png");
    pub const TRIGGER: &[u8] = include_bytes!("dmg_sound/03-trigger.gb");
    pub const EXPECTED_TRIGGER: &[u8] = include_bytes!("dmg_sound/03-trigger.png");
    pub const SWEEP: &[u8] = include_bytes!("dmg_sound/04-sweep.gb");
    pub const EXPECTED_SWEEP: &[u8] = include_bytes!("dmg_sound/04-sweep.png");
    pub const SWEEP_DETAILS: &[u8] = include_bytes!("dmg_sound/05-sweep details.gb");
    pub const EXPECTED_SWEEP_DETAILS: &[u8] = include_bytes!("dmg_sound/05-sweep details.png");
    pub const OVERFLOW_ON_TRIGGER: &[u8] = include_bytes!("dmg_sound/06-overflow on trigger.gb");
    pub const EXPECTED_OVERFLOW_ON_TRIGGER: &[u8] = include_bytes!("dmg_sound/06-overflow on trigger.png");
    pub const LENGTH_SWEEP_PERIOD_SYNC: &[u8] = include_bytes!("dmg_sound/07-len sweep period sync.gb");
    pub const EXPECTED_LENGTH_SWEEP_PERIOD_SYNC: &[u8] = include_bytes!("dmg_sound/07-len sweep period sync.png");
    pub const LENGTH_COUNTER_DURING_POWER: &[u8] = include_bytes!("dmg_sound/08-len ctr during power.gb");
    pub const EXPECTED_LENGTH_COUNTER_DURING_POWER: &[u8] = include_bytes!("dmg_sound/08-len ctr during power.png");
    pub const WAVE_READ_WHILE_ON: &[u8] = include_bytes!("dmg_sound/09-wave read while on.gb");
    /// Promoted by A16 once the wave channel was fixed, and only after checking the frame
    /// against gambatte's — the two are byte-identical.
    pub const EXPECTED_WAVE_READ_WHILE_ON: &[u8] = include_bytes!("dmg_sound/09-wave read while on.png");
    pub const WAVE_TRIGGER_WHILE_ON: &[u8] = include_bytes!("dmg_sound/10-wave trigger while on.gb");
    /// Promoted by A16 once the wave channel was fixed, and only after checking the frame
    /// against gambatte's — the two are byte-identical.
    pub const EXPECTED_WAVE_TRIGGER_WHILE_ON: &[u8] = include_bytes!("dmg_sound/10-wave trigger while on.png");
    pub const REGISTERS_AFTER_POWER: &[u8] = include_bytes!("dmg_sound/11-regs after power.gb");
    pub const EXPECTED_REGISTERS_AFTER_POWER: &[u8] = include_bytes!("dmg_sound/11-regs after power.png");
    pub const WAVE_WRITE_WHILE_ON: &[u8] = include_bytes!("dmg_sound/12-wave write while on.gb");
    /// Promoted by A16 once the wave channel was fixed, and only after checking the frame
    /// against gambatte's — the two are byte-identical.
    pub const EXPECTED_WAVE_WRITE_WHILE_ON: &[u8] = include_bytes!("dmg_sound/12-wave write while on.png");
}

pub mod acid {
    pub const ROM: &[u8] = include_bytes!("dmg-acid2/dmg-acid2.gb");
    pub const EXPECTED_DMG: &[u8] = include_bytes!("dmg-acid2/reference-dmg.png");
}

/// `cgb-acid2` v1.1 — the CGB counterpart of `dmg-acid2`, and Phase B's acceptance test. It
/// covers BG map attributes, the eight-palette BG and OBJ banks, sprite priority by OAM index,
/// LCDC bit 0 as a master priority override, and 8x16 sprites out of VRAM bank 1.
///
/// Unlike the audio suites this one ships its own reference image, so nothing had to be promoted
/// from `gb`'s own output. The upstream README specifies the 5-bit to 8-bit expansion as
/// `(c << 3) | (c >> 2)`, which is what [`crate::lcd_palette::LcdColor::from_rgb555`] does, so the
/// PNG compares byte for byte with no colour-correction curve in the way.
///
/// Source: <https://github.com/mattcurrie/cgb-acid2> (ROM from the v1.1 release,
/// `img/reference.png` from `master`, both fetched 2026-08-05).
pub mod cgb_acid {
    pub const ROM: &[u8] = include_bytes!("cgb-acid2/cgb-acid2.gbc");
    pub const EXPECTED: &[u8] = include_bytes!("cgb-acid2/reference.png");
}

pub mod button_test {
    pub const ROM: &[u8] = include_bytes!("button_test/rom.gb");
    pub const EXPECTED_A: &[u8] = include_bytes!("button_test/a.png");
    pub const EXPECTED_B: &[u8] = include_bytes!("button_test/b.png");
    pub const EXPECTED_SELECT: &[u8] = include_bytes!("button_test/select.png");
    pub const EXPECTED_START: &[u8] = include_bytes!("button_test/start.png");
    pub const EXPECTED_UP: &[u8] = include_bytes!("button_test/up.png");
    pub const EXPECTED_DOWN: &[u8] = include_bytes!("button_test/down.png");
    pub const EXPECTED_LEFT: &[u8] = include_bytes!("button_test/left.png");
    pub const EXPECTED_RIGHT: &[u8] = include_bytes!("button_test/right.png");
}

pub mod homebrew {
    pub const TEST_CART: &[u8] = include_bytes!("Jayro's Test Cart v2.3.0.gb");
}

pub mod roms {
    use std::io::BufReader;
    use image::{ImageFormat, ImageReader, RgbImage};

    pub fn parse_png(data: &[u8]) -> RgbImage {
        ImageReader::with_format(BufReader::new(std::io::Cursor::new(data)), ImageFormat::Png)
            .decode()
            .expect("Failed to decode expected image")
            .to_rgb8()
    }
}
/// Blargg's remaining DMG suites, wired by task A14 of `docs/compatibility/10-implementation-plan.md`.
///
/// These are **expected to fail** and are `#[ignore]`d — they exist to quantify the gap left by
/// advancing peripherals once per instruction rather than per M-cycle. Do not "fix" them without
/// reading that plan: `mem_timing`, `halt_bug` and `interrupt_time` need the deferred M-cycle
/// timing refactor, and `oam_bug` needs the DMG OAM corruption quirk, which gambatte does not
/// model either.
///
/// Source: `c-sp/game-boy-test-roms` v7.0.
pub mod blargg_timing {
    pub const MEM_TIMING: &[u8] = include_bytes!("mem_timing/mem_timing.gb");
    /// Hardware reference from `c-sp/game-boy-test-roms` v7.0 — what a *passing* run looks like.
    pub const EXPECTED_MEM_TIMING: &[u8] = include_bytes!("mem_timing/expected.png");
    pub const MEM_TIMING_READ: &[u8] = include_bytes!("mem_timing/01-read_timing.gb");
    pub const MEM_TIMING_WRITE: &[u8] = include_bytes!("mem_timing/02-write_timing.gb");
    pub const MEM_TIMING_MODIFY: &[u8] = include_bytes!("mem_timing/03-modify_timing.gb");

    pub const MEM_TIMING_2: &[u8] = include_bytes!("mem_timing_2/mem_timing.gb");
    pub const EXPECTED_MEM_TIMING_2: &[u8] = include_bytes!("mem_timing_2/expected.png");
    pub const MEM_TIMING_2_READ: &[u8] = include_bytes!("mem_timing_2/01-read_timing.gb");
    pub const MEM_TIMING_2_WRITE: &[u8] = include_bytes!("mem_timing_2/02-write_timing.gb");
    pub const MEM_TIMING_2_MODIFY: &[u8] = include_bytes!("mem_timing_2/03-modify_timing.gb");

    pub const HALT_BUG: &[u8] = include_bytes!("halt_bug.gb");
    pub const EXPECTED_HALT_BUG: &[u8] = include_bytes!("halt_bug.png");
    pub const INTERRUPT_TIME: &[u8] = include_bytes!("interrupt_time.gb");
    pub const EXPECTED_INTERRUPT_TIME: &[u8] = include_bytes!("interrupt_time.png");
}

/// Blargg's `oam_bug` suite — the DMG OAM corruption quirk. See [`blargg_timing`] for why these
/// are `#[ignore]`d.
pub mod blargg_oam_bug {
    pub const ROM: &[u8] = include_bytes!("oam_bug/oam_bug.gb");
    /// Hardware reference from `c-sp/game-boy-test-roms` v7.0.
    pub const EXPECTED: &[u8] = include_bytes!("oam_bug/expected.png");
    pub const LCD_SYNC: &[u8] = include_bytes!("oam_bug/1-lcd_sync.gb");
    pub const CAUSES: &[u8] = include_bytes!("oam_bug/2-causes.gb");
    pub const NON_CAUSES: &[u8] = include_bytes!("oam_bug/3-non_causes.gb");
    pub const SCANLINE_TIMING: &[u8] = include_bytes!("oam_bug/4-scanline_timing.gb");
    pub const TIMING_BUG: &[u8] = include_bytes!("oam_bug/5-timing_bug.gb");
    pub const TIMING_NO_BUG: &[u8] = include_bytes!("oam_bug/6-timing_no_bug.gb");
    pub const TIMING_EFFECT: &[u8] = include_bytes!("oam_bug/7-timing_effect.gb");
    pub const INSTR_EFFECT: &[u8] = include_bytes!("oam_bug/8-instr_effect.gb");
}

/// **D10.** The mooneye MBC test ROMs, from `c-sp/game-boy-test-roms` v7.0 (MIT).
///
/// ⚠️ **These are stored lz4-compressed and must be decompressed before use** — see
/// [`mooneye::rom`]. Raw they are 22 MB, which is 15x the rest of this repository's committed
/// binary data put together, and almost all of it is padding: `mbc5/rom_64Mb.gb` alone is 8 MB of
/// mostly-nothing testing that bank 511 is addressable. Compressed the whole set is ~90 KB.
///
/// Regenerate with [`mooneye::tests::compress_mooneye_roms`] if the upstream set ever changes.
///
/// Behind the `hwtests` feature so a default build carries none of it.
#[cfg(feature = "hwtests")]
pub mod mooneye {
    /// Decompress one of the ROMs below into a real cartridge image.
    pub fn rom(compressed: &[u8]) -> Vec<u8> {
        lz4_flex::decompress_size_prepended(compressed)
            .expect("a committed mooneye ROM should decompress")
    }

    macro_rules! mooneye_roms {
        ($($konst:ident => $file:literal),* $(,)?) => {
            $(pub const $konst: &[u8] = include_bytes!(concat!("mooneye/", $file, ".lz4"));)*
            /// Every ROM with its upstream name, for the harness to iterate.
            pub const ALL: &[(&str, &[u8])] = &[$(($file, $konst)),*];
        };
    }

    mooneye_roms! {
        MBC1_BITS_BANK1 => "mbc1-bits_bank1",
        MBC1_BITS_BANK2 => "mbc1-bits_bank2",
        MBC1_BITS_MODE => "mbc1-bits_mode",
        MBC1_BITS_RAMG => "mbc1-bits_ramg",
        MBC1_MULTICART_ROM_8MB => "mbc1-multicart_rom_8Mb",
        MBC1_RAM_64KB => "mbc1-ram_64kb",
        MBC1_RAM_256KB => "mbc1-ram_256kb",
        MBC1_ROM_512KB => "mbc1-rom_512kb",
        MBC1_ROM_1MB => "mbc1-rom_1Mb",
        MBC1_ROM_2MB => "mbc1-rom_2Mb",
        MBC1_ROM_4MB => "mbc1-rom_4Mb",
        MBC1_ROM_8MB => "mbc1-rom_8Mb",
        MBC1_ROM_16MB => "mbc1-rom_16Mb",
        MBC2_BITS_RAMG => "mbc2-bits_ramg",
        MBC2_BITS_ROMB => "mbc2-bits_romb",
        MBC2_BITS_UNUSED => "mbc2-bits_unused",
        MBC2_RAM => "mbc2-ram",
        MBC2_ROM_512KB => "mbc2-rom_512kb",
        MBC2_ROM_1MB => "mbc2-rom_1Mb",
        MBC2_ROM_2MB => "mbc2-rom_2Mb",
        MBC5_ROM_512KB => "mbc5-rom_512kb",
        MBC5_ROM_1MB => "mbc5-rom_1Mb",
        MBC5_ROM_2MB => "mbc5-rom_2Mb",
        MBC5_ROM_4MB => "mbc5-rom_4Mb",
        MBC5_ROM_8MB => "mbc5-rom_8Mb",
        MBC5_ROM_16MB => "mbc5-rom_16Mb",
        MBC5_ROM_32MB => "mbc5-rom_32Mb",
        MBC5_ROM_64MB => "mbc5-rom_64Mb",
    }

    #[cfg(test)]
    mod tests {
        /// Rebuild `src/roms/mooneye/*.lz4` from an extracted `game-boy-test-roms` release.
        ///
        /// ```text
        /// MOONEYE_SRC=/path/to/mooneye-test-suite/emulator-only \
        ///   cargo test --release --features hwtests --bin gb -- compress_mooneye_roms --ignored --nocapture
        /// ```
        ///
        /// Same shape as the blip golden-vector regeneration: a tool, not an assertion, so it is
        /// `#[ignore]`d on top of its feature gate.
        #[test]
        #[ignore = "tool: rebuilds the committed mooneye ROMs from an extracted release"]
        fn compress_mooneye_roms() {
            let src = std::env::var("MOONEYE_SRC").expect("set MOONEYE_SRC to .../emulator-only");
            let out = concat!(env!("CARGO_MANIFEST_DIR"), "/src/roms/mooneye");
            let (mut count, mut raw, mut packed) = (0usize, 0usize, 0usize);

            for mapper in ["mbc1", "mbc2", "mbc5"] {
                let dir = std::path::Path::new(&src).join(mapper);
                let mut entries: Vec<_> = std::fs::read_dir(&dir)
                    .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "gb"))
                    .collect();
                entries.sort();

                for path in entries {
                    let data = std::fs::read(&path).expect("read ROM");
                    let name = path.file_stem().unwrap().to_string_lossy();
                    let compressed = lz4_flex::compress_prepend_size(&data);
                    let target = format!("{out}/{mapper}-{name}.lz4");
                    std::fs::write(&target, &compressed).expect("write");
                    println!("{mapper}-{name}: {} -> {} bytes", data.len(), compressed.len());
                    count += 1;
                    raw += data.len();
                    packed += compressed.len();
                }
            }
            println!("\n{count} ROMs: {raw} bytes raw -> {packed} bytes committed");
        }
    }
}
