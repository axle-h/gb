use gb::mmu::MMU;
use crate::pokemon::map::Map;
pub use poke_core::map_header::*;

pub trait MapHeaderReader {
    fn read_map_header(&self, map: Map) -> Result<MapHeader, String>;
}

impl MapHeaderReader for MMU {
    fn read_map_header(&self, map: Map) -> Result<MapHeader, String> {
        MapHeader::read(map)
    }
}

#[cfg(test)]
mod tests {
    use crate::pokemon::symbols::pokered_symbols;
    use super::*;

    #[test]
    fn test_read_oaks_lab() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        let oaks_lab = mmu.read_map_header(Map::OaksLab).unwrap();
        assert_eq!(oaks_lab.tileset, TileSetId::Dojo);
        assert_eq!(oaks_lab.height, 6);
        assert_eq!(oaks_lab.width, 5);
        assert_eq!(oaks_lab.blocks_address, pokered_symbols::OaksLab_Blocks.address);
        assert_eq!(oaks_lab.text_address, pokered_symbols::OaksLab_TextPointers.address);
        assert_eq!(oaks_lab.script_address, pokered_symbols::OaksLab_Script.address);
        assert_eq!(oaks_lab.objects_address, pokered_symbols::OaksLab_Object.address);
        assert_eq!(oaks_lab.connections().len(), 0);
    }

    #[test]
    fn test_read_pallet_town() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        let pallet_town = mmu.read_map_header(Map::PalletTown).unwrap();
        assert_eq!(pallet_town.tileset, TileSetId::Overworld);
        assert_eq!(pallet_town.height, 9);
        assert_eq!(pallet_town.width, 10);
        assert_eq!(pallet_town.blocks_address, pokered_symbols::PalletTown_Blocks.address);
        assert_eq!(pallet_town.text_address, pokered_symbols::PalletTown_TextPointers.address);
        assert_eq!(pallet_town.script_address, pokered_symbols::PalletTown_Script.address);
        assert_eq!(pallet_town.objects_address, pokered_symbols::PalletTown_Object.address);
        assert_eq!(pallet_town.connections().len(), 2);
        assert_eq!(pallet_town.north_connection.unwrap().direction, MapConnectionDirection::North);
        assert_eq!(pallet_town.north_connection.unwrap().map, Map::Route1);
        // Route 1 is 10 wide × 18 tall; offset 0 → full-width strip, aligned flush
        assert_eq!(pallet_town.north_connection.unwrap().strip_src,          0x4192);
        assert_eq!(pallet_town.north_connection.unwrap().strip_dest,         0xc6eb);
        assert_eq!(pallet_town.north_connection.unwrap().strip_length,       10); // all 10 columns shared
        assert_eq!(pallet_town.north_connection.unwrap().connected_map_width, 10);
        assert_eq!(pallet_town.north_connection.unwrap().y_alignment,        35); // 18*2-1
        assert_eq!(pallet_town.north_connection.unwrap().x_alignment,        0);
        assert_eq!(pallet_town.north_connection.unwrap().view_pointer,       0xc809);
        assert_eq!(pallet_town.south_connection.unwrap().direction, MapConnectionDirection::South);
        assert_eq!(pallet_town.south_connection.unwrap().map, Map::Route21);
        // Route 21 is 10 wide × 45 tall; full-width strip, aligned flush
        assert_eq!(pallet_town.south_connection.unwrap().strip_src,          0x506d);
        assert_eq!(pallet_town.south_connection.unwrap().strip_dest,         0xc7ab);
        assert_eq!(pallet_town.south_connection.unwrap().strip_length,       10);
        assert_eq!(pallet_town.south_connection.unwrap().connected_map_width, 10);
        assert_eq!(pallet_town.south_connection.unwrap().y_alignment,        0);
        assert_eq!(pallet_town.south_connection.unwrap().x_alignment,        0);
        assert_eq!(pallet_town.south_connection.unwrap().view_pointer,       0xc6f9);
    }

    #[test]
    fn test_read_celadon_city() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        let celadon_city = mmu.read_map_header(Map::CeladonCity).unwrap();
        assert_eq!(celadon_city.tileset, TileSetId::Overworld);
        assert_eq!(celadon_city.height, 18);
        assert_eq!(celadon_city.width, 25);
        assert_eq!(celadon_city.blocks_address, pokered_symbols::CeladonCity_Blocks.address);
        assert_eq!(celadon_city.text_address, pokered_symbols::CeladonCity_TextPointers.address);
        assert_eq!(celadon_city.script_address, pokered_symbols::CeladonCity_Script.address);
        assert_eq!(celadon_city.objects_address, pokered_symbols::CeladonCity_Object.address);
        assert_eq!(celadon_city.connections().len(), 2);
        assert_eq!(celadon_city.west_connection.unwrap().direction, MapConnectionDirection::West);
        assert_eq!(celadon_city.west_connection.unwrap().map, Map::Route16);
        // Route 16 is 20 wide × 9 tall; height-strip of 9, connected map x-aligned to right edge
        assert_eq!(celadon_city.west_connection.unwrap().strip_src,          0x4b95);
        assert_eq!(celadon_city.west_connection.unwrap().strip_dest,         0xc7c1);
        assert_eq!(celadon_city.west_connection.unwrap().strip_length,       9);  // 9 rows shared
        assert_eq!(celadon_city.west_connection.unwrap().connected_map_width, 20);
        assert_eq!(celadon_city.west_connection.unwrap().y_alignment,        -8); // aligned offset
        assert_eq!(celadon_city.west_connection.unwrap().x_alignment,        39); // 20*2-1
        assert_eq!(celadon_city.west_connection.unwrap().view_pointer,       0xc716);
        assert_eq!(celadon_city.east_connection.unwrap().direction, MapConnectionDirection::East);
        assert_eq!(celadon_city.east_connection.unwrap().map, Map::Route7);
        // Route 7 is 10 wide × 9 tall; height-strip of 9, x-aligned to left edge of Route 7
        assert_eq!(celadon_city.east_connection.unwrap().strip_src,          0x4051);
        assert_eq!(celadon_city.east_connection.unwrap().strip_dest,         0xc7dd);
        assert_eq!(celadon_city.east_connection.unwrap().strip_length,       9);
        assert_eq!(celadon_city.east_connection.unwrap().connected_map_width, 10);
        assert_eq!(celadon_city.east_connection.unwrap().y_alignment,        -8);
        assert_eq!(celadon_city.east_connection.unwrap().x_alignment,        0);
        assert_eq!(celadon_city.east_connection.unwrap().view_pointer,       0xc6f9);
    }

    #[test]
    fn test_read_cerulean_city() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        let celadon_city = mmu.read_map_header(Map::CeruleanCity).unwrap();
        assert_eq!(celadon_city.tileset, TileSetId::Overworld);
        assert_eq!(celadon_city.height, 18);
        assert_eq!(celadon_city.width, 20);
        assert_eq!(celadon_city.blocks_address, pokered_symbols::CeruleanCity_Blocks.address);
        assert_eq!(celadon_city.text_address, pokered_symbols::CeruleanCity_TextPointers.address);
        assert_eq!(celadon_city.script_address, pokered_symbols::CeruleanCity_Script.address);
        assert_eq!(celadon_city.objects_address, pokered_symbols::CeruleanCity_Object.address);
        assert_eq!(celadon_city.connections().len(), 4);
        assert_eq!(celadon_city.north_connection.unwrap().direction, MapConnectionDirection::North);
        assert_eq!(celadon_city.north_connection.unwrap().map, Map::Route24);
        // Route 24 is 10 wide × 18 tall; full-width strip, x-offset -10 (connected map shifted
        // left)
        assert_eq!(celadon_city.north_connection.unwrap().strip_src,          0x477d);
        assert_eq!(celadon_city.north_connection.unwrap().strip_dest,         0xc6f0);
        assert_eq!(celadon_city.north_connection.unwrap().strip_length,       10);
        assert_eq!(celadon_city.north_connection.unwrap().connected_map_width, 10);
        assert_eq!(celadon_city.north_connection.unwrap().y_alignment,        35); // 18*2-1
        assert_eq!(celadon_city.north_connection.unwrap().x_alignment,        -10);
        assert_eq!(celadon_city.north_connection.unwrap().view_pointer,       0xc809);
        assert_eq!(celadon_city.south_connection.unwrap().direction, MapConnectionDirection::South);
        assert_eq!(celadon_city.south_connection.unwrap().map, Map::Route5);
        // Route 5 is 10 wide × 18 tall; full-width strip, x-offset -10
        assert_eq!(celadon_city.south_connection.unwrap().strip_src,          0x45d2);
        assert_eq!(celadon_city.south_connection.unwrap().strip_dest,         0xc912);
        assert_eq!(celadon_city.south_connection.unwrap().strip_length,       10);
        assert_eq!(celadon_city.south_connection.unwrap().connected_map_width, 10);
        assert_eq!(celadon_city.south_connection.unwrap().y_alignment,        0);
        assert_eq!(celadon_city.south_connection.unwrap().x_alignment,        -10);
        assert_eq!(celadon_city.south_connection.unwrap().view_pointer,       0xc6f9);
        assert_eq!(celadon_city.west_connection.unwrap().direction, MapConnectionDirection::West);
        assert_eq!(celadon_city.west_connection.unwrap().map, Map::Route4);
        // Route 4 is 45 wide × 9 tall; height-strip of 9, x-aligned to right edge (45*2-1=89)
        assert_eq!(celadon_city.west_connection.unwrap().strip_src,          0x4416);
        assert_eq!(celadon_city.west_connection.unwrap().strip_dest,         0xc79e);
        assert_eq!(celadon_city.west_connection.unwrap().strip_length,       9);
        assert_eq!(celadon_city.west_connection.unwrap().connected_map_width, 45);
        assert_eq!(celadon_city.west_connection.unwrap().y_alignment,        -8);
        assert_eq!(celadon_city.west_connection.unwrap().x_alignment,        89); // 45*2-1
        assert_eq!(celadon_city.west_connection.unwrap().view_pointer,       0xc748);
        assert_eq!(celadon_city.east_connection.unwrap().direction, MapConnectionDirection::East);
        assert_eq!(celadon_city.east_connection.unwrap().map, Map::Route9);
        // Route 9 is 30 wide × 9 tall; height-strip of 9, x-aligned to left edge
        assert_eq!(celadon_city.east_connection.unwrap().strip_src,          0x46fe);
        assert_eq!(celadon_city.east_connection.unwrap().strip_dest,         0xc7b5);
        assert_eq!(celadon_city.east_connection.unwrap().strip_length,       9);
        assert_eq!(celadon_city.east_connection.unwrap().connected_map_width, 30);
        assert_eq!(celadon_city.east_connection.unwrap().y_alignment,        -8);
        assert_eq!(celadon_city.east_connection.unwrap().x_alignment,        0);
        assert_eq!(celadon_city.east_connection.unwrap().view_pointer,       0xc70d);
    }
}