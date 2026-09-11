use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use gb::geometry::Point8;
use gb::joypad::JoypadButton;
use gb::mmu::MMU;
use crate::pokemon::bag::BagReader;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeader, MapHeaderReader, TileSetId};
use crate::pokemon::sprite::{PictureId, Sprite, SpriteFacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::{JumpDirection, MetaTile, WarpEvent};
use gb::ram::ROM;

#[derive(Clone)]
pub struct MapMetadata {
    pub map: Map,
    pub map_header: MapHeader,
    pub map_data: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    /// Counters and desks the player can talk across (`wTilesetTalkingOverTiles`).
    pub talking_over_tiles: HashSet<u8>,
    pub warp_events: Vec<WarpEvent>,
    /// The tileset is in `WaterTilesets`, so its water and shore ids are water.
    pub is_water_tileset: bool,
    /// The tileset's tall-grass tile id, zero for none.
    pub grass_tile_id: u8,
    pub connected_strips: Vec<ConnectedMapStrip>,
    /// Ledge tile ids to jump direction; only the Overworld tileset has `HandleLedges`.
    pub ledge_tiles: HashMap<u8, JumpDirection>,
    /// Raw tile pairs the player may not step between though each is passable, the elevation
    /// boundaries of `TilePairCollisionsLand`.
    pub tile_pair_collisions: Vec<(u8, u8)>,
    /// `TilePairCollisionsWater`, checked on mounting Surf, stepping ashore and every surfing move;
    /// in Seafoam it forbids surfing on or off from plain cave floor.
    pub tile_pair_collisions_water: Vec<(u8, u8)>,
    /// The tile grid without sprites, computed once and cloned under each sprite overlay.
    pub meta_tiles_base: Vec<MetaTile>,
    /// Bottom-left raw tile id of each meta-tile, the sub-tile the cartridge's collision check reads.
    pub raw_tile_ids: Vec<u8>,
}

impl std::fmt::Debug for MapMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapMetadata({}, {}x{} blocks, tileset {:?})",
               self.map, self.map_header.width, self.map_header.height, self.map_header.tileset)
    }
}

impl MapMetadata {
    pub const BLOCK_TILE_WIDTH: usize = 4;
    pub const BLOCK_TILES: usize = Self::BLOCK_TILE_WIDTH * Self::BLOCK_TILE_WIDTH;
    pub const TILES_PER_META: usize = 2;

    pub fn tile_id(&self, tile_x: usize, tile_y: usize) -> u8 {
        let block_x = tile_x / Self::BLOCK_TILE_WIDTH;
        let block_y = tile_y / Self::BLOCK_TILE_WIDTH;
        let block_index = self.map_data[block_x + block_y * self.map_header.width as usize] as usize;
        let block_offset = block_index * Self::BLOCK_TILES;
        let tile_offset = (tile_x % Self::BLOCK_TILE_WIDTH) + (tile_y % Self::BLOCK_TILE_WIDTH) * Self::BLOCK_TILE_WIDTH;
        self.tileset_data[block_offset + tile_offset]
    }

    pub fn is_empty(&self, tile_x: usize, tile_y: usize) -> bool {
        self.collision_tiles.contains(&self.tile_id(tile_x, tile_y))
    }

    pub fn is_water(&self, tile_x: usize, tile_y: usize) -> bool {
        is_water_tile_id(self.tile_id(tile_x, tile_y), self.is_water_tileset, self.map_header.tileset)
    }

    pub fn dimensions(&self) -> MapDimensions {
        MapDimensions {
            meta_height: self.map_header.height as usize * Self::TILES_PER_META,
            meta_width: self.map_header.width as usize * Self::TILES_PER_META,
            north_extra: self.map_header.north_connection.is_some() as usize,
            east_extra: self.map_header.east_connection.is_some() as usize,
            south_extra: self.map_header.south_connection.is_some() as usize,
            west_extra: self.map_header.west_connection.is_some() as usize,
        }
    }

    pub fn meta_tiles(&self, sprites: &[Sprite]) -> Vec<MetaTile> {
        let mut result = self.meta_tiles_base.clone();
        let dimensions = self.dimensions();
        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();

        for sprite in sprites.iter().filter(|s| !s.hidden) {
            let mx = sprite.position.x as usize + dimensions.west_extra;
            let my = sprite.position.y as usize + dimensions.north_extra;
            if mx < exp_width && my < exp_height {
                let idx = mx + my * exp_width;
                result[idx] = MetaTile::Sprite(sprite.name);
            }
        }

        result
    }

    /// Each visible person's square and the tile the sprite overlay painted over.
    pub fn underfoot(&self, sprites: &[Sprite]) -> Vec<(Point8, MetaTile)> {
        let dimensions = self.dimensions();
        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();
        sprites.iter().filter(|s| !s.hidden).filter_map(|sprite| {
            let mx = sprite.position.x as usize + dimensions.west_extra;
            let my = sprite.position.y as usize + dimensions.north_extra;
            (mx < exp_width && my < exp_height).then(|| (
                Point8 { x: mx as u8, y: my as u8 },
                self.meta_tiles_base[mx + my * exp_width],
            ))
        }).collect()
    }

    pub fn build_meta_tiles_base(&self) -> Vec<MetaTile> {
        let dimensions = self.dimensions();

        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();

        let mut result = vec![MetaTile::Obstacle; exp_width * exp_height];

        let width_tiles  = self.map_header.width  as usize * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_header.height as usize * Self::BLOCK_TILE_WIDTH;
        for tile_y in 0..height_tiles {
            let my = tile_y / Self::TILES_PER_META + dimensions.north_extra;
            for tile_x in 0..width_tiles {
                let mx    = tile_x / Self::TILES_PER_META + dimensions.west_extra;
                let index = mx + my * exp_width;
                if result[index] != MetaTile::Water {
                    if self.is_water(tile_x, tile_y) {
                        result[index] = MetaTile::Water;
                    } else if tile_x % Self::TILES_PER_META == 0 && tile_y % Self::TILES_PER_META == 1 {
                        // The bottom-left sub-tile is the one the cartridge checks.
                        let tile_id = self.tile_id(tile_x, tile_y);
                        if self.map_header.tileset.cut_tree_tile_id() == Some(tile_id) {
                            result[index] = MetaTile::CutTree;
                        } else if let Some(&dir) = self.ledge_tiles.get(&tile_id) {
                            result[index] = MetaTile::Jump(dir);
                        } else if self.grass_tile_id != 0 && tile_id == self.grass_tile_id
                            && self.is_empty(tile_x, tile_y)
                            && matches!(result[index], MetaTile::Obstacle | MetaTile::Empty)
                        {
                            result[index] = MetaTile::Grass;
                        } else if result[index] == MetaTile::Obstacle && self.is_empty(tile_x, tile_y) {
                            result[index] = MetaTile::Empty;
                        } else if result[index] == MetaTile::Obstacle && self.talking_over_tiles.contains(&tile_id) {
                            result[index] = MetaTile::Counter;
                        }
                    }
                }
            }
        }

        for warp in &self.warp_events {
            let mx = warp.position.x as usize + dimensions.west_extra;
            let my = warp.position.y as usize + dimensions.north_extra;
            if mx < exp_width && my < exp_height {
                // Only where at least one raw sub-tile is walkable.
                if result[mx + my * exp_width] != MetaTile::Obstacle {
                    result[mx + my * exp_width] = warp.tile();
                }
            }
        }

        for (strip, strip_idx, mx, my) in self.strip_cells() {
            result[mx + my * exp_width] = strip.meta_tile_at(strip_idx);
        }

        result
    }

    /// Every connection-strip cell as `(strip, strip_idx, mx, my)`, the one placement both the
    /// classifier and the renderer use.
    pub fn strip_cells(&self) -> impl Iterator<Item = (&ConnectedMapStrip, usize, usize, usize)> {
        let dimensions = self.dimensions();
        let (exp_width, exp_height) = (dimensions.full_width(), dimensions.full_height());
        self.connected_strips.iter().flat_map(move |strip| {
            let strip_meta = strip.strip_length as usize * Self::TILES_PER_META;
            (0..strip_meta).filter_map(move |i| match strip.direction {
                MapConnectionDirection::North | MapConnectionDirection::South => {
                    let mx = strip.meta_align_offset + dimensions.west_extra + i;
                    let my = match strip.direction == MapConnectionDirection::North {
                        true => 0,
                        false => dimensions.north_extra + dimensions.meta_height,
                    };
                    (mx < exp_width).then_some((strip, i, mx, my))
                }
                MapConnectionDirection::West | MapConnectionDirection::East => {
                    let my = strip.meta_align_offset + dimensions.north_extra + i;
                    let mx = match strip.direction == MapConnectionDirection::West {
                        true => 0,
                        false => dimensions.west_extra + dimensions.meta_width,
                    };
                    (my < exp_height).then_some((strip, i, mx, my))
                }
            })
        })
    }

    pub fn build_raw_tile_ids(&self) -> Vec<u8> {
        let dimensions = self.dimensions();
        let exp_width  = dimensions.full_width();
        let exp_height = dimensions.full_height();
        let mut ids = vec![0xFFu8; exp_width * exp_height];

        let width_tiles  = self.map_header.width  as usize * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_header.height as usize * Self::BLOCK_TILE_WIDTH;
        for tile_y in (1..height_tiles).step_by(Self::TILES_PER_META) {
            let my = tile_y / Self::TILES_PER_META + dimensions.north_extra;
            for tile_x in (0..width_tiles).step_by(Self::TILES_PER_META) {
                let mx = tile_x / Self::TILES_PER_META + dimensions.west_extra;
                ids[mx + my * exp_width] = self.tile_id(tile_x, tile_y);
            }
        }
        ids
    }

    /// Overlay runtime `ReplaceTileBlock` door blocks onto a built grid.
    pub fn apply_door_blocks(&self, result: &mut [MetaTile], doors: &[DoorBlock]) {
        let dims = self.dimensions();
        let exp_width = dims.full_width();
        for d in doors {
            for sub_y in 0..Self::TILES_PER_META {
                for sub_x in 0..Self::TILES_PER_META {
                    let mx = d.block_x as usize * Self::TILES_PER_META + sub_x + dims.west_extra;
                    let my = d.block_y as usize * Self::TILES_PER_META + sub_y + dims.north_extra;
                    if mx >= exp_width { continue; }
                    let idx = mx + my * exp_width;
                    if idx >= result.len() { continue; }
                    let tile_off = (sub_x * Self::TILES_PER_META)
                        + (sub_y * Self::TILES_PER_META + 1) * Self::BLOCK_TILE_WIDTH;
                    let tile = self.tileset_data
                        .get(d.block_id as usize * Self::BLOCK_TILES + tile_off)
                        .copied()
                        .unwrap_or(0xFF);
                    result[idx] = if self.collision_tiles.contains(&tile) {
                        MetaTile::Empty
                    } else {
                        MetaTile::Obstacle
                    };
                }
            }
        }
    }
}

impl MapMetadata {
    /// Card Key door tiles `$18` and `$24`: walls while locked, passable once the key opens them on
    /// approach, though the static tileset marks them impassable.
    pub fn apply_card_key_doors(&self, result: &mut [MetaTile], locked: bool) {
        for (idx, &tile) in self.raw_tile_ids.iter().enumerate() {
            if tile == 0x18 || tile == 0x24 {
                result[idx] = if locked { MetaTile::Obstacle } else { MetaTile::Empty };
            }
        }
    }
}

impl MapMetadata {
    /// Pokémon Mansion 3F's floor holes as warps to 1F, the only way to 1F's right side and the B1F
    /// stairs.
    pub fn apply_mansion_holes(&self, result: &mut [MetaTile]) {
        if self.map != Map::PokemonMansion3F { return; }
        let w = self.dimensions().full_width();
        for (x, y) in [(16usize, 14usize), (17, 14), (19, 14)] {
            let idx = x + y * w;
            if idx < result.len() {
                result[idx] = MetaTile::Warp { to_map: Map::PokemonMansion1F, to_position: Point8 { x: 16, y: 14 } };
            }
        }
    }

    /// Victory Road 3F's floor hole as a warp to 2F, the only way onto 2F's east side and the exit.
    pub fn apply_victory_road_holes(&self, result: &mut [MetaTile]) {
        if self.map != Map::VictoryRoad3F { return; }
        let w = self.dimensions().full_width();
        let idx = 23 + 15 * w;
        if idx < result.len() {
            result[idx] = MetaTile::Warp { to_map: Map::VictoryRoad2F, to_position: Point8 { x: 22, y: 16 } };
        }
    }

    /// Seafoam Islands' floor holes, two a floor, as warps to the `DungeonWarpData` landing below.
    pub fn apply_seafoam_holes(&self, result: &mut [MetaTile]) {
        let holes: &[((u8, u8), Map, (u8, u8))] = match self.map {
            Map::SeafoamIslands1F  => &[((17, 6), Map::SeafoamIslandsB1F, (18, 7)),
                                        ((24, 6), Map::SeafoamIslandsB1F, (23, 7))],
            Map::SeafoamIslandsB1F => &[((18, 6), Map::SeafoamIslandsB2F, (19, 7)),
                                        ((23, 6), Map::SeafoamIslandsB2F, (22, 7))],
            Map::SeafoamIslandsB2F => &[((19, 6), Map::SeafoamIslandsB3F, (18, 7)),
                                        ((22, 6), Map::SeafoamIslandsB3F, (19, 7))],
            Map::SeafoamIslandsB3F => &[((3, 16), Map::SeafoamIslandsB4F, (4, 14)),
                                        ((6, 16), Map::SeafoamIslandsB4F, (5, 14))],
            _ => return,
        };
        let w = self.dimensions().full_width();
        for &((x, y), to_map, (tx, ty)) in holes {
            let idx = x as usize + y as usize * w;
            if idx < result.len() {
                result[idx] = MetaTile::Warp { to_map, to_position: Point8 { x: tx, y: ty } };
            }
        }
    }

    /// Seafoam B3F's current at (15,8) force-walks the player onto B4F's east water, walled off from
    /// Articuno; no current is modelled, so the tile is an obstacle.
    pub fn apply_seafoam_currents(&self, result: &mut [MetaTile]) {
        if self.map != Map::SeafoamIslandsB3F { return; }
        let w = self.dimensions().full_width();
        let idx = 15 + 8 * w;
        if idx < result.len() {
            result[idx] = MetaTile::Obstacle;
        }
    }
}

pub fn map_has_card_key_doors(map: Map) -> bool {
    matches!(map,
        Map::SilphCo1F | Map::SilphCo2F | Map::SilphCo3F | Map::SilphCo4F | Map::SilphCo5F
        | Map::SilphCo6F | Map::SilphCo7F | Map::SilphCo8F | Map::SilphCo9F | Map::SilphCo10F
        | Map::SilphCo11F)
}

/// A runtime door block that is currently closed, to be overlaid on the static map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoorBlock {
    /// Block coordinates, as passed to `ReplaceTileBlock`.
    pub block_x: u8,
    pub block_y: u8,
    /// The block drawn when the door is shut (`wNewTileBlockID`).
    pub block_id: u8,
}

struct DoorSpec {
    block_x: u8,
    block_y: u8,
    closed_block_id: u8,
    /// Open if any clause has all its `(wEventFlags byte, bit mask)` pairs set.
    open_clauses: &'static [&'static [(u16, u8)]],
}

/// Event-gated door blocks per map (`*DoorCallbackScript`).
fn map_door_specs(map: Map) -> &'static [DoorSpec] {
    match map {
        // The elevator door, shut until Rocket 5 is beaten.
        Map::RocketHideoutB1F => &[DoorSpec {
            block_x: 12, block_y: 8, closed_block_id: 0x54,
            open_clauses: &[&[(206, 0x80)], &[(206, 0x20)]],
        }],
        // Giovanni's door, shut until both Rockets are beaten.
        Map::RocketHideoutB4F => &[DoorSpec {
            block_x: 12, block_y: 5, closed_block_id: 0x2d,
            open_clauses: &[&[(212, 0x20)], &[(212, 0x04), (212, 0x08)]],
        }],
        _ => &[],
    }
}

fn closed_door_blocks(mmu: &MMU, map: Map) -> Vec<DoorBlock> {
    let base = pokered_symbols::wEventFlags.address;
    map_door_specs(map).iter().filter_map(|spec| {
        let open = spec.open_clauses.iter().any(|clause| {
            clause.iter().all(|&(byte, bit)| mmu.read(base + byte) & bit != 0)
        });
        (!open).then_some(DoorBlock {
            block_x: spec.block_x, block_y: spec.block_y, block_id: spec.closed_block_id,
        })
    }).collect()
}

const BIT_STANDING_ON_WARP: u8 = 0b100;

/// A warp entry that a map script cancels, and the events that stop it cancelling.
struct WarpGateSpec {
    /// Raw coordinates, without connection padding.
    at: Point8,
    /// Live once all these `(wEventFlags byte, bit mask)` pairs are set.
    live_when_all_set: &'static [(u16, u8)],
}

/// The warps a map's own script cancels. Kept tiny: withholding a real door is how a floor loses
/// its only exit, so only a refusal proved in the cartridge's source goes in.
fn map_warp_gate_specs(map: Map) -> &'static [WarpGateSpec] {
    match map {
        // The two staircases out of B4F's east pocket.
        Map::SeafoamIslandsB4F => &[
            WarpGateSpec { at: Point8 { x: 20, y: 17 }, live_when_all_set: &[(313, 0x01), (313, 0x02)] },
            WarpGateSpec { at: Point8 { x: 21, y: 17 }, live_when_all_set: &[(313, 0x01), (313, 0x02)] },
        ],
        _ => &[],
    }
}

/// A lift's doors lead wherever `wWarpEntries` says: the floor it was entered from until the panel
/// picks another, which the ROM's table, written for one floor, cannot know.
fn with_live_exits(mmu: &MMU, metadata: &MapMetadata) -> MapMetadata {
    let mut live = metadata.clone();
    let count = mmu.read_pointer(&pokered_symbols::wNumberOfWarps) as usize;
    for (index, warp) in live.warp_events.iter_mut().enumerate().take(count) {
        let entry = pokered_symbols::wWarpEntries.address + index as u16 * 4;
        let (warp_id, raw_map) = (mmu.read(entry + 2) as u16, mmu.read(entry + 3));
        let Some(map) = Map::from_repr(raw_map).filter(|map| map.header_pointer().is_some()) else { continue };
        if let Ok(position) = mmu.read_destination_warp_position(map, warp_id) {
            warp.destination_map = map;
            warp.destination_position = position;
        }
    }
    // The tiles were placed from the ROM's table when the map was first read.
    let dimensions = live.dimensions();
    for warp in live.warp_events.clone() {
        let at = warp.position.x as usize + dimensions.west_extra
            + (warp.position.y as usize + dimensions.north_extra) * dimensions.full_width();
        if let Some(tile @ MetaTile::Warp { .. }) = live.meta_tiles_base.get_mut(at) {
            *tile = warp.tile();
        }
    }
    live
}

/// The squares on `map` whose warp a script is cancelling now, in raw coordinates.
pub(crate) fn script_cancelled_warps(mmu: &MMU, map: Map) -> Vec<Point8> {
    let base = pokered_symbols::wEventFlags.address;
    map_warp_gate_specs(map).iter().filter_map(|spec| {
        let live = spec.live_when_all_set.iter()
            .all(|&(byte, bit)| mmu.read(base + byte) & bit != 0);
        (!live).then_some(spec.at)
    }).collect()
}

/// The largest door block on `map`, so the tileset load covers it.
fn max_door_block_id(map: Map) -> usize {
    map_door_specs(map).iter().map(|d| d.closed_block_id as usize).max().unwrap_or(0)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MapDimensions {
    pub meta_height: usize,
    pub meta_width: usize,

    /// One extra meta-tile row or column per connected direction.
    pub north_extra: usize,
    pub east_extra: usize,
    pub south_extra: usize,
    pub west_extra: usize,
}

impl MapDimensions {
    pub fn full_width(&self) -> usize {
        self.meta_width + self.west_extra + self.east_extra
    }

    pub fn full_height(&self) -> usize {
        self.meta_height + self.north_extra + self.south_extra
    }
}

#[derive(Clone)]
pub struct ConnectedMapStrip {
    pub direction: MapConnectionDirection,
    pub map: Map,
    /// The connected map's border row or column of blocks.
    pub border_blocks: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    pub is_water_tileset: bool,
    pub tileset: TileSetId,
    /// Which meta-row (N/S) or meta-column (E/W) of each block borders us: 0 for south or east.
    pub block_sub_offset: u8,
    pub strip_length: u8,
    /// Where the strip begins along our edge, in meta-tiles.
    pub meta_align_offset: usize,
    pub to_border_coord: u8,
    /// Strip cell `i` lands at `to_strip_start + i` along the connected map's edge.
    pub to_strip_start: u8,
    /// Blocks to skip where `strip_src` begins before the tile aligned with our edge.
    pub border_blocks_start_offset: usize,
}

impl ConnectedMapStrip {
    /// The four tile ids that draw the cell at `strip_idx`, reading order; `meta_tile_at` is what
    /// it means.
    pub fn tile_ids_at(&self, strip_idx: usize) -> Option<[Option<u8>; 4]> {
        let block_idx = strip_idx / 2 + self.border_blocks_start_offset;
        let block_id = *self.border_blocks.get(block_idx)?;

        let (sub_col, sub_row) = match self.direction {
            MapConnectionDirection::North | MapConnectionDirection::South =>
                (strip_idx % 2, self.block_sub_offset as usize),
            MapConnectionDirection::East | MapConnectionDirection::West =>
                (self.block_sub_offset as usize, strip_idx % 2),
        };

        let base = sub_col * 2 + sub_row * 8;
        let block_start = block_id as usize * MapMetadata::BLOCK_TILES;
        Some([base, base + 1, base + 4, base + 5]
            .map(|idx| self.tileset_data.get(block_start + idx).copied()))
    }

    fn meta_tile_at(&self, strip_idx: usize) -> MetaTile {

        let Some(tile_indices) = self.tile_ids_at(strip_idx) else { return MetaTile::Obstacle };

        let has_water = tile_indices.iter().flatten().any(|&tile_id| self.is_water_tile(tile_id));

        // Bottom-left only, as in-map tiles: the cartridge collides against that sub-tile alone.
        let has_walkable = matches!(tile_indices[2], Some(tile_id) if self.collision_tiles.contains(&tile_id));

        if has_water {
            MetaTile::ConnectionWater(self.map)
        } else if has_walkable {
            let perp = self.to_strip_start.saturating_add(strip_idx as u8);
            let to_position = match self.direction {
                MapConnectionDirection::North | MapConnectionDirection::South =>
                    Point8 { x: perp, y: self.to_border_coord },
                MapConnectionDirection::East | MapConnectionDirection::West =>
                    Point8 { x: self.to_border_coord, y: perp },
            };
            MetaTile::Connection { to_map: self.map, to_position }
        } else {
            MetaTile::Obstacle
        }
    }

    fn is_water_tile(&self, tile_id: u8) -> bool {
        is_water_tile_id(tile_id, self.is_water_tileset, self.tileset)
    }
}

fn is_water_tile_id(tile_id: u8, is_water_tileset: bool, tileset: TileSetId) -> bool {
    const WATER: u8 = 0x14;
    const EASTERN_SHORE: u8 = 0x32;
    const SAFARI_ZONE_EASTERN_SHORE: u8 = 0x48;
    if !is_water_tileset {
        return false;
    }
    if tileset == TileSetId::Overworld
        && (tile_id == EASTERN_SHORE || tile_id == SAFARI_ZONE_EASTERN_SHORE) {
        return true;
    }
    tile_id == WATER
}

/// The `wCurMapHeader` block, tileset to connection flags.
const MAP_HEADER_BYTES: usize = 10;

/// `wCurMapTextPtr`, the one field the game rewrites while a map is loaded, so never compared.
const MAP_HEADER_TEXT_PTR: std::ops::Range<usize> = 5..7;

pub fn map_sprites_are_loaded(mmu: &impl DmgPointerRead) -> bool {
    // Slot 0 is the player and is not in `wNumSprites`; the loader fills 1..=15.
    let filled = (1..=0xFu16)
        .filter(|i| mmu.read_pointer(&(pokered_symbols::wSpriteDataStart + (i << 4))) != 0)
        .count();
    filled == mmu.read_pointer(&pokered_symbols::wNumSprites) as usize
}

const SURFING: u8 = 2;

/// The cartridge has finished loading the map `wCurMap` already names.
pub fn map_header_is_loaded(mmu: &impl DmgPointerRead, map: Map) -> bool {
    let Some(rom) = map.header_pointer() else { return true };
    let live = mmu.read_pointer_vec(&pokered_symbols::wCurMapHeader, MAP_HEADER_BYTES);
    let rom = &crate::pokemon::rom_gfx::rom_slice(rom)[..MAP_HEADER_BYTES];
    live.iter().zip(rom).enumerate()
        .all(|(i, (live, rom))| MAP_HEADER_TEXT_PTR.contains(&i) || live == rom)
}

pub trait MapMetadataReader {
    fn read_map_metadata(&self, map: Map) -> Result<MapMetadata, String>;

    fn read_current_map(&self) -> Result<CurrentMap, String>;
}

/// A cache for `read_map_metadata`, safe for ever because ROM data never changes.
#[derive(Default)]
pub struct MapMetadataCache(RefCell<HashMap<Map, Arc<MapMetadata>>>);

impl std::fmt::Debug for MapMetadataCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapMetadataCache({} entries)", self.0.borrow().len())
    }
}

impl MapMetadataCache {
    pub fn read_current_map(&self, mmu: &MMU) -> Result<CurrentMap, String> {
        let map = Map::from_repr(mmu.read_pointer(&pokered_symbols::wCurMap))
            .ok_or_else(|| "Invalid map number".to_string())?;
        let player_direction_raw = mmu.read_pointer(&pokered_symbols::wPlayerDirection);
        Ok(CurrentMap {
            sprites_loaded: map_sprites_are_loaded(mmu),
            sprites: mmu.read_sprites()?,
            metadata: if map_uses_runtime_blocks(map) {
                Arc::new(mmu.read_map_metadata_runtime(map)?)
            } else if crate::pokemon::tile_map::elevator_for(map).is_some() {
                Arc::new(with_live_exits(mmu, &*self.read_map(mmu, map)?))
            } else {
                self.read_map(mmu, map)?
            },
            player_position: Point8 {
                x: mmu.read_pointer(&pokered_symbols::wXCoord),
                y: mmu.read_pointer(&pokered_symbols::wYCoord),
            },
            player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
            grass_encounter_rate: mmu.read_pointer(&pokered_symbols::wGrassRate),
            closed_doors: closed_door_blocks(mmu, map),
            script_cancelled_warps: script_cancelled_warps(mmu, map),
            standing_on_warp: mmu.read_pointer(&pokered_symbols::wMovementFlags) & BIT_STANDING_ON_WARP != 0,
            card_key_locked: map_has_card_key_doors(map) && !mmu.read_bag().contains(&crate::pokemon::item::ItemId::CardKey),
            header_loaded: map_header_is_loaded(mmu, map),
            surfing: mmu.read_pointer(&pokered_symbols::wWalkBikeSurfState) == SURFING,
        })
    }

    pub fn read_map(&self, mmu: &MMU, map: Map) -> Result<Arc<MapMetadata>, String> {
        if let Some(arc) = self.0.borrow().get(&map) {
            return Ok(Arc::clone(arc));
        }
        let arc = Arc::new(mmu.read_map_metadata(map)?);
        self.0.borrow_mut().insert(map, Arc::clone(&arc));
        Ok(arc)
    }
}

impl MapMetadataReader for MMU {

    fn read_map_metadata(&self, map: Map) -> Result<MapMetadata, String> {
        let map_header = self.read_map_header(map)?;
        let map_data = self.rom_data_from_rom_pointer(&map_header.blocks_pointer(), map_header.height as usize * map_header.width as usize).to_vec();
        self.finish_map_metadata(map, map_header, map_data)
    }

    fn read_current_map(&self) -> Result<CurrentMap, String> {
        let map = Map::from_repr(self.read_pointer(&pokered_symbols::wCurMap))
            .ok_or_else(|| "Invalid map number".to_string())?;
        let player_direction_raw = self.read_pointer(&pokered_symbols::wPlayerDirection);

        Ok(
            CurrentMap {
                sprites_loaded: map_sprites_are_loaded(self),
                sprites: self.read_sprites()?,
                metadata: if map_uses_runtime_blocks(map) {
                    Arc::new(self.read_map_metadata_runtime(map)?)
                } else if crate::pokemon::tile_map::elevator_for(map).is_some() {
                    Arc::new(with_live_exits(self, &self.read_map_metadata(map)?))
                } else {
                    Arc::new(self.read_map_metadata(map)?)
                },
                player_position: Point8 {
                    x: self.read_pointer(&pokered_symbols::wXCoord),
                    y: self.read_pointer(&pokered_symbols::wYCoord),
                },
                player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                    .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
                grass_encounter_rate: self.read_pointer(&pokered_symbols::wGrassRate),
                closed_doors: closed_door_blocks(self, map),
                script_cancelled_warps: script_cancelled_warps(self, map),
                standing_on_warp: self.read_pointer(&pokered_symbols::wMovementFlags) & BIT_STANDING_ON_WARP != 0,
            card_key_locked: map_has_card_key_doors(map) && !self.read_bag().contains(&crate::pokemon::item::ItemId::CardKey),
                header_loaded: map_header_is_loaded(self, map),
                surfing: self.read_pointer(&pokered_symbols::wWalkBikeSurfState) == SURFING,
            }
        )
    }
}

/// Every map `ReplaceTileBlock` rewrites, built from the live `wOverworldMap` instead of ROM. A map
/// missing here is offered rows through closed doors, and no finished-game fixture can show it.
pub fn map_uses_runtime_blocks(map: Map) -> bool {
    matches!(map,
        Map::PokemonMansion1F | Map::PokemonMansion2F | Map::PokemonMansion3F
        | Map::PokemonMansionB1F | Map::CinnabarGym
        | Map::VictoryRoad1F | Map::VictoryRoad2F | Map::VictoryRoad3F
        | Map::LoreleisRoom | Map::BrunosRoom | Map::AgathasRoom | Map::LancesRoom | Map::ChampionsRoom
        | Map::VermilionGym)
}

/// The two halves of `MapMetadataReader`.
trait MapMetadataInternals {
    fn finish_map_metadata(&self, map: Map, map_header: MapHeader, map_data: Vec<u8>) -> Result<MapMetadata, String>;
    fn read_map_metadata_runtime(&self, map: Map) -> Result<MapMetadata, String>;
}

/// The raw ROM and RAM reads the two above are built out of.
trait MapRomReader {
    fn read_warp_events(&self, cur_map: Map, map_header: &MapHeader) -> Result<Vec<WarpEvent>, String>;
    fn read_tileset_header(&self, tileset: TileSetId) -> TilesetHeader;
    fn read_destination_warp_position(&self, dest_map: Map, dest_warp_id: u16) -> Result<Point8, String>;
    fn find_outdoor_entry_map(&self, indoor_map: Map, gate_warp_index: u16) -> Option<Map>;
    fn read_ledge_tiles(&self) -> HashMap<u8, JumpDirection>;
    fn read_tile_pair_collisions(&self, table: &crate::pokemon::symbols::DmgPointer, tileset: u8) -> Vec<(u8, u8)>;
    fn read_collision_tiles(&self, ptr: u16) -> HashSet<u8>;
    fn load_connected_strips(&self, map_header: &MapHeader) -> Vec<ConnectedMapStrip>;
    fn read_sprites(&self) -> Result<Vec<Sprite>, String>;
}

impl MapMetadataInternals for MMU {
    /// Shared tail of construction, given a block map from ROM or from `wOverworldMap`.
    fn finish_map_metadata(&self, map: Map, map_header: MapHeader, map_data: Vec<u8>) -> Result<MapMetadata, String> {
        let ts = self.read_tileset_header(map_header.tileset);
        let collision_tiles = self.read_collision_tiles(ts.coll_ptr);
        // Cover every referenced block and any runtime door block.
        let max_block_id = (*map_data.iter().max().unwrap() as usize).max(max_door_block_id(map));
        let tileset_data = self.rom_data_from_pointer(ts.bank, ts.blocks_ptr, (max_block_id + 1) * MapMetadata::BLOCK_TILES).to_vec();

        let warp_events = self.read_warp_events(map, &map_header)?;
        let tileset_id = map_header.tileset as u8;
        let water_tilesets = self.rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16);
        let is_water_tileset = water_tilesets.iter().take_while(|&&b| b != 0xFF).any(|&b| b == tileset_id);
        let connected_strips = self.load_connected_strips(&map_header);
        let ledge_tiles = if map_header.tileset == TileSetId::Overworld {
            self.read_ledge_tiles()
        } else {
            HashMap::new()
        };
        let tile_pair_collisions =
            self.read_tile_pair_collisions(&pokered_symbols::TilePairCollisionsLand, tileset_id);
        let tile_pair_collisions_water =
            self.read_tile_pair_collisions(&pokered_symbols::TilePairCollisionsWater, tileset_id);

        let mut metadata = MapMetadata {
            map, map_header, map_data, tileset_data, collision_tiles,
            talking_over_tiles: ts.talking_over_tiles, warp_events, is_water_tileset,
            grass_tile_id: ts.grass_tile, connected_strips, ledge_tiles, tile_pair_collisions,
            tile_pair_collisions_water,
            meta_tiles_base: Vec::new(), raw_tile_ids: Vec::new(),
        };
        metadata.meta_tiles_base = metadata.build_meta_tiles_base();
        metadata.raw_tile_ids = metadata.build_raw_tile_ids();
        Ok(metadata)
    }

    fn read_map_metadata_runtime(&self, map: Map) -> Result<MapMetadata, String> {
        const BORDER: usize = 3;
        let map_header = self.read_map_header(map)?;
        let (w, h) = (map_header.width as usize, map_header.height as usize);
        let stride = w + BORDER * 2;
        let base = pokered_symbols::wOverworldMap.address;
        let mut map_data = vec![0u8; w * h];
        for by in 0..h {
            for bx in 0..w {
                map_data[bx + by * w] = self.read(base + ((by + BORDER) * stride + (bx + BORDER)) as u16);
            }
        }
        self.finish_map_metadata(map, map_header, map_data)
    }
}

struct TilesetHeader {
    bank: usize,
    blocks_ptr: u16,
    coll_ptr: u16,
    talking_over_tiles: HashSet<u8>,
    grass_tile: u8,
}

impl MapRomReader for MMU {
    fn read_warp_events(&self, cur_map: Map, map_header: &MapHeader) -> Result<Vec<WarpEvent>, String> {
        // From ROM, because the runtime `wLastMap` resolution goes stale between indoor floors.
        let objects_pointer = map_header.objects_pointer();
        let warp_count = self.read_pointer(&(objects_pointer + 1)) as u16;
        let mut result = vec![];
        for index in 0..warp_count {
            let base = objects_pointer + (2 + index * 4);
            let entry = self.rom_data_from_rom_pointer(&base, 4);
            let raw_map_id = entry[3];
            let dest_warp_id = entry[2] as u16;
            let map_id = if raw_map_id == 0xFF {
                // `LAST_MAP`: the outdoor map whose warp table points back here.
                match self.find_outdoor_entry_map(cur_map, index) {
                    Some(m) => m,
                    None => continue,
                }
            } else if raw_map_id == cur_map as u8 {
                // A teleporter within the map, as in Saffron Gym's maze.
                cur_map
            } else {
                Map::from_repr(raw_map_id)
                    .ok_or_else(|| format!("Invalid map number {raw_map_id}"))?
            };
            // An elevator's exits point at a headerless placeholder, redirected once a floor is picked.
            let destination_position = if map_id.header_pointer().is_some() {
                self.read_destination_warp_position(map_id, dest_warp_id)
                    .unwrap_or(Point8 { y: 0, x: 0 })
            } else {
                Point8 { y: 0, x: 0 }
            };
            result.push(WarpEvent {
                position: Point8 { y: entry[0], x: entry[1] },
                destination_map: map_id,
                destination_position,
            });
        }
        Ok(result)
    }

    fn read_tileset_header(&self, tileset: TileSetId) -> TilesetHeader {
        const TILESET_ENTRY_SIZE: u16 = 12;
        let entry = pokered_symbols::Tilesets + tileset as u16 * TILESET_ENTRY_SIZE;
        let bank       = self.read_pointer(&entry) as usize;
        let blocks_ptr = self.read_pointer_u16_le(&(entry + 1));
        let coll_ptr   = self.read_pointer_u16_le(&(entry + 5));
        let talking_over_tiles = (0u16..3)
            .map(|i| self.read_pointer(&(entry + 7 + i)))
            .filter(|&b| b != 0xFF)
            .collect();
        let grass_tile = self.read_pointer(&(entry + 10));
        TilesetHeader { bank, blocks_ptr, coll_ptr, talking_over_tiles, grass_tile }
    }

    /// Where the player lands taking warp `dest_warp_id` into `dest_map`.
    fn read_destination_warp_position(&self, dest_map: Map, dest_warp_id: u16) -> Result<Point8, String> {
        let header = self.read_map_header(dest_map)?;
        let objects_pointer = header.objects_pointer();
        let warp_count = self.read_pointer(&(objects_pointer + 1)) as u16;
        if dest_warp_id >= warp_count {
            return Err(format!(
                "dest_warp_id {dest_warp_id} out of range (map {dest_map} has {warp_count} warps)"
            ));
        }
        let base = objects_pointer + (2 + dest_warp_id * 4);
        let dest_entry = self.rom_data_from_rom_pointer(&base, 4);
        Ok(Point8 { y: dest_entry[0], x: dest_entry[1] })
    }

    /// The outdoor map with a warp into `indoor_map`, preferring the one back to our warp.
    fn find_outdoor_entry_map(&self, indoor_map: Map, gate_warp_index: u16) -> Option<Map> {
        let map_banks = self.rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT);
        let mut fallback = None;
        for id in 0..Map::COUNT {
            let Some(outdoor_map) = Map::from_repr(id as u8) else { continue };
            if outdoor_map == indoor_map { continue; }
            let Ok(header) = self.read_map_header(outdoor_map) else { continue };
            if !matches!(header.tileset, TileSetId::Overworld | TileSetId::Plateau | TileSetId::Cavern) { continue; }
            let bank        = map_banks[id] as usize;
            let warp_count  = self.rom_data_from_pointer(bank, header.objects_address + 1, 1)[0] as u16;
            for wi in 0..warp_count {
                let entry = self.rom_data_from_pointer(bank, header.objects_address + 2 + wi * 4, 4);
                if entry[3] == indoor_map as u8 {
                    if entry[2] as u16 == gate_warp_index { return Some(outdoor_map); }
                    fallback.get_or_insert(outdoor_map);
                }
            }
        }
        fallback
    }

    fn read_ledge_tiles(&self) -> HashMap<u8, JumpDirection> {
        let data = self.rom_data_from_rom_pointer(&pokered_symbols::LedgeTiles, 64);
        let mut result = HashMap::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0xFF { break; }
            let tile_in_front   = data[i + 2];
            let dir_flags       = data[i + 3];
            let dir = match dir_flags {
                0x80 => JumpDirection::South,
                0x20 => JumpDirection::West,
                0x10 => JumpDirection::East,
                _    => { i += 4; continue; }
            };
            result.insert(tile_in_front, dir);
            i += 4;
        }
        result
    }

    /// The pairs of a tile-pair-collision table that apply to `tileset`.
    fn read_tile_pair_collisions(&self, table: &crate::pokemon::symbols::DmgPointer, tileset: u8) -> Vec<(u8, u8)> {
        let data = self.rom_data_from_rom_pointer(table, 64);
        let mut pairs = vec![];
        let mut i = 0;
        while i + 2 < data.len() {
            if data[i] == 0xFF { break; }
            if data[i] == tileset {
                pairs.push((data[i + 1], data[i + 2]));
            }
            i += 3;
        }
        pairs
    }

    /// The `$FF`-terminated walkable tile list at a bank-0 address.
    fn read_collision_tiles(&self, ptr: u16) -> HashSet<u8> {
        let mut tiles = HashSet::new();
        for index in 0..256u16 {
            let byte = self.read(ptr + index);
            if byte == 0xFF { break; }
            tiles.insert(byte);
        }
        tiles
    }

    fn load_connected_strips(&self, map_header: &MapHeader) -> Vec<ConnectedMapStrip> {
        let all_map_banks: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT)
            .to_vec();
        let water_tilesets: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16)
            .to_vec();

        map_header.connections()
            .into_iter()
            .filter_map(|connection| {
                let connected_map_bank = all_map_banks[connection.map as usize] as usize;
                let connected_header = self.read_map_header(connection.map).ok()?;

                let (border_blocks, block_sub_offset): (Vec<u8>, u8) = match connection.direction {
                    MapConnectionDirection::South => {
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            connection.strip_src,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 0)
                    }
                    MapConnectionDirection::North => {
                        // `strip_src` is the top of the 3-block-deep strip; the border row is its last.
                        let addr = connection.strip_src + 2 * connection.connected_map_width as u16;
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            addr,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 1)
                    }
                    MapConnectionDirection::East => {
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 0)
                    }
                    MapConnectionDirection::West => {
                        // `strip_src` is column width−3; the border column is width−1.
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16 + 2,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 1)
                    }
                };

                if border_blocks.is_empty() {
                    return None;
                }
                let max_block_id = *border_blocks.iter().max().unwrap() as usize;

                let tileset = connected_header.tileset;
                let ts = self.read_tileset_header(tileset);

                let tileset_data = self.rom_data_from_pointer(
                    ts.bank,
                    ts.blocks_ptr,
                    (max_block_id + 1) * MapMetadata::BLOCK_TILES,
                ).to_vec();

                let collision_tiles = self.read_collision_tiles(ts.coll_ptr);

                let tileset_id_byte = tileset as u8;
                let is_water_tileset = water_tilesets.iter()
                    .take_while(|&&b| b != 0xFF)
                    .any(|&b| b == tileset_id_byte);

                let meta_align_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (-(connection.x_alignment as i32)).max(0) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (-(connection.y_alignment as i32)).max(0) as usize,
                };

                let border_blocks_start_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (connection.x_alignment as i32 / 2).max(0).min(3) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (connection.y_alignment as i32 / 2).max(0).min(3) as usize,
                };

                let (to_border_coord, to_strip_start) = match connection.direction {
                    MapConnectionDirection::North =>
                        (connection.y_alignment as u8,
                         connection.x_alignment.max(0) as u8),
                    MapConnectionDirection::South =>
                        (0u8,
                         connection.x_alignment.max(0) as u8),
                    MapConnectionDirection::East =>
                        (0u8,
                         connection.y_alignment.max(0) as u8),
                    MapConnectionDirection::West =>
                        (connection.x_alignment as u8,
                         connection.y_alignment.max(0) as u8),
                };

                Some(ConnectedMapStrip {
                    direction: connection.direction,
                    map: connection.map,
                    border_blocks,
                    tileset_data,
                    collision_tiles,
                    is_water_tileset,
                    tileset,
                    block_sub_offset,
                    strip_length: connection.strip_length,
                    meta_align_offset,
                    to_border_coord,
                    to_strip_start,
                    border_blocks_start_offset,
                })
            })
            .collect()
    }

    fn read_sprites(&self) -> Result<Vec<Sprite>, String> {
        let map = Map::from_repr(self.read_pointer(&pokered_symbols::wCurMap)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_sprites = map.sprites();

        let toggleable_objects = self.read_pointer_vec(
            &pokered_symbols::wToggleableObjectFlags,
            (pokered_symbols::wToggleableObjectFlagsEnd.address - pokered_symbols::wToggleableObjectFlags.address) as usize
        );

        let mut sprites: Vec<Sprite> = Vec::new();
        for index in 1..=0xFu16 { // slot 0 is always the player
            let offset = index << 4;
            let picture_id = match PictureId::from_repr(self.read(pokered_symbols::wSpriteDataStart.address | offset)) {
                Some(picture_id) => picture_id,
                None => continue
            };
            let map_sprite = match map_sprites.get(index as usize - 1) {
                Some(map_sprite) => map_sprite,
                None => continue
            };

            let hidden = match map_sprite.hidden_object_id {
                Some(hidden_object_bit) => {
                    let mask = 1 << hidden_object_bit % 8;
                    (toggleable_objects[(hidden_object_bit / 8) as usize] & mask) == mask
                }
                None => false,
            };

            let sprite_image_index = self.read(pokered_symbols::wSpritePlayerStateData1ImageIndex.address | offset);
            // `SpriteFacing`, not `PlayerFacingDirection` — different byte, different encoding.
            let facing = SpriteFacing::from_repr(
                self.read(pokered_symbols::wSpritePlayerStateData1FacingDirection.address | offset)
            ).unwrap_or_default();

            let sprite = Sprite {
                index: index as u8,
                picture_id,
                position: if picture_id == PictureId::Red {
                    Point8 {
                        x: self.read_pointer(&pokered_symbols::wXCoord),
                        y: self.read_pointer(&pokered_symbols::wYCoord)
                    }
                } else {
                    Point8 {
                        // Wrapping: a sprite slot that has not been filled in yet reads below the
                        // 4-tile border, and the release build has always wrapped here.
                        x: self.read(pokered_symbols::wSpritePlayerStateData2MapX.address | offset).wrapping_sub(4),
                        y: self.read(pokered_symbols::wSpritePlayerStateData2MapY.address | offset).wrapping_sub(4)
                    }
                },
                on_screen: sprite_image_index != 0xFF,
                hidden,
                facing,
                name: map_sprite.name
            };
            sprites.push(sprite);
        }
        Ok(sprites)
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PlayerFacingDirection {
    #[default]
    Up = 8,
    Down = 4,
    Left = 2,
    Right = 1,
}

impl Into<JoypadButton> for PlayerFacingDirection {
    fn into(self) -> JoypadButton {
        match self {
            PlayerFacingDirection::Up => JoypadButton::Up,
            PlayerFacingDirection::Down => JoypadButton::Down,
            PlayerFacingDirection::Left => JoypadButton::Left,
            PlayerFacingDirection::Right => JoypadButton::Right,
        }
    }
}

pub struct CurrentMap {
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub sprites: Vec<Sprite>,
    pub metadata: Arc<MapMetadata>,
    /// Event-gated `ReplaceTileBlock` doors that are shut now.
    pub closed_doors: Vec<DoorBlock>,
    /// `wGrassRate`, out of 256; zero in every town, whose tall grass still looks ordinary.
    pub grass_encounter_rate: u8,
    /// On a Silph Co floor without the Card Key, whose door tiles are then walls.
    pub card_key_locked: bool,
    /// False mid-transition, while `wCurMap` names the map being entered and all else belongs to the
    /// one being left; lands in `MetaTileMap::position_settled`.
    pub header_loaded: bool,
    /// The cartridge branches on this before any collision, and a warp does not fire while surfing.
    pub surfing: bool,
    /// False while the sprite table is filling: an intra-map teleport reloads without changing
    /// `wCurMap`, so only this sees it.
    pub sprites_loaded: bool,
    /// `BIT_STANDING_ON_WARP`, set when a completed step lands on a warp entry.
    pub standing_on_warp: bool,
    /// Squares whose warp this map's script is cancelling, in raw coordinates.
    pub script_cancelled_warps: Vec<Point8>,
}

impl CurrentMap {
    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        let mut result = self.metadata.meta_tiles(&self.sprites);
        self.metadata.apply_door_blocks(&mut result, &self.closed_doors);
        // Elsewhere `$18` and `$24` are ordinary tiles.
        if map_has_card_key_doors(self.metadata.map) {
            self.metadata.apply_card_key_doors(&mut result, self.card_key_locked);
        }
        // Each of these is inert on every other map.
        self.metadata.apply_mansion_holes(&mut result);
        self.metadata.apply_victory_road_holes(&mut result);
        self.metadata.apply_seafoam_holes(&mut result);
        self.metadata.apply_seafoam_currents(&mut result);
        result
    }

    pub fn underfoot(&self) -> Vec<(Point8, MetaTile)> {
        self.metadata.underfoot(&self.sprites)
    }
}

#[cfg(test)]
mod test {
    use crate::pokemon::roms::POKERED;
    use crate::pokemon::tile_map::MetaTileMap;
    use super::*;

    /// The two shore ids belong to the overworld and nowhere else.
    #[test]
    fn a_shore_tile_id_is_only_a_shore_in_the_overworld() {
        const EASTERN_SHORE: u8 = 0x32;
        const SAFARI_SHORE: u8 = 0x48;
        const WATER: u8 = 0x14;

        for id in [EASTERN_SHORE, SAFARI_SHORE] {
            assert!(is_water_tile_id(id, true, TileSetId::Overworld),
                "{id:#x} is a shore in the tileset it comes from");
            for elsewhere in [TileSetId::Gym, TileSetId::Forest, TileSetId::ShipPort,
                              TileSetId::Cavern, TileSetId::Plateau, TileSetId::Facility] {
                assert!(!is_water_tile_id(id, true, elsewhere),
                    "{id:#x} is not water in {elsewhere:?}; it is whatever that tileset draws there");
            }
        }

        // `$14` is water in every water tileset: it is how every pool, lake and sea is found.
        for tileset in [TileSetId::Overworld, TileSetId::Gym, TileSetId::Cavern] {
            assert!(is_water_tile_id(WATER, true, tileset), "{tileset:?} keeps its real water");
        }
        assert!(!is_water_tile_id(WATER, false, TileSetId::House));
    }

    /// Warp destinations match the disassembly's map objects.
    #[test]
    fn test_warp_event_destination_position() {

        let mmu = MMU::from_rom(POKERED).unwrap();

        let pt_header = mmu.read_map_header(Map::PalletTown).unwrap();
        let pt_warps  = mmu.read_warp_events(Map::PalletTown, &pt_header).unwrap();
        assert_eq!(pt_warps.len(), 3);

        assert_eq!(pt_warps[0].position,             Point8 { y: 5, x: 5  });
        assert_eq!(pt_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(pt_warps[0].destination_position, Point8 { y: 7, x: 2  });

        assert_eq!(pt_warps[1].position,        Point8 { y: 5,  x: 13 });
        assert_eq!(pt_warps[1].destination_map, Map::BluesHouse);

        assert_eq!(pt_warps[2].position,             Point8 { y: 11, x: 12 });
        assert_eq!(pt_warps[2].destination_map,      Map::OaksLab);
        assert_eq!(pt_warps[2].destination_position, Point8 { y: 11, x: 5  });

        let rh1_header = mmu.read_map_header(Map::RedsHouse1F).unwrap();
        let rh1_warps  = mmu.read_warp_events(Map::RedsHouse1F, &rh1_header).unwrap();
        assert_eq!(rh1_warps.len(), 3);

        // `LAST_MAP` exits resolve to Pallet Town.
        assert_eq!(rh1_warps[0].position,             Point8 { y: 7, x: 2 });
        assert_eq!(rh1_warps[0].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[0].destination_position, Point8 { y: 5, x: 5 });

        assert_eq!(rh1_warps[1].position,             Point8 { y: 7, x: 3 });
        assert_eq!(rh1_warps[1].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[1].destination_position, Point8 { y: 5, x: 5 });

        assert_eq!(rh1_warps[2].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh1_warps[2].destination_map,      Map::RedsHouse2F);
        assert_eq!(rh1_warps[2].destination_position, Point8 { y: 1, x: 7 });

        let rh2_header = mmu.read_map_header(Map::RedsHouse2F).unwrap();
        let rh2_warps  = mmu.read_warp_events(Map::RedsHouse2F, &rh2_header).unwrap();
        assert_eq!(rh2_warps.len(), 1);
        assert_eq!(rh2_warps[0].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh2_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(rh2_warps[0].destination_position, Point8 { y: 1, x: 7 });
    }

    /// A connection cell's `to_position` is the raw coordinate the cartridge writes on crossing.
    #[test]
    fn test_connection_tile_to_position() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        let pt_meta = mmu.read_map_metadata(Map::PalletTown).unwrap();
        let north_strip = pt_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route1)
            .expect("PalletTown should have a north strip to Route1");

        for i in 0..north_strip.strip_length as usize * 2 {
            let tile = north_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route1, "strip idx {i}: wrong map");
                assert_eq!(to_position.y, 35, "strip idx {i}: should land on raw bottom row of Route1 (y_align=35)");
                assert_eq!(to_position.x, i as u8, "strip idx {i}: x should equal strip index");
            }
        }

        let celadon_meta = mmu.read_map_metadata(Map::CeladonCity).unwrap();
        let east_strip = celadon_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route7)
            .expect("CeladonCity should have an east strip to Route7");

        for i in 0..east_strip.strip_length as usize * 2 {
            let tile = east_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route7, "strip idx {i}: wrong map");
                assert_eq!(to_position.x, 0, "strip idx {i}: should land on raw left column of Route7 (x_align=0)");
                assert_eq!(to_position.y, i as u8, "strip idx {i}: y should equal strip index");
            }
        }

        let west_strip = celadon_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route16)
            .expect("CeladonCity should have a west strip to Route16");

        for i in 0..west_strip.strip_length as usize * 2 {
            let tile = west_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route16, "strip idx {i}: wrong map");
                assert_eq!(to_position.x, 39, "strip idx {i}: should land on Route16 right column (20*2-1)");
                assert_eq!(to_position.y, i as u8, "strip idx {i}: y should equal strip index");
            }
        }
    }

    #[test]
    fn test_route2_connection_to_viridian_city() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let map = mmu.read_map_metadata(Map::Route2).unwrap();
        let current_map = CurrentMap {
            player_position: Point8 { x: 8, y: 72 },
            player_direction: PlayerFacingDirection::Up,
            sprites: vec![],
            metadata: Arc::new(map),
            closed_doors: vec![],
            grass_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        };
        let tile_map = MetaTileMap::new(&current_map);
        println!("{}", tile_map);

        tile_map.actions().into_iter()
            .find(|a| matches!(a.tile, MetaTile::Connection { to_map, .. } if to_map == Map::ViridianCity))
            .expect("Route2 should have a connection to ViridianCity");

    }

    /// A north strip is the connected map's bottom block row.
    #[test]
    fn test_north_strip_reads_the_connected_maps_border_row() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        // Strip length 13 against connected width 45, where a wrong stride diverges.
        let route3 = mmu.read_map_metadata(Map::Route3).unwrap();
        let strip = route3.connected_strips.iter()
            .find(|s| s.map == Map::Route4)
            .expect("Route3 should have a north strip to Route4");
        assert_eq!(strip.direction, MapConnectionDirection::North);
        assert_eq!(
            strip.border_blocks,
            vec![0x2c, 0x2c, 0x29, 0x01, 0x01, 0x01, 0x1a, 0x3e, 0x3f, 0x3f, 0x2c, 0x2c, 0x2c],
            "Route3's north strip must be Route4's bottom block row, not a row from its middle"
        );

        let pallet = mmu.read_map_metadata(Map::PalletTown).unwrap();
        let control = pallet.connected_strips.iter()
            .find(|s| s.map == Map::Route1)
            .expect("PalletTown should have a north strip to Route1");
        assert_eq!(
            control.border_blocks,
            vec![0x0a, 0x6e, 0x0a, 0x0a, 0x4d, 0x0b, 0x4e, 0x0a, 0x0a, 0x6d],
            "the equal-width case must be unchanged"
        );
    }

    /// A one-way ledge on the far side of a border is not a way out of this map.
    #[test]
    fn test_a_ledge_across_a_border_is_not_a_connection() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        let route3 = mmu.read_map_metadata(Map::Route3).unwrap();
        let strip = route3.connected_strips.iter()
            .find(|s| s.map == Map::Route4)
            .expect("Route3 should have a north strip to Route4");
        for i in 0..strip.strip_length as usize * 2 {
            let expected_connection = (6..=11).contains(&i);
            match strip.meta_tile_at(i) {
                MetaTile::Connection { to_map, to_position } => {
                    assert!(expected_connection, "strip idx {i}: a ledge or wall offered as a crossing");
                    assert_eq!(to_map, Map::Route4);
                    assert_eq!(to_position, Point8 { x: i as u8, y: 17 });
                }
                other => assert!(!expected_connection, "strip idx {i}: real crossing lost, got {other:?}"),
            }
        }

        // Cycling Road's south wall: among its south-only ledges, x = 10 is the one way up.
        let route18 = mmu.read_map_metadata(Map::Route18).unwrap();
        let cycling = route18.connected_strips.iter()
            .find(|s| s.map == Map::Route17)
            .expect("Route18 should have a north strip to Route17");
        for i in 0..cycling.strip_length as usize * 2 {
            let tile = cycling.meta_tile_at(i);
            if [6, 7, 8, 9, 11, 12, 13].contains(&i) {
                assert_eq!(tile, MetaTile::Obstacle, "strip idx {i}: Route17's ledge is not a crossing");
            } else if i == 10 {
                assert!(matches!(tile, MetaTile::Connection { to_map: Map::Route17, .. }),
                    "strip idx 10 is the only real way up onto Cycling Road, got {tile:?}");
            }
        }
    }

    /// Route 3's east corridor reaches Route 4 by walking west to a real crossing.
    #[test]
    fn test_route3_east_corridor_routes_west_to_reach_route4() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let metadata = mmu.read_map_metadata(Map::Route3).unwrap();
        let current_map = CurrentMap {
            player_position: Point8 { x: 63, y: 0 },
            player_direction: PlayerFacingDirection::Up,
            sprites: vec![],
            metadata: Arc::new(metadata),
            closed_doors: vec![],
            grass_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        };
        let tile_map = MetaTileMap::new(&current_map);
        println!("{tile_map}");

        let action = tile_map.actions().into_iter()
            .find(|a| matches!(a.tile, MetaTile::Connection { to_map, .. } if to_map == Map::Route4))
            .expect("Route3's east corridor should still have a way into Route4");

        assert_eq!(action.destination.y, 0, "the crossing is on the north border row");
        assert!(
            (58..=62).contains(&action.destination.x),
            "expected a real crossing at expanded x 58..=62, got {:?} — x 63/64 are Route4's ledge",
            action.destination
        );
        assert!(
            action.route.len() > 1,
            "reaching it means walking west first, not one press of Up: {:?}", action.route
        );
    }

}
