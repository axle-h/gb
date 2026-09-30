use gb::mmu::MMU;
use crate::pokemon::map::Map;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};
pub use poke_core::map_header::*;

pub trait MapHeaderReader {
    fn read_map_header(&self, map: Map) -> Result<MapHeader, String>;
}

impl MapHeaderReader for MMU {
    fn read_map_header(&self, map: Map) -> Result<MapHeader, String> {
        MapHeader::read(map)
    }
}

/// Where the cartridge keeps `map`'s own header, from its `MapHeaderPointers` and `MapHeaderBanks`;
/// `None` for a map that borrows another's.
pub fn rom_header_pointer(map: Map) -> Option<DmgPointer> {
    map.has_header().then(|| {
        let address = rom_slice(pokered_symbols::MapHeaderPointers + map as u16 * 2);
        let bank = rom_slice(pokered_symbols::MapHeaderBanks + map as u16)[0];
        DmgPointer { bank: DmgBank::ROM { bank }, address: u16::from_le_bytes([address[0], address[1]]) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_s_header_is_where_its_label_is() {
        assert_eq!(rom_header_pointer(Map::OaksLab), Some(pokered_symbols::OaksLab_h));
        assert_eq!(rom_header_pointer(Map::UndergroundPathRoute7Copy), Some(pokered_symbols::UndergroundPathRoute7Copy_h));
        assert_eq!(rom_header_pointer(Map::UnusedMap0B), None, "borrows Saffron City's");
    }
}
