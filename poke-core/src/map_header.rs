use crate::map::Map;
use itertools::Itertools;
use bitflags::{bitflags, Flags};
use crate::symbols::{DmgBank, DmgPointer};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum TileSetId {
    #[default]
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
    
    /// Raw tile IDs in this tileset that warp the player the moment they step onto them —
    /// `pokered/data/tilesets/warp_tile_ids.asm`, read by `CheckWarpsNoCollision`.
    pub fn warp_tile_ids(&self) -> &'static [u8] {
        match self {
            Self::Overworld => &[0x1B, 0x58],
            // The `db` entries in that file fall through into the next label, so Gate/Museum/
            // ForestGate pick up RedsHouse's two ids as well, Facility picks up Cemetery's and
            // Underground's, and Cemetery picks up Underground's.
            Self::ForestGate | Self::Museum | Self::Gate => &[0x3B, 0x1A, 0x1C],
            Self::RedsHouse1 | Self::RedsHouse2 => &[0x1A, 0x1C],
            Self::Mart | Self::Pokecenter => &[0x5E],
            Self::Forest => &[0x5A, 0x5C, 0x3A],
            Self::Dojo | Self::Gym => &[0x4A],
            Self::House => &[0x54, 0x5C, 0x32],
            Self::Ship => &[0x37, 0x39, 0x1E, 0x4A],
            Self::Interior => &[0x15, 0x55, 0x04],
            Self::Cavern => &[0x18, 0x1A, 0x22],
            Self::Lobby => &[0x1A, 0x1C, 0x38],
            Self::Mansion => &[0x1A, 0x1C, 0x53],
            Self::Lab => &[0x34],
            Self::Facility => &[0x43, 0x58, 0x20, 0x1B, 0x13],
            Self::Cemetery => &[0x1B, 0x13],
            Self::Underground => &[0x13],
            Self::Plateau => &[0x1B, 0x3B],
            Self::ShipPort | Self::Club => &[],
        }
    }

    /// Tiles that warp the player the moment they step onto them by a second mechanism entirely —
    /// `data/tilesets/warp_pad_hole_tile_ids.asm`, read by `IsPlayerStandingOnWarpPadOrHole`
    /// (`engine/overworld/player_animations.asm`).
    pub fn warp_pad_and_hole_tile_ids(&self) -> &'static [u8] {
        match self {
            Self::Facility => &[0x20, 0x11],
            Self::Cavern   => &[0x22],
            Self::Interior => &[0x55],
            _ => &[],
        }
    }

    /// The tiles that make `ExtraWarpCheck`'s "function 2" pass, per direction faced.
    pub fn warp_carpet_tile_ids(facing: crate::sprite::PlayerFacingDirection) -> &'static [u8] {
        use crate::sprite::PlayerFacingDirection as Facing;
        match facing {
            Facing::Down  => &[0x01, 0x12, 0x17, 0x3D, 0x04, 0x18, 0x33],
            Facing::Up    => &[0x01, 0x5C],
            Facing::Left  => &[0x1A, 0x4B],
            Facing::Right => &[0x0F, 0x4E],
        }
    }

    /// Whether `ExtraWarpCheck` dispatches to `IsWarpTileInFrontOfPlayer` ("function 2") rather
    /// than `IsPlayerFacingEdgeOfMap` ("function 1") on this tileset.
    pub fn warp_check_reads_the_tile_in_front(&self) -> bool {
        matches!(self, Self::Overworld | Self::Ship | Self::ShipPort | Self::Plateau)
    }

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
    pub struct MapConnectionDirectionFlags : u8 {
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

    /// Pointer into the connected map's block data indicating which row/column of blocks forms
    /// the shared border (the 3-block-deep strip that is pre-loaded for seamless scrolling).
    pub strip_src: u16,

    /// Pointer into the overworld map buffer (`wOverworldMap`) where the strip will be placed so
    /// the game can render it when the player approaches the edge.
    pub strip_dest: u16,

    /// Number of blocks in the connection strip:
    pub strip_length: u8,

    /// Width of the connected map in blocks. Needed to stride through its block data correctly.
    pub connected_map_width: u8,

    /// Y tile-offset of the connected map relative to the current map. Units are tiles (2 tiles =
    /// 1 block).
    pub y_alignment: i8,

    /// X tile-offset of the connected map relative to the current map. Units are tiles (2 tiles =
    /// 1 block).
    pub x_alignment: i8,

    /// Pointer into the overworld buffer representing where the game's camera window into the
    /// connected map begins (used by the renderer when near the border).
    pub view_pointer: u16,
}

/// How far a map's tile-map coordinates sit from its raw warp-table ones: one column if the map
/// has a western connection strip, one row if it has a northern one
/// (`MapDimensions::{west,north}_extra`). Read straight out of the ROM, so it can be asked about
/// a map the player is not on — which is what a menu row needs to say where a warp comes out in
/// the coordinates the picture of that map uses.
pub fn strip_offset(map: Map) -> (u8, u8) {
    let Some(pointer) = map.header_pointer() else { return (0, 0) };
    // Byte 9 of the header is the connection flags; see `MACRO map_header`.
    let flags = crate::rom_gfx::rom_slice(pointer)[9];
    let flags = MapConnectionDirectionFlags::from_bits_truncate(flags);
    (flags.contains(MapConnectionDirectionFlags::West) as u8, flags.contains(MapConnectionDirectionFlags::North) as u8)
}

impl MapHeader {
    /// `MACRO map_header`, read out of the cartridge.
    pub fn read(map: Map) -> Result<MapHeader, String> {
        let pointer = map.header_pointer()
            .ok_or("Map header pointer was null".to_string())?;
        let rom = crate::rom_gfx::rom_slice(pointer);
        let byte = |offset: u16| rom[offset as usize];
        let word = |offset: u16| u16::from_le_bytes([rom[offset as usize], rom[offset as usize + 1]]);

        let connections_byte = byte(9);
        let connections = MapConnectionDirectionFlags::from_bits(connections_byte)
            .ok_or("Map connection flags contained unknown bits".to_string())?;
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

            let map_connection_pointer = 10 + i as u16 * CONNECTION_LENGTH_BYTES;
            // Byte 0: connected map ID
            let map = Map::from_repr(byte(map_connection_pointer))
                .ok_or("Unknown map in map connection".to_string())?;
            // Bytes 1-2: pointer into connected map's block data (strip source)
            let strip_src = word(map_connection_pointer + 1);
            // Bytes 3-4: pointer into overworld buffer (strip destination)
            let strip_dest = word(map_connection_pointer + 3);
            // Byte 5: number of blocks in the connection strip
            let strip_length = byte(map_connection_pointer + 5);
            // Byte 6: width of the connected map in blocks
            let connected_map_width = byte(map_connection_pointer + 6);
            // Byte 7: signed Y tile-offset of the connected map relative to the current map
            let y_alignment = byte(map_connection_pointer + 7) as i8;
            // Byte 8: signed X tile-offset of the connected map relative to the current map
            let x_alignment = byte(map_connection_pointer + 8) as i8;
            // Bytes 9-10: camera window pointer into the overworld buffer
            let view_pointer = word(map_connection_pointer + 9);

            let connection = MapConnection {
                map,
                direction: dir_flag.try_into()?,
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

        let objects_address_pointer = 10 + connection_count as u16 * CONNECTION_LENGTH_BYTES;
        Ok(
            MapHeader {
                header_bank: pointer.bank.id(),
                tileset: TileSetId::from_repr(byte(0))
                    .ok_or("Unknown map header bank".to_string())?,
                height: byte(1),
                width: byte(2),
                blocks_address: word(3),
                text_address: word(5),
                script_address: word(7),
                north_connection,
                east_connection,
                south_connection,
                west_connection,
                objects_address: word(objects_address_pointer),
            }
        )
    }
}
