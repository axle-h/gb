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
    use super::*;

    fn serial_console_test(cart: &[u8]) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = MachineCycles::from_m(100_000_000);
        let mut cycles = MachineCycles::ZERO;
        let mut output = String::new();
        let mut failed = false;
        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::from_m(1000));

            output = gb.core.mmu().serial()
                .buffered_bytes()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();

            if output.contains("Passed") {
                return;
            } else if !failed && output.contains("Failed") {
                // Run for a few more cycles to collect more output
                max_cycles = cycles + MachineCycles::from_m(10_000);
                failed = true;
            }
        }

        panic!("Test failed with output: {}", output);
    }

    mod blargg {
        use super::*;
        use crate::roms::blarg::*;

        #[test]
        fn cpu() {
            // this slows down the test but I think it is necessary as it fails in different ways to the individual roms
            serial_console_test(CPU_INSTRUCTIONS);
        }

        #[test]
        fn cpu_01_special() {
            serial_console_test(CPU_INSTRUCTIONS_01);
        }

        #[test]
        fn cpu_02_interrupts() {
            serial_console_test(CPU_INSTRUCTIONS_02);
        }

        #[test]
        fn cpu_03_op_sp_hl() {
            serial_console_test(CPU_INSTRUCTIONS_03);
        }

        #[test]
        fn cpu_04_op_r_imm() {
            serial_console_test(CPU_INSTRUCTIONS_04);
        }

        #[test]
        fn cpu_05_op_rp() {
            serial_console_test(CPU_INSTRUCTIONS_05);
        }

        #[test]
        fn cpu_06_ld_r_r() {
            serial_console_test(CPU_INSTRUCTIONS_06);
        }

        #[test]
        fn cpu_07_jr_jp_call_ret_rst() {
            serial_console_test(CPU_INSTRUCTIONS_07);
        }

        #[test]
        fn cpu_08_misc_instrs() {
            serial_console_test(CPU_INSTRUCTIONS_08);
        }

        #[test]
        fn cpu_09_op_r_r() {
            serial_console_test(CPU_INSTRUCTIONS_09);
        }

        #[test]
        fn cpu_10_bit_ops() {
            serial_console_test(CPU_INSTRUCTIONS_10);
        }

        #[test]
        fn cpu_11_op_a_hl() {
            serial_console_test(CPU_INSTRUCTIONS_11);
        }

        #[test]
        fn instruction_timing() {
            serial_console_test(INSTRUCTION_TIMING);
        }
    }

    mod joypad {
        use std::io::BufReader;
        use image::{ImageFormat, ImageReader};
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

        fn test_button(button: JoypadButton, expected_image: &[u8]) {
            let mut gb = GameBoy::dmg(ROM);
            gb.run(MachineCycles::from_m(400_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .press_button(button);

            gb.run(MachineCycles::from_m(20_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .release_button(button);

            gb.run(MachineCycles::from_m(20_000));

            let result = gb.core().mmu().ppu().screenshot();

            let expected_image = ImageReader::with_format(BufReader::new(std::io::Cursor::new(expected_image)), ImageFormat::Png)
                .decode()
                .expect("Failed to decode expected image")
                .to_rgb8();

            if result != expected_image {
                let result_path = format!("target/button_test_result_{}.png", button);
                result.save(result_path.clone()).expect("Failed to save result image");
                panic!("Test failed, saved result image to {}", result_path);
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
                let result_path = "target/ppu_test_result.png";
                result.save(result_path).expect("Failed to save result image");
                panic!("PPU test failed, saved result image to {}", result_path);
            }
        }
    }
}