

pub mod blarg {
    pub const CPU_INSTRUCTIONS: &[u8] = include_bytes!("cpu_instrs/cpu_instrs.gb");
    pub const CPU_INSTRUCTIONS_01: &[u8] = include_bytes!("cpu_instrs/01-special.gb");
    pub const CPU_INSTRUCTIONS_02: &[u8] = include_bytes!("cpu_instrs/02-interrupts.gb");
    pub const CPU_INSTRUCTIONS_03: &[u8] = include_bytes!("cpu_instrs/03-op sp,hl.gb");
    pub const CPU_INSTRUCTIONS_04: &[u8] = include_bytes!("cpu_instrs/04-op r,imm.gb");
    pub const CPU_INSTRUCTIONS_05: &[u8] = include_bytes!("cpu_instrs/05-op rp.gb");
    pub const CPU_INSTRUCTIONS_06: &[u8] = include_bytes!("cpu_instrs/06-ld r,r.gb");
    pub const CPU_INSTRUCTIONS_07: &[u8] = include_bytes!("cpu_instrs/07-jr,jp,call,ret,rst.gb");
    pub const CPU_INSTRUCTIONS_08: &[u8] = include_bytes!("cpu_instrs/08-misc instrs.gb");
    pub const CPU_INSTRUCTIONS_09: &[u8] = include_bytes!("cpu_instrs/09-op r,r.gb");
    pub const CPU_INSTRUCTIONS_10: &[u8] = include_bytes!("cpu_instrs/10-bit ops.gb");
    pub const CPU_INSTRUCTIONS_11: &[u8] = include_bytes!("cpu_instrs/11-op a,(hl).gb");

    pub const INSTRUCTION_TIMING: &[u8] = include_bytes!("instr_timing.gb");
}

pub mod test {
    pub const DMG_ACID: &[u8] = include_bytes!("dmg-acid2.gb");
}

pub mod commercial {
    pub const TETRIS: &[u8] = include_bytes!("tetris.gb");
}
