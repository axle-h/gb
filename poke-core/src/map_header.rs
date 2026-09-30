use crate::map::Map;
use crate::tables;
use bitflags::{bitflags, Flags};

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
    /// `data/tilesets/warp_tile_ids.asm`, read by `CheckWarpsNoCollision`. A list runs on through
    /// the labels after it, so Gate, Museum and ForestGate share RedsHouse's ids.
    pub fn warp_tile_ids(&self) -> &'static [u8] {
        tables::WARP_TILE_IDS[*self as usize]
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
        tables::WARP_CARPET_TILE_IDS[match facing { Facing::Down => 0, Facing::Up => 1, Facing::Left => 2, Facing::Right => 3 }]
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
    pub tileset: TileSetId,
    pub height: u8,
    pub width: u8,
    /// Row-major, `width` to a row.
    pub blocks: &'static [u8],
    pub north_connection: Option<MapConnection>,
    pub east_connection: Option<MapConnection>,
    pub south_connection: Option<MapConnection>,
    pub west_connection: Option<MapConnection>,
}

impl MapHeader {
    pub fn connections(&self) -> Vec<MapConnection> {
        [self.north_connection, self.east_connection, self.south_connection, self.west_connection].into_iter().flatten().collect()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// A `connection` as `LoadTileBlockMap` and `CheckMapConnections` use it. The three block offsets
/// are the macro's, with the label each is added to in the cartridge left off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapConnection {
    /// Direction this connection faces, relative to the current map.
    pub direction: MapConnectionDirection,

    /// The ID of the adjacent map.
    pub map: Map,

    /// The block of the connected map the 3-block-deep strip starts at.
    pub strip_src_block: u16,

    /// Where in `wOverworldMap` the strip goes.
    pub strip_dest_block: u16,

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

    /// `wCurrentTileBlockMapViewPointer` in the connected map's `wOverworldMap` on crossing.
    pub view_block: u16,
}

impl MapConnection {
    /// The `connection` macro's arithmetic over both maps' sizes and the offset written.
    fn new(from: &tables::MapHeader, connection: &tables::Connection) -> Result<Self, String> {
        let map = Map::from_repr(connection.map).ok_or("Unknown map in map connection")?;
        let to = tables::MAP_HEADERS[connection.map as usize].ok_or_else(|| format!("{map} has no header"))?;
        let flags = MapConnectionDirectionFlags::from_bits(connection.direction).ok_or("Unknown connection direction")?;
        let direction = MapConnectionDirection::try_from(flags)?;
        let (width, height) = (to.width as i32, to.height as i32);
        let (current_width, current_height) = (from.width as i32, from.height as i32);
        let offset = connection.offset as i32;
        let (mut src, mut tgt) = (0, offset + 3);
        if tgt < 2 {
            src = -tgt;
            tgt = 0;
        }
        let (block, dest, view, y, x, length) = match direction {
            MapConnectionDirection::North => (width * (height - 3) + src, tgt, (width + 6) * height + 1, height * 2 - 1, offset * -2,
                (current_width + 3 - offset).min(width)),
            MapConnectionDirection::South => (src, (current_width + 6) * (current_height + 3) + tgt, width + 7, 0, offset * -2,
                (current_width + 3 - offset).min(width)),
            MapConnectionDirection::West => (width * src + width - 3, (current_width + 6) * tgt, (width + 6) * 2 - 6, offset * -2, width * 2 - 1,
                (current_height + 3 - offset).min(height)),
            MapConnectionDirection::East => (width * src, (current_width + 6) * tgt + current_width + 3, width + 7, offset * -2, 0,
                (current_height + 3 - offset).min(height)),
        };
        Ok(MapConnection {
            direction,
            map,
            strip_src_block: block as u16,
            strip_dest_block: dest as u16,
            strip_length: (length - src) as u8,
            connected_map_width: to.width,
            y_alignment: y as u8 as i8,
            x_alignment: x as u8 as i8,
            view_block: view as u16,
        })
    }
}

/// How far a map's tile-map coordinates sit from its raw warp-table ones: one column if the map
/// has a western connection strip, one row if it has a northern one
/// (`MapDimensions::{west,north}_extra`). It can be asked about a map the player is not on, which
/// is what a menu row needs to say where a warp comes out in the coordinates the picture of that
/// map uses.
pub fn strip_offset(map: Map) -> (u8, u8) {
    let Ok(header) = MapHeader::read(map) else { return (0, 0) };
    (header.west_connection.is_some() as u8, header.north_connection.is_some() as u8)
}

impl MapHeader {
    /// `map`'s own `map_header`, or an error for a map that borrows another's.
    pub fn read(map: Map) -> Result<MapHeader, String> {
        let source = tables::MAP_HEADERS[map as usize].ok_or_else(|| format!("{map} has no header of its own"))?;
        let mut header = MapHeader {
            tileset: TileSetId::from_repr(source.tileset).ok_or("Unknown tileset")?,
            height: source.height,
            width: source.width,
            blocks: source.blocks,
            north_connection: None,
            east_connection: None,
            south_connection: None,
            west_connection: None,
        };
        for connection in source.connections {
            let connection = MapConnection::new(&source, connection)?;
            *match connection.direction {
                MapConnectionDirection::East => &mut header.east_connection,
                MapConnectionDirection::West => &mut header.west_connection,
                MapConnectionDirection::South => &mut header.south_connection,
                MapConnectionDirection::North => &mut header.north_connection,
            } = Some(connection);
        }
        Ok(header)
    }
}
