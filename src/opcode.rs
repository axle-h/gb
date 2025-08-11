use std::fmt::Display;
use crate::core::Fetch;

/// https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum Register {
    B = 0,
    C = 1,
    D = 2,
    E = 3,
    H = 4,
    L = 5,
    #[strum(serialize = "(HL)")]
    mHL = 6,
    A = 7,
}

impl Register {
    pub fn from_u8(value: u8) -> Self {
        Register::from_repr(value).expect("Invalid Register8 value")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum Register16 {
    BC = 0,
    DE = 1,
    HL = 2,
    SP = 3,
}

impl Register16 {
    pub fn from_u8(value: u8) -> Self {
        Register16::from_repr(value).expect("Invalid Register16 value")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum Register16Mem {
    #[strum(serialize = "(BC)")]
    BC = 0,
    #[strum(serialize = "(DE)")]
    DE = 1,
    #[strum(serialize = "(HL+)")]
    HLIncrement = 2,
    #[strum(serialize = "(HL-)")]
    HLDecrement = 3,
}

impl Register16Mem {
    pub fn from_u8(value: u8) -> Self {
        Register16Mem::from_repr(value).expect("Invalid Register16Mem value")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum Register16Stack {
    #[strum(serialize = "BC")]
    BC = 0,
    #[strum(serialize = "DE")]
    DE = 1,
    #[strum(serialize = "HL")]
    HL = 2,
    #[strum(serialize = "AF")]
    AF = 3,
}

impl Register16Stack {
    pub fn from_u8(value: u8) -> Self {
        Register16Stack::from_repr(value).expect("Invalid Register16Stack value")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum JumpCondition {
    #[strum(serialize = "NZ")]
    NotZero = 0,
    #[strum(serialize = "Z")]
    Zero = 1,
    #[strum(serialize = "NC")]
    NotCarry = 2,
    #[strum(serialize = "C")]
    Carry = 3,
}

impl JumpCondition {
    pub fn from_u8(value: u8) -> Self {
        JumpCondition::from_repr(value).expect("Invalid JumpCondition value")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display)]
pub enum OpCode {
    // *** 8-bit Load instructions ***

    /// LD r, r’: Load register (register)
    /// Load to the 8-bit register r, data from the 8-bit register r'.
    /// Opcode 0b01xxxyyy/various
    /// Duration 1 machine cycle
    #[strum(to_string = "LD {destination}, {source}")]
    Load { destination: Register, source: Register },

    /// LD r, n: Load register (immediate)
    /// Load to the 8-bit register r, the immediate data n.
    #[strum(to_string = "LD {register}, {value:#04x}")]
    LoadImmediate { register: Register, value: u8 },

    /// LD (r16mem), A: Load from accumulator (indirect 16-bit register)
    /// Load to the absolute address specified by the 16-bit register r16mem, data from the 8-bit A register.
    #[strum(to_string = "LD {register}, A")]
    LoadIndirectAccumulator { register: Register16Mem },

    /// LD A, (r16mem): Load accumulator (indirect 16-bit register)
    /// Load to the 8-bit A register, data from the absolute address specified by the 16-bit register r16mem.
    #[strum(to_string = "LD A, {register}")]
    LoadAccumulatorIndirect { register: Register16Mem },

    /// LD A, (nn): Load accumulator (direct)
    /// Load to the 8-bit A register, data from the absolute address specified by the 16-bit operand nn.
    #[strum(to_string = "LD A, ({address:#06x})")]
    LoadAccumulatorDirect { address: u16 },

    /// LD (nn), A: Load from accumulator (direct)
    /// Load to the absolute address specified by the 16-bit operand nn, data from the 8-bit A register.
    #[strum(to_string = "LD ({address:#06x}), A")]
    LoadDirectAccumulator { address: u16 },

    /// LDH A, (C): Load accumulator (indirect 0xFF00+C)
    /// Load to the 8-bit A register, data from the address specified by the 8-bit C register. The full
    /// 16-bit absolute address is obtained by setting the most significant byte to 0xFF and the least
    /// significant byte to the value of C, so the possible range is 0xFF00-0xFFFF.
    #[strum(to_string = "LDH A, (C)")]
    LoadHighAccumulatorIndirect,

    /// LDH (C), A: Load from accumulator (indirect 0xFF00+C)
    /// Load to the address specified by the 8-bit C register, data from the 8-bit A register. The full
    /// 16-bit absolute address is obtained by setting the most significant byte to 0xFF and the least
    /// significant byte to the value of C, so the possible range is 0xFF00-0xFFFF.
    #[strum(to_string = "LDH (C), A")]
    LoadHighIndirectAccumulator,


    /// LDH (n), A: Load from accumulator (direct 0xFF00+n)
    /// Load to the address specified by the 8-bit immediate data n, data from the 8-bit A register. The
    /// full 16-bit absolute address is obtained by setting the most significant byte to 0xFF and the
    /// least significant byte to the value of n, so the possible range is 0xFF00-0xFFFF.
    #[strum(to_string = "LDH ({lsb:#04x}), A")]
    LoadHighDirectAccumulator { lsb: u8 },

    /// LDH A, (n): Load accumulator (direct 0xFF00+n)
    /// Load to the 8-bit A register, data from the address specified by the 8-bit immediate data n. The
    /// full 16-bit absolute address is obtained by setting the most significant byte to 0xFF and the
    /// least significant byte to the value of n, so the possible range is 0xFF00-0xFFFF.
    #[strum(to_string = "LDH A, ({lsb:#04x})")]
    LoadHighAccumulatorDirect { lsb: u8 },

    // *** 16-bit Load instructions ***

    /// LD rr, nn: Load 16-bit register / register pair
    /// Load to the 16-bit register rr, the immediate 16-bit data nn.
    #[strum(to_string = "LD {register}, {value:#06x}")]
    Load16Immediate { register: Register16, value: u16 },

    /// LD (nn), SP: Load from stack pointer (direct)
    /// Load to the absolute address specified by the 16-bit operand nn, data from the 16-bit SP register.
    #[strum(to_string = "LD ({address:#06x}), SP")]
    LoadDirectStackPointer { address: u16 },

    /// LD SP, HL: Load stack pointer from HL
    /// Load to the 16-bit SP register, data from the 16-bit HL register
    #[strum(to_string = "LD SP, HL")]
    LoadStackPointerHL,

    /// PUSH rr: Push to stack
    /// Push to the stack memory, data from the 16-bit register rr.
    #[strum(to_string = "PUSH {register}")]
    Push { register: Register16Stack },

    /// POP rr: Pop from stack
    /// Pops to the 16-bit register rr, data from the stack memory.
    /// This instruction does not do calculations that affect flags, but POP AF completely replaces the
    /// F register value, so all flags are changed based on the 8-bit data that is read from memory.
    #[strum(to_string = "POP {register}")]
    Pop { register: Register16Stack },

    /// LD HL, SP+e: Load HL from adjusted stack pointer
    /// Load to the HL register, 16-bit data calculated by adding the signed 8-bit operand e to the 16-
    /// bit value of the SP register.
    /// Flags: Z = 0, N = 0, H = *, C = *
    #[strum(to_string = "LD HL, SP{offset:+}")]
    LoadHLAdjustedStackPointer { offset: i8 },

    // *** 8-bit arithmetic and logical instructions ***

    /// ADD r: Add (register)
    /// Adds to the 8-bit A register, the 8-bit register r, and stores the result back into the A register
    /// Flags Z = *, N = 0, H = *, C = *
    #[strum(to_string = "ADD A, {register}")]
    Add { register: Register },

    /// ADD n: Add (immediate)
    /// Adds to the 8-bit A register, the immediate 8-bit data n, and stores the result back into the A register.
    /// Flags Z = *, N = 0, H = *, C = *
    #[strum(to_string = "ADD A, {value:#04x}")]
    AddImmediate { value: u8 },

    /// ADC r: Add with carry (register)
    /// Adds to the 8-bit A register, the carry flag and the 8-bit register r, and stores the result back
    /// into the A register.
    /// Flags Z = *, N = 0, H = *, C = *
    #[strum(to_string = "ADC A, {register}")]
    AddWithCarry { register: Register },

    /// ADC n: Add with carry (immediate)
    /// Adds to the 8-bit A register, the carry flag and the immediate data n, and stores the result back
    /// into the A register.
    /// Flags Z = *, N = 0, H = *, C = *
    #[strum(to_string = "ADC A, {value:#04x}")]
    AddWithCarryImmediate { value: u8 },

    /// SUB r: Subtract (register)
    /// Subtracts from the 8-bit A register, the 8-bit register r, and stores the result back into the A
    /// register.
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "SUB A, {register}")]
    Subtract { register: Register },

    /// SUB n: Subtract (immediate)
    /// Subtracts from the 8-bit A register, the immediate data n, and stores the result back into the A
    /// register
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "SUB A, {value:#04x}")]
    SubtractImmediate { value: u8 },

    /// SBC r: Subtract with carry (register)
    /// Subtracts from the 8-bit A register, the carry flag and the 8-bit register r, and stores the result
    /// back into the A register.
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "SBC A, {register}")]
    SubtractWithCarry { register: Register },

    /// SBC n: Subtract with carry (immediate)
    /// Subtracts from the 8-bit A register, the carry flag and the immediate data n, and stores the
    /// result back into the A register.
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "SBC A, {value:#04x}")]
    SubtractWithCarryImmediate { value: u8 },

    /// CP r: Compare (register)
    /// Subtracts from the 8-bit A register, the 8-bit register r, and updates flags based on the result.
    /// This instruction is basically identical to SUB r, but does not update the A register.
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "CP A, {register}")]
    Compare { register: Register },

    /// CP n: Compare (immediate)
    /// Subtracts from the 8-bit A register, the immediate data n, and updates flags based on the result.
    /// This instruction is basically identical to SUB n, but does not update the A register.
    /// Flags Z = *, N = 1, H = *, C = *
    #[strum(to_string = "CP A, {value:#04x}")]
    CompareImmediate { value: u8 },

    /// INC r: Increment (register)
    /// Increments data in the 8-bit register r.
    /// Flags Z = *, N = 0, H = *
    #[strum(to_string = "INC {register}")]
    Increment { register: Register },

    /// DEC r: Decrement (register)
    /// Decrements data in the 8-bit register r
    /// Flags Z = *, N = 1, H = *
    #[strum(to_string = "DEC {register}")]
    Decrement { register: Register },

    /// AND r: Bitwise AND (register)
    /// Performs a bitwise AND operation between the 8-bit A register and the 8-bit register r, and
    /// stores the result back into the A register
    /// Flags Z = *, N = 0, H = 1, C = 0
    #[strum(to_string = "AND A, {register}")]
    And { register: Register },

    /// AND n: Bitwise AND (immediate)
    /// Performs a bitwise AND operation between the 8-bit A register and immediate data n, and
    /// stores the result back into the A register.
    /// Flags Z = *, N = 0, H = 1, C = 0
    #[strum(to_string = "AND A, {value:#04x}")]
    AndImmediate { value: u8 },

    /// OR r: Bitwise OR (register)
    /// Performs a bitwise OR operation between the 8-bit A register and the 8-bit register r, and stores
    /// the result back into the A register.
    /// Flags Z = *, N = 0, H = 0, C = 0
    #[strum(to_string = "OR A, {register}")]
    Or { register: Register },

    /// OR n: Bitwise OR (immediate)
    /// Performs a bitwise OR operation between the 8-bit A register and immediate data n, and stores
    /// the result back into the A register.
    /// Flags Z = *, N = 0, H = 0, C = 0
    #[strum(to_string = "OR A, {value:#04x}")]
    OrImmediate { value: u8 },

    /// XOR r: Bitwise XOR (register)
    /// Performs a bitwise XOR operation between the 8-bit A register and the 8-bit register r, and
    /// stores the result back into the A register.
    /// Flags Z = *, N = 0, H = 0, C = 0
    #[strum(to_string = "XOR A, {register}")]
    Xor { register: Register },

    /// XOR n: Bitwise XOR (immediate)
    /// Performs a bitwise XOR operation between the 8-bit A register and immediate data n, and
    /// stores the result back into the A register
    /// Flags Z = *, N = 0, H = 0, C = 0
    #[strum(to_string = "XOR A, {value:#04x}")]
    XorImmediate { value: u8 },

    /// CCF: Complement carry flag
    /// Flips the carry flag, and clears the N and H flags.
    /// Flags N = 0, H = 0, C = *
    #[strum(to_string = "CCF")]
    ComplementCarryFlag,

    /// SCF: Set carry flag
    /// Sets the carry flag, and clears the N and H flags.
    /// Flags N = 0, H = 0, C = 1
    #[strum(to_string = "SCF")]
    SetCarryFlag,

    /// DAA: Decimal adjust accumulator
    /// Z = *, H = 0, C = *
    #[strum(to_string = "DAA")]
    DecimalAdjustAccumulator,

    /// CPL: Complement accumulator
    /// Flips all the bits in the 8-bit A register, and sets the N and H flags.
    /// N = 1, H = 1
    #[strum(to_string = "CPL")]
    ComplementAccumulator,

    // *** 16-bit arithmetic instructions ***

    /// INC rr: Increment 16-bit register
    /// Increments data in the 16-bit register rr
    #[strum(to_string = "INC {register}")]
    Increment16 { register: Register16 },

    /// DEC rr: Decrement 16-bit register
    /// Decrements data in the 16-bit register rr.
    #[strum(to_string = "DEC {register}")]
    Decrement16 { register: Register16 },

    /// ADD HL, rr: Add (16-bit register)
    /// Adds to the 16-bit HL register pair, the 16-bit register rr, and stores the result back into the HL
    /// register pair.
    /// Flags N = 0, H = *, C = *
    #[strum(to_string = "ADD HL, {register}")]
    Add16 { register: Register16 },

    /// ADD SP, e: Add to stack pointer (relative)
    /// Loads to the 16-bit SP register, 16-bit data calculated by adding the signed 8-bit operand e to
    /// the 16-bit value of the SP register.
    /// Flags Z = 0, N = 0, H = *, C = *
    #[strum(to_string = "ADD SP, {offset}")]
    AddStackPointer { offset: i8 },

    // *** Rotate, shift, and bit operation instructions ***

    /// RLCA: Rotate left circular (accumulator)
    /// Rotates the 8-bit A register value left in a circular manner (carry flag is updated but not used).
    /// Every bit is shifted to the left (e.g. bit 1 value is copied from bit 0). Bit 7 is copied both to bit
    /// 0 and the carry flag. Note that unlike the related RLC r  instruction, RLCA always sets the zero
    /// flag to 0 without looking at the resulting value of the calculation.
    /// Flags Z = 0, N = 0, H = 0, C = *
    #[strum(to_string = "RLCA")]
    RotateLeftWithCarryAccumulator,

    /// RRCA: Rotate right circular (accumulator)
    /// Rotates the 8-bit A register value right in a circular manner (carry flag is updated but not used).
    /// Every bit is shifted to the right (e.g. bit 1 value is copied to bit 0). Bit 0 is copied both to bit 7
    /// and the carry flag. Note that unlike the related RRC r  instruction, RRCA always sets the zero
    /// flag to 0 without looking at the resulting value of the calculation.
    /// Flags Z = 0, N = 0, H = 0, C = *
    #[strum(to_string = "RRCA")]
    RotateRightWithCarryAccumulator,

    /// RLA: Rotate left (accumulator)
    /// Rotates the 8-bit A register value left through the carry flag.
    /// Every bit is shifted to the left (e.g. bit 1 value is copied from bit 0). The carry flag is copied to bit
    /// 0, and bit 7 is copied to the carry flag. Note that unlike the related RL r  instruction, RLA always
    /// sets the zero flag to 0 without looking at the resulting value of the calculation.
    /// Flags Z = 0, N = 0, H = 0, C = *
    #[strum(to_string = "RLA")]
    RotateLeftAccumulator,

    /// RRA: Rotate right (accumulator)
    /// Rotates the 8-bit A register value right through the carry flag.
    /// Every bit is shifted to the right (e.g. bit 1 value is copied to bit 0). The carry flag is copied to bit
    /// 7, and bit 0 is copied to the carry flag. Note that unlike the related RR r  instruction, RRA always
    /// sets the zero flag to 0 without looking at the resulting value of the calculation.
    /// Flags Z = 0, N = 0, H = 0, C = *
    #[strum(to_string = "RRA")]
    RotateRightAccumulator,

    /// RLC r: Rotate left circular (register)
    /// Rotates the 8-bit register r value left in a circular manner (carry flag is updated but not used).
    /// Every bit is shifted to the left (e.g. bit 1 value is copied from bit 0). Bit 7 is copied both to bit 0
    /// and the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "RLC {register}")]
    RotateLeftCircular { register: Register },

    /// RRC r: Rotate right circular (register)
    /// Rotates the 8-bit register r value right in a circular manner (carry flag is updated but not used).
    /// Every bit is shifted to the right (e.g. bit 1 value is copied to bit 0). Bit 0 is copied both to bit 7
    /// and the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "RRC {register}")]
    RotateRightCircular { register: Register },

    /// RL r: Rotate left (register)
    /// Rotates the 8-bit register r value left through the carry flag.
    /// Every bit is shifted to the left (e.g. bit 1 value is copied from bit 0). The carry flag is copied to bit
    /// 0, and bit 7 is copied to the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "RL {register}")]
    RotateLeft { register: Register },

    /// RR r: Rotate right (register)
    /// Rotates the 8-bit register r value right through the carry flag.
    /// Every bit is shifted to the right (e.g. bit 1 value is copied to bit 0). The carry flag is copied to bit
    /// 7, and bit 0 is copied to the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "RR {register}")]
    RotateRight { register: Register },

    /// SLA r: Shift left arithmetic (register)
    /// Shifts the 8-bit register r value left by one bit using an arithmetic shift.
    /// Bit 7 is shifted to the carry flag, and bit 0 is set to a fixed value of 0.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "SLA {register}")]
    ShiftLeftArithmetic { register: Register },

    /// SRA r: Shift right arithmetic (register)
    /// Shifts the 8-bit register r value right by one bit using an arithmetic shift.
    /// Bit 7 retains its value, and bit 0 is shifted to the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "SRA {register}")]
    ShiftRightArithmetic { register: Register },

    /// SWAP r: Swap nibbles (register)
    /// Swaps the high and low 4-bit nibbles of the 8-bit register r.
    /// Flags Z = *, N = 0, H = 0, C = 0
    #[strum(to_string = "SWAP {register}")]
    Swap { register: Register },

    /// SRL r: Shift right logical (register)
    /// Shifts the 8-bit register r value right by one bit using a logical shift.
    /// Bit 7 is set to a fixed value of 0, and bit 0 is shifted to the carry flag.
    /// Flags Z = *, N = 0, H = 0, C = *
    #[strum(to_string = "SRL {register}")]
    ShiftRightLogical { register: Register },

    /// BIT b, r: Test bit (register)
    /// Tests the bit b of the 8-bit register r.
    /// The zero flag is set to 1 if the chosen bit is 0, and 0 otherwise.
    /// Flags Z = *, N = 0, H = 1
    #[strum(to_string = "BIT {bit}, {register}")]
    TestBit { register: Register, bit: u8 },

    /// RES b, r: Reset bit (register)
    /// Resets the bit b of the 8-bit register r to 0
    #[strum(to_string = "RES {bit}, {register}")]
    ResetBit { register: Register, bit: u8 },

    /// SET b, r: Set bit (register)
    /// Sets the bit b of the 8-bit register r to 1.
    #[strum(to_string = "SET {bit}, {register}")]
    SetBit { register: Register, bit: u8 },

    // *** Control flow instructions ***

    /// JP nn: Jump
    /// Unconditional jump to the absolute address specified by the 16-bit immediate operand nn.
    #[strum(to_string = "JP {address:#06x}")]
    Jump { address: u16 },

    /// JP HL: Jump to HL
    /// Unconditional jump to the absolute address specified by the 16-bit register HL
    #[strum(to_string = "JP HL")]
    JumpHL,

    /// P cc, nn: Jump (conditional)
    /// Conditional jump to the absolute address specified by the 16-bit operand nn, depending on the
    /// condition cc.
    #[strum(to_string = "JP {condition}, {address:#06x}")]
    JumpConditional { condition: JumpCondition, address: u16 },

    /// JR e: Relative jump
    /// Unconditional jump to the relative address specified by the signed 8-bit operand e.
    #[strum(to_string = "JR {offset}")]
    JumpRelative { offset: i8 },

    /// JR cc, e: Relative jump (conditional)
    /// Conditional jump to the relative address specified by the signed 8-bit operand e, depending on
    /// the condition cc.
    #[strum(to_string = "JR {condition}, {offset}")]
    JumpRelativeConditional { condition: JumpCondition, offset: i8 },

    /// CALL nn: Call function
    /// Unconditional call to the function at the absolute address specified by the 16-bit operand nn
    #[strum(to_string = "CALL {address:#06x}")]
    Call { address: u16 },

    /// CALL cc, nn: Call function (conditional)
    /// Conditional function call to the absolute address specified by the 16-bit operand nn, depending
    /// on the condition cc.
    #[strum(to_string = "CALL {condition}, {address:#06x}")]
    CallConditional { condition: JumpCondition, address: u16 },

    /// RET: Return from function
    /// Unconditional return from a function.
    #[strum(to_string = "RET")]
    Return,

    /// RET cc: Return from function (conditional)
    /// Conditional return from a function, depending on the condition cc.
    #[strum(to_string = "RET {condition}")]
    ReturnConditional { condition: JumpCondition },

    /// RETI: Return from interrupt handler
    /// Unconditional return from a function. Also enables interrupts by setting IME=1.
    #[strum(to_string = "RETI")]
    ReturnInterrupt,

    /// RST n: Restart
    /// Unconditional function call to the absolute fixed address defined by the opcode.
    #[strum(to_string = "RST ${lsb:02X}")]
    Restart { lsb: u8 },

    // *** Miscellaneous instructions ***
    /// HALT: Halt system clock
    #[strum(serialize = "HALT")]
    Halt,

    /// STOP: Stop system and main clocks
    #[strum(serialize = "STOP")]
    Stop,

    /// NOP: No operation
    /// No operation. This instruction doesn’t do anything, but can be used to add a delay of one
    /// machine cycle and increment PC by one.
    #[strum(serialize = "NOP")]
    Nop,

    /// DI: Disable interrupts
    /// Disables interrupt handling by setting IME=0 and cancelling any scheduled effects of the EI
    /// instruction if any.
    #[strum(serialize = "DI")]
    DisableInterrupts,

    /// EI: Enable interrupts
    /// Schedules interrupt handling to be enabled after the next machine cycle.
    #[strum(serialize = "EI")]
    EnableInterrupts,

    #[strum(serialize = "ILLEGAL_{raw:02X}")]
    Illegal { raw: u8 },
}

impl OpCode {
    pub fn machine_cycles(&self) -> u8 {
        match self {
            OpCode::Illegal { .. } => 1,
            OpCode::Nop => 1,
            OpCode::Halt => 1,
            OpCode::Stop => 1,
            OpCode::DisableInterrupts | OpCode::EnableInterrupts => 1,
            OpCode::Load { source, destination } =>
                if source == &Register::mHL || destination == &Register::mHL { 2 } else { 1 },
            OpCode::LoadImmediate { register, .. } => if register == &Register::mHL { 3 } else { 2 },
            OpCode::LoadIndirectAccumulator { .. } => 2,
            OpCode::LoadAccumulatorIndirect { .. } => 2,
            OpCode::LoadAccumulatorDirect { .. } => 4,
            OpCode::LoadDirectAccumulator { .. } => 4,
            OpCode::LoadHighAccumulatorIndirect => 2,
            OpCode::LoadHighIndirectAccumulator => 2,
            OpCode::LoadHighDirectAccumulator { .. } => 3,
            OpCode::LoadHighAccumulatorDirect { .. } => 3,
            OpCode::Load16Immediate { .. } => 3,
            OpCode::LoadDirectStackPointer { .. } => 5,
            OpCode::LoadStackPointerHL => 2,
            OpCode::Push { .. } => 4,
            OpCode::Pop { .. } => 3,
            OpCode::LoadHLAdjustedStackPointer { .. } => 3,
            OpCode::Add { register } | OpCode::AddWithCarry { register } |
            OpCode::Subtract { register } | OpCode::SubtractWithCarry { register } |
            OpCode::Compare { register } |
            OpCode::And { register } | OpCode::Or { register } | OpCode::Xor { register } =>
                if register == &Register::mHL { 2 } else { 1 },
            OpCode::AddImmediate { .. } | OpCode::AddWithCarryImmediate { .. } |
            OpCode::SubtractImmediate { .. } | OpCode::SubtractWithCarryImmediate { .. } |
            OpCode::CompareImmediate { .. } |
            OpCode::AndImmediate { .. } | OpCode::OrImmediate { .. } | OpCode::XorImmediate { .. } => 2,
            OpCode::Increment { register } | OpCode::Decrement { register } =>
                if register == &Register::mHL { 3 } else { 1 },
            OpCode::ComplementCarryFlag | OpCode::SetCarryFlag | OpCode::DecimalAdjustAccumulator | OpCode::ComplementAccumulator => 1,
            OpCode::Increment16 { .. } | OpCode::Decrement16 { .. } | OpCode::Add16 { .. } => 2,
            OpCode::AddStackPointer { .. } => 4,
            OpCode::RotateLeftWithCarryAccumulator | OpCode::RotateLeftAccumulator | OpCode::RotateRightWithCarryAccumulator | OpCode::RotateRightAccumulator => 1,
            OpCode::RotateRightCircular { register } | OpCode::RotateLeftCircular { register } |
            OpCode::RotateRight { register } | OpCode::RotateLeft { register } |
            OpCode::ShiftLeftArithmetic { register } | OpCode::ShiftRightArithmetic { register } |
            OpCode::Swap { register } | OpCode::ShiftRightLogical { register } =>
                if register == &Register::mHL { 4 } else { 2 },
            OpCode::TestBit { register, .. } => if register == &Register::mHL { 3 } else { 2 },
            OpCode::ResetBit { register, .. } | OpCode::SetBit { register, .. } => if register == &Register::mHL { 4 } else { 2 },
            OpCode::Jump { .. } => 4,
            OpCode::JumpHL => 1,
            OpCode::JumpConditional { .. } => 4, // TODO 4 is true, 3 is false
            OpCode::JumpRelative { .. } => 3,
            OpCode::JumpRelativeConditional { .. } => 3, // TODO 3 is true, 2 is false
            OpCode::Call { .. } => 6,
            OpCode::CallConditional { .. } => 6, // TODO 6 is true, 3 is false
            OpCode::Return | OpCode::ReturnInterrupt | OpCode::Restart { .. } => 4,
            OpCode::ReturnConditional { ..} => 5, // TODO 5 is true, 2 is false
            _ => unreachable!("Machine cycles not defined for opcode: {:?}", self),
        }
    }

    pub fn parse(fetch: &mut impl Fetch) -> Self {
        let raw = RawOpCode(fetch.fetch_u8());
        match raw.0 {
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                // These opcodes are not valid in the base instruction set, but are used in the extended
                // instruction set (CB prefix) or other special cases.
                OpCode::Illegal { raw: raw.0 }
            }
            0x00 => OpCode::Nop, // 0x00 NOP
            0x07 => OpCode::RotateLeftWithCarryAccumulator, // 0x07 RLCA
            0x0F => OpCode::RotateRightWithCarryAccumulator, // 0x0F RRCA
            0x17 => OpCode::RotateLeftAccumulator, // 0x17 RLA
            0x1F => OpCode::RotateRightAccumulator, // 0x1F RRA
            0x27 => OpCode::DecimalAdjustAccumulator, // 0x27 DAA
            0x2F => OpCode::ComplementAccumulator, // 0x2F CPL
            0x37 => OpCode::SetCarryFlag, // 0x37 SCF
            0x3F => OpCode::ComplementCarryFlag, // 0x3F CCF
            0x10 => OpCode::Stop, // 0x10 STOP
            0x76 => OpCode::Halt, // 0x76 HALT
            0xF3 => OpCode::DisableInterrupts, // 0xF3 DI
            0xFB => OpCode::EnableInterrupts, // 0xFB EI
            0xC9 => OpCode::Return, // 0xC9 RET
            0xD9 => OpCode::ReturnInterrupt, // 0xD9 RETI
            0x18 => OpCode::JumpRelative { offset: fetch.fetch_i8() }, // 0x18 JR e
            0x08 => OpCode::LoadDirectStackPointer { address: fetch.fetch_u16() }, // 0x08 LD (nn), SP
            0xE0 => OpCode::LoadHighDirectAccumulator { lsb: fetch.fetch_u8() }, // 0xE0 LDH A, (n)
            0xF0 => OpCode::LoadHighAccumulatorDirect { lsb: fetch.fetch_u8() }, // 0xF0 LDH (n), A
            0xE9 => OpCode::JumpHL, // 0xE9 JP HL
            0xE2 => OpCode::LoadHighIndirectAccumulator, // 0xE2 LD (C), A
            0xEA => OpCode::LoadDirectAccumulator { address: fetch.fetch_u16() }, // 0xEA LD (nn), A
            0xF2 => OpCode::LoadHighAccumulatorIndirect, // 0xF2 LD A, (C)
            0xFA => OpCode::LoadAccumulatorDirect { address: fetch.fetch_u16() }, // 0xFA LD A, (nn)
            0xC3 => OpCode::Jump { address: fetch.fetch_u16() }, // 0xC3 JP nn

            0xC6 => OpCode::AddImmediate { value: fetch.fetch_u8() }, // 0xC6 ADD A, n
            0xCE => OpCode::AddWithCarryImmediate { value: fetch.fetch_u8() }, // 0xCE ADC A, n
            0xD6 => OpCode::SubtractImmediate { value: fetch.fetch_u8() }, // 0xD6 SUB A, n
            0xDE => OpCode::SubtractWithCarryImmediate { value: fetch.fetch_u8() }, // 0xDE SBC A, n
            0xE6 => OpCode::AndImmediate { value: fetch.fetch_u8() }, // 0xE6 AND A, n
            0xEE => OpCode::XorImmediate { value: fetch.fetch_u8() }, // 0xEE XOR A, n
            0xF6 => OpCode::OrImmediate { value: fetch.fetch_u8() }, // 0xF6 OR A, n
            0xFE => OpCode::CompareImmediate { value: fetch.fetch_u8() }, // 0xFE CP A, n

            0xCD => OpCode::Call { address: fetch.fetch_u16() }, // 0xCD CALL nn

            0xE8 => OpCode::AddStackPointer { offset: fetch.fetch_i8() }, // 0xE8 ADD SP, e
            0xF8 => OpCode::LoadHLAdjustedStackPointer { offset: fetch.fetch_i8() }, // 0xF8 LD HL, SP+e
            0xF9 => OpCode::LoadStackPointerHL, // 0xF9 LD SP, HL

            0xCB => OpCode::parse_cb(fetch),
            _ => {
                match raw.x() {
                    0b00 => match raw.z() {
                        0b000 => OpCode::JumpRelativeConditional {
                            condition: raw.condition(),
                            offset: fetch.fetch_i8(),
                        },
                        0b001 => {
                            let register = Register16::from_u8(raw.p());
                            if raw.q() {
                                // add hl, r16
                                OpCode::Add16 { register }
                            } else {
                                // ld r16, imm16
                                OpCode::Load16Immediate { register, value: fetch.fetch_u16() }
                            }
                        }
                        0b010 => {
                            let register = Register16Mem::from_u8(raw.p());
                            if raw.q() {
                                // ld a, [r16mem]
                                OpCode::LoadAccumulatorIndirect { register }
                            } else {
                                // ld [r16mem], a
                                OpCode::LoadIndirectAccumulator { register }
                            }
                        }
                        0b011 => {
                            let register = Register16::from_u8(raw.p());
                            if raw.q() {
                                // inc r16
                                OpCode::Decrement16 { register }
                            } else {
                                // dec r16
                                OpCode::Increment16 { register }
                            }
                        }
                        0b100 => OpCode::Increment { register: Register::from_u8(raw.y()) },
                        0b101 => OpCode::Decrement { register: Register::from_u8(raw.y()) },
                        0b110 => OpCode::LoadImmediate { register: Register::from_u8(raw.y()), value: fetch.fetch_u8() },
                        _ => unreachable!(),
                    },
                    0b01 => OpCode::Load { source: Register::from_u8(raw.z()), destination: Register::from_u8(raw.y()) },
                    0b10 => {
                        let register = Register::from_u8(raw.z());
                        match raw.y() {
                            0b000 => OpCode::Add { register },
                            0b001 => OpCode::AddWithCarry { register },
                            0b010 => OpCode::Subtract { register },
                            0b011 => OpCode::SubtractWithCarry { register },
                            0b100 => OpCode::And { register },
                            0b101 => OpCode::Xor { register },
                            0b110 => OpCode::Or { register },
                            0b111 => OpCode::Compare { register },
                            _ => unreachable!(),
                        }
                    },
                    0b11 => {
                        match raw.z() {
                            0b000 => OpCode::ReturnConditional { condition: raw.condition() },
                            0b010 => OpCode::JumpConditional { condition: raw.condition(), address: fetch.fetch_u16() },
                            0b100 => OpCode::CallConditional { condition: raw.condition(), address: fetch.fetch_u16() },
                            0b111 => OpCode::Restart { lsb: raw.y() * 8 },
                            0b001 => OpCode::Pop { register: Register16Stack::from_u8(raw.p()) },
                            0b101 => OpCode::Push { register: Register16Stack::from_u8(raw.p()) },
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn parse_cb(fetch: &mut impl Fetch) -> Self {
        let raw = RawOpCode(fetch.fetch_u8());
        match raw.x() {
            0b00 => match raw.y() {
                0b000 => OpCode::RotateLeftCircular { register: Register::from_u8(raw.z()) },
                0b001 => OpCode::RotateRightCircular { register: Register::from_u8(raw.z()) },
                0b010 => OpCode::RotateLeft { register: Register::from_u8(raw.z()) },
                0b011 => OpCode::RotateRight { register: Register::from_u8(raw.z()) },
                0b100 => OpCode::ShiftLeftArithmetic { register: Register::from_u8(raw.z()) },
                0b101 => OpCode::ShiftRightArithmetic { register: Register::from_u8(raw.z()) },
                0b110 => OpCode::Swap { register: Register::from_u8(raw.z()) },
                0b111 => OpCode::ShiftRightLogical { register: Register::from_u8(raw.z()) },
                _ => unreachable!(),
            },
            0b01 => OpCode::TestBit { bit: raw.y(), register: Register::from_u8(raw.z()) },
            0b10 => OpCode::ResetBit { bit: raw.y(), register: Register::from_u8(raw.z()) },
            0b11 => OpCode::SetBit { bit: raw.y(), register: Register::from_u8(raw.z()) },
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawOpCode(u8);

impl RawOpCode {
    pub fn x(&self) -> u8 {
        (self.0 >> 6) & 0b11 // bit 6 and 7
    }

    pub fn y(&self) -> u8 {
        (self.0 >> 3) & 0b111 // bits 3, 4, and 5
    }

    pub fn condition(&self) -> JumpCondition {
        JumpCondition::from_u8((self.0 >> 3) & 0b11) // bits 3 & 4
    }

    pub fn z(&self) -> u8 {
        self.0 & 0b111 // bits 0, 1, and 2
    }

    pub fn p(&self) -> u8 {
        (self.0 >> 4) & 0b11 // bits 4 and 5
    }

    pub fn q(&self) -> bool {
        self.0 & 0b00001000 == 0b00001000 // bit 3
    }
}

impl Display for RawOpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#04x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use Register::{mHL, A, B, C, D, E, H, L};
    use super::*;

    #[test]
    fn register8_display() {
        assert_eq!(B.to_string(), "B");
        assert_eq!(C.to_string(), "C");
        assert_eq!(D.to_string(), "D");
        assert_eq!(E.to_string(), "E");
        assert_eq!(H.to_string(), "H");
        assert_eq!(L.to_string(), "L");
        assert_eq!(mHL.to_string(), "(HL)");
        assert_eq!(A.to_string(), "A");
    }

    #[test]
    fn register16_display() {
        assert_eq!(Register16::BC.to_string(), "BC");
        assert_eq!(Register16::DE.to_string(), "DE");
        assert_eq!(Register16::HL.to_string(), "HL");
        assert_eq!(Register16::SP.to_string(), "SP");
    }

    struct StubFetch {
        data: Vec<u8>,
        index: usize,
    }

    impl StubFetch {
        fn new(data: Vec<u8>) -> Self {
            StubFetch { data, index: 0 }
        }

        fn from_u8(data: u8) -> Self {
            StubFetch { data: vec![data], index: 0 }
        }

        fn from_u8_imm16(data: u8, imm: u16) -> Self {
            let [lsb, msb] = imm.to_le_bytes();
            StubFetch { data: vec![data, lsb, msb], index: 0 }
        }

        fn from_u8_imm8(data: u8, imm: u8) -> Self {
            StubFetch { data: vec![data, imm], index: 0 }
        }

        fn parses(&mut self, expected: OpCode) {
            assert_eq!(OpCode::parse(self), expected);
        }

        fn reset(&mut self) {
            self.index = 0;
        }
    }

    impl Fetch for StubFetch {
        fn fetch_u8(&mut self) -> u8 {
            let value = self.data[self.index];
            self.index = (self.index + 1) % self.data.len();
            value
        }
    }

    /// Macro to generate tests for opcode parsing, string representation, and machine cycles
    /// Usage: opcode_tests! {
    ///     nop: 0x00 => "NOP", 1,
    ///     ld_bc_n16: 0x01, 0x34, 0x12 => "LD BC, 0x1234", 3,
    ///     ld_bc_a: 0x02 => "LD (BC), A", 2,
    /// }
    macro_rules! opcode_tests {
        ($($test_name:ident: $($byte:expr),+ => $expected_string:expr, $expected_cycles:expr),*$(,)?) => {
            $(
                #[test]
                fn $test_name() {
                    let bytes = vec![$($byte),+];
                    let mut fetch = StubFetch::new(bytes);
                    let opcode = OpCode::parse(&mut fetch);
                    assert_eq!(opcode.to_string(), $expected_string);
                    assert_eq!(opcode.machine_cycles(), $expected_cycles);
                }
            )*
        };
    }

    mod unprefixed {
        use super::*;
        opcode_tests! {
            nop: 0x00 => "NOP", 1,
            ld_bc_n16: 0x01, 0x34, 0x12 => "LD BC, 0x1234", 3,
            ld_bc_a: 0x02 => "LD (BC), A", 2,
            inc_bc: 0x03 => "INC BC", 2,
            inc_b: 0x04 => "INC B", 1,
            dec_b: 0x05 => "DEC B", 1,
            ld_b_n8: 0x06, 0x12 => "LD B, 0x12", 2,
            rlca: 0x07 => "RLCA", 1,
            ld_a16_sp: 0x08, 0x34, 0x12 => "LD (0x1234), SP", 5,
            add_hl_bc: 0x09 => "ADD HL, BC", 2,
            ld_a_bc: 0x0A => "LD A, (BC)", 2,
            dec_bc: 0x0B => "DEC BC", 2,
            inc_c: 0x0C => "INC C", 1,
            dec_c: 0x0D => "DEC C", 1,
            ld_c_n8: 0x0E, 0x12 => "LD C, 0x12", 2,
            rrca: 0x0F => "RRCA", 1,
            stop: 0x10 => "STOP", 1,
            ld_de_n16: 0x11, 0x34, 0x12 => "LD DE, 0x1234", 3,
            ld_de_a: 0x12 => "LD (DE), A", 2,
            inc_de: 0x13 => "INC DE", 2,
            inc_d: 0x14 => "INC D", 1,
            dec_d: 0x15 => "DEC D", 1,
            ld_d_n8: 0x16, 0x12 => "LD D, 0x12", 2,
            rla: 0x17 => "RLA", 1,
            jr_e8: 0x18, 0x7B => "JR 123", 3,
            add_hl_de: 0x19 => "ADD HL, DE", 2,
            ld_a_de: 0x1A => "LD A, (DE)", 2,
            dec_de: 0x1B => "DEC DE", 2,
            inc_e: 0x1C => "INC E", 1,
            dec_e: 0x1D => "DEC E", 1,
            ld_e_n8: 0x1E, 0x12 => "LD E, 0x12", 2,
            rra: 0x1F => "RRA", 1,
            jr_nz_e8: 0x20, 0x7B => "JR NZ, 123", 3,
            ld_hl_n16: 0x21, 0x34, 0x12 => "LD HL, 0x1234", 3,
            ld_hl_increment_a: 0x22 => "LD (HL+), A", 2,
            inc16_hl: 0x23 => "INC HL", 2,
            inc_h: 0x24 => "INC H", 1,
            dec_h: 0x25 => "DEC H", 1,
            ld_h_n8: 0x26, 0x12 => "LD H, 0x12", 2,
            daa: 0x27 => "DAA", 1,
            jr_z_e8: 0x28, 0x7B => "JR Z, 123", 3,
            add_hl_hl: 0x29 => "ADD HL, HL", 2,
            ld_a_hl_increment: 0x2A => "LD A, (HL+)", 2,
            dec16_hl: 0x2B => "DEC HL", 2,
            inc_l: 0x2C => "INC L", 1,
            dec_l: 0x2D => "DEC L", 1,
            ld_l_n8: 0x2E, 0x12 => "LD L, 0x12", 2,
            cpl: 0x2F => "CPL", 1,
            jr_nc_e8: 0x30, 0x7B => "JR NC, 123", 3,
            ld_sp_n16: 0x31, 0x34, 0x12 => "LD SP, 0x1234", 3,
            ld_hl_decrement_a: 0x32 => "LD (HL-), A", 2,
            inc_sp: 0x33 => "INC SP", 2,
            inc_hl: 0x34 => "INC (HL)", 3,
            dec_hl: 0x35 => "DEC (HL)", 3,
            ld_hl_n8: 0x36, 0x12 => "LD (HL), 0x12", 3,
            scf: 0x37 => "SCF", 1,
            jr_c_e8: 0x38, 0x7B => "JR C, 123", 3,
            add_hl_sp: 0x39 => "ADD HL, SP", 2,
            ld_a_hl_decrement: 0x3A => "LD A, (HL-)", 2,
            dec_sp: 0x3B => "DEC SP", 2,
            inc_a: 0x3C => "INC A", 1,
            dec_a: 0x3D => "DEC A", 1,
            ld_a_n8: 0x3E, 0x12 => "LD A, 0x12", 2,
            ccf: 0x3F => "CCF", 1,
            ld_b_b: 0x40 => "LD B, B", 1,
            ld_b_c: 0x41 => "LD B, C", 1,
            ld_b_d: 0x42 => "LD B, D", 1,
            ld_b_e: 0x43 => "LD B, E", 1,
            ld_b_h: 0x44 => "LD B, H", 1,
            ld_b_l: 0x45 => "LD B, L", 1,
            ld_b_hl: 0x46 => "LD B, (HL)", 2,
            ld_b_a: 0x47 => "LD B, A", 1,
            ld_c_b: 0x48 => "LD C, B", 1,
            ld_c_c: 0x49 => "LD C, C", 1,
            ld_c_d: 0x4A => "LD C, D", 1,
            ld_c_e: 0x4B => "LD C, E", 1,
            ld_c_h: 0x4C => "LD C, H", 1,
            ld_c_l: 0x4D => "LD C, L", 1,
            ld_c_hl: 0x4E => "LD C, (HL)", 2,
            ld_c_a: 0x4F => "LD C, A", 1,
            ld_d_b: 0x50 => "LD D, B", 1,
            ld_d_c: 0x51 => "LD D, C", 1,
            ld_d_d: 0x52 => "LD D, D", 1,
            ld_d_e: 0x53 => "LD D, E", 1,
            ld_d_h: 0x54 => "LD D, H", 1,
            ld_d_l: 0x55 => "LD D, L", 1,
            ld_d_hl: 0x56 => "LD D, (HL)", 2,
            ld_d_a: 0x57 => "LD D, A", 1,
            ld_e_b: 0x58 => "LD E, B", 1,
            ld_e_c: 0x59 => "LD E, C", 1,
            ld_e_d: 0x5A => "LD E, D", 1,
            ld_e_e: 0x5B => "LD E, E", 1,
            ld_e_h: 0x5C => "LD E, H", 1,
            ld_e_l: 0x5D => "LD E, L", 1,
            ld_e_hl: 0x5E => "LD E, (HL)", 2,
            ld_e_a: 0x5F => "LD E, A", 1,
            ld_h_b: 0x60 => "LD H, B", 1,
            ld_h_c: 0x61 => "LD H, C", 1,
            ld_h_d: 0x62 => "LD H, D", 1,
            ld_h_e: 0x63 => "LD H, E", 1,
            ld_h_h: 0x64 => "LD H, H", 1,
            ld_h_l: 0x65 => "LD H, L", 1,
            ld_h_hl: 0x66 => "LD H, (HL)", 2,
            ld_h_a: 0x67 => "LD H, A", 1,
            ld_l_b: 0x68 => "LD L, B", 1,
            ld_l_c: 0x69 => "LD L, C", 1,
            ld_l_d: 0x6A => "LD L, D", 1,
            ld_l_e: 0x6B => "LD L, E", 1,
            ld_l_h: 0x6C => "LD L, H", 1,
            ld_l_l: 0x6D => "LD L, L", 1,
            ld_l_hl: 0x6E => "LD L, (HL)", 2,
            ld_l_a: 0x6F => "LD L, A", 1,
            ld_hl_b: 0x70 => "LD (HL), B", 2,
            ld_hl_c: 0x71 => "LD (HL), C", 2,
            ld_hl_d: 0x72 => "LD (HL), D", 2,
            ld_hl_e: 0x73 => "LD (HL), E", 2,
            ld_hl_h: 0x74 => "LD (HL), H", 2,
            ld_hl_l: 0x75 => "LD (HL), L", 2,
            halt: 0x76 => "HALT", 1,
            ld_hl_a: 0x77 => "LD (HL), A", 2,
            ld_a_b: 0x78 => "LD A, B", 1,
            ld_a_c: 0x79 => "LD A, C", 1,
            ld_a_d: 0x7A => "LD A, D", 1,
            ld_a_e: 0x7B => "LD A, E", 1,
            ld_a_h: 0x7C => "LD A, H", 1,
            ld_a_l: 0x7D => "LD A, L", 1,
            ld_a_hl: 0x7E => "LD A, (HL)", 2,
            ld_a_a: 0x7F => "LD A, A", 1,
            add_a_b: 0x80 => "ADD A, B", 1,
            add_a_c: 0x81 => "ADD A, C", 1,
            add_a_d: 0x82 => "ADD A, D", 1,
            add_a_e: 0x83 => "ADD A, E", 1,
            add_a_h: 0x84 => "ADD A, H", 1,
            add_a_l: 0x85 => "ADD A, L", 1,
            add_a_hl: 0x86 => "ADD A, (HL)", 2,
            add_a_a: 0x87 => "ADD A, A", 1,
            adc_a_b: 0x88 => "ADC A, B", 1,
            adc_a_c: 0x89 => "ADC A, C", 1,
            adc_a_d: 0x8A => "ADC A, D", 1,
            adc_a_e: 0x8B => "ADC A, E", 1,
            adc_a_h: 0x8C => "ADC A, H", 1,
            adc_a_l: 0x8D => "ADC A, L", 1,
            adc_a_hl: 0x8E => "ADC A, (HL)", 2,
            adc_a_a: 0x8F => "ADC A, A", 1,
            sub_a_b: 0x90 => "SUB A, B", 1,
            sub_a_c: 0x91 => "SUB A, C", 1,
            sub_a_d: 0x92 => "SUB A, D", 1,
            sub_a_e: 0x93 => "SUB A, E", 1,
            sub_a_h: 0x94 => "SUB A, H", 1,
            sub_a_l: 0x95 => "SUB A, L", 1,
            sub_a_hl: 0x96 => "SUB A, (HL)", 2,
            sub_a_a: 0x97 => "SUB A, A", 1,
            sbc_a_b: 0x98 => "SBC A, B", 1,
            sbc_a_c: 0x99 => "SBC A, C", 1,
            sbc_a_d: 0x9A => "SBC A, D", 1,
            sbc_a_e: 0x9B => "SBC A, E", 1,
            sbc_a_h: 0x9C => "SBC A, H", 1,
            sbc_a_l: 0x9D => "SBC A, L", 1,
            sbc_a_hl: 0x9E => "SBC A, (HL)", 2,
            sbc_a_a: 0x9F => "SBC A, A", 1,
            and_a_b: 0xA0 => "AND A, B", 1,
            and_a_c: 0xA1 => "AND A, C", 1,
            and_a_d: 0xA2 => "AND A, D", 1,
            and_a_e: 0xA3 => "AND A, E", 1,
            and_a_h: 0xA4 => "AND A, H", 1,
            and_a_l: 0xA5 => "AND A, L", 1,
            and_a_hl: 0xA6 => "AND A, (HL)", 2,
            and_a_a: 0xA7 => "AND A, A", 1,
            xor_a_b: 0xA8 => "XOR A, B", 1,
            xor_a_c: 0xA9 => "XOR A, C", 1,
            xor_a_d: 0xAA => "XOR A, D", 1,
            xor_a_e: 0xAB => "XOR A, E", 1,
            xor_a_h: 0xAC => "XOR A, H", 1,
            xor_a_l: 0xAD => "XOR A, L", 1,
            xor_a_hl: 0xAE => "XOR A, (HL)", 2,
            xor_a_a: 0xAF => "XOR A, A", 1,
            or_a_b: 0xB0 => "OR A, B", 1,
            or_a_c: 0xB1 => "OR A, C", 1,
            or_a_d: 0xB2 => "OR A, D", 1,
            or_a_e: 0xB3 => "OR A, E", 1,
            or_a_h: 0xB4 => "OR A, H", 1,
            or_a_l: 0xB5 => "OR A, L", 1,
            or_a_hl: 0xB6 => "OR A, (HL)", 2,
            or_a_a: 0xB7 => "OR A, A", 1,
            cp_a_b: 0xB8 => "CP A, B", 1,
            cp_a_c: 0xB9 => "CP A, C", 1,
            cp_a_d: 0xBA => "CP A, D", 1,
            cp_a_e: 0xBB => "CP A, E", 1,
            cp_a_h: 0xBC => "CP A, H", 1,
            cp_a_l: 0xBD => "CP A, L", 1,
            cp_a_hl: 0xBE => "CP A, (HL)", 2,
            cp_a_a: 0xBF => "CP A, A", 1,
            ret_nz: 0xC0 => "RET NZ", 5,
            pop_bc: 0xC1 => "POP BC", 3,
            jp_nz_a16: 0xC2, 0x34, 0x12 => "JP NZ, 0x1234", 4,
            jp_a16: 0xC3, 0x34, 0x12 => "JP 0x1234", 4,
            call_nz_a16: 0xC4, 0x34, 0x12 => "CALL NZ, 0x1234", 6,
            push_bc: 0xC5 => "PUSH BC", 4,
            add_a_n8: 0xC6, 0x12 => "ADD A, 0x12", 2,
            rst_00: 0xC7 => "RST $00", 4,
            ret_z: 0xC8 => "RET Z", 5,
            ret: 0xC9 => "RET", 4,
            jp_z_a16: 0xCA, 0x34, 0x12 => "JP Z, 0x1234", 4,
            call_z_a16: 0xCC, 0x34, 0x12 => "CALL Z, 0x1234", 6,
            call_a16: 0xCD, 0x34, 0x12 => "CALL 0x1234", 6,
            adc_a_n8: 0xCE, 0x12 => "ADC A, 0x12", 2,
            rst_08: 0xCF => "RST $08", 4,
            ret_nc: 0xD0 => "RET NC", 5,
            pop_de: 0xD1 => "POP DE", 3,
            jp_nc_a16: 0xD2, 0x34, 0x12 => "JP NC, 0x1234", 4,
            illegal_d3: 0xD3 => "ILLEGAL_D3", 1,
            call_nc_a16: 0xD4, 0x34, 0x12 => "CALL NC, 0x1234", 6,
            push_de: 0xD5 => "PUSH DE", 4,
            sub_a_n8: 0xD6, 0x12 => "SUB A, 0x12", 2,
            rst_10: 0xD7 => "RST $10", 4,
            ret_c: 0xD8 => "RET C", 5,
            reti: 0xD9 => "RETI", 4,
            jp_c_a16: 0xDA, 0x34, 0x12 => "JP C, 0x1234", 4,
            illegal_db: 0xDB => "ILLEGAL_DB", 1,
            call_c_a16: 0xDC, 0x34, 0x12 => "CALL C, 0x1234", 6,
            illegal_dd: 0xDD => "ILLEGAL_DD", 1,
            sbc_a_n8: 0xDE, 0x12 => "SBC A, 0x12", 2,
            rst_18: 0xDF => "RST $18", 4,
            ldh_a8_a: 0xE0, 0x12 => "LDH (0x12), A", 3,
            pop_hl: 0xE1 => "POP HL", 3,
            ldh_c_a: 0xE2 => "LDH (C), A", 2,
            illegal_e3: 0xE3 => "ILLEGAL_E3", 1,
            illegal_e4: 0xE4 => "ILLEGAL_E4", 1,
            push_hl: 0xE5 => "PUSH HL", 4,
            and_a_n8: 0xE6, 0x12 => "AND A, 0x12", 2,
            rst_20: 0xE7 => "RST $20", 4,
            add_sp_e8: 0xE8, 0x7B => "ADD SP, 123", 4,
            jp_hl: 0xE9 => "JP HL", 1,
            ld_a16_a: 0xEA, 0x34, 0x12 => "LD (0x1234), A", 4,
            illegal_eb: 0xEB => "ILLEGAL_EB", 1,
            illegal_ec: 0xEC => "ILLEGAL_EC", 1,
            illegal_ed: 0xED => "ILLEGAL_ED", 1,
            xor_a_n8: 0xEE, 0x12 => "XOR A, 0x12", 2,
            rst_28: 0xEF => "RST $28", 4,
            ldh_a_a8: 0xF0, 0x12 => "LDH A, (0x12)", 3,
            pop_af: 0xF1 => "POP AF", 3,
            ldh_a_c: 0xF2 => "LDH A, (C)", 2,
            di: 0xF3 => "DI", 1,
            illegal_f4: 0xF4 => "ILLEGAL_F4", 1,
            push_af: 0xF5 => "PUSH AF", 4,
            or_a_n8: 0xF6, 0x12 => "OR A, 0x12", 2,
            rst_30: 0xF7 => "RST $30", 4,
            ld_hl_sp_increment_e8: 0xF8, 0x7B => "LD HL, SP+123", 3,
            ld_sp_hl: 0xF9 => "LD SP, HL", 2,
            ld_a_a16: 0xFA, 0x34, 0x12 => "LD A, (0x1234)", 4,
            ei: 0xFB => "EI", 1,
            illegal_fc: 0xFC => "ILLEGAL_FC", 1,
            illegal_fd: 0xFD => "ILLEGAL_FD", 1,
            cp_a_n8: 0xFE, 0x12 => "CP A, 0x12", 2,
            rst_38: 0xFF => "RST $38", 4,
        }
    }

    mod cb_prefixed {
        use super::*;
        opcode_tests! {
            rlc_b: 0xCB, 0x00 => "RLC B", 2,
            rlc_c: 0xCB, 0x01 => "RLC C", 2,
            rlc_d: 0xCB, 0x02 => "RLC D", 2,
            rlc_e: 0xCB, 0x03 => "RLC E", 2,
            rlc_h: 0xCB, 0x04 => "RLC H", 2,
            rlc_l: 0xCB, 0x05 => "RLC L", 2,
            rlc_hl: 0xCB, 0x06 => "RLC (HL)", 4,
            rlc_a: 0xCB, 0x07 => "RLC A", 2,
            rrc_b: 0xCB, 0x08 => "RRC B", 2,
            rrc_c: 0xCB, 0x09 => "RRC C", 2,
            rrc_d: 0xCB, 0x0A => "RRC D", 2,
            rrc_e: 0xCB, 0x0B => "RRC E", 2,
            rrc_h: 0xCB, 0x0C => "RRC H", 2,
            rrc_l: 0xCB, 0x0D => "RRC L", 2,
            rrc_hl: 0xCB, 0x0E => "RRC (HL)", 4,
            rrc_a: 0xCB, 0x0F => "RRC A", 2,
            rl_b: 0xCB, 0x10 => "RL B", 2,
            rl_c: 0xCB, 0x11 => "RL C", 2,
            rl_d: 0xCB, 0x12 => "RL D", 2,
            rl_e: 0xCB, 0x13 => "RL E", 2,
            rl_h: 0xCB, 0x14 => "RL H", 2,
            rl_l: 0xCB, 0x15 => "RL L", 2,
            rl_hl: 0xCB, 0x16 => "RL (HL)", 4,
            rl_a: 0xCB, 0x17 => "RL A", 2,
            rr_b: 0xCB, 0x18 => "RR B", 2,
            rr_c: 0xCB, 0x19 => "RR C", 2,
            rr_d: 0xCB, 0x1A => "RR D", 2,
            rr_e: 0xCB, 0x1B => "RR E", 2,
            rr_h: 0xCB, 0x1C => "RR H", 2,
            rr_l: 0xCB, 0x1D => "RR L", 2,
            rr_hl: 0xCB, 0x1E => "RR (HL)", 4,
            rr_a: 0xCB, 0x1F => "RR A", 2,
            sla_b: 0xCB, 0x20 => "SLA B", 2,
            sla_c: 0xCB, 0x21 => "SLA C", 2,
            sla_d: 0xCB, 0x22 => "SLA D", 2,
            sla_e: 0xCB, 0x23 => "SLA E", 2,
            sla_h: 0xCB, 0x24 => "SLA H", 2,
            sla_l: 0xCB, 0x25 => "SLA L", 2,
            sla_hl: 0xCB, 0x26 => "SLA (HL)", 4,
            sla_a: 0xCB, 0x27 => "SLA A", 2,
            sra_b: 0xCB, 0x28 => "SRA B", 2,
            sra_c: 0xCB, 0x29 => "SRA C", 2,
            sra_d: 0xCB, 0x2A => "SRA D", 2,
            sra_e: 0xCB, 0x2B => "SRA E", 2,
            sra_h: 0xCB, 0x2C => "SRA H", 2,
            sra_l: 0xCB, 0x2D => "SRA L", 2,
            sra_hl: 0xCB, 0x2E => "SRA (HL)", 4,
            sra_a: 0xCB, 0x2F => "SRA A", 2,
            swap_b: 0xCB, 0x30 => "SWAP B", 2,
            swap_c: 0xCB, 0x31 => "SWAP C", 2,
            swap_d: 0xCB, 0x32 => "SWAP D", 2,
            swap_e: 0xCB, 0x33 => "SWAP E", 2,
            swap_h: 0xCB, 0x34 => "SWAP H", 2,
            swap_l: 0xCB, 0x35 => "SWAP L", 2,
            swap_hl: 0xCB, 0x36 => "SWAP (HL)", 4,
            swap_a: 0xCB, 0x37 => "SWAP A", 2,
            srl_b: 0xCB, 0x38 => "SRL B", 2,
            srl_c: 0xCB, 0x39 => "SRL C", 2,
            srl_d: 0xCB, 0x3A => "SRL D", 2,
            srl_e: 0xCB, 0x3B => "SRL E", 2,
            srl_h: 0xCB, 0x3C => "SRL H", 2,
            srl_l: 0xCB, 0x3D => "SRL L", 2,
            srl_hl: 0xCB, 0x3E => "SRL (HL)", 4,
            srl_a: 0xCB, 0x3F => "SRL A", 2,
            bit_0_b: 0xCB, 0x40 => "BIT 0, B", 2,
            bit_0_c: 0xCB, 0x41 => "BIT 0, C", 2,
            bit_0_d: 0xCB, 0x42 => "BIT 0, D", 2,
            bit_0_e: 0xCB, 0x43 => "BIT 0, E", 2,
            bit_0_h: 0xCB, 0x44 => "BIT 0, H", 2,
            bit_0_l: 0xCB, 0x45 => "BIT 0, L", 2,
            bit_0_hl: 0xCB, 0x46 => "BIT 0, (HL)", 3,
            bit_0_a: 0xCB, 0x47 => "BIT 0, A", 2,
            bit_1_b: 0xCB, 0x48 => "BIT 1, B", 2,
            bit_1_c: 0xCB, 0x49 => "BIT 1, C", 2,
            bit_1_d: 0xCB, 0x4A => "BIT 1, D", 2,
            bit_1_e: 0xCB, 0x4B => "BIT 1, E", 2,
            bit_1_h: 0xCB, 0x4C => "BIT 1, H", 2,
            bit_1_l: 0xCB, 0x4D => "BIT 1, L", 2,
            bit_1_hl: 0xCB, 0x4E => "BIT 1, (HL)", 3,
            bit_1_a: 0xCB, 0x4F => "BIT 1, A", 2,
            bit_2_b: 0xCB, 0x50 => "BIT 2, B", 2,
            bit_2_c: 0xCB, 0x51 => "BIT 2, C", 2,
            bit_2_d: 0xCB, 0x52 => "BIT 2, D", 2,
            bit_2_e: 0xCB, 0x53 => "BIT 2, E", 2,
            bit_2_h: 0xCB, 0x54 => "BIT 2, H", 2,
            bit_2_l: 0xCB, 0x55 => "BIT 2, L", 2,
            bit_2_hl: 0xCB, 0x56 => "BIT 2, (HL)", 3,
            bit_2_a: 0xCB, 0x57 => "BIT 2, A", 2,
            bit_3_b: 0xCB, 0x58 => "BIT 3, B", 2,
            bit_3_c: 0xCB, 0x59 => "BIT 3, C", 2,
            bit_3_d: 0xCB, 0x5A => "BIT 3, D", 2,
            bit_3_e: 0xCB, 0x5B => "BIT 3, E", 2,
            bit_3_h: 0xCB, 0x5C => "BIT 3, H", 2,
            bit_3_l: 0xCB, 0x5D => "BIT 3, L", 2,
            bit_3_hl: 0xCB, 0x5E => "BIT 3, (HL)", 3,
            bit_3_a: 0xCB, 0x5F => "BIT 3, A", 2,
            bit_4_b: 0xCB, 0x60 => "BIT 4, B", 2,
            bit_4_c: 0xCB, 0x61 => "BIT 4, C", 2,
            bit_4_d: 0xCB, 0x62 => "BIT 4, D", 2,
            bit_4_e: 0xCB, 0x63 => "BIT 4, E", 2,
            bit_4_h: 0xCB, 0x64 => "BIT 4, H", 2,
            bit_4_l: 0xCB, 0x65 => "BIT 4, L", 2,
            bit_4_hl: 0xCB, 0x66 => "BIT 4, (HL)", 3,
            bit_4_a: 0xCB, 0x67 => "BIT 4, A", 2,
            bit_5_b: 0xCB, 0x68 => "BIT 5, B", 2,
            bit_5_c: 0xCB, 0x69 => "BIT 5, C", 2,
            bit_5_d: 0xCB, 0x6A => "BIT 5, D", 2,
            bit_5_e: 0xCB, 0x6B => "BIT 5, E", 2,
            bit_5_h: 0xCB, 0x6C => "BIT 5, H", 2,
            bit_5_l: 0xCB, 0x6D => "BIT 5, L", 2,
            bit_5_hl: 0xCB, 0x6E => "BIT 5, (HL)", 3,
            bit_5_a: 0xCB, 0x6F => "BIT 5, A", 2,
            bit_6_b: 0xCB, 0x70 => "BIT 6, B", 2,
            bit_6_c: 0xCB, 0x71 => "BIT 6, C", 2,
            bit_6_d: 0xCB, 0x72 => "BIT 6, D", 2,
            bit_6_e: 0xCB, 0x73 => "BIT 6, E", 2,
            bit_6_h: 0xCB, 0x74 => "BIT 6, H", 2,
            bit_6_l: 0xCB, 0x75 => "BIT 6, L", 2,
            bit_6_hl: 0xCB, 0x76 => "BIT 6, (HL)", 3,
            bit_6_a: 0xCB, 0x77 => "BIT 6, A", 2,
            bit_7_b: 0xCB, 0x78 => "BIT 7, B", 2,
            bit_7_c: 0xCB, 0x79 => "BIT 7, C", 2,
            bit_7_d: 0xCB, 0x7A => "BIT 7, D", 2,
            bit_7_e: 0xCB, 0x7B => "BIT 7, E", 2,
            bit_7_h: 0xCB, 0x7C => "BIT 7, H", 2,
            bit_7_l: 0xCB, 0x7D => "BIT 7, L", 2,
            bit_7_hl: 0xCB, 0x7E => "BIT 7, (HL)", 3,
            bit_7_a: 0xCB, 0x7F => "BIT 7, A", 2,
            res_0_b: 0xCB, 0x80 => "RES 0, B", 2,
            res_0_c: 0xCB, 0x81 => "RES 0, C", 2,
            res_0_d: 0xCB, 0x82 => "RES 0, D", 2,
            res_0_e: 0xCB, 0x83 => "RES 0, E", 2,
            res_0_h: 0xCB, 0x84 => "RES 0, H", 2,
            res_0_l: 0xCB, 0x85 => "RES 0, L", 2,
            res_0_hl: 0xCB, 0x86 => "RES 0, (HL)", 4,
            res_0_a: 0xCB, 0x87 => "RES 0, A", 2,
            res_1_b: 0xCB, 0x88 => "RES 1, B", 2,
            res_1_c: 0xCB, 0x89 => "RES 1, C", 2,
            res_1_d: 0xCB, 0x8A => "RES 1, D", 2,
            res_1_e: 0xCB, 0x8B => "RES 1, E", 2,
            res_1_h: 0xCB, 0x8C => "RES 1, H", 2,
            res_1_l: 0xCB, 0x8D => "RES 1, L", 2,
            res_1_hl: 0xCB, 0x8E => "RES 1, (HL)", 4,
            res_1_a: 0xCB, 0x8F => "RES 1, A", 2,
            res_2_b: 0xCB, 0x90 => "RES 2, B", 2,
            res_2_c: 0xCB, 0x91 => "RES 2, C", 2,
            res_2_d: 0xCB, 0x92 => "RES 2, D", 2,
            res_2_e: 0xCB, 0x93 => "RES 2, E", 2,
            res_2_h: 0xCB, 0x94 => "RES 2, H", 2,
            res_2_l: 0xCB, 0x95 => "RES 2, L", 2,
            res_2_hl: 0xCB, 0x96 => "RES 2, (HL)", 4,
            res_2_a: 0xCB, 0x97 => "RES 2, A", 2,
            res_3_b: 0xCB, 0x98 => "RES 3, B", 2,
            res_3_c: 0xCB, 0x99 => "RES 3, C", 2,
            res_3_d: 0xCB, 0x9A => "RES 3, D", 2,
            res_3_e: 0xCB, 0x9B => "RES 3, E", 2,
            res_3_h: 0xCB, 0x9C => "RES 3, H", 2,
            res_3_l: 0xCB, 0x9D => "RES 3, L", 2,
            res_3_hl: 0xCB, 0x9E => "RES 3, (HL)", 4,
            res_3_a: 0xCB, 0x9F => "RES 3, A", 2,
            res_4_b: 0xCB, 0xA0 => "RES 4, B", 2,
            res_4_c: 0xCB, 0xA1 => "RES 4, C", 2,
            res_4_d: 0xCB, 0xA2 => "RES 4, D", 2,
            res_4_e: 0xCB, 0xA3 => "RES 4, E", 2,
            res_4_h: 0xCB, 0xA4 => "RES 4, H", 2,
            res_4_l: 0xCB, 0xA5 => "RES 4, L", 2,
            res_4_hl: 0xCB, 0xA6 => "RES 4, (HL)", 4,
            res_4_a: 0xCB, 0xA7 => "RES 4, A", 2,
            res_5_b: 0xCB, 0xA8 => "RES 5, B", 2,
            res_5_c: 0xCB, 0xA9 => "RES 5, C", 2,
            res_5_d: 0xCB, 0xAA => "RES 5, D", 2,
            res_5_e: 0xCB, 0xAB => "RES 5, E", 2,
            res_5_h: 0xCB, 0xAC => "RES 5, H", 2,
            res_5_l: 0xCB, 0xAD => "RES 5, L", 2,
            res_5_hl: 0xCB, 0xAE => "RES 5, (HL)", 4,
            res_5_a: 0xCB, 0xAF => "RES 5, A", 2,
            res_6_b: 0xCB, 0xB0 => "RES 6, B", 2,
            res_6_c: 0xCB, 0xB1 => "RES 6, C", 2,
            res_6_d: 0xCB, 0xB2 => "RES 6, D", 2,
            res_6_e: 0xCB, 0xB3 => "RES 6, E", 2,
            res_6_h: 0xCB, 0xB4 => "RES 6, H", 2,
            res_6_l: 0xCB, 0xB5 => "RES 6, L", 2,
            res_6_hl: 0xCB, 0xB6 => "RES 6, (HL)", 4,
            res_6_a: 0xCB, 0xB7 => "RES 6, A", 2,
            res_7_b: 0xCB, 0xB8 => "RES 7, B", 2,
            res_7_c: 0xCB, 0xB9 => "RES 7, C", 2,
            res_7_d: 0xCB, 0xBA => "RES 7, D", 2,
            res_7_e: 0xCB, 0xBB => "RES 7, E", 2,
            res_7_h: 0xCB, 0xBC => "RES 7, H", 2,
            res_7_l: 0xCB, 0xBD => "RES 7, L", 2,
            res_7_hl: 0xCB, 0xBE => "RES 7, (HL)", 4,
            res_7_a: 0xCB, 0xBF => "RES 7, A", 2,
            set_0_b: 0xCB, 0xC0 => "SET 0, B", 2,
            set_0_c: 0xCB, 0xC1 => "SET 0, C", 2,
            set_0_d: 0xCB, 0xC2 => "SET 0, D", 2,
            set_0_e: 0xCB, 0xC3 => "SET 0, E", 2,
            set_0_h: 0xCB, 0xC4 => "SET 0, H", 2,
            set_0_l: 0xCB, 0xC5 => "SET 0, L", 2,
            set_0_hl: 0xCB, 0xC6 => "SET 0, (HL)", 4,
            set_0_a: 0xCB, 0xC7 => "SET 0, A", 2,
            set_1_b: 0xCB, 0xC8 => "SET 1, B", 2,
            set_1_c: 0xCB, 0xC9 => "SET 1, C", 2,
            set_1_d: 0xCB, 0xCA => "SET 1, D", 2,
            set_1_e: 0xCB, 0xCB => "SET 1, E", 2,
            set_1_h: 0xCB, 0xCC => "SET 1, H", 2,
            set_1_l: 0xCB, 0xCD => "SET 1, L", 2,
            set_1_hl: 0xCB, 0xCE => "SET 1, (HL)", 4,
            set_1_a: 0xCB, 0xCF => "SET 1, A", 2,
            set_2_b: 0xCB, 0xD0 => "SET 2, B", 2,
            set_2_c: 0xCB, 0xD1 => "SET 2, C", 2,
            set_2_d: 0xCB, 0xD2 => "SET 2, D", 2,
            set_2_e: 0xCB, 0xD3 => "SET 2, E", 2,
            set_2_h: 0xCB, 0xD4 => "SET 2, H", 2,
            set_2_l: 0xCB, 0xD5 => "SET 2, L", 2,
            set_2_hl: 0xCB, 0xD6 => "SET 2, (HL)", 4,
            set_2_a: 0xCB, 0xD7 => "SET 2, A", 2,
            set_3_b: 0xCB, 0xD8 => "SET 3, B", 2,
            set_3_c: 0xCB, 0xD9 => "SET 3, C", 2,
            set_3_d: 0xCB, 0xDA => "SET 3, D", 2,
            set_3_e: 0xCB, 0xDB => "SET 3, E", 2,
            set_3_h: 0xCB, 0xDC => "SET 3, H", 2,
            set_3_l: 0xCB, 0xDD => "SET 3, L", 2,
            set_3_hl: 0xCB, 0xDE => "SET 3, (HL)", 4,
            set_3_a: 0xCB, 0xDF => "SET 3, A", 2,
            set_4_b: 0xCB, 0xE0 => "SET 4, B", 2,
            set_4_c: 0xCB, 0xE1 => "SET 4, C", 2,
            set_4_d: 0xCB, 0xE2 => "SET 4, D", 2,
            set_4_e: 0xCB, 0xE3 => "SET 4, E", 2,
            set_4_h: 0xCB, 0xE4 => "SET 4, H", 2,
            set_4_l: 0xCB, 0xE5 => "SET 4, L", 2,
            set_4_hl: 0xCB, 0xE6 => "SET 4, (HL)", 4,
            set_4_a: 0xCB, 0xE7 => "SET 4, A", 2,
            set_5_b: 0xCB, 0xE8 => "SET 5, B", 2,
            set_5_c: 0xCB, 0xE9 => "SET 5, C", 2,
            set_5_d: 0xCB, 0xEA => "SET 5, D", 2,
            set_5_e: 0xCB, 0xEB => "SET 5, E", 2,
            set_5_h: 0xCB, 0xEC => "SET 5, H", 2,
            set_5_l: 0xCB, 0xED => "SET 5, L", 2,
            set_5_hl: 0xCB, 0xEE => "SET 5, (HL)", 4,
            set_5_a: 0xCB, 0xEF => "SET 5, A", 2,
            set_6_b: 0xCB, 0xF0 => "SET 6, B", 2,
            set_6_c: 0xCB, 0xF1 => "SET 6, C", 2,
            set_6_d: 0xCB, 0xF2 => "SET 6, D", 2,
            set_6_e: 0xCB, 0xF3 => "SET 6, E", 2,
            set_6_h: 0xCB, 0xF4 => "SET 6, H", 2,
            set_6_l: 0xCB, 0xF5 => "SET 6, L", 2,
            set_6_hl: 0xCB, 0xF6 => "SET 6, (HL)", 4,
            set_6_a: 0xCB, 0xF7 => "SET 6, A", 2,
            set_7_b: 0xCB, 0xF8 => "SET 7, B", 2,
            set_7_c: 0xCB, 0xF9 => "SET 7, C", 2,
            set_7_d: 0xCB, 0xFA => "SET 7, D", 2,
            set_7_e: 0xCB, 0xFB => "SET 7, E", 2,
            set_7_h: 0xCB, 0xFC => "SET 7, H", 2,
            set_7_l: 0xCB, 0xFD => "SET 7, L", 2,
            set_7_hl: 0xCB, 0xFE => "SET 7, (HL)", 4,
            set_7_a: 0xCB, 0xFF => "SET 7, A", 2,
        }
    }

    #[test]
    fn parses_every_unprefixed_opcode() {
        for byte in 0x00u8..=0xff {
            if byte == 0xCB {
                continue; // Skip CB prefix for now, as it has its own parsing logic
            }
            let mut fetch = StubFetch::from_u8_imm16(byte, 0x0000);
            OpCode::parse(&mut fetch);
        }
    }

    #[test]
    fn parses_every_cb_prefixed_opcode() {
        for byte in 0x00u8..=0xff {
            let mut fetch = StubFetch::from_u8_imm8(0xCB, byte);
            OpCode::parse(&mut fetch);
        }
    }
}
