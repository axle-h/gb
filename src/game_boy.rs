use std::time::Duration;
use crate::core::Core;
use crate::cycles::MachineCycles;

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

    pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
        let mut cycles = MachineCycles::ZERO;
        while cycles < min_cycles {
            let opcode = self.core.fetch();
            cycles += self.core.execute(opcode);
        }
        cycles
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use image::{ImageFormat, ImageReader, RgbImage};
    use crate::roms::roms::parse_png;
    use super::*;

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
/*
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

        #[test]
        fn wave_read_while_on() {
            ppu_test("audio-wave-read-while-on", WAVE_READ_WHILE_ON, EXPECTED_WAVE_READ_WHILE_ON);
        }

        #[test]
        fn wave_trigger_while_on() {
            ppu_test("audio-wave-trigger-while-on", WAVE_TRIGGER_WHILE_ON, EXPECTED_WAVE_TRIGGER_WHILE_ON);
        }

        #[test]
        fn registers_after_power() {
            ppu_test("audio-registers-after-power", REGISTERS_AFTER_POWER, EXPECTED_REGISTERS_AFTER_POWER);
        }

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

    fn serial_console_test(name: &str, cart: &[u8]) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = MachineCycles::from_m(1_000_000);
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