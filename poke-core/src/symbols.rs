pub use crate::pointer::{DmgBank, DmgPointer};

include!(concat!(env!("OUT_DIR"), "/pokered_symbols.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sym_file() {
        // Test ROM entry
        let calc_checksum = pokered_symbols::CalcCheckSum;
        assert_eq!(calc_checksum.bank, DmgBank::ROM { bank: 0x1c });
        assert_eq!(calc_checksum.address, 0x7856);

        // Test WRAM entry
        let enemy_mon = pokered_symbols::wEnemyMonUnmodifiedSpecial;
        assert_eq!(enemy_mon.bank, DmgBank::WRAM);
        assert_eq!(enemy_mon.address, 0xcd2c);

        // Test SRAM entry
        let cur_box = pokered_symbols::sCurBoxData;
        assert_eq!(cur_box.bank, DmgBank::SRAM { bank: 0x01 });
        assert_eq!(cur_box.address, 0xb0c0);

        // Test VRAM entry
        let tileset = pokered_symbols::vTileset;
        assert_eq!(tileset.bank, DmgBank::VRAM);
        assert_eq!(tileset.address, 0x9000);

        // Test HRAM entry
        let slide_amount = pokered_symbols::hSlideAmount;
        assert_eq!(slide_amount.bank, DmgBank::HRAM);
        assert_eq!(slide_amount.address, 0xff8b);

        // Test constants
        assert_eq!(pokered_symbols::ROUTE6GATE_GUARD, 0x01);
        assert_eq!(pokered_symbols::ROUTE6_COOLTRAINER_M1, 0x01);
        assert_eq!(pokered_symbols::ROUTE7GATE_GUARD, 0x01);
        assert_eq!(pokered_symbols::PEWTERPOKECENTER_GENTLEMAN, 0x02);
        assert_eq!(pokered_symbols::FUCHSIAPOKECENTER_ROCKER, 0x02);
        assert_eq!(pokered_symbols::GAMECORNERPRIZEROOM_GAMBLER, 0x02);
    }
}
