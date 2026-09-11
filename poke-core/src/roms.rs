pub const POKERED: &[u8] = include_bytes!("../../vendor/pokered/pokered.gbc");

/// The size of one switchable ROM bank, and of bank 0.
pub const ROM_BANK_SIZE: usize = 0x4000;
