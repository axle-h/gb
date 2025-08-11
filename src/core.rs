use crate::memory::{Memory, MemoryInterface};
use crate::opcode::{OpCode, Register, Register16Mem};
use crate::registers::RegisterSet;

pub struct Core {
    registers: RegisterSet,
    memory: Memory
}

impl Core {
    pub fn dmg_empty() -> Self {
        Self {
            registers: RegisterSet::dmg(),
            memory: Memory::dmg_empty()
        }
    }

    fn register(&self, register: Register) -> u8 {
        match register {
            Register::B => self.registers.b,
            Register::C => self.registers.c,
            Register::D => self.registers.d,
            Register::E => self.registers.e,
            Register::H => self.registers.h,
            Register::L => self.registers.l,
            Register::mHL => self.memory.read(self.registers.hl()),
            Register::A => self.registers.a,
        }
    }

    fn set_register(&mut self, register: Register, value: u8) {
        match register {
            Register::B => self.registers.b = value,
            Register::C => self.registers.c = value,
            Register::D => self.registers.d = value,
            Register::E => self.registers.e = value,
            Register::H => self.registers.h = value,
            Register::L => self.registers.l = value,
            Register::mHL => self.memory.write(self.registers.hl(), value),
            Register::A => self.registers.a = value,
        }
    }

    fn register16_mem(&mut self, register: Register16Mem) -> u16 {
        match register {
            Register16Mem::BC => self.registers.bc(),
            Register16Mem::DE => self.registers.de(),
            Register16Mem::HLIncrement => self.registers.hl_increment(),
            Register16Mem::HLDecrement => self.registers.hl_decrement(),
        }
    }

    fn read_register16_mem(&mut self, register: Register16Mem) -> u8 {
        let address = self.register16_mem(register);
        self.memory.read(address)
    }

    pub fn fetch(&mut self) -> OpCode {
        OpCode::parse(self)
    }

    pub fn execute(&mut self, opcode: OpCode) {
        // TODO track cycles and handle interrupts
        match opcode {
            OpCode::Load { source, destination } => {
                self.set_register(destination, self.register(source));
            }
            OpCode::LoadImmediate { register, value } => {
                self.set_register(register, value);
            }
            OpCode::LoadIndirectAccumulator { register } => {
                let address = self.register16_mem(register);
                self.memory.write(address, self.registers.a);
            }
            OpCode::LoadAccumulatorIndirect { register } => {
                let address = self.register16_mem(register);
                self.registers.a = self.memory.read(address);
            }
            OpCode::LoadAccumulatorDirect { address } => {
                self.registers.a = self.memory.read(address);
            }
            OpCode::LoadDirectAccumulator { address } => {
                self.memory.write(address, self.registers.a);
            }
            OpCode::LoadHighAccumulatorIndirect => {
                let address = 0xFF00 | (self.registers.c as u16);
                self.registers.a = self.memory.read(address);
            }
            OpCode::LoadHighIndirectAccumulator => {
                let address = 0xFF00 | (self.registers.c as u16);
                self.memory.write(address, self.registers.a);
            }
            OpCode::LoadHighDirectAccumulator { lsb } => {
                let address = 0xFF00 | (lsb as u16);
                self.memory.write(address, self.registers.a);
            }
            OpCode::LoadHighAccumulatorDirect { lsb } => {
                let address = 0xFF00 | (lsb as u16);
                self.registers.a = self.memory.read(address);
            }
            OpCode::Load16Immediate { .. } => {}
            OpCode::LoadDirectStackPointer { .. } => {}
            OpCode::LoadStackPointerHL => {}
            OpCode::Push { .. } => {}
            OpCode::Pop { .. } => {}
            OpCode::LoadHLAdjustedStackPointer { .. } => {}
            OpCode::Add { .. } => {}
            OpCode::AddImmediate { .. } => {}
            OpCode::AddWithCarry { .. } => {}
            OpCode::AddWithCarryImmediate { .. } => {}
            OpCode::Subtract { .. } => {}
            OpCode::SubtractImmediate { .. } => {}
            OpCode::SubtractWithCarry { .. } => {}
            OpCode::SubtractWithCarryImmediate { .. } => {}
            OpCode::Compare { .. } => {}
            OpCode::CompareImmediate { .. } => {}
            OpCode::Increment { .. } => {}
            OpCode::Decrement { .. } => {}
            OpCode::And { .. } => {}
            OpCode::AndImmediate { .. } => {}
            OpCode::Or { .. } => {}
            OpCode::OrImmediate { .. } => {}
            OpCode::Xor { .. } => {}
            OpCode::XorImmediate { .. } => {}
            OpCode::ComplementCarryFlag => {}
            OpCode::SetCarryFlag => {}
            OpCode::DecimalAdjustAccumulator => {}
            OpCode::ComplementAccumulator => {}
            OpCode::Increment16 { .. } => {}
            OpCode::Decrement16 { .. } => {}
            OpCode::Add16 { .. } => {}
            OpCode::AddStackPointer { .. } => {}
            OpCode::RotateLeftWithCarryAccumulator => {}
            OpCode::RotateRightWithCarryAccumulator => {}
            OpCode::RotateLeftAccumulator => {}
            OpCode::RotateRightAccumulator => {}
            OpCode::RotateLeftCircular { .. } => {}
            OpCode::RotateRightCircular { .. } => {}
            OpCode::RotateLeft { .. } => {}
            OpCode::RotateRight { .. } => {}
            OpCode::ShiftLeftArithmetic { .. } => {}
            OpCode::ShiftRightArithmetic { .. } => {}
            OpCode::Swap { .. } => {}
            OpCode::ShiftRightLogical { .. } => {}
            OpCode::TestBit { .. } => {}
            OpCode::ResetBit { .. } => {}
            OpCode::SetBit { .. } => {}
            OpCode::Jump { .. } => {}
            OpCode::JumpHL => {}
            OpCode::JumpConditional { .. } => {}
            OpCode::JumpRelative { .. } => {}
            OpCode::JumpRelativeConditional { .. } => {}
            OpCode::Call { .. } => {}
            OpCode::CallConditional { .. } => {}
            OpCode::Return => {}
            OpCode::ReturnConditional { .. } => {}
            OpCode::ReturnInterrupt => {}
            OpCode::Restart { .. } => {}
            OpCode::Halt => {}
            OpCode::Stop => {}
            OpCode::Nop => {}
            OpCode::DisableInterrupts => {}
            OpCode::EnableInterrupts => {}
            OpCode::Illegal { .. } => {}
        }
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
        let opcode = self.memory.read(self.registers.pc);
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
                            let mut core = Core::dmg_empty();
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
                let mut core = Core::dmg_empty();
                core.registers.set_hl(0xC000); // first byte of WRAM
                core.set_register(register, 0x42);
                core.execute(OpCode::Load { source: register, destination: mHL });
                assert_eq!(core.memory.read(0xC000), 0x42);
            }
            // special case for H & L
            let mut core = Core::dmg_empty();
            core.registers.set_hl(0xC010);
            core.execute(OpCode::Load { source: H, destination: mHL });
            assert_eq!(core.memory.read(0xC010), 0xC0);

            core.registers.set_hl(0xC010);
            core.execute(OpCode::Load { source: L, destination: mHL });
            assert_eq!(core.memory.read(0xC010), 0x10);
        }

        #[test]
        fn ld_r_hl() {
            for register in [B, C, D, E, A] {
                let mut core = Core::dmg_empty();
                core.registers.set_hl(0xC000); // first byte of WRAM
                core.memory.write(0xC000, 0x42);
                core.execute(OpCode::Load { source: mHL, destination: register });
                assert_eq!(core.register(register), 0x42);
            }
            // special case for H & L
            let mut core = Core::dmg_empty();
            core.registers.set_hl(0xC010);
            core.memory.write(0xC010, 0x11);
            core.execute(OpCode::Load { source: mHL, destination: H });
            assert_eq!(core.register(H), 0x11);

            core.registers.set_hl(0xC010);
            core.memory.write(0xC010, 0x10);
            core.execute(OpCode::Load { source: mHL, destination: L });
            assert_eq!(core.register(L), 0x10);
        }

        #[test]
        fn ld_r_n() {
            let mut core = Core::dmg_empty();
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
            let mut core = Core::dmg_empty();
            core.registers.set_bc(0xC000);
            core.registers.a = 0x42;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::BC });
            assert_eq!(core.memory.read(0xC000), 0x42);

            // Test LoadIndirectAccumulator with DE
            core.registers.set_de(0xC010);
            core.registers.a = 0x99;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::DE });
            assert_eq!(core.memory.read(0xC010), 0x99);

            // Test LoadIndirectAccumulator with HL+ (should increment HL after)
            core.registers.set_hl(0xC020);
            core.registers.a = 0xAA;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::HLIncrement });
            assert_eq!(core.memory.read(0xC020), 0xAA);
            assert_eq!(core.registers.hl(), 0xC021);

            // Test LoadIndirectAccumulator with HL- (should decrement HL after)
            core.registers.set_hl(0xC030);
            core.registers.a = 0xBB;
            core.execute(OpCode::LoadIndirectAccumulator { register: Register16Mem::HLDecrement });
            assert_eq!(core.memory.read(0xC030), 0xBB);
            assert_eq!(core.registers.hl(), 0xC02F);
        }

        #[test]
        fn ld_a_indirect() {
            use crate::opcode::Register16Mem;

            // Test LoadAccumulatorIndirect with BC
            let mut core = Core::dmg_empty();
            core.registers.set_bc(0xC000);
            core.memory.write(0xC000, 0x33);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::BC });
            assert_eq!(core.registers.a, 0x33);

            // Test LoadAccumulatorIndirect with DE
            core.registers.set_de(0xC010);
            core.memory.write(0xC010, 0x77);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::DE });
            assert_eq!(core.registers.a, 0x77);

            // Test LoadAccumulatorIndirect with HL+ (should increment HL after)
            core.registers.set_hl(0xC020);
            core.memory.write(0xC020, 0xCC);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::HLIncrement });
            assert_eq!(core.registers.a, 0xCC);
            assert_eq!(core.registers.hl(), 0xC021);

            // Test LoadAccumulatorIndirect with HL- (should decrement HL after)
            core.registers.set_hl(0xC030);
            core.memory.write(0xC030, 0xDD);
            core.execute(OpCode::LoadAccumulatorIndirect { register: Register16Mem::HLDecrement });
            assert_eq!(core.registers.a, 0xDD);
            assert_eq!(core.registers.hl(), 0xC02F);
        }

        #[test]
        fn ld_a_direct() {
            // Test LoadAccumulatorDirect - load accumulator from direct address
            let mut core = Core::dmg_empty();

            // Test with WRAM address
            core.memory.write(0xC000, 0x88);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC000 });
            assert_eq!(core.registers.a, 0x88);

            // Test with different address and value
            core.memory.write(0xC123, 0xAB);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC123 });
            assert_eq!(core.registers.a, 0xAB);

            // Test with high memory address (0xFF80-0xFFFE range)
            core.memory.write(0xFF80, 0x55);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xFF80 });
            assert_eq!(core.registers.a, 0x55);

            // Test edge case with 0x00 value
            core.memory.write(0xC200, 0x00);
            core.execute(OpCode::LoadAccumulatorDirect { address: 0xC200 });
            assert_eq!(core.registers.a, 0x00);
        }

        #[test]
        fn ld_direct_a() {
            // Test LoadDirectAccumulator - store accumulator to direct address
            let mut core = Core::dmg_empty();

            // Test with WRAM address
            core.registers.a = 0x77;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC000 });
            assert_eq!(core.memory.read(0xC000), 0x77);

            // Test with different address and value
            core.registers.a = 0xCD;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC456 });
            assert_eq!(core.memory.read(0xC456), 0xCD);

            // Test with high memory address (0xFF80-0xFFFE range)
            core.registers.a = 0x99;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xFF80 });
            assert_eq!(core.memory.read(0xFF80), 0x99);

            // Test edge case with 0x00 value
            core.registers.a = 0x00;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC300 });
            assert_eq!(core.memory.read(0xC300), 0x00);

            // Test edge case with 0xFF value
            core.registers.a = 0xFF;
            core.execute(OpCode::LoadDirectAccumulator { address: 0xC400 });
            assert_eq!(core.memory.read(0xC400), 0xFF);
        }

        #[test]
        fn ldh_a_c() {
            // Test LoadHighAccumulatorIndirect - load accumulator from 0xFF00+C
            let mut core = Core::dmg_empty();

            // Test with C = 0x00 (address 0xFF00)
            core.registers.c = 0x00;
            core.memory.write(0xFF00, 0x42);
            core.execute(OpCode::LoadHighAccumulatorIndirect);
            assert_eq!(core.registers.a, 0x42);

            // Test with C = 0x80 (address 0xFF80)
            core.registers.c = 0x80;
            core.memory.write(0xFF80, 0xAB);
            core.execute(OpCode::LoadHighAccumulatorIndirect);
            assert_eq!(core.registers.a, 0xAB);

            // Test with C = 0xFF (address 0xFFFF)
            core.registers.c = 0xFF;
            core.memory.write(0xFFFF, 0x33);
            core.execute(OpCode::LoadHighAccumulatorIndirect);
            assert_eq!(core.registers.a, 0x33);

            // Test with C = 0x50 (address 0xFF50 - common I/O register area)
            core.registers.c = 0x50;
            core.memory.write(0xFF50, 0x7F);
            core.execute(OpCode::LoadHighAccumulatorIndirect);
            assert_eq!(core.registers.a, 0x7F);
        }

        #[test]
        fn ldh_c_a() {
            // Test LoadHighIndirectAccumulator - store accumulator to 0xFF00+C
            let mut core = Core::dmg_empty();

            // Test with C = 0x00 (address 0xFF00)
            core.registers.c = 0x00;
            core.registers.a = 0x55;
            core.execute(OpCode::LoadHighIndirectAccumulator);
            assert_eq!(core.memory.read(0xFF00), 0x55);

            // Test with C = 0x80 (address 0xFF80)
            core.registers.c = 0x80;
            core.registers.a = 0xCD;
            core.execute(OpCode::LoadHighIndirectAccumulator);
            assert_eq!(core.memory.read(0xFF80), 0xCD);

            // Test with C = 0xFF (address 0xFFFF)
            core.registers.c = 0xFF;
            core.registers.a = 0x99;
            core.execute(OpCode::LoadHighIndirectAccumulator);
            assert_eq!(core.memory.read(0xFFFF), 0x99);

            // Test with C = 0x40 (address 0xFF40 - LCD control register)
            core.registers.c = 0x40;
            core.registers.a = 0x91;
            core.execute(OpCode::LoadHighIndirectAccumulator);
            assert_eq!(core.memory.read(0xFF40), 0x91);
        }

        #[test]
        fn ldh_a_n() {
            // Test LoadHighAccumulatorDirect - load accumulator from 0xFF00+n
            let mut core = Core::dmg_empty();

            // Test with n = 0x00 (address 0xFF00)
            core.memory.write(0xFF00, 0x88);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0x00 });
            assert_eq!(core.registers.a, 0x88);

            // Test with n = 0x80 (address 0xFF80)
            core.memory.write(0xFF80, 0xEE);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0x80 });
            assert_eq!(core.registers.a, 0xEE);

            // Test with n = 0xFF (address 0xFFFF)
            core.memory.write(0xFFFF, 0x11);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0xFF });
            assert_eq!(core.registers.a, 0x11);

            // Test with n = 0x44 (address 0xFF44 - LCD Y coordinate register)
            core.memory.write(0xFF44, 0x90);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0x44 });
            assert_eq!(core.registers.a, 0x90);

            // Test edge case with 0x00 value
            core.memory.write(0xFF10, 0x00);
            core.execute(OpCode::LoadHighAccumulatorDirect { lsb: 0x10 });
            assert_eq!(core.registers.a, 0x00);
        }

        #[test]
        fn ldh_n_a() {
            // Test LoadHighDirectAccumulator - store accumulator to 0xFF00+n
            let mut core = Core::dmg_empty();

            // Test with n = 0x00 (address 0xFF00)
            core.registers.a = 0x77;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x00 });
            assert_eq!(core.memory.read(0xFF00), 0x77);

            // Test with n = 0x80 (address 0xFF80)
            core.registers.a = 0xBB;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x80 });
            assert_eq!(core.memory.read(0xFF80), 0xBB);

            // Test with n = 0xFF (address 0xFFFF)
            core.registers.a = 0x22;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0xFF });
            assert_eq!(core.memory.read(0xFFFF), 0x22);

            // Test with n = 0x41 (address 0xFF41 - LCD status register)
            core.registers.a = 0x85;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x41 });
            assert_eq!(core.memory.read(0xFF41), 0x85);

            // Test edge cases
            core.registers.a = 0x00;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x20 });
            assert_eq!(core.memory.read(0xFF20), 0x00);

            core.registers.a = 0xFF;
            core.execute(OpCode::LoadHighDirectAccumulator { lsb: 0x30 });
            assert_eq!(core.memory.read(0xFF30), 0xFF);
        }
    }

    #[test]
    fn core_initialization() {
        let core = Core::dmg_empty();
        assert_eq!(core.registers.a, 0x01);
        assert_eq!(core.registers.f.z, true);
        assert_eq!(core.registers.pc, 0x0100);
        assert_eq!(core.memory.read(0xFF00), 0); // Assuming memory is initialized to zero
    }
}