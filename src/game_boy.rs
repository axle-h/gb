use crate::core::Core;
use crate::cycles::MachineCycles;
use crate::model::Model;
use crate::savestate::{SectionReader, SectionWriter};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GameBoy {
    core: Core
}

impl GameBoy {
    /// A Game Boy (DMG). **89 call sites depend on this behaving exactly as it always has** —
    /// add constructors beside it rather than changing it.
    pub fn dmg(cart: &[u8]) -> Self {
        Self::new(cart, Model::Dmg)
    }

    /// A Game Boy Color. A cartridge with CGB header support gets the full colour hardware; a
    /// DMG-only cartridge — Pokémon Red among them — runs in compatibility mode with the
    /// boot ROM's title-derived palette. See [`crate::model::ColorMode`].
    pub fn cgb(cart: &[u8]) -> Self {
        Self::new(cart, Model::Cgb)
    }

    pub fn new(cart: &[u8], model: Model) -> Self {
        Self {
            core: Core::new(cart, model)
        }
    }

    pub fn dmg_hello_world() -> Self {
        Self::dmg(crate::roms::acid::ROM)
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
        let mut cycles = MachineCycles::ZERO;
        while cycles < min_cycles {
            // C2: HALT is 65% of the M-cycles a real game emulates. Stepping it one virtual `Nop`
            // at a time is what made it cost full price — see [`Core::skip_halt`].
            if self.core.mode() == crate::core::CoreMode::Halt {
                cycles += self.core.skip_halt(min_cycles - cycles);
            } else {
                let opcode = self.core.fetch();
                cycles += self.core.execute(opcode);
            }
        }
        cycles
    }

    pub fn reset(&mut self) {
        self.core.reset();
    }

    pub fn dump_sram(&self) -> Vec<u8> {
        self.core.mmu().dump_sram()
    }

    pub fn dump_sram_to_file(&self, path: &str) -> Result<(), String> {
        let data = self.dump_sram();
        std::fs::write(path, &data).map_err(|e| e.to_string())
    }

    pub fn restore_sram(&mut self, data: &[u8]) -> Result<(), String> {
        self.core.mmu_mut().restore_sram(data)
    }

    pub fn restore_sram_from_file(&mut self, path: &str) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        self.restore_sram(&data)
    }

    /// Write every save-state section this build knows about. Exposed for tests that need to
    /// manipulate the container; ordinary callers want [`GameBoy::save_state`].
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        self.core.write_sections(writer)
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let mut writer = SectionWriter::new();
        self.write_sections(&mut writer)?;
        Ok(writer.finish())
    }

    pub fn save_state_to_file(&self, path: &str) -> Result<(), String> {
        let data = self.save_state()?;
        std::fs::write(path, &data).map_err(|e| e.to_string())
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let reader = SectionReader::parse(data)?;

        // Applied to a copy so a failure part-way through cannot leave a half-loaded machine.
        // The copy also carries the ROM image, which is never serialised.
        let mut candidate = self.clone();
        candidate.core.read_sections(&reader)?;

        if candidate.core.mmu().header() != self.core.mmu().header() {
            return Err(format!("Incompatible save state, expected {:?}, got {:?}", self.core.mmu().header(), candidate.core.mmu().header()));
        }

        *self = candidate;
        Ok(())
    }

    pub fn load_state_from_file(&mut self, path: &str) -> Result<(), String> {
        let data = std::fs::read(path)
            .map_err(|e| e.to_string())?;
        self.load_state(&data)
    }

    pub fn save_screenshot_to_file(&self, path: &str) -> Result<(), String> {
        let image = self.core().mmu().ppu().screenshot();
        image.save(path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use image::RgbImage;
    use crate::ram::RAM;
    use crate::roms::roms::parse_png;
    use super::*;

    /// A9: throughput of the **emulator core alone**. The Pokémon agent, its policy and all
    /// observation are excluded — this loop calls nothing but [`GameBoy::run`].
    ///
    /// The existing `pokemon::integration_tests::fixture::bench_emulation_throughput` measures a
    /// full `agent.step()` instead, which is a different number and lives in the wrong module.
    /// Phase C is scored against *this* one.
    ///
    /// ```text
    /// cargo test --release --features bench --bin gb -- \
    ///   game_boy::tests::bench_core_throughput --exact --nocapture
    /// ```
    ///
    /// Behind a feature rather than `#[ignore]`d, so a benchmark never shows up in the ignored
    /// count next to tests that are actually blocked.
    #[cfg(feature = "bench")]
    #[test]
    fn bench_core_throughput() {
        use std::time::Instant;

        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const WARM_UP_FRAMES: usize = 60;

        // Two knobs, for `perf` rather than for the benchmark itself. 600 frames is 10 s of game
        // time, which at present speeds is ~0.1 s of wall clock — far too short to sample. And a
        // profile of all three workloads at once is a profile of none of them.
        //   BENCH_FRAMES=60000 BENCH_ONLY=pokemon perf record -F 999 -g -- <binary> …
        let measured_frames: usize = std::env::var("BENCH_FRAMES")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(600);
        let only = std::env::var("BENCH_ONLY").unwrap_or_default();

        fn pokemon_in_game() -> GameBoy {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/at-celadon.bin");
            let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
            gb.load_state(&std::fs::read(path).expect("fixture")).expect("load fixture");
            gb
        }

        let workloads: Vec<(&str, Box<dyn Fn() -> GameBoy>)> = vec![
            // The representative case: a real game, mid-play, HALTing as games do.
            ("pokemon-red (fixture)", Box::new(pokemon_in_game)),
            // Never HALTs, so it isolates raw dispatch from idle-skipping.
            ("cpu_instrs.gb", Box::new(|| GameBoy::dmg(crate::roms::blargg_cpu::ROM))),
            // PPU-heavy.
            ("dmg-acid2.gb", Box::new(|| GameBoy::dmg(crate::roms::acid::ROM))),
        ];

        println!("\n{:<24} {:>12} {:>16} {:>10}", "workload", "realtime", "t-cycles/s", "frames");
        println!("{}", "-".repeat(66));

        for (name, build) in &workloads {
            if !only.is_empty() && !name.contains(&only) {
                continue;
            }
            let mut gb = build();
            for _ in 0..WARM_UP_FRAMES {
                gb.run(FRAME);
            }

            let start = Instant::now();
            let mut cycles = MachineCycles::ZERO;
            for _ in 0..measured_frames {
                cycles += gb.run(FRAME);
            }
            let elapsed = start.elapsed();

            let t_cycles_per_sec = cycles.t_cycles() as f64 / elapsed.as_secs_f64();
            let realtime = t_cycles_per_sec / MachineCycles::CPU_FREQ as f64;
            println!(
                "{name:<24} {realtime:>11.1}x {:>16.0} {measured_frames:>10}",
                t_cycles_per_sec
            );
        }
        println!();
    }

    /// ⭐ **C2's correctness test, and the one that matters.** The HALT fast-path sleeps through
    /// whole idle spans in a single [`MMU::update`]; this proves the machine it wakes up in is
    /// bit-identical to the one that stepped every M-cycle of that span individually.
    ///
    /// The framebuffer is compared as well as the state, because [`PartialEq`] deliberately
    /// excludes it (see the note on `PPU`) — and rendering is exactly where a skip that jumped an
    /// event would show up first.
    ///
    /// It only means anything if the workload actually HALTs, so the ROMs here are a real game and
    /// a PPU test that both idle heavily, and the assertion below fails if HALT is not reached.
    #[test]
    fn the_halt_fast_path_matches_stepping_cycle_by_cycle() {
        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const FRAMES: usize = 120;

        /// The pre-C2 run loop, verbatim: fetch and execute, HALT included.
        fn run_stepped(gb: &mut GameBoy, min_cycles: MachineCycles) {
            let mut cycles = MachineCycles::ZERO;
            while cycles < min_cycles {
                let opcode = gb.core_mut().fetch();
                cycles += gb.core_mut().execute(opcode);
            }
        }

        let pokemon = || {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/at-celadon.bin");
            let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
            gb.load_state(&std::fs::read(path).expect("fixture")).expect("load fixture");
            gb
        };
        let workloads: Vec<(&str, Box<dyn Fn() -> GameBoy>)> = vec![
            ("pokemon-red", Box::new(pokemon)),
            ("dmg-acid2", Box::new(|| GameBoy::dmg(crate::roms::acid::ROM))),
            // A CGB too: the skip converts the PPU and APU deadlines out of the video clock, and
            // that conversion only exists because a CGB can halve it.
            ("cgb-acid2", Box::new(|| GameBoy::cgb(crate::roms::cgb_acid::ROM))),
            ("pokemon-red (cgb compat)", Box::new(|| GameBoy::cgb(crate::pokemon::roms::POKERED))),
        ];

        for (name, build) in &workloads {
            let (mut fast, mut slow) = (build(), build());
            let mut halted = false;
            for _ in 0..FRAMES {
                fast.run(FRAME);
                run_stepped(&mut slow, FRAME);
                halted |= slow.core().mode() == crate::core::CoreMode::Halt;
                assert_eq!(fast, slow, "{name}: machine state diverged");
                assert!(
                    fast.core().mmu().ppu().lcd() == slow.core().mmu().ppu().lcd(),
                    "{name}: framebuffer diverged",
                );
            }
            assert!(halted, "{name} never HALTs, so it proves nothing about the fast path");
        }
    }

    /// A1: `reset()` used to be `todo!()`, so any caller panicked.
    #[test]
    fn reset_matches_fresh_construction() {
        let mut a = GameBoy::dmg(crate::roms::acid::ROM);
        a.run(MachineCycles::from_m(500_000));
        assert_ne!(a, GameBoy::dmg(crate::roms::acid::ROM), "test is vacuous if running changed nothing");

        a.reset();
        assert_eq!(a, GameBoy::dmg(crate::roms::acid::ROM));
    }

    /// A1's guarantee has to hold for a CGB too — including re-applying the boot palette, which
    /// a fresh compatibility-mode machine has and a naively reset one would not.
    #[test]
    fn cgb_reset_matches_fresh_construction() {
        for cart in [crate::roms::cgb_acid::ROM, crate::pokemon::roms::POKERED] {
            let mut a = GameBoy::cgb(cart);
            a.run(MachineCycles::from_m(500_000));
            assert_ne!(a, GameBoy::cgb(cart), "test is vacuous if running changed nothing");

            a.reset();
            assert_eq!(a, GameBoy::cgb(cart));
            assert_eq!(a.core().registers().a, 0x11, "and it still reports a CGB");
        }
    }

    /// Battery-backed cartridge RAM does not lose its contents when the console is reset.
    #[test]
    fn reset_preserves_sram() {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        let sram: Vec<u8> = (0..gb.dump_sram().len()).map(|i| (i % 251) as u8).collect();
        gb.restore_sram(&sram).expect("restore sram");

        gb.reset();

        assert_eq!(gb.dump_sram(), sram);
    }

    /// A2: the `Stop` and `Crash` arms of `Core::execute` return `MachineCycles::ZERO`, which was
    /// reported as making `run()` livelock. It does not — that value is only one addend of what
    /// `execute` returns, and the opcode's own cost is always at least one M-cycle. Kept as a
    /// guard so that if anyone ever does make `execute` return zero, this fails instead of hanging.
    #[test]
    fn run_terminates_after_stop() {
        let mut gb = GameBoy::dmg_hello_world();
        // STOP, then a pad byte, in work RAM — ROM ignores writes.
        gb.core_mut().mmu_mut().write(0xC000, 0x10);
        gb.core_mut().mmu_mut().write(0xC001, 0x00);
        gb.core_mut().registers_mut().pc = 0xC000;

        let cycles = gb.run(MachineCycles::from_m(10_000));

        assert!(cycles >= MachineCycles::from_m(10_000));
        assert_eq!(gb.core().mode(), crate::core::CoreMode::Stop, "nothing should have woken it");
    }

    #[test]
    fn save_and_load_state() {
        // Create a GameBoy and run it for some cycles to change its state
        let mut original_gb = GameBoy::dmg_hello_world();
        original_gb.run(MachineCycles::from_m(10_000));

        // Save the state
        let saved_state = original_gb.save_state()
            .expect("Failed to save state");

        // Create a new GameBoy and run it for different cycles
        let mut different_gb = GameBoy::dmg_hello_world();
        different_gb.run(MachineCycles::from_m(2000));

        // Load the saved state
        let mut loaded_gb = GameBoy::dmg_hello_world();
        loaded_gb.load_state(&saved_state).expect("Failed to load state");

        // Verify the loaded state matches the original
        assert_eq!(original_gb, loaded_gb);
    }

    mod blargg_cpu {
        use super::*;
        use crate::roms::blargg_cpu::*;

        #[test]
        fn cpu_01_special() {
            serial_console_test("cpu-01", SPECIAL_01);
        }

        #[test]
        fn cpu_02_interrupts() {
            serial_console_test("cpu-02", INTERRUPTS_02);
        }

        #[test]
        fn cpu_03_op_sp_hl() {
            serial_console_test("cpu-03", OP_SP_HL_03);
        }

        #[test]
        fn cpu_04_op_r_imm() {
            serial_console_test("cpu-04", OP_R_IMM_04);
        }

        #[test]
        fn cpu_05_op_rp() {
            serial_console_test("cpu-05", OP_RP_05);
        }

        #[test]
        fn cpu_06_ld_r_r() {
            serial_console_test("cpu-06", LD_R_R_06);
        }

        #[test]
        fn cpu_07_jr_jp_call_ret_rst() {
            serial_console_test("cpu-07", JR_JP_CALL_RET_RST_07);
        }

        #[test]
        fn cpu_08_misc_instrs() {
            serial_console_test("cpu-08", MISC_INSTRUCTIONS_08);
        }

        #[test]
        fn cpu_09_op_r_r() {
            serial_console_test("cpu-09", OP_R_R_09);
        }

        #[test]
        fn cpu_10_bit_ops() {
            serial_console_test("cpu-10", BIT_OPS_10);
        }

        #[test]
        fn cpu_11_op_a_hl() {
            serial_console_test("cpu-11", OP_A_HL_11);
        }

        /// The whole suite in one run, off the combined 64 KB ROM — so unlike the eleven
        /// sub-tests above, this one actually exercises MBC bank switching.
        ///
        /// It needs roughly **2.5x** the default budget: the sub-tests run back to back, and
        /// `11-op a,(hl)` alone is most of it. Under a 40M budget it stops after `10:ok  11` and
        /// looks exactly like a hang, which is what it was mistaken for once already.
        #[test]
        fn all() {
            serial_console_test_within("cpu-all", ROM, MachineCycles::from_m(60_000_000));
        }

        #[test]
        fn instruction_timing() {
            serial_console_test("instruction-timing", INSTRUCTION_TIMING);
        }
    }

    mod blargg_dmg_sound {
        use crate::roms::blargg_dmg_sound::*;
        use super::*;

        /// The whole suite in one run, off the combined 64 KB ROM — so unlike the twelve
        /// sub-tests below, this one exercises MBC bank switching. **It is D1's acceptance test.**
        ///
        /// Blargg's runner selects each sub-test by writing its index to the bank register. On the
        /// fourth write hardware *masks* `4` down to bank 0, where the runner's terminator lives;
        /// gb used to *clamp* it to 3 and re-run bank 3's test forever, printing `NN:ok` past 12
        /// and on past 99. D1 gave [`crate::mmu::MMU::set_rom_bank_register`] the per-mapper mask
        /// (this cartridge is MBC1, five bits; pokered's MBC3 needs seven) and this went green.
        ///
        /// The reference is gambatte's own frame. The budget is ~50 s of emulated time, which is
        /// what the ROM needs to finish.
        #[test]
        fn all() {
            ppu_test_within("audio-all", ROM, EXPECTED_ALL, MachineCycles::from_m(60_000_000));
        }

        #[test]
        fn registers() {
            ppu_test("audio-registers", REGISTERS, EXPECTED_REGISTERS);
        }

        #[test]
        fn length_counter() {
            ppu_test("audio-length-counter", LENGTH_COUNTER, EXPECTED_LENGTH_COUNTER);
        }

        #[test]
        fn trigger() {
            ppu_test("audio-trigger", TRIGGER, EXPECTED_TRIGGER);
        }

        #[test]
        fn sweep() {
            ppu_test("audio-sweep", SWEEP, EXPECTED_SWEEP);
        }

        #[test]
        fn sweep_details() {
            ppu_test("audio-sweep-details", SWEEP_DETAILS, EXPECTED_SWEEP_DETAILS);
        }

        #[test]
        fn overflow_on_trigger() {
            ppu_test("audio-overflow-on-trigger", OVERFLOW_ON_TRIGGER, EXPECTED_OVERFLOW_ON_TRIGGER);
        }

        #[test]
        fn length_sweep_period_sync() {
            ppu_test("audio-length-sweep-period-sync", LENGTH_SWEEP_PERIOD_SYNC, EXPECTED_LENGTH_SWEEP_PERIOD_SYNC);
        }

        #[test]
        fn length_counter_during_power() {
            ppu_test("audio-length-counter-during-power", LENGTH_COUNTER_DURING_POWER, EXPECTED_LENGTH_COUNTER_DURING_POWER);
        }

        /// A16. Reading wave RAM while channel 3 plays: `0xFF` unless the read lands on the exact
        /// tick the channel fetches its next sample.
        #[test]
        fn wave_read_while_on() {
            ppu_test("audio-wave-read-while-on", WAVE_READ_WHILE_ON, EXPECTED_WAVE_READ_WHILE_ON);
        }

        /// A16. Retriggering channel 3 one tick before a sample fetch corrupts the start of wave
        /// RAM — a DMG-only quirk.
        #[test]
        fn wave_trigger_while_on() {
            ppu_test("audio-wave-trigger-while-on", WAVE_TRIGGER_WHILE_ON, EXPECTED_WAVE_TRIGGER_WHILE_ON);
        }

        #[test]
        fn registers_after_power() {
            ppu_test("audio-registers-after-power", REGISTERS_AFTER_POWER, EXPECTED_REGISTERS_AFTER_POWER);
        }

        /// A16. Writing wave RAM while channel 3 plays, under the same one-tick aperture as
        /// [`wave_read_while_on`].
        #[test]
        fn wave_write_while_on() {
            ppu_test("audio-wave-write-while-on", WAVE_WRITE_WHILE_ON, EXPECTED_WAVE_WRITE_WHILE_ON);
        }

    }

    mod joypad {
        use crate::joypad::JoypadButton;
        use super::*;
        use crate::roms::button_test::*;

        #[test]
        fn button_a() {
            test_button(JoypadButton::A, EXPECTED_A);
        }

        #[test]
        fn button_b() {
            test_button(JoypadButton::B, EXPECTED_B);
        }

        #[test]
        fn button_select() {
            test_button(JoypadButton::Select, EXPECTED_SELECT);
        }

        #[test]
        fn button_start() {
            test_button(JoypadButton::Start, EXPECTED_START);
        }

        #[test]
        fn button_up() {
            test_button(JoypadButton::Up, EXPECTED_UP);
        }

        #[test]
        fn button_down() {
            test_button(JoypadButton::Down, EXPECTED_DOWN);
        }

        #[test]
        fn button_left() {
            test_button(JoypadButton::Left, EXPECTED_LEFT);
        }

        #[test]
        fn button_right() {
            test_button(JoypadButton::Right, EXPECTED_RIGHT);
        }

        fn test_button(button: JoypadButton, expected_screenshot: &[u8]) {
            let mut gb = GameBoy::dmg(ROM);
            gb.run(MachineCycles::from_m(400_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .press_button(button);

            gb.run(MachineCycles::from_m(20_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .release_button(button);

            gb.run(MachineCycles::from_m(20_000));

            let result = gb.core().mmu().ppu().screenshot();

            let expected_screenshot = parse_png(expected_screenshot);
            if result != expected_screenshot {
                gb_test_failed_with_screenshot(result, &format!("{}-button", button), "screenshot does not match");
            }
        }
    }



    mod ppu {
        use std::io::BufReader;
        use image::{ImageFormat, ImageReader};
        use crate::roms::acid::*;
        use super::*;

        #[test]
        fn ppu() {
            let mut gb = GameBoy::dmg(ROM);
            gb.run(MachineCycles::from_m(180_000));

            let result = gb.core().mmu().ppu().screenshot();
            let expected_image = ImageReader::with_format(BufReader::new(std::io::Cursor::new(EXPECTED_DMG)), ImageFormat::Png)
                .decode()
                .expect("Failed to decode expected image")
                .to_rgb8();

            if result != expected_image {
                gb_test_failed_with_screenshot(result, "ppu", "screenshot does not match");
            }
        }

        /// B10 — Phase B's acceptance test. `cgb-acid2` on a Game Boy Color, against the
        /// reference image that ships with the ROM (see [`crate::roms::cgb_acid`]).
        #[test]
        fn cgb_ppu() {
            cgb_ppu_test("cgb-acid2", crate::roms::cgb_acid::ROM, crate::roms::cgb_acid::EXPECTED);
        }

        /// ⭐ B5, the headline deliverable: Pokémon Red is a DMG-only cartridge, so on a Game Boy
        /// Color the **boot ROM** picks its palette from the title checksum. Booting it through
        /// [`GameBoy::cgb`] must therefore come out red-tinted rather than grey, and the same ROM
        /// through [`GameBoy::dmg`] must be untouched.
        ///
        /// The palette lookup itself is pinned in `boot_palette::tests`; what this adds is that
        /// it reaches the screen — the *same picture*, shade for shade, recoloured. Several
        /// checkpoints through the intro, because a single one can easily land on a fade frame
        /// that is entirely black or entirely white and prove nothing.
        #[test]
        fn pokemon_red_boots_in_colour_on_a_cgb() {
            use crate::lcd_palette::LcdColor;
            use crate::model::ColorMode;

            fn ramp(colors: [u16; 4]) -> [image::Rgb<u8>; 4] {
                colors.map(LcdColor::from_rgb555).map(|c| c.to_rgb())
            }
            // Combination 13: BG and OBJ1 from pool palette 4 (red), OBJ0 from palette 3 (green).
            let background = ramp([0x7FFF, 0x421F, 0x1CF2, 0x0000]);
            let object0 = ramp([0x7FFF, 0x1BEF, 0x0200, 0x0000]);
            let shades = [0xFFu8, 0xAA, 0x55, 0x00];

            let mut cgb = GameBoy::cgb(crate::pokemon::roms::POKERED);
            let mut dmg = GameBoy::dmg(crate::pokemon::roms::POKERED);
            assert_eq!(cgb.core().mmu().color_mode(), ColorMode::CgbCompat);
            assert_eq!(dmg.core().mmu().color_mode(), ColorMode::Dmg);

            let mut saw_colour = false;
            // The copyright screen, the Game Freak logo, and the Nidorino/Gengar intro.
            for checkpoint in [3_000_000u64, 8_000_000, 12_000_000] {
                let step = MachineCycles::from_m(checkpoint) - cgb_elapsed(checkpoint);
                cgb.run(step);
                dmg.run(step);
                let colour = cgb.core().mmu().ppu().screenshot();
                let grey = dmg.core().mmu().ppu().screenshot();

                for (x, y, pixel) in grey.enumerate_pixels() {
                    // The DMG path is untouched: still exactly gb's four greys.
                    assert_eq!(pixel.0[0], pixel.0[1], "DMG pixel ({x},{y}) is not grey");
                    let shade = shades.iter().position(|&s| s == pixel.0[0])
                        .unwrap_or_else(|| panic!("DMG pixel ({x},{y}) = {pixel:?} is not a gb shade"));

                    // ...and the CGB frame is the same shade, through the background ramp or —
                    // for a sprite pixel — the object ramp.
                    let actual = *colour.get_pixel(x, y);
                    assert!(
                        actual == background[shade] || actual == object0[shade],
                        "pixel ({x},{y}) at {checkpoint}M: shade {shade} rendered as {actual:?}, \
                         expected {:?} (BG) or {:?} (OBJ0)", background[shade], object0[shade]
                    );
                    saw_colour |= actual.0[0] != actual.0[1];
                }
            }
            assert!(saw_colour, "test is vacuous unless some CGB frame is actually coloured");
        }

        /// The checkpoints in [`pokemon_red_boots_in_colour_on_a_cgb`] are absolute, but `run`
        /// takes a delta; this turns one into the other. `run` overshoots by at most one
        /// instruction, and both machines execute the same instruction stream, so they stay in
        /// lockstep.
        fn cgb_elapsed(checkpoint: u64) -> MachineCycles {
            const CHECKPOINTS: [u64; 3] = [3_000_000, 8_000_000, 12_000_000];
            let index = CHECKPOINTS.iter().position(|&c| c == checkpoint).expect("a known checkpoint");
            MachineCycles::from_m(if index == 0 { 0 } else { CHECKPOINTS[index - 1] })
        }

        /// The same ROM on a **DMG**, where it is supposed to say so and stop. It is a
        /// CGB-exclusive cartridge (`0x143 = 0xC0`), and gb must not pretend otherwise: this
        /// asserts `GameBoy::dmg` still reports a DMG in `A` and does not silently gain colour.
        #[test]
        fn cgb_acid2_on_a_dmg_stays_monochrome() {
            use crate::model::ColorMode;

            let mut gb = GameBoy::dmg(crate::roms::cgb_acid::ROM);
            gb.run(MachineCycles::from_m(2_000_000));

            assert_eq!(gb.core().mmu().color_mode(), ColorMode::Dmg);
            let shades: std::collections::HashSet<_> =
                gb.core().mmu().ppu().screenshot().pixels().copied().collect();
            assert!(
                shades.iter().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]),
                "a DMG must only ever emit greys, got {shades:?}"
            );
        }
    }

    /// A14: blargg's remaining DMG suites. **All of these are expected to fail** — they exist to
    /// quantify gaps that `docs/compatibility/10-implementation-plan.md` has deliberately left
    /// open. Each `#[ignore]` reason names the blocker. Do not "fix" one without reading it.
    ///
    /// Three harnesses are in play, because these ROMs report differently:
    ///
    /// * `serial_console_test` — the four `mem_timing` ROMs, the only ones here that talk over the
    ///   link port. They give the most useful output of the lot: `01-read_timing` reports
    ///   `F0:2-3 FA:2-4 CB 46:2-3 …`, i.e. those reads take 3 M-cycles where hardware takes 2.
    /// * `ppu_test` — the five *combined* suite ROMs, which write to the screen and for which
    ///   `c-sp/game-boy-test-roms` v7.0 ships a hardware reference image.
    /// * `screenshot_pending` — the individual sub-ROMs, which write to the screen and have **no**
    ///   reference image. See that function for how to promote one.
    ///
    /// ```text
    /// cargo test --release --bin gb -- game_boy::tests::blargg_timing --ignored
    /// cargo test --release --bin gb -- game_boy::tests::blargg_oam_bug --ignored
    /// ```
    mod blargg_timing {
        use super::*;
        use crate::roms::blargg_timing::*;

        // WON'T FIX until the M-cycle timing refactor happens. Plan §0.2 defers it explicitly
        // and forbids starting it, so there is no task ID to point at — it is a standing gap, and
        // these ROMs are how we measure it.

        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn mem_timing() { ppu_test("mem_timing", MEM_TIMING, EXPECTED_MEM_TIMING); }
        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn mem_timing_01_read() { serial_console_test("mem_timing-01", MEM_TIMING_READ); }
        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn mem_timing_02_write() { serial_console_test("mem_timing-02", MEM_TIMING_WRITE); }
        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn mem_timing_03_modify() { serial_console_test("mem_timing-03", MEM_TIMING_MODIFY); }

        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn mem_timing_2() { ppu_test("mem_timing-2", MEM_TIMING_2, EXPECTED_MEM_TIMING_2); }
        // No reference image ships for the sub-ROMs; promote a dump once M-cycle timing lands.
        #[test] #[ignore = "expected failure: needs M-cycle timing (plan §0.2); no reference image yet"]
        fn mem_timing_2_01_read() { screenshot_pending("mem_timing-2-01", MEM_TIMING_2_READ); }
        #[test] #[ignore = "expected failure: needs M-cycle timing (plan §0.2); no reference image yet"]
        fn mem_timing_2_02_write() { screenshot_pending("mem_timing-2-02", MEM_TIMING_2_WRITE); }
        #[test] #[ignore = "expected failure: needs M-cycle timing (plan §0.2); no reference image yet"]
        fn mem_timing_2_03_modify() { screenshot_pending("mem_timing-2-03", MEM_TIMING_2_MODIFY); }

        #[test] #[ignore = "expected failure: needs M-cycle timing, deferred by plan §0.2"]
        fn halt_bug() { ppu_test("halt_bug", HALT_BUG, EXPECTED_HALT_BUG); }
        /// **This one passes**, so it is not ignored.
        ///
        /// Read the reference image before being surprised: `interrupt_time-dmg.png` shows
        /// `Failed` and the checksum `7F8F4AAF`. That is what *real DMG hardware* prints — the
        /// ROM targets CGB double-speed behaviour and legitimately fails on DMG. Matching it
        /// byte for byte, checksum included, is therefore a pass, and a fairly strong one.
        #[test]
        fn interrupt_time() { ppu_test("interrupt_time", INTERRUPT_TIME, EXPECTED_INTERRUPT_TIME); }
    }

    /// A14: the DMG OAM corruption quirk. **WON'T FIX** — the plan does not schedule it in any
    /// phase, and gambatte does not model it either, so there is nothing to compare against.
    /// Kept because they are the only measurement of that gap we have.
    mod blargg_oam_bug {
        use super::*;
        use crate::roms::blargg_oam_bug::*;

        #[test] #[ignore = "won't fix: DMG OAM corruption quirk, unscheduled in the plan; gambatte does not model it either"]
        fn oam_bug() { ppu_test("oam_bug", ROM, EXPECTED); }

        // No reference image ships for the sub-ROMs; promote a dump only if the quirk is ever done.
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_1_lcd_sync() { screenshot_pending("oam_bug-1", LCD_SYNC); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_2_causes() { screenshot_pending("oam_bug-2", CAUSES); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_3_non_causes() { screenshot_pending("oam_bug-3", NON_CAUSES); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_4_scanline_timing() { screenshot_pending("oam_bug-4", SCANLINE_TIMING); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_5_timing_bug() { screenshot_pending("oam_bug-5", TIMING_BUG); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_6_timing_no_bug() { screenshot_pending("oam_bug-6", TIMING_NO_BUG); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_7_timing_effect() { screenshot_pending("oam_bug-7", TIMING_EFFECT); }
        #[test] #[ignore = "won't fix: DMG OAM corruption quirk (unscheduled); no reference image yet"]
        fn oam_bug_8_instr_effect() { screenshot_pending("oam_bug-8", INSTR_EFFECT); }
    }

    fn serial_console_test(name: &str, cart: &[u8]) {
        serial_console_test_within(name, cart, MachineCycles::from_m(25_000_000));
    }

    /// [`serial_console_test`] with an explicit cycle budget, for ROMs that need longer than the
    /// default. Only a *failing* run spends the whole budget — a pass returns as soon as the ROM
    /// says so.
    fn serial_console_test_within(name: &str, cart: &[u8], budget: MachineCycles) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = budget;
        let mut cycles = MachineCycles::ZERO;
        let mut serial_output = String::new();
        let mut failed = false;
        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::from_m(1000));

            serial_output = gb.core.mmu().serial()
                .buffered_bytes()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();

            if serial_output.contains("Passed") {
                return;
            } else if !failed && serial_output.contains("Failed") {
                // Run for a few more cycles to collect more output
                max_cycles = cycles + MachineCycles::from_m(10_000);
                failed = true;
            }
        }

        gb_test_failed(&gb, name, &serial_output);
    }


    fn ppu_test(name: &str, cart: &[u8], expected_screenshot: &[u8]) {
        ppu_test_within(name, cart, expected_screenshot, MachineCycles::from_m(20_000_000));
    }

    /// [`ppu_test`] on a Game Boy Color.
    fn cgb_ppu_test(name: &str, cart: &[u8], expected_screenshot: &[u8]) {
        let expected_screenshot = parse_png(expected_screenshot);
        let mut gb = GameBoy::cgb(cart);
        let mut cycles = MachineCycles::ZERO;
        let mut last_screenshot = gb.core().mmu().ppu().screenshot();

        while cycles < MachineCycles::from_m(20_000_000) {
            cycles += gb.run(MachineCycles::from_m(1000));
            last_screenshot = gb.core().mmu().ppu().screenshot();
            if last_screenshot == expected_screenshot {
                return;
            }
        }
        gb_test_failed_with_screenshot(last_screenshot, name, "screenshot does not match");
    }

    /// [`ppu_test`] with an explicit cycle budget, for ROMs that need longer than the default.
    /// Only the budget for a *failing* run costs anything — a match returns immediately.
    fn ppu_test_within(name: &str, cart: &[u8], expected_screenshot: &[u8], max_cycles: MachineCycles) {
        let expected_screenshot = parse_png(expected_screenshot);
        let mut gb = GameBoy::dmg(cart);
        let mut cycles = MachineCycles::ZERO;
        let mut last_screenshot = gb.core().mmu().ppu().screenshot();

        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::from_m(1000));
            last_screenshot = gb.core().mmu().ppu().screenshot();

            if last_screenshot == expected_screenshot {
                return;
            }
        }

        gb_test_failed_with_screenshot(last_screenshot, name, "screenshot does not match");
    }

    /// For a screen-output ROM that has **no reference image yet**. Runs it, dumps what the screen
    /// actually shows to `target/test_failure_<name>.png`, and always fails.
    ///
    /// That dump is the raw material for a reference: once the feature the ROM covers is actually
    /// implemented, run this, open the PNG, and if it reads as a pass (blargg prints `Passed`, or
    /// per-test `OK`), move it into `src/roms/…` and switch the test to [`ppu_test`]. Promoting a
    /// screenshot **before** the feature works would just freeze our own wrong output as the
    /// expectation, so don't.
    fn screenshot_pending(name: &str, cart: &[u8]) {
        let mut gb = GameBoy::dmg(cart);
        gb.run(MachineCycles::from_m(20_000_000));
        gb_test_failed_with_screenshot(
            gb.core().mmu().ppu().screenshot(),
            name,
            "no reference screenshot yet — inspect the dump, and promote it only once the feature \
             under test is implemented (see the test's #[ignore] reason)",
        );
    }

    fn gb_test_failed(gb: &GameBoy, name: &str, reason: &str) {
        let image = gb.core().mmu().ppu().screenshot();
        gb_test_failed_with_screenshot(image, name, reason);
    }

    fn gb_test_failed_with_screenshot(image: RgbImage, name: &str, reason: &str) {
        let result_path = &format!("target/test_failure_{}.png", name);
        image.save(result_path).expect("Failed to save result image");
        panic!("{} test failed, saved result image to {}, reason: {}", name, result_path, reason);
    }
}