//! `poke_core::rom_gfx`'s decoders, and reading the cartridge itself.

pub use poke_core::rom_gfx::*;
use super::roms::{POKERED, ROM_BANK_SIZE};
use super::symbols::{DmgBank, DmgPointer};

/// A ROM pointer as a slice running to the end of its bank. Bank 0 is a raw file offset and every
/// other bank a `0x4000` window; read ROM through here rather than redoing that arithmetic.
pub fn rom_slice(pointer: DmgPointer) -> &'static [u8] {
    let DmgBank::ROM { bank } = pointer.bank else {
        panic!("{pointer} is not a ROM pointer");
    };
    let bank = bank as usize;
    let window = if bank == 0 { 0 } else { ROM_BANK_SIZE };
    &POKERED[bank * ROM_BANK_SIZE + (pointer.address as usize - window)..(bank + 1) * ROM_BANK_SIZE]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bank 0 is not windowed and every other bank is.
    #[test]
    fn bank_zero_is_not_windowed_and_every_other_bank_is() {
        let bank_0 = DmgPointer { bank: DmgBank::ROM { bank: 0 }, address: 0x0100 };
        assert_eq!(rom_slice(bank_0)[..16], POKERED[0x0100..0x0110]);

        let bank_1 = DmgPointer { bank: DmgBank::ROM { bank: 1 }, address: 0x4100 };
        assert_eq!(rom_slice(bank_1)[..16], POKERED[0x4100..0x4110]);

        let bank_9 = DmgPointer { bank: DmgBank::ROM { bank: 9 }, address: 0x4000 };
        assert_eq!(rom_slice(bank_9)[..16], POKERED[9 * ROM_BANK_SIZE..9 * ROM_BANK_SIZE + 16]);
    }
}
