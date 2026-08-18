use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::bag::BagReader;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeader, MapHeaderReader, TileSetId};
use crate::pokemon::sprite::{PictureId, Sprite, SpriteFacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::{JumpDirection, MetaTile, WarpEvent};
use crate::ram::ROM;

#[derive(Clone)]
pub struct MapMetadata {
    pub map: Map,
    pub map_header: MapHeader,
    pub map_data: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    /// Tile IDs from `wTilesetTalkingOverTiles` — counters/desks the player can
    /// interact through by facing them and pressing A (pokered "talking over" mechanic).
    pub talking_over_tiles: HashSet<u8>,
    pub warp_events: Vec<WarpEvent>,
    /// True if the current tileset is listed in the WaterTilesets table,
    /// meaning tile $14/$32/$48 should be treated as water/shore.
    pub is_water_tileset: bool,
    /// Tile ID for tall grass in the current tileset (from `wGrassTile`).
    /// Zero means this map has no grass tile.
    pub grass_tile_id: u8,
    pub connected_strips: Vec<ConnectedMapStrip>,
    /// Maps ledge tile IDs to their jump direction. Only populated for the Overworld tileset,
    /// which is the only tileset where HandleLedges fires.
    pub ledge_tiles: HashMap<u8, JumpDirection>,
    /// Pairs of raw tile IDs that the player may not walk *between* in this tileset, even
    /// though both tiles are individually passable (pokered `TilePairCollisionsLand` filtered
    /// to the current tileset). Used to simulate elevation boundaries — e.g. in the Cavern
    /// tileset you cannot step between tile $20 and tile $05. The pair is unordered: a move is
    /// blocked if either (standing, front) or (front, standing) matches an entry.
    pub tile_pair_collisions: Vec<(u8, u8)>,
    /// The same, but from pokered `TilePairCollisionsWater` — the table the game checks whenever
    /// **water is involved**: mounting Surf (`UsedSurf` → `.tryToSurf`), stepping ashore again
    /// (`.tryToStopSurfing`), and every move made while surfing (`CollisionCheckOnWater`). In the
    /// Cavern tileset it holds `($14, $05)`, i.e. inside Seafoam Islands you may not surf on or off
    /// the water from a plain cave floor tile — only from the shore tiles ($15, …). Without this the
    /// BFS plans crossings the game refuses with "No SURFing here!".
    pub tile_pair_collisions_water: Vec<(u8, u8)>,
    /// Pre-computed tile grid without sprites. Tiles, warps, and connection strips are all
    /// ROM-derived and never change, so this is computed once at construction and cloned
    /// in `meta_tiles` before the sprite overlay is applied.
    pub meta_tiles_base: Vec<MetaTile>,
    /// Bottom-left raw tile ID of each expanded meta-tile (parallel to `meta_tiles_base`).
    /// This is the sub-tile pokered's collision check reads (`lda_coord 8,9` for the player's
    /// standing tile, and the bottom-left of the meta-tile in front for the destination).
    /// `0xFF` marks border/connection-strip cells that have no source-map tile. Needed to
    /// evaluate `tile_pair_collisions` during pathfinding.
    pub raw_tile_ids: Vec<u8>,
}

/// ⚠️ **Deliberately a summary, not `#[derive(Debug)]`.** This is reachable from `MetaTileMap`'s
/// `Debug`, which is what every failing `assert!` in the agent prints — and a derived impl would put
/// the whole block map, the whole blockset and every connection strip into that message. Four facts
/// identify a `MapMetadata`; the rest is a wall.
impl std::fmt::Debug for MapMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapMetadata({}, {}x{} blocks, tileset {:?})",
               self.map, self.map_header.width, self.map_header.height, self.map_header.tileset)
    }
}

impl MapMetadata {
    pub const BLOCK_TILE_WIDTH: usize = 4; // a block is 4x4 tiles
    pub const BLOCK_TILES: usize = Self::BLOCK_TILE_WIDTH * Self::BLOCK_TILE_WIDTH;
    pub const TILES_PER_META: usize = 2; // a meta tile on the map is 2x2 graphical tiles

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

    /// Returns true if this sub-tile is a water or shore tile.
    ///
    /// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`.
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
                // Warps are already in the base and take priority over sprites,
                // matching the original ordering where warps were applied after sprites.
                if !matches!(result[idx], MetaTile::Warp { .. }) {
                    result[idx] = MetaTile::Sprite(sprite.name);
                }
            }
        }

        result
    }

    pub fn build_meta_tiles_base(&self) -> Vec<MetaTile> {
        let dimensions = self.dimensions();

        // Connection strips expand the map by one meta-tile row/column per direction.
        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();

        let mut result = vec![MetaTile::Obstacle; exp_width * exp_height];

        // Fill current map tiles, shifted by the connection offsets.
        //
        // pokered collision checking reads the bottom-left raw tile of the destination 2×2
        // meta-tile in all four movement directions (GetTileAndCoordsInFrontOfPlayer uses
        // lda_coord(8,11), (8,7), (6,9), (10,9) — always the bottom-left of the target
        // meta-tile).  All classification checks therefore use only that sub-tile.
        //
        // Exception: water detection scans all four sub-tiles so that shore-transition blocks
        // (which mix passable and water tile IDs) are conservatively treated as Water.
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
                        // Bottom-left sub-tile: the one pokered actually checks.
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
                // Only register the warp if the position is physically accessible
                // (has at least one walkable raw sub-tile).  Warp positions with no
                // walkable sub-tiles are impassable in the game (e.g. the duplicate
                // exit tile at x=4 in ViridianForestSouthGate) and must stay as
                // Obstacle so the BFS does not route to them.
                if result[mx + my * exp_width] != MetaTile::Obstacle {
                    result[mx + my * exp_width] = warp.tile();
                }
            }
        }

        // Fill the extra border rows/columns from connected map strips.
        for (strip, strip_idx, mx, my) in self.strip_cells() {
            result[mx + my * exp_width] = strip.meta_tile_at(strip_idx);
        }

        result
    }

    /// Every connection-strip meta-tile and where it lands in the **expanded** grid, as
    /// `(strip, strip_idx, mx, my)`.
    ///
    /// Shared so that the classification in [`Self::build_meta_tiles_base`] and the *drawing* in
    /// [`crate::llm::map_image`] cannot place the same strip differently — the arithmetic is four
    /// sign-sensitive cases and the failure mode of getting one wrong is a border row that looks
    /// perfectly reasonable one tile out of true.
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

    /// Build the bottom-left raw tile ID grid parallel to `meta_tiles_base`.
    ///
    /// For each in-map meta-tile this is `tile_id(mx*2, my*2 + 1)` — the bottom-left sub-tile,
    /// the one pokered's collision and tile-pair checks read. Border/connection-strip cells are
    /// left as `0xFF` (they belong to an adjacent map and are never the player's standing tile).
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

    /// Overlay runtime `ReplaceTileBlock` door blocks onto an already-built `meta_tiles` grid.
    ///
    /// Several maps swap a single 4×4-tile map block between a "wall" block and a "floor" block at
    /// runtime, gated on event flags (pokered `*DoorCallbackScript`, e.g. RocketHideoutB1F's door
    /// opens once the guarding Rocket is beaten). The static ROM `map_data` always holds the *open*
    /// (floor) block, so BFS would route straight through a door that is actually shut. For every
    /// currently-closed door we reclassify its four meta-tiles from the *closed* block's tiles
    /// (bottom-left sub-tile, matching the normal collision read) so pathfinding avoids it.
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
                    // Bottom-left sub-tile of this meta-cell within the closed block.
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
    /// Card Key door tiles in the Facility tileset (pokered `card_key.asm`: `wTileInFrontOfPlayer`
    /// == $18 or $24). While locked they are walls; once the Card Key is held the game opens them on
    /// approach, so force them passable then (the static tileset marks them impassable).
    pub fn apply_card_key_doors(&self, result: &mut [MetaTile], locked: bool) {
        for (idx, &tile) in self.raw_tile_ids.iter().enumerate() {
            if tile == 0x18 || tile == 0x24 {
                result[idx] = if locked { MetaTile::Obstacle } else { MetaTile::Empty };
            }
        }
    }
}

impl MapMetadata {
    /// Pokémon Mansion 3F floor holes: stepping onto any of them drops the player to 1F, landing at
    /// (16,14) (pokered `PokemonMansion3F.asm` `.holeCoords` + `special_warps.asm` `DungeonWarpData`).
    /// Model them as inter-map warps so BFS routes 3F → hole → 1F automatically — the only way to reach
    /// 1F's right side, and thus the B1F staircase down to the Secret Key.
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

    /// Victory Road 3F has one floor hole at (23,15). Stepping on it drops the player to VR2F
    /// (`IsPlayerOnDungeonWarp` + `DungeonWarpData VICTORY_ROAD_2F` → the fly-warp landing (22,16)) —
    /// the only way onto VR2F's east side that reaches the Route 23 exit. Model it as a `Warp` so BFS
    /// routes 3F → hole → 2F automatically. (Pushing a boulder onto this same hole instead reveals a
    /// hidden VR2F boulder; that is handled separately by the boulder solver.)
    pub fn apply_victory_road_holes(&self, result: &mut [MetaTile]) {
        if self.map != Map::VictoryRoad3F { return; }
        let w = self.dimensions().full_width();
        let idx = 23 + 15 * w;
        if idx < result.len() {
            result[idx] = MetaTile::Warp { to_map: Map::VictoryRoad2F, to_position: Point8 { x: 22, y: 16 } };
        }
    }

    /// Seafoam Islands floor holes. Each floor has two (pokered `SeafoamIslands*.asm`
    /// `Seafoam{1,2,3,4}HolesCoords`); standing on one drops the player to the floor below, landing
    /// at the matching `DungeonWarpData` `fly_warp` (`data/maps/special_warps.asm`). Modelled as
    /// inter-map warps so BFS routes through them.
    ///
    /// The B3F pair matters most: it is the **only** way onto B4F's west lake, and hence the only way
    /// to Articuno. B4F's single shore tile (7,11) is refused by pokered's `IsSurfingAllowed`
    /// ("The current is much too fast!") until both boulders are down — but *falling* through a B3F
    /// hole lands the player at (4,14)/(5,14), which `ForcedBikeOrSurfMaps` puts straight into Surf,
    /// bypassing the gate entirely.
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

    /// Seafoam Islands B3F strong-current trap tile. Surfing onto (15,8) hands control to
    /// `SeafoamIslandsB3FDefaultScript`, which force-walks the player DOWN 6 / RIGHT 5 / DOWN 3 into
    /// the (20,17) warp and out onto B4F's *east* water — a region walled off from Articuno. The agent
    /// models no currents, so the tile is simply marked impassable and BFS routes around it.
    ///
    /// The trap is armed until the two **B2F** boulders are dropped
    /// (`CheckBothEventsSet EVENT_SEAFOAM3_BOULDER{1,2}_DOWN_HOLE; ret z` — Z, and so the early `ret`,
    /// means *both are down*). Those boulders start hidden behind the whole 1F→B1F→B2F chain, so on
    /// the Articuno route the trap is always live and this stays unconditional.
    ///
    /// The floors' two other currents are left unmodelled because the route never rides them: B3F's
    /// (18/19, 7) — armed on the same SEAFOAM3 condition — can only be reached by falling through a
    /// B2F hole, which is why there is no walking route back east (the agent looped B2F → B3F → B4F
    /// proving it); and B4F's (4,14)/(5,14) is armed on SEAFOAM4, which the route drops *before*
    /// falling onto those tiles.
    pub fn apply_seafoam_currents(&self, result: &mut [MetaTile]) {
        if self.map != Map::SeafoamIslandsB3F { return; }
        let w = self.dimensions().full_width();
        let idx = 15 + 8 * w;
        if idx < result.len() {
            result[idx] = MetaTile::Obstacle;
        }
    }
}

/// True on the Silph Co floors that have card-key doors.
pub fn map_has_card_key_doors(map: Map) -> bool {
    matches!(map,
        Map::SilphCo1F | Map::SilphCo2F | Map::SilphCo3F | Map::SilphCo4F | Map::SilphCo5F
        | Map::SilphCo6F | Map::SilphCo7F | Map::SilphCo8F | Map::SilphCo9F | Map::SilphCo10F
        | Map::SilphCo11F)
}

/// A runtime door block that is currently *closed* (a wall), to be overlaid on the static map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoorBlock {
    /// Block coordinates (in 4×4-tile blocks) passed to pokered `ReplaceTileBlock` as `lb bc, y, x`.
    pub block_x: u8,
    pub block_y: u8,
    /// The tileset block ID drawn when the door is shut (pokered `wNewTileBlockID`).
    pub block_id: u8,
}

/// Static description of an event-gated `ReplaceTileBlock` door for one map.
struct DoorSpec {
    block_x: u8,
    block_y: u8,
    /// Block ID drawn when shut.
    closed_block_id: u8,
    /// The door is *open* if ANY clause is fully satisfied; each clause is a set of
    /// `(wEventFlags byte offset, bit mask)` that must ALL be set.
    open_clauses: &'static [&'static [(u16, u8)]],
}

/// Event-gated door blocks per map (pokered `*DoorCallbackScript`).
fn map_door_specs(map: Map) -> &'static [DoorSpec] {
    match map {
        // Elevator door: shut ($54) until Rocket 5 (trainer 4) is beaten (EVENT_677 / TRAINER_4).
        Map::RocketHideoutB1F => &[DoorSpec {
            block_x: 12, block_y: 8, closed_block_id: 0x54,
            open_clauses: &[&[(206, 0x80)], &[(206, 0x20)]],
        }],
        // Giovanni-room door: shut ($2d) until both Rockets (trainers 0 & 1) are beaten.
        Map::RocketHideoutB4F => &[DoorSpec {
            block_x: 12, block_y: 5, closed_block_id: 0x2d,
            open_clauses: &[&[(212, 0x20)], &[(212, 0x04), (212, 0x08)]],
        }],
        _ => &[],
    }
}

/// Read the event flags and return the door blocks on `map` that are currently shut.
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

/// Largest block ID referenced by any door spec for `map` (so the tileset load covers it).
fn max_door_block_id(map: Map) -> usize {
    map_door_specs(map).iter().map(|d| d.closed_block_id as usize).max().unwrap_or(0)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MapDimensions {
    pub meta_height: usize,
    pub meta_width: usize,

    /// Extra meta-tile rows/columns added for each connected direction.
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

/// Loaded border strip from an adjacent connected map, used to expand the rendered tilemap
/// by one meta-tile row (N/S connections) or column (E/W connections).
#[derive(Clone)]
pub struct ConnectedMapStrip {
    pub direction: MapConnectionDirection,
    pub map: Map,
    /// Block IDs along the single-block-deep border row/column of the connected map.
    pub border_blocks: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    pub is_water_tileset: bool,
    pub tileset: TileSetId,
    /// Sub-position within each block:
    /// N/S: 0 = top meta-row (south connection), 1 = bottom meta-row (north connection).
    /// E/W: 0 = left meta-col (east connection),  1 = right meta-col (west connection).
    pub block_sub_offset: u8,
    pub strip_length: u8,
    /// Offset (in meta-tiles) along the perpendicular axis where the strip begins.
    /// x-offset for N/S connections, y-offset for E/W connections.
    pub meta_align_offset: usize,
    /// The coordinate in the connected map on the *fixed* axis (i.e. the border itself):
    ///   North → y = connected_height * 2 - 1  (bottom row of connected map)
    ///   South → y = 0                          (top row)
    ///   East  → x = 0                          (left column)
    ///   West  → x = connected_width  * 2 - 1  (right column)
    pub to_border_coord: u8,
    /// Where the strip begins along the *perpendicular* axis in the connected map (meta-tiles).
    /// Strip tile `i` (0-based meta-tile index) lands at perpendicular coord `to_strip_start + i`.
    ///   N/S → x start in connected map = max(0,  x_alignment)
    ///   E/W → y start in connected map = max(0,  y_alignment)
    pub to_strip_start: u8,
    /// Number of blocks to skip at the start of `border_blocks` before applying strip meta-tile
    /// index `i`.  Arises when the strip source pointer (`strip_src`) begins earlier in the
    /// connected map than the tile that aligns with the current map's edge.
    ///
    /// The overworld buffer has a 3-block margin on each side; the strip is placed at buffer
    /// column `_tgt = max(0, alignment + 3)`.  When `_tgt < 3` the strip data starts to the
    /// *left* of the current map's column 0, so the first `(3 - _tgt)` blocks in
    /// `border_blocks` are off-screen and must be skipped.
    ///   N/S: `max(0, min(3, x_alignment / 2))`
    ///   E/W: `max(0, min(3, y_alignment / 2))`
    pub border_blocks_start_offset: usize,
}

impl ConnectedMapStrip {
    /// The four graphical tile ids that draw the meta-tile at `strip_idx`, as **top-left,
    /// top-right, bottom-left, bottom-right** — what the strip *looks like*, as against
    /// [`Self::meta_tile_at`]'s answer to what it means.
    ///
    /// `None` for a strip position with no block behind it; a `None` element for a block whose
    /// blockset was truncated (`tileset_data` is sized to the blocks a map actually references).
    ///
    /// ⚠️ **The strip has its own [`Self::tileset`], and it is often not the bordered map's** —
    /// Route 23 is `Plateau` against Route 22's `Overworld`. Drawing these ids against the wrong
    /// sheet produces plausible rubble rather than an error.
    pub fn tile_ids_at(&self, strip_idx: usize) -> Option<[Option<u8>; 4]> {
        let block_idx = strip_idx / 2 + self.border_blocks_start_offset;
        let block_id = *self.border_blocks.get(block_idx)?;

        // For N/S: strip_idx selects left(0)/right(1) within each block; sub_offset is the row.
        // For E/W: strip_idx selects top(0)/bottom(1) within each block; sub_offset is the col.
        let (sub_col, sub_row) = match self.direction {
            MapConnectionDirection::North | MapConnectionDirection::South =>
                (strip_idx % 2, self.block_sub_offset as usize),
            MapConnectionDirection::East | MapConnectionDirection::West =>
                (self.block_sub_offset as usize, strip_idx % 2),
        };

        // Indices of the four graphical tiles that form this 2×2 meta-tile within the block.
        // tile_offset = tx_in_block + ty_in_block * BLOCK_TILE_WIDTH (4)
        let base = sub_col * 2 + sub_row * 8;
        let block_start = block_id as usize * MapMetadata::BLOCK_TILES;
        Some([base, base + 1, base + 4, base + 5]
            .map(|idx| self.tileset_data.get(block_start + idx).copied()))
    }

    /// Returns the MetaTile for position `strip_idx` (0..strip_length*2) in this border strip.
    fn meta_tile_at(&self, strip_idx: usize) -> MetaTile {

        let Some(tile_indices) = self.tile_ids_at(strip_idx) else { return MetaTile::Obstacle };

        let has_water = tile_indices.iter().flatten().any(|&tile_id| self.is_water_tile(tile_id));

        // ⚠️ **Bottom-left only, exactly as `build_meta_tiles_base` classifies in-map tiles** —
        // `GetTileAndCoordsInFrontOfPlayer` collides against that one sub-tile and no other. Asking
        // instead whether *any* of the four is passable lets a block's scenery vouch for a surface
        // that cannot be walked on, and a ledge id is never in a tileset's passable list, so the
        // one thing the loose test reliably let through was a one-way ledge on the far side of a
        // border. Route 4's bottom row at x=12,13 is `39 39 36 37`: grass above, south-only ledge
        // on the ground. Both read as crossings into Route 4, and since `actions()` offers only the
        // *nearest* crossing per map, a player standing under one is offered nothing else and can
        // never leave the map. A deployed run spent a day walking into it.
        //
        // Water keeps the loose test on purpose — see `is_water_tile_id`: shore blocks genuinely
        // mix water and land ids, and treating them as water is the conservative answer there.
        let has_walkable = matches!(tile_indices[2], Some(tile_id) if self.collision_tiles.contains(&tile_id));

        if has_water {
            MetaTile::ConnectionWater(self.map)
        } else if has_walkable {
            // Compute the exact tile in the connected map that this strip position leads to.
            // `strip_idx` is a meta-tile index along the strip (0 = first meta-tile).
            // The perpendicular coordinate in the connected map is `to_strip_start + strip_idx`.
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

/// Returns true if `tile_id` is a water or shore tile for the given tileset.
///
/// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`:
/// - Tile `$14` is the universal water tile.
/// - Tiles `$32` (eastern shore) and `$48` (Safari Zone shore) are shore tiles,
///   **unless** the tileset is `ShipPort` (Vermilion Dock), where `$32` is dock planks.
/// - Returns false for any tileset not listed in the `WaterTilesets` ROM table.
///
/// ⚠️ **`Forest` is excluded from the shore ids too, and the ROM does not do that.** The difference is
/// what the answer is *used for*: the ROM asks this about the one tile in front of the player when
/// Surf is used, and `IsSurfingAllowed` refuses anyway; here the answer classifies every sub-tile of
/// every block, and a false positive becomes a tile the BFS will happily route across. `WaterTilesets`
/// lists `FOREST`, whose only map is **Viridian Forest** — which has no water at all, and six
/// impassable bush blocks whose ids are `$32`/`$48`. One of them sits between the forest's south exit
/// and everything else, so the agent routed into it, tried to mount Surf on dry land, and the leg died
/// on "No SURFing on land!".
///
/// Passability is deliberately *not* the test here: real water is **not** in a tileset's land
/// collision list — `CollisionCheckOnWater` is a separate path — so requiring it drops every genuine
/// water tile in the game (measured: 12 surf/fishing legs fail).
fn is_water_tile_id(tile_id: u8, is_water_tileset: bool, tileset: TileSetId) -> bool {
    const WATER: u8 = 0x14;
    const EASTERN_SHORE: u8 = 0x32;
    const SAFARI_ZONE_EASTERN_SHORE: u8 = 0x48;
    if !is_water_tileset {
        return false;
    }
    let shore_ids_mean_shore = !matches!(tileset, TileSetId::ShipPort | TileSetId::Forest);
    if shore_ids_mean_shore && (tile_id == EASTERN_SHORE || tile_id == SAFARI_ZONE_EASTERN_SHORE) {
        return true;
    }
    tile_id == WATER
}

pub trait MapMetadataReader {
    fn read_map_metadata(&self, map: Map) -> Result<MapMetadata, String>;

    fn read_current_map(&self) -> Result<CurrentMap, String>;
}

/// Pokémon-layer cache for `read_map_metadata`. ROM data never changes during a session,
/// so results are deterministic per map and safe to cache indefinitely.
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
            metadata: if map_uses_runtime_blocks(map) {
                Arc::new(mmu.read_map_metadata_runtime(map)?)
            } else {
                self.read_map(mmu, map)?
            },
            player_position: Point8 {
                x: mmu.read_pointer(&pokered_symbols::wXCoord),
                y: mmu.read_pointer(&pokered_symbols::wYCoord),
            },
            player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
            sprites: mmu.read_sprites()?,
            grass_encounter_rate: mmu.read_pointer(&pokered_symbols::wGrassRate),
            closed_doors: closed_door_blocks(mmu, map),
            card_key_locked: map_has_card_key_doors(map) && !mmu.read_bag().contains(&crate::pokemon::item::ItemId::CardKey),
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
                metadata: if map_uses_runtime_blocks(map) {
                    Arc::new(self.read_map_metadata_runtime(map)?)
                } else {
                    Arc::new(self.read_map_metadata(map)?)
                },
                player_position: Point8 {
                    x: self.read_pointer(&pokered_symbols::wXCoord),
                    y: self.read_pointer(&pokered_symbols::wYCoord),
                },
                player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                    .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
                sprites: self.read_sprites()?,
                grass_encounter_rate: self.read_pointer(&pokered_symbols::wGrassRate),
                closed_doors: closed_door_blocks(self, map),
            card_key_locked: map_has_card_key_doors(map) && !self.read_bag().contains(&crate::pokemon::item::ItemId::CardKey),
            }
        )
    }
}

/// Maps whose walkable layout is rewritten at runtime by `ReplaceTileBlock` in a way the static ROM
/// blocks can't capture — Pokémon Mansion switch-gates and the Cinnabar Gym gate blocks. For these,
/// build metadata from the live `wOverworldMap` block buffer instead of ROM (see
/// `MMU::read_map_metadata_runtime`). Everything else stays on the cached ROM path.
pub fn map_uses_runtime_blocks(map: Map) -> bool {
    matches!(map,
        Map::PokemonMansion1F | Map::PokemonMansion2F | Map::PokemonMansion3F
        | Map::PokemonMansionB1F | Map::CinnabarGym
        // Victory Road: boulder-on-switch `ReplaceTileBlock`s open barriers to the up-ladders.
        | Map::VictoryRoad1F | Map::VictoryRoad2F | Map::VictoryRoad3F
        // Elite Four rooms: beating each member runs a `ReplaceTileBlock` that opens the door up to the
        // next room, so the tile map must reflect the live block state to route to the (now-open) exit.
        | Map::LoreleisRoom | Map::BrunosRoom | Map::AgathasRoom | Map::LancesRoom | Map::ChampionsRoom)
}

impl MMU {
    /// Shared tail of map-metadata construction, given the block map (`map_data`) from either the ROM
    /// (static) or `wOverworldMap` (runtime). Everything downstream (tile classification, meta-tiles)
    /// reads block IDs from `map_data`, so overriding it is all that's needed to reflect runtime tiles.
    fn finish_map_metadata(&self, map: Map, map_header: MapHeader, map_data: Vec<u8>) -> Result<MapMetadata, String> {
        let ts = self.read_tileset_header(map_header.tileset);
        let collision_tiles = self.read_collision_tiles(ts.coll_ptr);
        // Size the tileset read to cover every block actually referenced (ROM or runtime) plus any
        // runtime door block.
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

    /// Build map metadata from the CURRENT runtime block map (`wOverworldMap`) rather than the static
    /// ROM blocks, so switch-toggled gates (Pokémon Mansion) and gym gate blocks (Cinnabar Gym) are
    /// reflected in routing. `wOverworldMap` holds the map's blocks inside a 3-block border, stride
    /// `width + 6`; de-border it back to a plain `width × height` block map.
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

impl MMU {
    fn read_warp_events(&self, cur_map: Map, map_header: &MapHeader) -> Result<Vec<WarpEvent>, String> {
        // Read directly from ROM so we get the raw destination byte (including 0xFF / self-ref)
        // before pokémon Red's runtime wLastMap resolution, which becomes stale when
        // navigating between indoor floors (e.g. Red's House 1F → 2F → 1F).
        //
        // Object-data layout: [border_block(1), warp_count(1), warp_entries(4 each), ...]
        // Each warp entry is: [Y, X, dest_warp_id, dest_map_id]
        //   dest_warp_id is stored 0-indexed (the ASM source uses 1-based but subtracts 1 via
        //   the `warp_event` macro: `db \4 - 1`).
        let objects_pointer = map_header.objects_pointer();
        let warp_count = self.read_pointer(&(objects_pointer + 1)) as u16;
        let mut result = vec![];
        for index in 0..warp_count {
            let base = objects_pointer + (2 + index * 4);
            let entry = self.rom_data_from_rom_pointer(&base, 4);
            let raw_map_id = entry[3];
            let dest_warp_id = entry[2] as u16; // 0-indexed into destination map's warp table
            let map_id = if raw_map_id == 0xFF {
                // LAST_MAP building-exit: the true destination is the outdoor map whose warp table
                // points back here. Deep-interior maps you can only reach from *inside* the building
                // (e.g. Silph Co 11F — the elevator/pad-only Giovanni floor) have no such outdoor map,
                // so this can't be resolved statically. Skip the warp rather than failing the whole map
                // read (it's the building-exit warp, not needed for routing).
                match self.find_outdoor_entry_map(cur_map, index) {
                    Some(m) => m,
                    None => continue,
                }
            } else if raw_map_id == cur_map as u8 {
                // Self-referential warp = an INTERNAL teleporter (Saffron Gym's warp maze): the game
                // teleports you to warp #dest_warp_id on THIS same map. (Not a building exit — those use
                // LAST_MAP above.)
                cur_map
            } else {
                Map::from_repr(raw_map_id)
                    .ok_or_else(|| format!("Invalid map number {raw_map_id}"))?
            };
            // A warp can target a header-less placeholder map — e.g. the Silph Co / Rocket Hideout
            // elevators, whose exits point at UNUSED_MAP_ED and are redirected at runtime once the
            // player picks a floor. We can't compute a landing tile for those, but the warp itself
            // is real, so fall back to (0,0) rather than failing the whole map read (which would
            // freeze the agent the moment it stepped into the elevator).
            // A LAST_MAP / self-referential building-exit warp stores a dummy `dest_warp_id` (the game
            // ignores it and returns you via `wLastMap` at runtime), which can be out of range for the
            // resolved outdoor map's warp table (e.g. Saffron Gym's exit → SaffronCity index 22, but
            // Saffron has 8 warps). Don't fail the whole map read over it — fall back to (0,0); the exit
            // landing isn't needed for routing.
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

    /// Reads the ROM `Tilesets` table entry for `tileset` and returns the derived header fields.
    ///
    /// Tilesets entry layout (12 bytes each):
    ///   [0]     bank
    ///   [1..2]  blocks_ptr (LE)
    ///   [3..4]  gfx_ptr    (LE, unused)
    ///   [5..6]  coll_ptr   (LE)
    ///   [7..9]  3 counter/talking-over tile IDs (0xFF = unused)
    ///   [10]    grass tile ID (0xFF = no grass)
    ///   [11]    animation type (unused here)
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

    /// Returns the tile position (`Point8 { y, x }`) that the player lands on after taking a warp
    /// that targets `dest_map` at warp-table index `dest_warp_id` (0-indexed).
    ///
    /// The destination position is simply the `[Y, X]` of the `dest_warp_id`-th entry in
    /// `dest_map`'s object data warp table — the same value the game engine reads via
    /// `LoadDestinationWarpPosition` (home/overworld.asm) indexed by `wDestinationWarpID`.
    ///
    /// The `warp_event` ASM macro emits `\4 - 1` for the dest-warp field, so the raw ROM byte
    /// is already 0-indexed.  This function accepts it as-is.
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

    /// Scans every Overworld-tileset map in ROM for a warp tile whose destination map
    /// equals `indoor_map`.  Returns the first match — this is the outdoor map the
    /// player should be returned to when they step on a self-referential / LAST_MAP exit warp.
    /// The outdoor map a `LAST_MAP` (building-exit) warp returns to. `gate_warp_index` is the warp's
    /// index in THIS map's warp table: a GATE that connects two outdoor maps (e.g. Route 22 Gate joins
    /// Route 22 to the south and Route 23 to the north) has LAST_MAP warps on both edges, and each side
    /// must resolve to the *right* map. We disambiguate by matching the outdoor warp's `dest_warp_id`
    /// (which points back at a specific warp of this map) to `gate_warp_index`; a normal one-exit
    /// building has no such ambiguity, so fall back to any outdoor map whose warp points here.
    fn find_outdoor_entry_map(&self, indoor_map: Map, gate_warp_index: u16) -> Option<Map> {
        let map_banks = self.rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT);
        let mut fallback = None;
        for id in 0..Map::COUNT {
            let Some(outdoor_map) = Map::from_repr(id as u8) else { continue };
            if outdoor_map == indoor_map { continue; }
            let Ok(header) = self.read_map_header(outdoor_map) else { continue };
            // Consider any outdoor-style map that could be the return side. Gates join routes that may
            // use non-Overworld tilesets (Route 23 is PLATEAU, cave routes are CAVERN), so don't restrict
            // to Overworld — the "has a warp pointing back at us" check below is the real constraint.
            if !matches!(header.tileset, TileSetId::Overworld | TileSetId::Plateau | TileSetId::Cavern) { continue; }
            let bank        = map_banks[id] as usize;
            let warp_count  = self.rom_data_from_pointer(bank, header.objects_address + 1, 1)[0] as u16;
            for wi in 0..warp_count {
                let entry = self.rom_data_from_pointer(bank, header.objects_address + 2 + wi * 4, 4);
                if entry[3] == indoor_map as u8 {
                    // Exact match: this outdoor warp returns to *our* warp → the correct side of a gate.
                    if entry[2] as u16 == gate_warp_index { return Some(outdoor_map); }
                    fallback.get_or_insert(outdoor_map);
                }
            }
        }
        fallback
    }

    /// Reads the `LedgeTiles` ROM table and returns a map of tile_id → JumpDirection.
    ///
    /// The table has 4-byte entries terminated by 0xFF:
    ///   [facing_direction, tile_under_player, tile_in_front, jump_direction_flags]
    /// `tile_in_front` (byte 2) is the ledge tile ID; `jump_direction_flags` (byte 3) encodes
    /// the direction: 0x80=south, 0x20=west, 0x10=east (same bit layout as wPlayerMovingDirection).
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

    /// Reads a tile-pair-collision table (`TilePairCollisionsLand` / `…Water`) and returns the
    /// `(tile1, tile2)` pairs that apply to `tileset`. The table is 3-byte entries
    /// `[tileset, tile1, tile2]` terminated by `0xFF`. Mirrors `CheckForTilePairCollisions` in
    /// home/overworld.asm. The Land table governs walking; the Water table governs mounting,
    /// dismounting and moving while surfing.
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

    /// Reads an FF-terminated list of walkable tile IDs from a bank-0 ROM address.
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
        // Each entry in the Tilesets ROM table is 12 bytes:
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

                // Read the single block row/column that borders the current map.
                let (border_blocks, block_sub_offset): (Vec<u8>, u8) = match connection.direction {
                    MapConnectionDirection::South => {
                        // First block row of the connected map.
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            connection.strip_src,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 0)
                    }
                    MapConnectionDirection::North => {
                        // strip_src points to the start of the 3-block-deep strip
                        // (connected_height − 3 rows in). The border row is the 3rd row down —
                        // and a row is `connected_map_width` blocks long, as the E/W arms below
                        // already stride by.
                        //
                        // ⚠️ **Not `strip_length`.** The two are equal for most north connections,
                        // which is what made this survive: every case the older tests cover reads
                        // the same row either way. They differ for seven — worst of all Route 3 →
                        // Route 4, 13 against 45, where the strip came from the middle of Route 4
                        // and painted almost all of Route 3's north edge as a way out. Route 6 →
                        // Saffron City is the same bug with the opposite symptom: no crossing at all.
                        let addr = connection.strip_src + 2 * connection.connected_map_width as u16;
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            addr,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 1)
                    }
                    MapConnectionDirection::East => {
                        // strip_src points to column 0 of each row; stride by connected_map_width.
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
                        // strip_src points to column (width−3); border column is +2 (column width−1).
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

                // When `strip_src` points into the connected map at an offset > 0 (i.e. some blocks
                // to the left/above the current map's edge are included), the strip data starts
                // earlier than the tile that aligns with the current map's column/row 0.
                // The overworld buffer has a 3-block margin, so `_tgt = max(0, alignment+3)` blocks
                // of the strip precede the current map.  When _tgt < 3, the first (3-_tgt) blocks
                // in border_blocks are off-screen and must be skipped when looking up meta tiles.
                //   N/S: offset = max(0, min(3, x_alignment / 2))
                //   E/W: offset = max(0, min(3, y_alignment / 2))
                let border_blocks_start_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (connection.x_alignment as i32 / 2).max(0).min(3) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (connection.y_alignment as i32 / 2).max(0).min(3) as usize,
                };

                // `to_border_coord` is the fixed-axis coordinate in the connected map's **raw**
                // meta-tile space (wXCoord/wYCoord values set by the game engine after the
                // connection transition) where the player lands:
                //   North → y = y_alignment  (bottom row of the connected map)
                //   South → y = 0            (top row of the connected map)
                //   East  → x = 0            (left column of the connected map)
                //   West  → x = x_alignment  (right column of the connected map)
                //
                // `to_strip_start` is where the strip begins along the perpendicular axis in the
                // connected map's raw coordinate space:
                //   N/S   → x_start = max(0, x_alignment)
                //   E/W   → y_start = max(0, y_alignment)
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
        for index in 1..=0xFu16 { // do not read index=0 as it is always the player
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
            // ⚠️ `SpriteFacing`, not `PlayerFacingDirection` — different byte, different encoding.
            // An unrecognised value means the slot is mid-update; down is what the game draws.
            let facing = SpriteFacing::from_repr(
                self.read(pokered_symbols::wSpritePlayerStateData1FacingDirection.address | offset)
            ).unwrap_or_default();

            let sprite = Sprite {
                index: index as u8,
                picture_id,
                position: if picture_id == PictureId::Red {
                    // Read player position from the map state
                    Point8 {
                        x: self.read_pointer(&pokered_symbols::wXCoord),
                        y: self.read_pointer(&pokered_symbols::wYCoord)
                    }
                } else {
                    Point8 {
                        x: self.read(pokered_symbols::wSpritePlayerStateData2MapX.address | offset) - 4,
                        y: self.read(pokered_symbols::wSpritePlayerStateData2MapY.address | offset) - 4
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
    /// Runtime `ReplaceTileBlock` door blocks that are currently shut (event-gated). Empty for
    /// maps with no such doors. Applied over the static map so BFS avoids closed doorways.
    pub closed_doors: Vec<DoorBlock>,
    /// `wGrassRate` — the chance, out of 256, that a step in this map's tall grass rolls a wild
    /// encounter. **Zero means there are none at all**, and that is not a rare edge: every town and
    /// city in the game points at `NothingWildMons` (`data/wild/grass_water.asm`), while still
    /// drawing perfectly ordinary tall grass.
    ///
    /// ⚠️ **Grass you can see and grass that holds Pokémon are two different facts.** `wGrassTile`
    /// is a *tileset* property, so Pallet Town's grass is literally the same tile as Route 1's and
    /// reads as [`MetaTile::Grass`] either way; this is the *map* property that says whether
    /// standing in it can ever do anything. `TryDoWildEncounter` compares a random byte against this
    /// with `jr nc, .CantEncounter2`, so a rate of zero never fires — it is impossible, not unlikely.
    pub grass_encounter_rate: u8,
    /// True on Silph Co floors while the **Card Key** is not yet in the bag: the card-key door tiles
    /// ($18/$24) are then impassable walls (the game refuses to open them without the key), so BFS
    /// must route around them. Once the key is held they open on approach and become passable.
    pub card_key_locked: bool,
}

impl CurrentMap {
    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        let mut result = self.metadata.meta_tiles(&self.sprites);
        self.metadata.apply_door_blocks(&mut result, &self.closed_doors);
        // Only on Silph Co floors: elsewhere tiles $18/$24 are ordinary tiles, not card-key doors.
        if map_has_card_key_doors(self.metadata.map) {
            self.metadata.apply_card_key_doors(&mut result, self.card_key_locked);
        }
        // Pokémon Mansion 3F floor holes → warp down to 1F (inert on every other map).
        self.metadata.apply_mansion_holes(&mut result);
        // Victory Road 3F floor hole → fall down to 2F (inert on every other map).
        self.metadata.apply_victory_road_holes(&mut result);
        // Seafoam Islands floor holes → fall to the floor below (inert on every other map).
        self.metadata.apply_seafoam_holes(&mut result);
        // Seafoam Islands B3F strong-current trap tile → impassable (inert on every other map).
        self.metadata.apply_seafoam_currents(&mut result);
        result
    }
}

#[cfg(test)]
mod test {
    use crate::pokemon::roms::POKERED;
    use crate::pokemon::tile_map::MetaTileMap;
    use super::*;

    /// Verifies that `WarpEvent::destination_position` is resolved correctly from ROM data by
    /// cross-checking with known map objects in the pokered disassembly.
    ///
    /// Key: the `warp_event` ASM macro takes args `(x, y, map, warp_id)` and emits `db y, x, …`,
    /// so the ROM byte order is [Y, X, dest_warp_id, dest_map].  All expected values are derived
    /// directly from pokered/data/maps/objects/*.asm.
    ///
    /// Data sources:
    ///   PalletTown.asm:
    ///     warp_event  5,  5, REDS_HOUSE_1F, 1  →  x=5,  y=5  → ROM [Y=5,  X=5,  dest_warp=0]
    ///     warp_event 13,  5, BLUES_HOUSE,   1  →  x=13, y=5  → ROM [Y=5,  X=13, dest_warp=0]
    ///     warp_event 12, 11, OAKS_LAB,      2  →  x=12, y=11 → ROM [Y=11, X=12, dest_warp=1]
    ///   RedsHouse1F.asm:
    ///     warp_event  2,  7, LAST_MAP,       1  → x=2, y=7 → ROM [Y=7, X=2, dest_warp=0]
    ///     warp_event  3,  7, LAST_MAP,       1  → x=3, y=7 → ROM [Y=7, X=3, dest_warp=0]
    ///     warp_event  7,  1, REDS_HOUSE_2F,  1  → x=7, y=1 → ROM [Y=1, X=7, dest_warp=0]
    ///   RedsHouse2F.asm:
    ///     warp_event  7,  1, REDS_HOUSE_1F,  3  → x=7, y=1 → ROM [Y=1, X=7, dest_warp=2]
    ///   OaksLab.asm:
    ///     warp_event  4, 11, LAST_MAP, 3  → x=4,  y=11 → ROM [Y=11, X=4, dest_warp=2]
    ///     warp_event  5, 11, LAST_MAP, 3  → x=5,  y=11 → ROM [Y=11, X=5, dest_warp=2]  ← index 1
    #[test]
    fn test_warp_event_destination_position() {

        let mmu = MMU::from_rom(POKERED).unwrap();

        // ── Pallet Town ──────────────────────────────────────────────────────────
        use crate::pokemon::symbols::pokered_symbols;
        let pt_header = mmu.read_map_header(Map::PalletTown).unwrap();
        let pt_warps  = mmu.read_warp_events(Map::PalletTown, &pt_header).unwrap();
        assert_eq!(pt_warps.len(), 3);

        // warp 0: x=5, y=5 → source tile (y=5, x=5); dest=RedsHouse1F[0]
        // RedsHouse1F warp[0] = warp_event 2, 7, … → ROM [Y=7, X=2] → {y:7, x:2}
        assert_eq!(pt_warps[0].position,             Point8 { y: 5, x: 5  });
        assert_eq!(pt_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(pt_warps[0].destination_position, Point8 { y: 7, x: 2  });

        // warp 1: x=13, y=5 → source tile (y=5, x=13); dest=BluesHouse[0]
        assert_eq!(pt_warps[1].position,        Point8 { y: 5,  x: 13 });
        assert_eq!(pt_warps[1].destination_map, Map::BluesHouse);

        // warp 2: x=12, y=11 → source tile (y=11, x=12); dest=OaksLab[1]
        // OaksLab warp[1] = warp_event 5, 11, … → ROM [Y=11, X=5] → {y:11, x:5}
        assert_eq!(pt_warps[2].position,             Point8 { y: 11, x: 12 });
        assert_eq!(pt_warps[2].destination_map,      Map::OaksLab);
        assert_eq!(pt_warps[2].destination_position, Point8 { y: 11, x: 5  });

        // ── Red's House 1F ────────────────────────────────────────────────────────
        let rh1_header = mmu.read_map_header(Map::RedsHouse1F).unwrap();
        let rh1_warps  = mmu.read_warp_events(Map::RedsHouse1F, &rh1_header).unwrap();
        assert_eq!(rh1_warps.len(), 3);

        // warps 0 & 1: LAST_MAP exits → PalletTown[0]
        // PalletTown warp[0] = warp_event 5, 5, … → ROM [Y=5, X=5] → {y:5, x:5}
        assert_eq!(rh1_warps[0].position,             Point8 { y: 7, x: 2 });
        assert_eq!(rh1_warps[0].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[0].destination_position, Point8 { y: 5, x: 5 });

        assert_eq!(rh1_warps[1].position,             Point8 { y: 7, x: 3 });
        assert_eq!(rh1_warps[1].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[1].destination_position, Point8 { y: 5, x: 5 });

        // warp 2: x=7, y=1 → source tile (y=1, x=7); dest=RedsHouse2F[0]
        // RedsHouse2F warp[0] = warp_event 7, 1, … → ROM [Y=1, X=7] → {y:1, x:7}
        assert_eq!(rh1_warps[2].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh1_warps[2].destination_map,      Map::RedsHouse2F);
        assert_eq!(rh1_warps[2].destination_position, Point8 { y: 1, x: 7 });

        // ── Red's House 2F ────────────────────────────────────────────────────────
        // warp_event 7, 1, REDS_HOUSE_1F, 3 → stored dest_warp_id=2
        // RedsHouse1F[2] = warp_event 7, 1, REDS_HOUSE_2F, 1 → ROM [Y=1, X=7] → {y:1, x:7}
        let rh2_header = mmu.read_map_header(Map::RedsHouse2F).unwrap();
        let rh2_warps  = mmu.read_warp_events(Map::RedsHouse2F, &rh2_header).unwrap();
        assert_eq!(rh2_warps.len(), 1);
        assert_eq!(rh2_warps[0].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh2_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(rh2_warps[0].destination_position, Point8 { y: 1, x: 7 });
    }

    /// Verifies that every `MetaTile::Connection` tile in a strip carries the correct
    /// `to_position` — i.e. the **raw** wXCoord/wYCoord value the game engine writes
    /// after the connection transition (before `MetaTileMap::new()` adds any extras).
    ///
    /// PalletTown north → Route1 (10×18 blocks, x_alignment=0, y_alignment=35):
    ///   raw wYCoord after transition = 35  (bottom row of Route1)
    ///   strip_idx 0 → connected x=0, y=35
    ///   strip_idx 19 → connected x=19, y=35
    ///
    /// CeladonCity east → Route7 (10×9 blocks, y_alignment=-8, x_alignment=0):
    ///   raw wXCoord after transition = 0  (left column of Route7)
    ///   strip_idx 0 → connected x=0, y=0
    ///   strip_idx 17 → connected x=0, y=17
    ///
    /// CeladonCity west → Route16 (20×9 blocks, y_alignment=-8, x_alignment=39):
    ///   raw wXCoord after transition = 39  (right column of Route16)
    ///   strip_idx 0 → connected x=39, y=0
    #[test]
    fn test_connection_tile_to_position() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        // ── PalletTown north → Route1 ─────────────────────────────────────────
        // y_alignment=35 → raw y = 35 (bottom row of Route1 in wYCoord space).
        // x_alignment=0; no extras → x = strip_idx.
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

        // ── CeladonCity east → Route7 ─────────────────────────────────────────
        // x_alignment=0 → raw x = 0 (left column of Route7 in wXCoord space).
        // y_alignment=-8 (.max(0)=0) → y = strip_idx.
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

        // ── CeladonCity west → Route16 ────────────────────────────────────────
        // x_alignment=39 → raw x = 39 (right column of Route16 in wXCoord space).
        // Route16 has no west connection so west_extra=0; value unchanged.
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
    fn dump_route1_viridian_connection_data() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let r1 = mmu.read_map_header(Map::Route1).unwrap();
        println!("Route1: height={} width={}", r1.height, r1.width);
        if let Some(c) = r1.north_connection {
            println!("  Route1 north→{:?}: y_align={} x_align={} strip_len={} connected_w={}", c.map, c.y_alignment, c.x_alignment, c.strip_length, c.connected_map_width);
        }
        if let Some(c) = r1.south_connection {
            println!("  Route1 south→{:?}: y_align={} x_align={} strip_len={} connected_w={}", c.map, c.y_alignment, c.x_alignment, c.strip_length, c.connected_map_width);
        }
        let vc = mmu.read_map_header(Map::ViridianCity).unwrap();
        println!("ViridianCity: height={} width={}", vc.height, vc.width);
        if let Some(c) = vc.north_connection {
            println!("  ViridianCity north→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
        }
        if let Some(c) = vc.south_connection {
            println!("  ViridianCity south→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
        }
        let r2 = mmu.read_map_header(Map::Route2).unwrap();
        println!("Route2: height={} width={}", r2.height, r2.width);
        if let Some(c) = r2.south_connection {
            println!("  Route2 south→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
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
        };
        let tile_map = MetaTileMap::new(&current_map);
        println!("{}", tile_map);

        tile_map.actions().into_iter()
            .find(|a| matches!(a.tile, MetaTile::Connection { to_map, .. } if to_map == Map::ViridianCity))
            .expect("Route2 should have a connection to ViridianCity");

    }

    /// The block row a **north** strip is read from.
    ///
    /// `strip_src` points at block row `connected_height − 3`, column `_src`, of the connected map;
    /// the row that actually borders this map is two rows further down, and a row is
    /// `connected_map_width` blocks long — **not** `strip_length` blocks. The two are equal for most
    /// north connections, which is why every case the older tests cover passes either way. They
    /// differ for seven of them, Route 3 → Route 4 worst of all (13 against 45): the strip was read
    /// from the middle of Route 4 and painted almost all of Route 3's north edge as a crossing,
    /// including the one-way ledge that a deployed run then walked into for a day.
    #[test]
    fn test_north_strip_reads_the_connected_maps_border_row() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        // Route3 → Route4: strip_length 13, connected width 45 — the case that diverges.
        // pokered/maps/Route4.blk, row 8 (of 9), columns 0..12.
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

        // Control: PalletTown → Route1 has strip_length == connected width (10), so it reads the
        // same row under either stride. pokered/maps/Route1.blk, row 17 (of 18).
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
    ///
    /// Strip cells are classified from the **bottom-left** sub-tile of the 2×2, the one
    /// `GetTileAndCoordsInFrontOfPlayer` reads — see the note in `build_meta_tiles_base`. Route 4's
    /// bottom row at x = 12,13 is `39 39 36 37`: grass on the block's top half, a south-only ledge on
    /// the surface actually walked on. Classifying on "any of the four sub-tiles is passable" made
    /// those read as crossings into Route 4, and because `actions()` only ever offers the *nearest*
    /// crossing per map, a player standing under one is offered nothing else and can never leave.
    #[test]
    fn test_a_ledge_across_a_border_is_not_a_connection() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        // ── Route3 north → Route4 ─────────────────────────────────────────────
        // 13 blocks = 26 meta-tiles, cell i landing on Route4 (i, 17).
        // Passable there only at x = 6..=11; x = 12,13 are south ledges and x = 14 a west ledge.
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

        // ── Route18 north → Route17 ───────────────────────────────────────────
        // Cycling Road's south wall: seven of the twenty cells are south-only ledges, and only
        // x = 10 is a genuine way up. The rest is sea, which stays `ConnectionWater` — the water
        // test deliberately still scans all four sub-tiles (see `is_water_tile_id`).
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

    /// The deployed wedge, reproduced without the emulator.
    ///
    /// A run stood at raw (63, 0) — the top row of Route 3's eastern corridor, reached by hopping
    /// down Route 4's one-way ledge — and was offered exactly one way into Route 4: the ledge
    /// directly overhead, one button press away. It held Up into it for 60 s, gave up, was offered
    /// the same thing again, and did that for a day. The real crossings are three to seven steps
    /// west, at expanded x = 58..=62.
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


