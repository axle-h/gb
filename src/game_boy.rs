use crate::core::Core;
use crate::cycles::MachineCycles;
use crate::savestate::{SectionReader, SectionWriter};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GameBoy {
    core: Core
}

impl GameBoy {
    pub fn dmg(cart: &[u8]) -> Self {
        Self {
            core: Core::dmg(cart)
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
            let opcode = self.core.fetch();
            cycles += self.core.execute(opcode);
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
    /// cargo test --release --bin gb -- game_boy::tests::bench_core_throughput --exact --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "benchmark"]
    fn bench_core_throughput() {
        use std::time::Instant;

        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const WARM_UP_FRAMES: usize = 60;
        const MEASURED_FRAMES: usize = 600;

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
            let mut gb = build();
            for _ in 0..WARM_UP_FRAMES {
                gb.run(FRAME);
            }

            let start = Instant::now();
            let mut cycles = MachineCycles::ZERO;
            for _ in 0..MEASURED_FRAMES {
                cycles += gb.run(FRAME);
            }
            let elapsed = start.elapsed();

            let t_cycles_per_sec = cycles.t_cycles() as f64 / elapsed.as_secs_f64();
            let realtime = t_cycles_per_sec / MachineCycles::CPU_FREQ as f64;
            println!(
                "{name:<24} {realtime:>11.1}x {:>16.0} {MEASURED_FRAMES:>10}",
                t_cycles_per_sec
            );
        }
        println!();
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

        #[test]
        fn instruction_timing() {
            serial_console_test("instruction-timing", INSTRUCTION_TIMING);
        }
    }

    mod blargg_dmg_sound {
        use crate::roms::blargg_dmg_sound::*;
        use super::*;

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
/*
        #[test]
        fn wave_read_while_on() {
            ppu_test("audio-wave-read-while-on", WAVE_READ_WHILE_ON, EXPECTED_WAVE_READ_WHILE_ON);
        }

        #[test]
        fn wave_trigger_while_on() {
            ppu_test("audio-wave-trigger-while-on", WAVE_TRIGGER_WHILE_ON, EXPECTED_WAVE_TRIGGER_WHILE_ON);
        }
*/
        #[test]
        fn registers_after_power() {
            ppu_test("audio-registers-after-power", REGISTERS_AFTER_POWER, EXPECTED_REGISTERS_AFTER_POWER);
        }
/*
        #[test]
        fn wave_write_while_on() {
            ppu_test("audio-wave-write-while-on", WAVE_WRITE_WHILE_ON, EXPECTED_WAVE_WRITE_WHILE_ON);
        }
*/
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
    }

    /// A14: blargg's remaining DMG suites. **These are expected to fail** — they measure the gap
    /// left by advancing peripherals once per instruction instead of per M-cycle, which
    /// `docs/compatibility/10-implementation-plan.md` explicitly defers. They are documentation,
    /// not a target: do not "fix" them without reading that plan first.
    ///
    /// **Only the four `mem_timing` ROMs actually report over serial.** They fail with useful
    /// per-opcode detail — e.g. `01-read_timing` reports `F0:2-3 FA:2-4 CB 46:2-3 ...`, meaning
    /// those reads take 3 M-cycles where hardware takes 2. That *is* the instruction-granularity
    /// gap, named instruction by instruction.
    ///
    /// The other six here, and all nine in [`blargg_oam_bug`], emit **nothing** over serial —
    /// they write their results to the screen. `serial_console_test` cannot score them, so they
    /// currently fail for a harness reason rather than a fidelity one. Task A15 covers that.
    ///
    /// Run them with:
    /// ```text
    /// cargo test --release --bin gb -- game_boy::tests::blargg_timing --ignored
    /// cargo test --release --bin gb -- game_boy::tests::blargg_oam_bug --ignored
    /// ```
    mod blargg_timing {
        use super::*;
        use crate::roms::blargg_timing::*;

        // BLOCKED ON: M-cycle timing (deferred — plan §0.2).
        #[test] #[ignore = "needs M-cycle timing"] fn mem_timing() { serial_console_test("mem_timing", MEM_TIMING); }
        #[test] #[ignore = "needs M-cycle timing"] fn mem_timing_01_read() { serial_console_test("mem_timing-01", MEM_TIMING_READ); }
        #[test] #[ignore = "needs M-cycle timing"] fn mem_timing_02_write() { serial_console_test("mem_timing-02", MEM_TIMING_WRITE); }
        #[test] #[ignore = "needs M-cycle timing"] fn mem_timing_03_modify() { serial_console_test("mem_timing-03", MEM_TIMING_MODIFY); }

        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn mem_timing_2() { serial_console_test("mem_timing-2", MEM_TIMING_2); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn mem_timing_2_01_read() { serial_console_test("mem_timing-2-01", MEM_TIMING_2_READ); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn mem_timing_2_02_write() { serial_console_test("mem_timing-2-02", MEM_TIMING_2_WRITE); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn mem_timing_2_03_modify() { serial_console_test("mem_timing-2-03", MEM_TIMING_2_MODIFY); }

        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn halt_bug() { serial_console_test("halt_bug", HALT_BUG); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15)"] fn interrupt_time() { serial_console_test("interrupt_time", INTERRUPT_TIME); }
    }

    /// A14: the DMG OAM corruption quirk, which `gb` does not model — and neither does gambatte.
    /// Out of scope for this plan entirely.
    mod blargg_oam_bug {
        use super::*;
        use crate::roms::blargg_oam_bug::*;

        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug() { serial_console_test("oam_bug", ROM); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_1_lcd_sync() { serial_console_test("oam_bug-1", LCD_SYNC); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_2_causes() { serial_console_test("oam_bug-2", CAUSES); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_3_non_causes() { serial_console_test("oam_bug-3", NON_CAUSES); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_4_scanline_timing() { serial_console_test("oam_bug-4", SCANLINE_TIMING); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_5_timing_bug() { serial_console_test("oam_bug-5", TIMING_BUG); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_6_timing_no_bug() { serial_console_test("oam_bug-6", TIMING_NO_BUG); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_7_timing_effect() { serial_console_test("oam_bug-7", TIMING_EFFECT); }
        #[test] #[ignore = "no serial output — needs a screen-based harness (A15); quirk also not modelled"] fn oam_bug_8_instr_effect() { serial_console_test("oam_bug-8", INSTR_EFFECT); }
    }

    fn serial_console_test(name: &str, cart: &[u8]) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = MachineCycles::from_m(25_000_000);
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
        let expected_screenshot = parse_png(expected_screenshot);
        let mut gb = GameBoy::dmg(cart);
        let max_cycles = MachineCycles::from_m(20_000_000);
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