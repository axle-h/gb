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
        Self::dmg(crate::roms::test::DMG_ACID)
    }

    pub fn update(&mut self, delta: Duration) {
        let delta_cycles = MachineCycles::of_real_time(delta.min(Duration::from_micros(33333)));
        self.run(delta_cycles);
    }

    pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
        let mut cycles = MachineCycles::ZERO;
        while cycles < min_cycles {
            let opcode = self.core.fetch();
            cycles += self.core.execute(opcode);
            cycles += self.core.handle_interrupts();
        }
        cycles
    }

    pub fn core(&self) -> &Core {
        &self.core
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_test(cart: &[u8]) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = MachineCycles::of_machine(100_000_000);
        let mut cycles = MachineCycles::ZERO;
        let mut output = String::new();
        let mut failed = false;
        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::of_machine(1000));

            output = gb.core.mmu().serial()
                .buffered_bytes()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();

            if output.contains("Passed") {
                break;
            } else if !failed && output.contains("Failed") {
                // Run for a few more cycles to collect more output
                max_cycles = cycles + MachineCycles::of_machine(10_000);
                failed = true;
            }
        }

        assert!(output.contains("Passed"), "Test failed with output: {}", output);
    }

    mod blargg {
        use super::*;
        use crate::roms::blarg::*;

        #[test]
        fn cpu() {
            // this slows down the test but I think it is necessary as it fails in different ways to the individual roms
            rom_test(CPU_INSTRUCTIONS);
        }

        #[test]
        fn cpu_01_special() {
            rom_test(CPU_INSTRUCTIONS_01);
        }

        #[test]
        fn cpu_02_interrupts() {
            rom_test(CPU_INSTRUCTIONS_02);
        }

        #[test]
        fn cpu_03_op_sp_hl() {
            rom_test(CPU_INSTRUCTIONS_03);
        }

        #[test]
        fn cpu_04_op_r_imm() {
            rom_test(CPU_INSTRUCTIONS_04);
        }

        #[test]
        fn cpu_05_op_rp() {
            rom_test(CPU_INSTRUCTIONS_05);
        }

        #[test]
        fn cpu_06_ld_r_r() {
            rom_test(CPU_INSTRUCTIONS_06);
        }

        #[test]
        fn cpu_07_jr_jp_call_ret_rst() {
            rom_test(CPU_INSTRUCTIONS_07);
        }

        #[test]
        fn cpu_08_misc_instrs() {
            rom_test(CPU_INSTRUCTIONS_08);
        }

        #[test]
        fn cpu_09_op_r_r() {
            rom_test(CPU_INSTRUCTIONS_09);
        }

        #[test]
        fn cpu_10_bit_ops() {
            rom_test(CPU_INSTRUCTIONS_10);
        }

        #[test]
        fn cpu_11_op_a_hl() {
            rom_test(CPU_INSTRUCTIONS_11);
        }

        #[test]
        fn instruction_timing() {
            rom_test(INSTRUCTION_TIMING);
        }
    }
}