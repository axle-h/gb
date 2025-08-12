

pub mod blarg {
    pub const CPU_INSTRUCTIONS: &[u8] = include_bytes!("cpu_instrs.gb");
}

pub mod test {
    pub const DMG_ACID: &[u8] = include_bytes!("dmg-acid2.gb");
}

pub mod commercial {
    pub const TETRIS: &[u8] = include_bytes!("tetris.gb");
}
