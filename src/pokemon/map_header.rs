use crate::mmu::MMU;
use crate::pokemon::map::Map;
use crate::ram::ROM;
use bitflags::{bitflags, Flags};
use itertools::Itertools;
use crate::pokemon::symbols::{DmgBank, DmgPointer, DmgPointerRead};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum TileSetId {
    Overworld = 0, // OVERWORLD
    RedsHouse1 = 1, // REDS_HOUSE_1
    Mart = 2, // MART
    Forest = 3, // FOREST
    RedsHouse2 = 4, // REDS_HOUSE_2
    Dojo = 5, // DOJO
    Pokecenter = 6, // POKECENTER
    Gym = 7, // GYM
    House = 8, // HOUSE
    ForestGate = 9, // FOREST_GATE
    Museum = 10, // MUSEUM
    Underground = 11, // UNDERGROUND
    Gate = 12, // GATE
    Ship = 13, // SHIP
    ShipPort = 14, // SHIP_PORT
    Cemetery = 15, // CEMETERY
    Interior = 16, // INTERIOR
    Cavern = 17, // CAVERN
    Lobby = 18, // LOBBY
    Mansion = 19, // MANSION
    Lab = 20, // LAB
    Club = 21, // CLUB
    Facility = 22, // FACILITY
    Plateau = 23, // PLATEAU
}

impl TileSetId {
    const GYM_CUT_TREE: u8 = 0x50;
    const OVERWORLD_CUT_TREE: u8 = 0x3d;
    
    pub fn cut_tree_tile_id(&self) -> Option<u8> {
        match self {  
            Self::Overworld => Some(Self::OVERWORLD_CUT_TREE),
            Self::Gym => Some(Self::GYM_CUT_TREE),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapHeader {
    pub header_bank: u8,
    pub tileset: TileSetId,
    pub height: u8,
    pub width: u8,
    pub blocks_address: u16,
    pub text_address: u16,
    pub script_address: u16,
    pub objects_address: u16,
    pub north_connection: Option<MapConnection>,
    pub east_connection: Option<MapConnection>,
    pub south_connection: Option<MapConnection>,
    pub west_connection: Option<MapConnection>,
}

impl MapHeader {
    pub fn connections(&self) -> Vec<MapConnection> {
        let mut connections = vec![];
        for connection in [self.north_connection, self.east_connection, self.south_connection, self.west_connection].into_iter() {
            if let Some(connection) = connection {
                connections.push(connection);
            }
        }
        connections
    }

    pub fn blocks_pointer(&self) -> DmgPointer {
        DmgPointer {
            bank: DmgBank::ROM { bank: self.header_bank },
            address: self.blocks_address,
        }
    }

    pub fn objects_pointer(&self) -> DmgPointer {
        DmgPointer {
            bank: DmgBank::ROM { bank: self.header_bank },
            address: self.objects_address,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct MapConnectionDirectionFlags : u8 {
        const East = 0x01;
        const West = 0x02;
        const South = 0x04;
        const North = 0x08;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapConnectionDirection {
    East,
    West,
    South,
    North,
}

impl TryFrom<MapConnectionDirectionFlags> for MapConnectionDirection {
    type Error = String;

    fn try_from(value: MapConnectionDirectionFlags) -> Result<Self, Self::Error> {
        if value.contains_unknown_bits() {
            Err("Map connection direction flags contained unknown bits".to_string())
        } else if value.is_empty() {
            Err("Map connection direction flags empty".to_string())
        } else if value.contains(MapConnectionDirectionFlags::North) {
            Ok(MapConnectionDirection::North)
        } else if value.contains(MapConnectionDirectionFlags::South) {
            Ok(MapConnectionDirection::South)
        } else if value.contains(MapConnectionDirectionFlags::East) {
            Ok(MapConnectionDirection::East)
        } else if value.contains(MapConnectionDirectionFlags::West) {
            Ok(MapConnectionDirection::West)
        } else {
            Err("Unknown direction flag".to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapConnection {
    /// Direction this connection faces, relative to the current map.
    pub direction: MapConnectionDirection,

    /// The ID of the adjacent map.
    pub map: Map,

    /// Pointer into the **connected map's block data** indicating which row/column of blocks
    /// forms the shared border (the 3-block-deep strip that is pre-loaded for seamless scrolling).
    pub strip_src: u16,

    /// Pointer into the **overworld map buffer** (`wOverworldMap`) where the strip will be placed
    /// so the game can render it when the player approaches the edge.
    pub strip_dest: u16,

    /// Number of blocks in the connection strip:
    /// - **North / South** connections: strip width (columns of blocks).
    /// - **East / West** connections: strip height (rows of blocks).
    pub strip_length: u8,

    /// Width of the connected map in blocks.
    /// Needed to stride through its block data correctly.
    pub connected_map_width: u8,

    /// Y tile-offset of the connected map relative to the current map.
    /// Units are tiles (2 tiles = 1 block). Negative means the connected map starts above/further up.
    /// - North: `connected_height * 2 - 1`
    /// - South: `0`
    /// - East / West: `-alignment_offset * 2`
    pub y_alignment: i8,

    /// X tile-offset of the connected map relative to the current map.
    /// Units are tiles (2 tiles = 1 block). Negative means the connected map starts to the left.
    /// - North / South: `-alignment_offset * 2`
    /// - East: `0`
    /// - West: `connected_width * 2 - 1`
    pub x_alignment: i8,

    /// Pointer into the overworld buffer representing where the game's camera window
    /// into the connected map begins (used by the renderer when near the border).
    pub view_pointer: u16,
}

pub trait MapHeaderReader {
    fn read_map_header(&self, pointer: DmgPointer) -> Option<MapHeader>;
}

impl MapHeaderReader for MMU {
    fn read_map_header(&self, pointer: DmgPointer) -> Option<MapHeader> {
        // see `MACRO map_header`

        let connections_byte = self.read_pointer(&(pointer + 9));
        let connections = MapConnectionDirectionFlags::from_bits(connections_byte)?;
        let connection_count = connections.iter().count();
        let mut north_connection = None;
        let mut east_connection = None;
        let mut south_connection = None;
        let mut west_connection = None;

        const CONNECTION_LENGTH_BYTES: u16 = 11;
        for (i, dir_flag) in connections.into_iter()
            .sorted_by_key(|dir| dir.bits())
            .rev() // Connections go in order: north, south, west, east
            .enumerate() {

            let map_connection_pointer = pointer + 10 + i as u16 * CONNECTION_LENGTH_BYTES;
            // Byte 0: connected map ID
            let map = Map::from_repr(self.read_pointer(&map_connection_pointer))?;
            // Bytes 1-2: pointer into connected map's block data (strip source)
            let strip_src = self.read_pointer_u16_le(&(map_connection_pointer + 1));
            // Bytes 3-4: pointer into overworld buffer (strip destination)
            let strip_dest = self.read_pointer_u16_le(&(map_connection_pointer + 3));
            // Byte 5: number of blocks in the connection strip
            let strip_length = self.read_pointer(&(map_connection_pointer + 5));
            // Byte 6: width of the connected map in blocks
            let connected_map_width = self.read_pointer(&(map_connection_pointer + 6));
            // Byte 7: signed Y tile-offset of the connected map relative to the current map
            let y_alignment = self.read_pointer(&(map_connection_pointer + 7)) as i8;
            // Byte 8: signed X tile-offset of the connected map relative to the current map
            let x_alignment = self.read_pointer(&(map_connection_pointer + 8)) as i8;
            // Bytes 9-10: camera window pointer into the overworld buffer
            let view_pointer = self.read_pointer_u16_le(&(map_connection_pointer + 9));

            let connection = MapConnection {
                map,
                direction: dir_flag.try_into().ok()?,
                strip_src,
                strip_dest,
                strip_length,
                connected_map_width,
                y_alignment,
                x_alignment,
                view_pointer,
            };
            match connection.direction {
                MapConnectionDirection::East => east_connection = Some(connection),
                MapConnectionDirection::West => west_connection = Some(connection),
                MapConnectionDirection::South => south_connection = Some(connection),
                MapConnectionDirection::North => north_connection = Some(connection),
            };
        }

        let objects_address_pointer = pointer + 10 + connection_count as u16 * CONNECTION_LENGTH_BYTES;
        Some(
            MapHeader {
                header_bank: pointer.bank.id(),
                tileset: TileSetId::from_repr(self.read_pointer(&pointer))?,
                height: self.read_pointer(&(pointer + 1)),
                width: self.read_pointer(&(pointer + 2)),
                blocks_address: self.read_pointer_u16_le(&(pointer + 3)),
                text_address: self.read_pointer_u16_le(&(pointer + 5)),
                script_address: self.read_pointer_u16_le(&(pointer + 7)),
                north_connection,
                east_connection,
                south_connection,
                west_connection,
                objects_address: self.read_pointer_u16_le(&objects_address_pointer),
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::pokemon::symbols::pokered_symbols;
    use super::*;

    #[test]
    fn test_read_oaks_lab() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        let oaks_lab = mmu.read_map_header(pokered_symbols::OaksLab_h).unwrap();
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
        let pallet_town = mmu.read_map_header(pokered_symbols::PalletTown_h).unwrap();
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
        let celadon_city = mmu.read_map_header(pokered_symbols::CeladonCity_h).unwrap();
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
        let celadon_city = mmu.read_map_header(pokered_symbols::CeruleanCity_h).unwrap();
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
        // Route 24 is 10 wide × 18 tall; full-width strip, x-offset -10 (connected map shifted left)
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