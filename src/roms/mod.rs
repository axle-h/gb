

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
    pub const EXPECTED_WAVE_READ_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
    pub const WAVE_TRIGGER_WHILE_ON: &[u8] = include_bytes!("dmg_sound/10-wave trigger while on.gb");
    pub const EXPECTED_WAVE_TRIGGER_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
    pub const REGISTERS_AFTER_POWER: &[u8] = include_bytes!("dmg_sound/11-regs after power.gb");
    pub const EXPECTED_REGISTERS_AFTER_POWER: &[u8] = include_bytes!("dmg_sound/11-regs after power.png");
    pub const WAVE_WRITE_WHILE_ON: &[u8] = include_bytes!("dmg_sound/12-wave write while on.gb");
    pub const EXPECTED_WAVE_WRITE_WHILE_ON: &[u8] = EXPECTED_REGISTERS; // TODO
}

pub mod acid {
    pub const ROM: &[u8] = include_bytes!("dmg-acid2/dmg-acid2.gb");
    pub const EXPECTED_DMG: &[u8] = include_bytes!("dmg-acid2/reference-dmg.png");
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

pub mod commercial {
    pub const TETRIS: &[u8] = include_bytes!("tetris.gb");
    pub const ALLEYWAY: &[u8] = include_bytes!("alleyway.gb");
    pub const POKEMON_RED: &[u8] = include_bytes!("pokemon-red.gb");
    pub const TARZAN: &[u8] = include_bytes!("tarzan.gb");
    pub const CHESSMASTER: &[u8] = include_bytes!("chessmaster.gb");
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