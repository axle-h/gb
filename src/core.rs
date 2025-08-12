use std::time::Duration;
use crate::mmu::MMU;
use crate::opcode::{OpCode, Register, Register16, Register16Mem, Register16Stack, JumpCondition};
use crate::registers::RegisterSet;
use crate::roms::test::DMG_ACID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreMode {
    Normal,
    Halt,
    Stop,
    Crash,
}

pub struct Core {
    registers: RegisterSet,
    mmu: MMU,
    interrupts_enabled: bool,
    machine_cycles: u64,
    mode: CoreMode
}

impl Core {
    pub fn dmg_hello_world() -> Self {
        Self {
            registers: RegisterSet::dmg(),
            mmu: MMU::from_rom(DMG_ACID).expect("could not load DMG_ACID ROM"),
            interrupts_enabled: false,
            machine_cycles: 0,
            mode: CoreMode::Normal,
        }
    }

    fn register(&self, register: Register) -> u8 {
        use Register::*;
        match register {
            B => self.registers.b,
            C => self.registers.c,
            D => self.registers.d,
            E => self.registers.e,
            H => self.registers.h,
            L => self.registers.l,
            mHL => self.mmu.read(self.registers.hl()),
            A => self.registers.a,
        }
    }

    fn set_register(&mut self, register: Register, value: u8) {
        use Register::*;
        match register {
            B => self.registers.b = value,
            C => self.registers.c = value,
            D => self.registers.d = value,
            E => self.registers.e = value,
            H => self.registers.h = value,
            L => self.registers.l = value,
            mHL => self.mmu.write(self.registers.hl(), value),
            A => self.registers.a = value,
        }
    }

    fn register16(&self, register: Register16) -> u16 {
        use Register16::*;
        match register {
            BC => self.registers.bc(),
            DE => self.registers.de(),
            HL => self.registers.hl(),
            SP => self.registers.sp,
        }
    }

    fn write_register16(&mut self, register: Register16, value: u16) {
        use Register16::*;
        match register {
            BC => self.registers.set_bc(value),
            DE => self.registers.set_de(value),
            HL => self.registers.set_hl(value),
            SP => self.registers.sp = value,
        }
    }

    fn register16_mem(&mut self, register: Register16Mem) -> u8 {
        use Register16Mem::*;
        let address = match register {
            BC => self.registers.bc(),
            DE => self.registers.de(),
            HLIncrement => self.registers.hl_increment(),
            HLDecrement => self.registers.hl_decrement(),
        };
        self.mmu.read(address)
    }

    fn write_register16_mem(&mut self, register: Register16Mem, value: u8) {
        use Register16Mem::*;
        let address = match register {
            BC => self.registers.bc(),
            DE => self.registers.de(),
            HLIncrement => self.registers.hl_increment(),
            HLDecrement => self.registers.hl_decrement(),
        };
        self.mmu.write(address, value);
    }

    fn register16_stack(&self, register: Register16Stack) -> u16 {
        use Register16Stack::*;
        match register {
            BC => self.registers.bc(),
            DE => self.registers.de(),
            HL => self.registers.hl(),
            AF => self.registers.af(),
        }
    }

    fn write_register16_stack(&mut self, register: Register16Stack, value: u16) {
        use Register16Stack::*;
        match register {
            BC => self.registers.set_bc(value),
            DE => self.registers.set_de(value),
            HL => self.registers.set_hl(value),
            AF => self.registers.set_af(value),
        }
    }

    /// update all internal state
    pub fn update(&mut self, delta: Duration) {
        self.mmu.update(delta);
    }

    pub fn fetch(&mut self) -> OpCode {
        if self.mode == CoreMode::Normal {
            OpCode::parse(self)
        } else {
            OpCode::Nop
        }
    }

    pub fn execute(&mut self, opcode: OpCode) {
        if self.mode != CoreMode::Normal {
            return;
        }

        match opcode {
            OpCode::Load { source, destination } => {
                self.set_register(destination, self.register(source));
            }
            OpCode::LoadImmediate { register, value } => {
                self.set_register(register, value);
            }
            OpCode::LoadIndirectAccumulator { register } => {
                self.write_register16_mem(register, self.registers.a);
            }
            OpCode::LoadAccumulatorIndirect { register } => {
                self.registers.a = self.register16_mem(register);
            }
            OpCode::LoadAccumulatorDirect { address } => {
                self.registers.a = self.mmu.read(address);
            }
            OpCode::LoadDirectAccumulator { address } => {
                self.mmu.write(address, self.registers.a);
            }
            OpCode::LoadHighAccumulatorIndirect => {
                let address = 0xFF00 | (self.registers.c as u16);
                self.registers.a = self.mmu.read(address);
            }
            OpCode::LoadHighIndirectAccumulator => {
                let address = 0xFF00 | (self.registers.c as u16);
                self.mmu.write(address, self.registers.a);
            }
            OpCode::LoadHighDirectAccumulator { lsb } => {
                let address = 0xFF00 | (lsb as u16);
                self.mmu.write(address, self.registers.a);
            }
            OpCode::LoadHighAccumulatorDirect { lsb } => {
                let address = 0xFF00 | (lsb as u16);
                self.registers.a = self.mmu.read(address);
            }
            OpCode::Load16Immediate { register, value } => {
                self.write_register16(register, value);
            }
            OpCode::LoadDirectStackPointer { address } => {
                self.mmu.write_u16_le(address, self.registers.sp);
            }
            OpCode::LoadStackPointerHL => {
                self.registers.sp = self.registers.hl();
            }
            OpCode::Push { register } => {
                self.push_stack(self.register16_stack(register));
            }
            OpCode::Pop { register } => {
                let value = self.pop_stack();
                self.write_register16_stack(register, value);
            }
            OpCode::LoadHLAdjustedStackPointer { offset } => {
                let adjusted_sp =self.alu_add_displacement(self.registers.sp, offset);
                self.registers.set_hl(adjusted_sp);
            }
            OpCode::Add { register } => {
                self.registers.a = self.alu_add(self.register(register), false);
            }
            OpCode::AddImmediate { value } => {
                self.registers.a = self.alu_add(value, false);
            }
            OpCode::AddWithCarry { register } => {
                self.registers.a = self.alu_add(self.register(register), true);
            }
            OpCode::AddWithCarryImmediate { value } => {
                self.registers.a = self.alu_add(value, true);
            }
            OpCode::Subtract { register } => {
                self.registers.a = self.alu_subtract(self.register(register), false);
            }
            OpCode::SubtractImmediate { value } => {
                self.registers.a = self.alu_subtract(value, false);
            }
            OpCode::SubtractWithCarry { register } => {
                self.registers.a = self.alu_subtract(self.register(register), true);
            }
            OpCode::SubtractWithCarryImmediate { value } => {
                self.registers.a = self.alu_subtract(value, true);
            }
            OpCode::Compare { register } => {
                self.alu_subtract(self.register(register), false);
            }
            OpCode::CompareImmediate { value } => {
                self.alu_subtract(value, false);
            }
            OpCode::Increment { register } => {
                let value = self.register(register);
                let result = self.alu_increment(value);
                self.set_register(register, result);
            }
            OpCode::Decrement { register } => {
                let value = self.register(register);
                let result = self.alu_decrement(value);
                self.set_register(register, result);
            }
            OpCode::And { register } => {
                let value = self.register(register);
                self.registers.a = self.alu_and(value);
            }
            OpCode::AndImmediate { value } => {
                self.registers.a = self.alu_and(value);
            }
            OpCode::Or { register } => {
                let value = self.register(register);
                self.registers.a = self.alu_or(value);
            }
            OpCode::OrImmediate { value } => {
                self.registers.a = self.alu_or(value);
            }
            OpCode::Xor { register } => {
                let value = self.register(register);
                self.registers.a = self.alu_xor(value);
            }
            OpCode::XorImmediate { value } => {
                self.registers.a = self.alu_xor(value);
            }
            OpCode::ComplementCarryFlag => {
                self.registers.flags.c = !self.registers.flags.c;
                self.registers.flags.n = false;
                self.registers.flags.h = false;
            }
            OpCode::SetCarryFlag => {
                self.registers.flags.c = true;
                self.registers.flags.n = false;
                self.registers.flags.h = false;
            }
            OpCode::DecimalAdjustAccumulator => {
                let mut result = self.registers.a as u16;
                if !self.registers.flags.n {
                    if self.registers.flags.h || (result & 0x0F) > 9 {
                        result = result.wrapping_add(0x06);
                    }
                    if self.registers.flags.c || result > 0x9F {
                        result = result.wrapping_add(0x60);
                    }
                } else {
                    if self.registers.flags.h {
                        result = result.wrapping_sub(0x06);
                    }
                    if self.registers.flags.c {
                        result = result.wrapping_sub(0x60);
                    }
                }

                self.registers.a = result as u8;
                self.registers.flags.z = self.registers.a == 0;
                self.registers.flags.h = false;
                self.registers.flags.c = result & 0x100 > 0;
            }
            OpCode::ComplementAccumulator => {
                self.registers.a = !self.registers.a;
                self.registers.flags.n = true;
                self.registers.flags.h = true;
            }
            OpCode::Increment16 { register } => {
                let value = self.register16(register);
                let result = value.wrapping_add(1);
                self.write_register16(register, result);
                // no flags are set
            }
            OpCode::Decrement16 { register } => {
                let value = self.register16(register);
                let result = value.wrapping_sub(1);
                self.write_register16(register, result);
                // no flags are set
            }
            OpCode::Add16 { register } => {
                let value = self.register16(register) as u32;
                let hl = self.registers.hl() as u32;
                let result = hl.wrapping_add(value);
                let carry_bits = hl ^ value ^ result;
                self.registers.flags.h = carry_bits & 0x1000 > 0;
                self.registers.flags.c = carry_bits & 0x10000 > 0;
                self.registers.set_hl(result as u16);
                self.registers.flags.n = false;
            }
            OpCode::AddStackPointer { offset } => {
                self.registers.sp = self.alu_add_displacement(self.registers.sp, offset);
            }
            OpCode::RotateLeftCircularAccumulator => {
                self.registers.a = self.alu_rotate_left(self.registers.a, true, false);
            }
            OpCode::RotateRightCircularAccumulator => {
                self.registers.a = self.alu_rotate_right(self.registers.a, true, false);
            }
            OpCode::RotateLeftAccumulator => {
                self.registers.a = self.alu_rotate_left(self.registers.a, false, false);
            }
            OpCode::RotateRightAccumulator => {
                self.registers.a = self.alu_rotate_right(self.registers.a, false, false);
            }
            OpCode::RotateLeftCircular { register } => {
                let value = self.register(register);
                let result = self.alu_rotate_left(value, true, true);
                self.set_register(register, result);
            }
            OpCode::RotateRightCircular { register } => {
                let value = self.register(register);
                let result = self.alu_rotate_right(value, true, true);
                self.set_register(register, result);
            }
            OpCode::RotateLeft { register } => {
                let value = self.register(register);
                let result = self.alu_rotate_left(value, false, true);
                self.set_register(register, result);
            }
            OpCode::RotateRight { register } => {
                let value = self.register(register);
                let result = self.alu_rotate_right(value, false, true);
                self.set_register(register, result);
            }
            OpCode::ShiftLeftArithmetic { register } => {
                self.registers.flags.c = false;
                let value = self.register(register);
                let result = self.alu_rotate_left(value, false, true);
                self.set_register(register, result);
            }
            OpCode::ShiftRightArithmetic { register } => {
                self.registers.flags.c = false;
                let value = self.register(register);
                let result = self.alu_rotate_right(value, false, true);
                self.set_register(register, result);
            }
            OpCode::Swap { register } => {
                let value = self.register(register);
                let high_nibble = (value & 0xF0) >> 4;
                let low_nibble = (value & 0x0F) << 4;
                let result = high_nibble | low_nibble;
                self.set_register(register, result);
                self.registers.flags.z = result == 0;
                self.registers.flags.n = false;
                self.registers.flags.h = false;
                self.registers.flags.c = false;
            }
            OpCode::ShiftRightLogical { register } => {
                self.registers.flags.c = false;
                let value = self.register(register);
                let result = self.alu_rotate_right(value, false, true);
                self.set_register(register, result);
            }
            OpCode::TestBit { register, bit } => {
                let value = self.register(register);
                let bit = (value >> bit) & 0x01;
                self.registers.flags.z = bit == 0;
                self.registers.flags.n = false;
                self.registers.flags.h = true;
            }
            OpCode::ResetBit { register, bit } => {
                let value = self.register(register);
                let result = value & !(1 << bit);
                self.set_register(register, result);
            }
            OpCode::SetBit { register, bit } => {
                let value = self.register(register);
                let result = value | (1 << bit);
                self.set_register(register, result);
            }
            OpCode::Jump { address } => {
                self.registers.pc = address;
            }
            OpCode::JumpHL => {
                self.registers.pc = self.registers.hl();
            }
            OpCode::JumpConditional { condition, address } => {
                if self.condition_met(condition) {
                    self.registers.pc = address;
                }
            }
            OpCode::JumpRelative { offset } => {
                self.registers.pc = self.registers.pc.wrapping_add_signed(offset as i16);
            }
            OpCode::JumpRelativeConditional { condition, offset } => {
                if self.condition_met(condition) {
                    self.registers.pc = self.registers.pc.wrapping_add_signed(offset as i16);
                }
            }
            OpCode::Call { address } => {
                self.push_stack(self.registers.pc);
                self.registers.pc = address;
            }
            OpCode::CallConditional { condition, address } => {
                if self.condition_met(condition) {
                    self.push_stack(self.registers.pc);
                    self.registers.pc = address;
                }
            }
            OpCode::Return => {
                self.registers.pc = self.pop_stack();
            }
            OpCode::ReturnConditional { condition } => {
                if self.condition_met(condition) {
                    self.registers.pc = self.pop_stack();
                }
            }
            OpCode::ReturnInterrupt => {
                self.registers.pc = self.pop_stack();
                self.interrupts_enabled = true;
            }
            OpCode::Restart { lsb } => {
                self.push_stack(self.registers.pc);
                self.registers.pc = lsb as u16;
            }
            OpCode::Halt => {
                self.mode = CoreMode::Halt;
            }
            OpCode::Stop => {
                self.mode = CoreMode::Stop;
            }
            OpCode::Nop => {}
            OpCode::DisableInterrupts => {
                self.interrupts_enabled = false;
            }
            OpCode::EnableInterrupts => {
                self.interrupts_enabled = true;
            }
            OpCode::Illegal { .. } => {
                self.mode = CoreMode::Crash;
            }
        }
        self.machine_cycles += opcode.machine_cycles();
    }

    pub fn handle_interrupts(&mut self) {
        if !self.interrupts_enabled {
            return;
        }

        if let Some(interrupt) = self.mmu.check_interrupts(self.mode) {
            self.mode = CoreMode::Normal; // clear halted state if an interrupt occurs

            // avoid nested interrupts
            self.interrupts_enabled = false;

            // 1. Two wait states are executed (2 M-cycles pass while nothing happens; presumably the CPU is executing nops during this time).
            self.machine_cycles += 2;
            // 2. The current value of the PC register is pushed onto the stack, consuming 2 more M-cycles.
            self.push_stack(self.registers.pc);
            self.machine_cycles += 2;
            // 3. The PC register is set to the address of the handler (one of: $40, $48, $50, $58, $60). This consumes one last M-cycle.
            self.registers.pc = interrupt.address();
            self.machine_cycles += 1;
        }
    }

    fn push_stack(&mut self, value: u16) {
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        self.mmu.write(self.registers.sp, (value >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        self.mmu.write(self.registers.sp, (value & 0xFF) as u8);
    }

    fn pop_stack(&mut self) -> u16 {
        let low = self.mmu.read(self.registers.sp);
        self.registers.sp = self.registers.sp.wrapping_add(1);
        let high = self.mmu.read(self.registers.sp);
        self.registers.sp = self.registers.sp.wrapping_add(1);
        u16::from_le_bytes([low, high])
    }

    fn alu_add_displacement(&mut self, a: u16, d: i8) -> u16 {
        let result = a.wrapping_add_signed(d as i16);
        self.update_carry_flags(a, d as u16, result);
        self.registers.flags.z = false;
        self.registers.flags.n = false;
        result
    }

    fn alu_add(&mut self, value: u8, carry: bool) -> u8 {
        let value = value as u16;
        let a = self.registers.a as u16;
        let result = a + value + if carry && self.registers.flags.c { 1 } else { 0 };
        self.update_carry_flags(a, value, result);
        self.registers.flags.n = false;
        let result = result as u8;
        self.registers.flags.z = result == 0;
        result
    }

    fn alu_subtract(&mut self, value: u8, carry: bool) -> u8 {
        let value = value as u16;
        let a = self.registers.a as u16;
        let result = a.wrapping_sub(value + if carry && self.registers.flags.c { 1 } else { 0 });
        self.update_carry_flags(a, value, result);
        self.registers.flags.n = true;
        let result = result as u8;
        self.registers.flags.z = result == 0;
        result
    }

    fn alu_increment(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        let carry_bits = value ^ 1 ^ result;
        self.registers.flags.z = result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = carry_bits & 0x10 > 0;
        result
    }

    fn alu_decrement(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        let carry_bits = value ^ 1 ^ result;
        self.registers.flags.z = result == 0;
        self.registers.flags.n = true;
        self.registers.flags.h = carry_bits & 0x10 > 0;
        result
    }

    fn alu_and(&mut self, value: u8) -> u8 {
        let result = self.registers.a & value;
        self.registers.flags.z = result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = true; // half carry is always set for AND
        self.registers.flags.c = false; // carry is always clear for AND
        result
    }

    fn alu_or(&mut self, value: u8) -> u8 {
        let result = self.registers.a | value;
        self.registers.flags.z = result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = false; // half carry is always clear for OR
        self.registers.flags.c = false; // carry is always clear for OR
        result
    }

    fn alu_xor(&mut self, value: u8) -> u8 {
        let result = self.registers.a ^ value;
        self.registers.flags.z = result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = false; // half carry is always clear for XOR
        self.registers.flags.c = false; // carry is always clear for XOR
        result
    }

    fn alu_rotate_left(&mut self, value: u8, circular: bool, z_flag: bool) -> u8 {
        let carry = value >> 7;
        let result = (value << 1) | if circular { carry } else { self.registers.flags.c as u8 };
        self.registers.flags.z = z_flag && result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = false;
        self.registers.flags.c = carry > 0;
        result
    }

    fn alu_rotate_right(&mut self, value: u8, circular: bool, z_flag: bool) -> u8 {
        let carry = value & 0x01;
        let result = (value >> 1) | (if circular { carry } else { self.registers.flags.c as u8 }) << 7;
        self.registers.flags.z = z_flag && result == 0;
        self.registers.flags.n = false;
        self.registers.flags.h = false;
        self.registers.flags.c = carry > 0;
        result
    }

    fn condition_met(&self, condition: JumpCondition) -> bool {
        use JumpCondition::*;
        match condition {
            Zero => self.registers.flags.z,
            NotZero => !self.registers.flags.z,
            Carry => self.registers.flags.c,
            NotCarry => !self.registers.flags.c,
        }
    }

    fn update_carry_flags(&mut self, a: u16, b: u16, result: u16) {
        let carry_bits = a ^ b ^ result;
        self.registers.flags.h = carry_bits & 0x10 > 0;
        self.registers.flags.c = carry_bits & 0x100 > 0;
    }
}

pub trait Fetch {
    fn fetch_u8(&mut self) -> u8;
    fn fetch_u16(&mut self) -> u16 {
        let low = self.fetch_u8();
        let high = self.fetch_u8();
        u16::from_le_bytes([low, high])
    }
    fn fetch_i8(&mut self) -> i8 {
        self.fetch_u8() as i8
    }
}

impl Fetch for Core {
    fn fetch_u8(&mut self) -> u8 {
        let opcode = self.mmu.read(self.registers.pc);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        opcode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod load8 {
        use super::*;
        use Register::*;

        macro_rules! register_to_register {
            ($reg:ident; $($other:ident),*) => {
                $(
                    paste::paste! {
                        #[test]
                        fn [<ld_ $reg:lower _ $other:lower>]() {
                            let mut core = Core::dmg_hello_world();
                            core.set_register($other, 0x42);
                            core.execute(OpCode::Load { source: $other, destination: $reg });
                            assert_eq!(core.register($reg), 0x42);
                        }
                    }
                )*
            };
        }

        register_to_register!(B; B, C, D, E, H, L, A);
        register_to_register!(C; B, C, D, E, H, L, A);
        register_to_register!(D; B, C, D, E, H, L, A);
        register_to_register!(E; B, C, D, E, H, L, A);
        register_to_register!(H; B, C, D, E, H, L, A);
        register_to_register!(L; B, C, D, E, H, L, A);
        register_to_register!(A; B, C, D, E, H, L, A);

        #[test]
        fn ld_hl_r() {
            for register in [B, C, D, E, A] {
                let mut core = Core::dmg_hello_world();
                core.registers.set_hl(0xC000); // first byte of WRAM
                core.set_register(register, 0x42);
                core.execute(OpCode::Load { source: register, destination: mHL });
                assert_eq!(core.mmu.read(0xC000), 0x42);
            }
            // special case for H & L
            let mut core = Core::dmg_hello_world();
            core.registers.set_hl(0xC010);
            core.execute(OpCode::Load { source: H, destination: mHL });
            assert_eq!(core.mmu.read(0xC010), 0xC0);

            core.registers.set_hl(0xC010);
            core.execute(OpCode::Load { source: L, destination: mHL });
            assert_eq!(core.mmu.read(0xC010), 0x10);
        }

        #[test]
        fn ld_r_hl() {
            for register in [B, C, D, E, A] {
                let mut core = Core::dmg_hello_world();
                core.registers.set_hl(0xC000); // first byte of WRAM
                core.mmu.write(0xC000, 0x42);
                core.execute(OpCode::Load { source: mHL, destination: register });
                assert_eq!(core.register(register), 0x42);
            }
            // special case for H & L
            let mut core = Core::dmg_hello_world();
            core.registers.set_hl(0xC010);
            core.mmu.write(0xC010, 0x11);
            core.execute(OpCode::Load { source: mHL, destination: H });
            assert_eq!(core.register(H), 0x11);

            core.registers.set_hl(0xC010);
            core.mmu.write(0xC010, 0x10);
            core.execute(OpCode::Load { source: mHL, destination: L });
            assert_eq!(core.register(L), 0x10);
        }

        #[test]
        fn ld_r_n() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::LoadImmediate { register: B, value: 0x42 });
            assert_eq!(core.registers.b, 0x42);

            core.execute(OpCode::LoadImmediate { register: C, value: 0xFF });
            assert_eq!(core.registers.c, 0xFF);

            core.execute(OpCode::LoadImmediate { register: D, value: 0x00 });
            assert_eq!(core.registers.d, 0x00);

            core.execute(OpCode::LoadImmediate { register: E, value: 0x7F });
            assert_eq!(core.registers.e, 0x7F);

            core.execute(OpCode::LoadImmediate { register: H, value: 0x80 });
            assert_eq!(core.registers.h, 0x80);

            core.execute(OpCode::LoadImmediate { register: L, value: 0x01 });
            assert_eq!(core.registers.l, 0x01);

            core.execute(OpCode::LoadImmediate { register: A, value: 0x55 });
            assert_eq!(core.registers.a, 0x55);
        }

        #[test]
        fn ld_indirect_a() {
            use crate::opcode::Register16Mem;

            // Test LoadIndirectAccumulator with BC
            let mut core = Core::dmg_hello_world();
            core.registers.set_bc(0xC000);
            core.registers.a = 0x42;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::BC });
            assert_eq!(core.mmu.read(0xC000), 0x42);

            // Test LoadIndirectAccumulator with DE
            core.registers.set_de(0xC010);
            core.registers.a = 0x99;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::DE });
            assert_eq!(core.mmu.read(0xC010), 0x99);

            // Test LoadIndirectAccumulator with HL+ (should increment HL after)
            core.registers.set_hl(0xC020);
            core.registers.a = 0xAA;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::HLIncrement });
            assert_eq!(core.mmu.read(0xC020), 0xAA);
            assert_eq!(core.registers.hl(), 0xC021);

            // Test LoadIndirectAccumulator with HL- (should decrement HL after)
            core.registers.set_hl(0xC030);
            core.registers.a = 0xBB;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::HLDecrement });
            assert_eq!(core.mmu.read(0xC030), 0xBB);
            assert_eq!(core.registers.hl(), 0xC02F);
        }

        #[test]
        fn ld_a_indirect() {
            use crate::opcode::Register16Mem;

            // Test LoadAccumulatorIndirect with BC
            let mut core = Core::dmg_hello_world();
            core.registers.set_bc(0xC000);
            core.mmu.write(0xC000, 0x33);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::BC });
            assert_eq!(core.registers.a, 0x33);

            // Test LoadAccumulatorIndirect with DE
            core.registers.set_de(0xC010);
            core.mmu.write(0xC010, 0x77);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::DE });
            assert_eq!(core.registers.a, 0x77);

            // Test LoadAccumulatorIndirect with HL+ (should increment HL after)
            core.registers.set_hl(0xC020);
            core.mmu.write(0xC020, 0xCC);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::HLIncrement });
            assert_eq!(core.registers.a, 0xCC);
            assert_eq!(core.registers.hl(), 0xC021);

            // Test LoadAccumulatorIndirect with HL- (should decrement HL after)
            core.registers.set_hl(0xC030);
            core.mmu.write(0xC030, 0xDD);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::HLDecrement });
            assert_eq!(core.registers.a, 0xDD);
            assert_eq!(core.registers.hl(), 0xC02F);
        }

        #[test]
        fn ld_a_direct() {
            // Test LoadAccumulatorDirect - load accumulator from direct address
            let mut core = Core::dmg_hello_world();

            // Test with WRAM address
            core.mmu.write(0xC000, 0x88);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC000 });
            assert_eq!(core.registers.a, 0x88);

            // Test with different address and value
            core.mmu.write(0xC123, 0xAB);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC123 });
            assert_eq!(core.registers.a, 0xAB);

            // Test with high memory address (0xFF80-0xFFFE range)
            core.mmu.write(0xFF80, 0x55);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xFF80 });
            assert_eq!(core.registers.a, 0x55);

            // Test edge case with 0x00 value
            core.mmu.write(0xC200, 0x00);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC200 });
            assert_eq!(core.registers.a, 0x00);
        }

        #[test]
        fn ld_direct_a() {
            // Test LoadDirectAccumulator - store accumulator to direct address
            let mut core = Core::dmg_hello_world();

            // Test with WRAM address
            core.registers.a = 0x77;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC000 });
            assert_eq!(core.mmu.read(0xC000), 0x77);

            // Test with different address and value
            core.registers.a = 0xCD;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC456 });
            assert_eq!(core.mmu.read(0xC456), 0xCD);

            // Test with high memory address (0xFF80-0xFFFE range)
            core.registers.a = 0x99;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xFF80 });
            assert_eq!(core.mmu.read(0xFF80), 0x99);

            // Test edge case with 0x00 value
            core.registers.a = 0x00;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC300 });
            assert_eq!(core.mmu.read(0xC300), 0x00);

            // Test edge case with 0xFF value
            core.registers.a = 0xFF;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC400 });
            assert_eq!(core.mmu.read(0xC400), 0xFF);
        }

        #[test]
        fn ldh_a_c() {
            let mut core = Core::dmg_hello_world();
            core.registers.c = 0x00;
            core.registers.a = 0x42;
            // this is the joypad register at 0xFF00, only the 5th and 6th bits are writeable
            core.mmu.write(0xFF00, 0x30);
            core.execute(OpCode::LoadHighAccumulatorIndirect);
            assert_eq!(core.registers.a, 0x3F); // all buttons released
        }

        #[test]
        fn ldh_c_a() {
            let mut core = Core::dmg_hello_world();
            core.registers.c = 0x00;
            core.registers.a = 0x30;
            core.mmu.write(0xFF00, 0x00);
            core.execute(OpCode::LoadHighIndirectAccumulator);
            assert_eq!(core.mmu.read(0xFF00), 0x3F); // all buttons released
        }

        #[test]
        fn ldh_a_n() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0x42;
            core.mmu.write(0xFF00, 0x30);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0x00 });
            assert_eq!(core.registers.a, 0x3F); // all buttons released
        }

        #[test]
        fn ldh_n_a() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0x30;
            core.mmu.write(0xFF00, 0x00);
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x00});
            assert_eq!(core.mmu.read(0xFF00), 0x3F); // all buttons released
        }


    }

    mod load16 {
        use super::*;
        use Register16::*;

        #[test]
        fn ld_r16_nn() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::Load16Immediate { register: BC, value: 0x1234 });
            assert_eq!(core.register16(BC), 0x1234);

            core.execute(OpCode::Load16Immediate { register: DE, value: 0x5678 });
            assert_eq!(core.register16(DE), 0x5678);

            core.execute(OpCode::Load16Immediate { register: HL, value: 0x9ABC });
            assert_eq!(core.register16(HL), 0x9ABC);

            core.execute(OpCode::Load16Immediate { register: SP, value: 0xDEF0 });
            assert_eq!(core.registers.sp, 0xDEF0);
        }

        #[test]
        fn ld_nn_sp() {
            let mut core = Core::dmg_hello_world();
            core.registers.sp = 0x1234;
            core.execute(OpCode::LoadDirectStackPointer { address: 0xC000 });
            assert_eq!(core.mmu.read_u16_le(0xC000), 0x1234);
        }

        #[test]
        fn ld_sp_hl() {
            let mut core = Core::dmg_hello_world();
            core.registers.set_hl(0x4242);
            core.execute(OpCode::LoadStackPointerHL);
            assert_eq!(core.registers.sp, 0x4242);
        }

        #[test]
        fn stack() {
            let mut core = Core::dmg_hello_world();

            // push some values onto stack from all 16-but registers
            core.registers.set_bc(0x1234);
            core.registers.set_de(0x5678);
            core.registers.set_hl(0x9ABC);
            core.registers.set_af(0xEFF0);
            core.execute(OpCode::Push { register: Register16Stack::BC });
            core.execute(OpCode::Push { register: Register16Stack::DE });
            core.execute(OpCode::Push { register: Register16Stack::HL });
            core.execute(OpCode::Push { register: Register16Stack::AF });

            assert_eq!(core.registers.sp, 0xFFFE - 8);
            assert_eq!(core.mmu.read_u16_le(0xFFFC), 0x1234);
            assert_eq!(core.mmu.read_u16_le(0xFFFA), 0x5678);
            assert_eq!(core.mmu.read_u16_le(0xFFF8), 0x9ABC);
            assert_eq!(core.mmu.read_u16_le(0xFFF6), 0xEFF0);

            // reset all registers to zero
            core.registers.set_bc(0x0000);
            core.registers.set_de(0x0000);
            core.registers.set_hl(0x0000);
            core.registers.set_af(0x0000);

            // pop them back from stack (in reverse)
            core.execute(OpCode::Pop { register: Register16Stack::AF });
            core.execute(OpCode::Pop { register: Register16Stack::HL });
            core.execute(OpCode::Pop { register: Register16Stack::DE });
            core.execute(OpCode::Pop { register: Register16Stack::BC });

            // values popped from stack
            assert_eq!(core.registers.sp, 0xFFFE); // SP incremented back up to top
            assert_eq!(core.registers.bc(), 0x1234);
            assert_eq!(core.registers.de(), 0x5678);
            assert_eq!(core.registers.hl(), 0x9ABC);
            assert_eq!(core.registers.af(), 0xEFF0);
        }

        #[test]
        fn ld_hl_sp_d() {
            let mut core = Core::dmg_hello_world();
            core.registers.sp = 0xFFFA;

            // Test negative offset
            core.execute(OpCode::LoadHLAdjustedStackPointer { offset: -10 });
            assert_eq!(core.registers.hl(), 0xFFF0);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h); // TODO logic matches gambatte, confirm this with blargg
            assert!(core.registers.flags.c);

            // Test positive offset
            core.execute(OpCode::LoadHLAdjustedStackPointer { offset: 5 });
            assert_eq!(core.registers.hl(), 0xFFFF);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test zero offset
            core.execute(OpCode::LoadHLAdjustedStackPointer { offset: 0 });
            assert_eq!(core.registers.hl(), 0xFFFA);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test carry && half carry
            core.registers.sp = 0xFFFE;
            core.execute(OpCode::LoadHLAdjustedStackPointer { offset: 2 });
            assert_eq!(core.registers.hl(), 0x0000);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(core.registers.flags.c);

            // Test half carry without carry
            core.registers.sp = 0x000E;
            core.execute(OpCode::LoadHLAdjustedStackPointer { offset: 2 });
            assert_eq!(core.registers.hl(), 0x0010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }
    }

    mod alu {
        use super::*;
        use Register::*;

        #[test]
        fn add() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.set_register(B, 0x02);
            core.execute(OpCode::Add { register: B });
            assert_eq!(core.register(A), 0x03);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry && half carry
            core.set_register(A, 0xFF);
            core.set_register(B, 0x01);
            core.execute(OpCode::Add { register: B });
            assert_eq!(core.register(A), 0x00);
            assert!(core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0xFF + 0x01
            assert!(core.registers.flags.c); // carry from 0xFF + 0x01
        }

        #[test]
        fn add_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.execute(OpCode::AddImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x03);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0xFF);
            core.execute(OpCode::AddImmediate { value: 0x10 });
            assert_eq!(core.register(A), 0x0F);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c); // carry from 0xFF + 0x01
        }

        #[test]
        fn add_with_carry() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.set_register(B, 0x02);
            core.registers.flags.c = false; // no carry
            core.execute(OpCode::AddWithCarry { register: B });
            assert_eq!(core.register(A), 0x03);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0xFF);
            core.set_register(B, 0x01);
            core.registers.flags.c = true; // carry is set
            core.execute(OpCode::AddWithCarry { register: B });
            assert_eq!(core.register(A), 0x01); // 0xFF + 0x01 + 1 (carry) = 0x01
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0xFF + 0x01 + 1
            assert!(core.registers.flags.c); // carry from 0xFF + 0x01 + 1
        }

        #[test]
        fn add_with_carry_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.registers.flags.c = false; // no carry
            core.execute(OpCode::AddWithCarryImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x03);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0xFF);
            core.registers.flags.c = true; // carry is set
            core.execute(OpCode::AddWithCarryImmediate { value: 0x01 });
            assert_eq!(core.register(A), 0x01); // 0xFF + 0x01 + 1 (carry) = 0x01
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0xFF + 0x01 + 1
            assert!(core.registers.flags.c); // carry from 0xFF + 0x01 + 1
        }

        #[test]
        fn subtract() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.set_register(B, 0x02);
            core.execute(OpCode::Subtract { register: B });
            assert_eq!(core.register(A), 0x01);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry && half carry
            core.set_register(A, 0x01);
            core.set_register(B, 0x02);
            core.execute(OpCode::Subtract { register: B });
            assert_eq!(core.register(A), 0xFF);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02
        }

        #[test]
        fn subtract_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.execute(OpCode::SubtractImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x01);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0x01);
            core.execute(OpCode::SubtractImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0xFF);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02
        }

        #[test]
        fn subtract_with_carry() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.set_register(B, 0x02);
            core.registers.flags.c = false; // no carry
            core.execute(OpCode::SubtractWithCarry { register: B });
            assert_eq!(core.register(A), 0x01);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0x01);
            core.set_register(B, 0x02);
            core.registers.flags.c = true; // carry is set
            core.execute(OpCode::SubtractWithCarry { register: B });
            assert_eq!(core.register(A), 0xFE); // 0x01 - 0x02 - 1 (carry)
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02 - 1
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02 - 1
        }

        #[test]
        fn subtract_with_carry_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.registers.flags.c = false; // no carry
            core.execute(OpCode::SubtractWithCarryImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x01);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0x01);
            core.registers.flags.c = true; // carry is set
            core.execute(OpCode::SubtractWithCarryImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0xFE); // 0x01 - 0x02 - 1 (carry)
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02 - 1
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02 - 1
        }

        #[test]
        fn compare() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.set_register(B, 0x02);
            core.execute(OpCode::Compare { register: B });
            assert_eq!(core.register(A), 0x03); // A should not change
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry && half carry
            core.set_register(A, 0x01);
            core.set_register(B, 0x02);
            core.execute(OpCode::Compare { register: B });
            assert_eq!(core.register(A), 0x01); // A should not change
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02
        }

        #[test]
        fn compare_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x03);
            core.execute(OpCode::CompareImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x03); // A should not change
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with carry
            core.set_register(A, 0x01);
            core.execute(OpCode::CompareImmediate { value: 0x02 });
            assert_eq!(core.register(A), 0x01); // A should not change
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h); // half carry from 0x01 - 0x02
            assert!(core.registers.flags.c); // carry from 0x01 - 0x02
        }

        #[test]
        fn increment() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.execute(OpCode::Increment { register: A });
            assert_eq!(core.register(A), 0x02);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);

            // Test carry
            core.set_register(A, 0xFF);
            core.execute(OpCode::Increment { register: A });
            assert_eq!(core.register(A), 0x00);
            assert!(core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
        }

        #[test]
        fn decrement() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0x01);
            core.execute(OpCode::Decrement { register: A });
            assert_eq!(core.register(A), 0x00);
            assert!(core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(!core.registers.flags.h);

            // Test carry
            core.set_register(A, 0x00);
            core.execute(OpCode::Decrement { register: A });
            assert_eq!(core.register(A), 0xFF);
            assert!(!core.registers.flags.z);
            assert!(core.registers.flags.n);
            assert!(core.registers.flags.h);
        }

        #[test]
        fn and() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0xF0);
            core.set_register(B, 0x0F);
            core.execute(OpCode::And { register: B });
            assert_eq!(core.register(A), 0x00);
            assert!(core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Test with non-zero result
            core.set_register(A, 0xFF);
            core.set_register(B, 0x0F);
            core.execute(OpCode::And { register: B });
            assert_eq!(core.register(A), 0x0F);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
       fn and_immediate() {
            let mut core = Core::dmg_hello_world();
            core.set_register(A, 0xF0);
            core.execute(OpCode::AndImmediate { value: 0x0F });
            assert_eq!(core.register(A), 0x00);
            assert!(core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(!core.registers.flags.c);
       }

       #[test]
       fn xor() {
           let mut core = Core::dmg_hello_world();
           core.set_register(A, 0xF0);
           core.set_register(B, 0x0F);
           core.execute(OpCode::Xor { register: B });
           assert_eq!(core.register(A), 0xFF);
           assert!(!core.registers.flags.z);
           assert!(!core.registers.flags.n);
           assert!(!core.registers.flags.h);
           assert!(!core.registers.flags.c);

           // Test with zero result
           core.set_register(A, 0xFF);
           core.set_register(B, 0xFF);
           core.execute(OpCode::Xor { register: B });
           assert_eq!(core.register(A), 0x00);
           assert!(core.registers.flags.z);
           assert!(!core.registers.flags.n);
           assert!(!core.registers.flags.h);
           assert!(!core.registers.flags.c);
       }

       #[test]
       fn xor_immediate() {
           let mut core = Core::dmg_hello_world();
           core.set_register(A, 0xFF);
           core.execute(OpCode::XorImmediate { value: 0x0F });
           assert_eq!(core.register(A), 0xF0);
           assert!(!core.registers.flags.z);
           assert!(!core.registers.flags.n);
           assert!(!core.registers.flags.h);
           assert!(!core.registers.flags.c);
       }

        #[test]
        fn compliment_carry_flag() {
            let mut core = Core::dmg_hello_world();
            core.registers.flags.c = false;
            core.execute(OpCode::ComplementCarryFlag);
            assert!(core.registers.flags.c); // carry flag should be set
            core.execute(OpCode::ComplementCarryFlag);
            assert!(!core.registers.flags.c); // carry flag should be cleared
        }

        #[test]
        fn set_carry_flag() {
            let mut core = Core::dmg_hello_world();
            core.registers.flags.c = false;
            core.execute(OpCode::SetCarryFlag);
            assert!(core.registers.flags.c); // carry flag should be set
            core.execute(OpCode::SetCarryFlag);
            assert!(core.registers.flags.c); // carry flag should remain set
        }

        #[test]
        fn decimal_adjust_add() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0x15;
            core.registers.b = 0x27;
            core.execute(OpCode::Add { register: B });
            assert_eq!(core.registers.a, 0x3C);
            core.execute(OpCode::DecimalAdjustAccumulator);
            assert_eq!(core.registers.a, 0x42);
        }

        #[test]
        fn decimal_adjust_subtract() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0x15;
            core.registers.b = 0x27;
            core.execute(OpCode::Subtract { register: B });
            assert_eq!(core.registers.a, 0xEE);
            core.execute(OpCode::DecimalAdjustAccumulator);
            assert_eq!(core.registers.a, 0x88);
        }

        #[test]
        fn compliment_accumulator() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0x55;
            core.execute(OpCode::ComplementAccumulator);
            assert_eq!(core.registers.a, 0xAA); // 0x55 complemented is 0xAA
            core.execute(OpCode::ComplementAccumulator);
            assert_eq!(core.registers.a, 0x55); // complement again should restore original value
        }
    }

    mod alu16 {
        use super::*;

        #[test]
        fn increment16() {
            let mut core = Core::dmg_hello_world();
            core.registers.set_bc(0x1234);
            core.execute(OpCode::Increment16 { register: Register16::BC });
            assert_eq!(core.registers.bc(), 0x1235);

            core.registers.set_de(0x5678);
            core.execute(OpCode::Increment16 { register: Register16::DE });
            assert_eq!(core.registers.de(), 0x5679);

            core.registers.set_hl(0x9ABC);
            core.execute(OpCode::Increment16 { register: Register16::HL });
            assert_eq!(core.registers.hl(), 0x9ABD);

            core.registers.sp = 0xFFFF;
            core.execute(OpCode::Increment16 { register: Register16::SP });
            assert_eq!(core.registers.sp, 0x0000); // wrap around
        }

        #[test]
        fn decrement16() {
            let mut core = Core::dmg_hello_world();
            core.registers.set_bc(0x1234);
            core.execute(OpCode::Decrement16 { register: Register16::BC });
            assert_eq!(core.registers.bc(), 0x1233);

            core.registers.set_de(0x5678);
            core.execute(OpCode::Decrement16 { register: Register16::DE });
            assert_eq!(core.registers.de(), 0x5677);

            core.registers.set_hl(0x9ABC);
            core.execute(OpCode::Decrement16 { register: Register16::HL });
            assert_eq!(core.registers.hl(), 0x9ABB);

            core.registers.sp = 0x0000;
            core.execute(OpCode::Decrement16 { register: Register16::SP });
            assert_eq!(core.registers.sp, 0xFFFF); // wrap around
        }

        #[test]
        fn add16() {
            let mut core = Core::dmg_hello_world();
            core.registers.set_hl(0xFF00);
            core.registers.set_de(0x100);
            core.execute(OpCode::Add16 { register: Register16::DE });
            assert_eq!(core.registers.hl(), 0x0000);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(core.registers.flags.c);
        }

        #[test]
        fn add_stack_pointer() {
            let mut core = Core::dmg_hello_world();
            core.registers.sp = 0xFFFE;
            core.execute(OpCode::AddStackPointer { offset: 1 });
            assert_eq!(core.registers.sp, 0xFFFF);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            core.execute(OpCode::AddStackPointer { offset: 10 });
            assert_eq!(core.registers.sp, 0x0009);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::AddStackPointer { offset: -10 });
            assert_eq!(core.registers.sp, 0xFFFF);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }
    }

    mod rotate_shift_bit {
        use super::*;

        #[test]
        fn rotate_left_circular_accumulator() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0b10101010;
            core.execute(OpCode::RotateLeftCircularAccumulator);
            assert_eq!(core.registers.a, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateLeftCircularAccumulator);
            assert_eq!(core.registers.a, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn rotate_right_circular_accumulator() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0b10101010;
            core.execute(OpCode::RotateRightCircularAccumulator);
            assert_eq!(core.registers.a, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            core.execute(OpCode::RotateRightCircularAccumulator);
            assert_eq!(core.registers.a, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c); // carry cleared
        }

        #[test]
        fn rotate_left_accumulator() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0b10101010;
            core.execute(OpCode::RotateLeftAccumulator);
            assert_eq!(core.registers.a, 0b01010100);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateLeftAccumulator);
            assert_eq!(core.registers.a, 0b10101001);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn rotate_right_accumulator() {
            let mut core = Core::dmg_hello_world();
            core.registers.a = 0b10101001;
            core.execute(OpCode::RotateRightAccumulator);
            assert_eq!(core.registers.a, 0b01010100);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateRightAccumulator);
            assert_eq!(core.registers.a, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn rotate_left_circular_register() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::RotateLeftCircular { register: Register::B });
            assert_eq!(core.registers.b, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateLeftCircular { register: Register::B });
            assert_eq!(core.registers.b, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn rotate_right_circular_register() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::RotateRightCircular { register: Register::B });
            assert_eq!(core.registers.b, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            core.execute(OpCode::RotateRightCircular { register: Register::B });
            assert_eq!(core.registers.b, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c); // carry cleared
        }

        #[test]
        fn rotate_left_register() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::RotateLeft { register: Register::B });
            assert_eq!(core.registers.b, 0b01010100);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateLeft { register: Register::B });
            assert_eq!(core.registers.b, 0b10101001);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn rotate_right_register() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101001;
            core.execute(OpCode::RotateRight { register: Register::B });
            assert_eq!(core.registers.b, 0b01010100);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::RotateRight { register: Register::B });
            assert_eq!(core.registers.b, 0b10101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn shift_left() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::ShiftLeftArithmetic { register: Register::B });
            assert_eq!(core.registers.b, 0b01010100);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);

            core.execute(OpCode::ShiftLeftArithmetic { register: Register::B });
            assert_eq!(core.registers.b, 0b10101000);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn shift_right() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::ShiftRightArithmetic { register: Register::B });
            assert_eq!(core.registers.b, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            core.execute(OpCode::ShiftRightArithmetic { register: Register::B });
            assert_eq!(core.registers.b, 0b00101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);
        }

        #[test]
        fn shift_right_logical() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101010;
            core.execute(OpCode::ShiftRightLogical { register: Register::B });
            assert_eq!(core.registers.b, 0b01010101);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            core.execute(OpCode::ShiftRightLogical { register: Register::B });
            assert_eq!(core.registers.b, 0b00101010);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(core.registers.flags.c);
        }

        #[test]
        fn swap() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0xAB;
            core.execute(OpCode::Swap { register: Register::B });
            assert_eq!(core.registers.b, 0xBA);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);

            // Swapping again should restore original value
            core.execute(OpCode::Swap { register: Register::B });
            assert_eq!(core.registers.b, 0xAB);
            assert!(!core.registers.flags.z);
            assert!(!core.registers.flags.n);
            assert!(!core.registers.flags.h);
            assert!(!core.registers.flags.c);
        }

        #[test]
        fn test_bit() {
            let mut core = Core::dmg_hello_world();
            core.registers.b = 0b10101110;
            for (bit, expected) in [true, false, false, false, true, false, true, false].into_iter().enumerate() {
                core.execute(OpCode::TestBit { register: Register::B, bit: bit as u8 });
                assert_eq!(core.registers.flags.z, expected, "Bit {} test failed", bit);
                assert!(!core.registers.flags.n);
                assert!(core.registers.flags.h);
            }
        }

        #[test]
        fn set_bit() {
            let mut core = Core::dmg_hello_world();
            core.registers.c = 0b00101010;
            core.execute(OpCode::SetBit { register: Register::C, bit: 2 });
            assert_eq!(core.registers.c, 0b00101110);
            core.execute(OpCode::SetBit { register: Register::C, bit: 2 });
            assert_eq!(core.registers.c, 0b00101110); // Setting the same bit again should not change it
            core.execute(OpCode::SetBit { register: Register::C, bit: 7 });
            assert_eq!(core.registers.c, 0b10101110);
        }

        #[test]
        fn reset_bit() {
            let mut core = Core::dmg_hello_world();
            core.registers.c = 0b10101110;
            core.execute(OpCode::ResetBit { register: Register::C, bit: 2 });
            assert_eq!(core.registers.c, 0b10101010);
            core.execute(OpCode::ResetBit { register: Register::C, bit: 2 });
            assert_eq!(core.registers.c, 0b10101010); // Resetting the same bit again should not change it
            core.execute(OpCode::ResetBit { register: Register::C, bit: 7 });
            assert_eq!(core.registers.c, 0b00101010);
        }
    }

    mod control_flow {
        use crate::joypad::JoypadButton;
        use super::*;

        #[test]
        fn jump() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::Jump { address: 0x0200 });
            assert_eq!(core.registers.pc, 0x0200);
        }

        #[test]
        fn jump_hl() {
            let mut core = Core::dmg_hello_world();
            core.registers.set_hl(0x0300);
            core.execute(OpCode::JumpHL);
            assert_eq!(core.registers.pc, 0x0300);
        }

        #[test]
        fn jump_if_zero() {
            let mut core = Core::dmg_hello_world();
            core.registers.flags.z = true;
            core.execute(OpCode::JumpConditional { address: 0x0400, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0400);

            core.execute(OpCode::JumpConditional { address: 0x0500, condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0x0400); // PC should not change

            core.registers.flags.z = false;
            core.execute(OpCode::JumpConditional { address: 0x0500, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0400); // PC should not change

            core.execute(OpCode::JumpConditional { address: 0x0500, condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0x0500);
        }
        #[test]
        fn jump_if_carry() {
            let mut core = Core::dmg_hello_world();
            core.registers.flags.c = true;
            core.execute(OpCode::JumpConditional { address: 0x0800, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0800);

            core.execute(OpCode::JumpConditional { address: 0x0900, condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0x0800); // PC should not change

            core.registers.flags.c = false;
            core.execute(OpCode::JumpConditional { address: 0x0900, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0800); // PC should not change

            core.execute(OpCode::JumpConditional { address: 0x0900, condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0x0900);
        }

        #[test]
        fn jump_relative() {
            let mut core = Core::dmg_hello_world();
            core.registers.pc = 0x0100; // Start at 0x0100

            // Positive offset
            core.execute(OpCode::JumpRelative { offset: 5 });
            assert_eq!(core.registers.pc, 0x0105);

            // Negative offset
            core.execute(OpCode::JumpRelative { offset: -3 });
            assert_eq!(core.registers.pc, 0x0102);
        }

        #[test]
        fn jump_relative_zero() {
            let mut core = Core::dmg_hello_world();
            core.registers.pc = 0x0000; // Start at 0x0000
            core.registers.flags.z = true;

            // Positive offset
            core.execute(OpCode::JumpRelativeConditional { offset: 5, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0005);

            // Negative offset
            core.execute(OpCode::JumpRelativeConditional { offset: -3, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0002);

            // If not zero, PC should not change
            core.registers.flags.z = false;
            core.execute(OpCode::JumpRelativeConditional { offset: 5, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0002);


            // Positive offset
            core.execute(OpCode::JumpRelativeConditional { offset: 1, condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0x0003);

            // Negative offset
            core.execute(OpCode::JumpRelativeConditional { offset: -4, condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0xFFFF);
        }

        #[test]
        fn jump_relative_carry() {
            let mut core = Core::dmg_hello_world();
            core.registers.pc = 0x0000; // Start at 0x0000
            core.registers.flags.c = true;

            // Positive offset
            core.execute(OpCode::JumpRelativeConditional { offset: 5, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0005);

            // Negative offset
            core.execute(OpCode::JumpRelativeConditional { offset: -3, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0002);

            // If not carry, PC should not change
            core.registers.flags.c = false;
            core.execute(OpCode::JumpRelativeConditional { offset: 5, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0002);

            // Positive offset
            core.execute(OpCode::JumpRelativeConditional { offset: 1, condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0x0003);

            // Negative offset
            core.execute(OpCode::JumpRelativeConditional { offset: -4, condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0xFFFF);
        }

        #[test]
        fn call_return() {
            let mut core = Core::dmg_hello_world();
            core.registers.sp = 0xFFFE; // Set stack pointer to the end of the stack
            core.interrupts_enabled = true; // Enable interrupts for the test

            // Call a subroutine
            core.execute(OpCode::Call { address: 0x0200 });
            assert_eq!(core.registers.pc, 0x0200); // PC should jump to the subroutine
            assert_eq!(core.registers.sp, 0xFFFC); // Stack pointer should decrement by 2
            assert_eq!(core.mmu.read_u16_le(0xFFFC), 0x0100); // Return address (PC before call) should be pushed onto the stack
            assert!(core.interrupts_enabled); // does not affect interrupts

            // Return from subroutine
            core.execute(OpCode::Return);
            assert_eq!(core.registers.pc, 0x0100); // PC should return to the address before the call
            assert_eq!(core.registers.sp, 0xFFFE); // Stack pointer should increment by 2
            assert!(core.interrupts_enabled); // does not affect interrupts

            // simulate an interrupt handler call
            core.interrupts_enabled = false;
            core.execute(OpCode::Call { address: 0x0300 });
            core.execute(OpCode::ReturnInterrupt);
            assert_eq!(core.registers.pc, 0x0100);
            assert_eq!(core.registers.sp, 0xFFFE);
            assert!(core.interrupts_enabled); // interrupts should be re-enabled after return
        }

        #[test]
        fn call_conditional() {
            let mut core = Core::dmg_hello_world();
            core.registers.sp = 0xFFFE; // Set stack pointer to the end of the stack

            // Call if zero
            core.registers.flags.z = true;
            core.execute(OpCode::CallConditional { address: 0x0200, condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0200);
            assert_eq!(core.registers.sp, 0xFFFC);
            assert_eq!(core.mmu.read_u16_le(0xFFFC), 0x0100);

            // Call if not zero
            core.execute(OpCode::CallConditional { address: 0x0300, condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0x0200); // PC should not change
            assert_eq!(core.registers.sp, 0xFFFC); // Stack pointer should not change

            // Call if carry
            core.registers.flags.c = true;
            core.execute(OpCode::CallConditional { address: 0x0400, condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0400);
            assert_eq!(core.registers.sp, 0xFFFA);
            assert_eq!(core.mmu.read_u16_le(0xFFFA), 0x0200);

            // Call if not carry
            core.execute(OpCode::CallConditional { address: 0x0500, condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0x0400); // PC should not change
        }

        #[test]
        fn return_conditional() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::Call { address: 0x0150 });

            // Return if zero
            core.registers.flags.z = true;
            core.execute(OpCode::ReturnConditional { condition: JumpCondition::Zero });
            assert_eq!(core.registers.pc, 0x0100);
            assert_eq!(core.registers.sp, 0xFFFE);

            // Return if not zero
            core.execute(OpCode::Call { address: 0x0150 });
            core.execute(OpCode::ReturnConditional { condition: JumpCondition::NotZero });
            assert_eq!(core.registers.pc, 0x0150); // not returned
            assert_eq!(core.registers.sp, 0xFFFC); // Stack pointer should not change

            // Return if carry
            core.registers.flags.c = true;
            core.execute(OpCode::ReturnConditional { condition: JumpCondition::Carry });
            assert_eq!(core.registers.pc, 0x0100);
            assert_eq!(core.registers.sp, 0xFFFE);

            // Return if not carry
            core.execute(OpCode::Call { address: 0x0150 });
            core.execute(OpCode::ReturnConditional { condition: JumpCondition::NotCarry });
            assert_eq!(core.registers.pc, 0x0150); // not returned
        }

        #[test]
        fn restart() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::Restart { lsb: 0x40 }); // Restart at 0x0040
            assert_eq!(core.registers.pc, 0x0040);
            assert_eq!(core.registers.sp, 0xFFFC);
        }

        #[test]
        fn nop() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::Nop);
        }

        #[test]
        fn halt() {
            let mut core = Core::dmg_hello_world();
            assert_eq!(core.mode, CoreMode::Normal);
            core.execute(OpCode::Halt);
            assert_eq!(core.mode, CoreMode::Halt);

            // interrupts wake it up
            core.interrupts_enabled = true;
            core.mmu.write(0xFFFF, 0xFF); // enable all interrupts
            core.mmu.write(0xFF0F, 0xFF); // request all interrupts
            core.update(Duration::from_millis(10));
            core.handle_interrupts();
            assert_eq!(core.mode, CoreMode::Normal);
        }

        #[test]
        fn stop() {
            let mut core = Core::dmg_hello_world();
            assert_eq!(core.mode, CoreMode::Normal);
            core.execute(OpCode::Stop);
            assert_eq!(core.mode, CoreMode::Stop);

            // interrupts wake it up
            core.interrupts_enabled = true;
            core.mmu.write(0xFFFF, 0xFF); // enable all interrupts
            core.mmu.write(0xFF0F, 0xFF); // request all interrupts
            core.update(Duration::from_millis(10));
            core.handle_interrupts();
            assert_eq!(core.mode, CoreMode::Normal);
        }

        #[test]
        fn enable_interrupts() {
            let mut core = Core::dmg_hello_world();
            assert!(!core.interrupts_enabled);
            core.execute(OpCode::EnableInterrupts);
            assert!(core.interrupts_enabled);
            core.execute(OpCode::DisableInterrupts);
            assert!(!core.interrupts_enabled);
        }
    }

    mod interrupts {
        use super::*;
        use crate::opcode::OpCode;

        #[test]
        fn interrupt_master_enabled() {
            let mut core = Core::dmg_hello_world();
            assert!(!core.interrupts_enabled);
            core.execute(OpCode::EnableInterrupts);
            assert!(core.interrupts_enabled);
            core.execute(OpCode::DisableInterrupts);
            assert!(!core.interrupts_enabled);
        }

        #[test]
        fn handle_interrupts_does_nothing_when_interrupt_master_disabled() {
            let mut core = Core::dmg_hello_world();
            core.mmu.write(0xFFFF, 0xFF); // enable all interrupts
            core.mmu.write(0xFF0F, 0xFF); // request all interrupts
            core.handle_interrupts();
            assert_eq!(core.registers.pc, 0x0100); // PC should not change
        }

        #[test]
        fn handle_interrupt() {
            let mut core = Core::dmg_hello_world();
            core.execute(OpCode::EnableInterrupts);

            core.mmu.write(0xFFFF, 0xFF); // enable all interrupts
            core.mmu.write(0xFF0F, 0xFF); // request all interrupts

            // run all interrupts in sequence
            let expected_interrupts = [0x0040, 0x0048, 0x0050, 0x0058, 0x0060];
            for expected_address in expected_interrupts {
                core.handle_interrupts();

                assert_eq!(core.registers.pc, expected_address);
                assert!(!core.interrupts_enabled);
                assert_eq!(core.registers.sp, 0xFFFC); // stack pointer decremented twice
                assert_eq!(core.mmu.read_u16_le(0xFFFC), 0x0100); // PC pushed onto stack

                core.execute(OpCode::ReturnInterrupt);
                assert!(core.interrupts_enabled); // interrupts are re-enabled
                assert_eq!(core.registers.pc, 0x0100); // PC restored from stack
                assert_eq!(core.registers.sp, 0xFFFE); // stack pointer incremented twice
            }

            // after that there should be no more interrupts
            core.handle_interrupts();
            assert_eq!(core.registers.pc, 0x0100); // PC should not change
        }
    }

    #[test]
    fn core_initialization() {
        let core = Core::dmg_hello_world();
        assert_eq!(core.registers.a, 0x01);
        assert_eq!(core.registers.flags.z, true);
        assert_eq!(core.registers.pc, 0x0100);
    }

    #[test]
    fn program_flow() {
        let mut core = Core::dmg_hello_world();
        assert_eq!(core.registers.pc, 0x0100); // PC should start at 0x0100

        let opcode = core.fetch();
        assert_eq!(opcode, OpCode::Nop); // Initial opcode should be Nop
        assert_eq!(core.registers.pc, 0x0101); // PC should increment by 1 after fetching an opcode
        core.execute(opcode);
        assert_eq!(core.registers.pc, 0x0101); // PC should remain at 0x0101 after executing Nop

        let opcode = core.fetch();
        assert_eq!(opcode, OpCode::Jump { address: 0x0150 });
        assert_eq!(core.registers.pc, 0x0104); // PC should increment by 3 for the Jump (opcode + 2 bytes address)
    }
}

